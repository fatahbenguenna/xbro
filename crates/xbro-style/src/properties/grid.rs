//! CSS Grid properties: track lists (`grid-template-columns` / `-rows` and
//! the `grid-template` / `grid` shorthands) and item placement
//! (`grid-{row,column}-{start,end}`, `grid-row`, `grid-column`, `grid-area`).
//!
//! Approximations: line names are validated loosely and dropped, named
//! lines in placements compute to `auto`, `grid-template-areas` strings are
//! not validated (only their row count is used), `subgrid` and `masonry`
//! compute to `none`, and track lists stop at [`MAX_GRID_TRACKS`] tracks.

use std::sync::Arc;

use xbro_css::values::split_commas;
use xbro_css::{ComponentValue as CV, Token};

use super::{push, LonghandId, Out, Spec, SpecGridTemplate, SpecTrack};
use crate::computed::{
    AutoRepeat, GridLine, GridTemplate, GridTrack, LengthPercentage, MAX_GRID_LINE, MAX_GRID_TRACKS,
};
use crate::values::{integer, length, LinearLen, MathExpr, ParseCtx, Resolver, NON_NEG_LP};

fn is_line_names(v: &CV) -> bool {
    matches!(v, CV::Block { open: '[', .. })
}

/// The significant items of a function argument.
fn significant(values: &[CV]) -> Vec<&CV> {
    values.iter().filter(|v| !v.is_whitespace()).collect()
}

/// `<track-breadth>` (or `<inflexible-breadth>` when `flex` is false).
fn breadth(v: &CV, flex: bool, cx: &ParseCtx) -> Option<SpecTrack> {
    if let Some(k) = v.ident() {
        return match k.to_ascii_lowercase().as_str() {
            "auto" => Some(SpecTrack::Auto),
            "min-content" | "-webkit-min-content" => Some(SpecTrack::MinContent),
            "max-content" | "-webkit-max-content" => Some(SpecTrack::MaxContent),
            _ => None,
        };
    }
    if let CV::Token(Token::Dimension { value, unit }) = v {
        if unit.eq_ignore_ascii_case("fr") {
            return (flex && *value >= 0.0 && value.is_finite()).then_some(SpecTrack::Fr(*value));
        }
    }
    length(v, NON_NEG_LP, cx).map(SpecTrack::Length)
}

/// `<track-size>`: a breadth, `minmax()` or `fit-content()`.
fn track_size(v: &CV, cx: &ParseCtx) -> Option<SpecTrack> {
    match v {
        CV::Function { name, args } if name.eq_ignore_ascii_case("minmax") => {
            let parts = split_commas(args);
            let [min, max] = parts.as_slice() else {
                return None;
            };
            let (min, max) = match (significant(min).as_slice(), significant(max).as_slice()) {
                ([min], [max]) => (breadth(min, false, cx)?, breadth(max, true, cx)?),
                _ => return None,
            };
            Some(SpecTrack::MinMax(Box::new(min), Box::new(max)))
        }
        CV::Function { name, args } if name.eq_ignore_ascii_case("fit-content") => {
            match significant(args).as_slice() {
                [arg] => length(arg, NON_NEG_LP, cx).map(SpecTrack::FitContent),
                _ => None,
            }
        }
        _ => breadth(v, true, cx),
    }
}

/// `<fixed-size>`: the only tracks allowed in and next to an auto-repeat.
fn is_fixed(t: &SpecTrack) -> bool {
    match t {
        SpecTrack::Length(_) => true,
        SpecTrack::MinMax(min, max) => {
            matches!(**min, SpecTrack::Length(_)) || matches!(**max, SpecTrack::Length(_))
        }
        _ => false,
    }
}

/// `[ <line-names>? <track-size> ]+ <line-names>?` (the tracks of a `repeat()`).
fn repeated_tracks(items: &[&CV], cx: &ParseCtx) -> Option<Vec<SpecTrack>> {
    let mut tracks = Vec::new();
    for v in items {
        if is_line_names(v) {
            continue;
        }
        let t = track_size(v, cx)?;
        if tracks.len() < MAX_GRID_TRACKS {
            tracks.push(t);
        }
    }
    (!tracks.is_empty()).then_some(tracks)
}

/// `<track-list> | <auto-track-list>` (line names are dropped, `repeat(N, …)`
/// is expanded up to [`MAX_GRID_TRACKS`] tracks).
fn track_list(items: &[&CV], cx: &ParseCtx) -> Option<SpecGridTemplate> {
    let mut tracks: Vec<SpecTrack> = Vec::new();
    let mut auto_repeat: Option<(bool, Vec<SpecTrack>)> = None;
    let mut auto_repeat_index = 0;
    for v in items {
        if is_line_names(v) {
            continue;
        }
        let CV::Function { name, args } = v else {
            let t = track_size(v, cx)?;
            if tracks.len() < MAX_GRID_TRACKS {
                tracks.push(t);
            }
            continue;
        };
        if !name.eq_ignore_ascii_case("repeat") {
            let t = track_size(v, cx)?;
            if tracks.len() < MAX_GRID_TRACKS {
                tracks.push(t);
            }
            continue;
        }
        let parts = split_commas(args);
        let [count, inner] = parts.as_slice() else {
            return None;
        };
        let repeated = repeated_tracks(&significant(inner), cx)?;
        let count = significant(count);
        let [count] = count.as_slice() else {
            return None;
        };
        if count.is_ident("auto-fill") || count.is_ident("auto-fit") {
            if auto_repeat.is_some() || !repeated.iter().all(is_fixed) {
                return None;
            }
            auto_repeat = Some((count.is_ident("auto-fill"), repeated));
            auto_repeat_index = tracks.len();
        } else {
            let n = integer(count).filter(|n| *n >= 1)?;
            let room = MAX_GRID_TRACKS.saturating_sub(tracks.len());
            let wanted = usize::try_from(n)
                .unwrap_or(usize::MAX)
                .saturating_mul(repeated.len());
            tracks.extend(repeated.iter().cycle().take(wanted.min(room)).cloned());
        }
    }
    if tracks.is_empty() && auto_repeat.is_none() {
        return None;
    }
    if auto_repeat.is_some() && !tracks.iter().all(is_fixed) {
        return None;
    }
    Some(SpecGridTemplate::new(
        tracks,
        auto_repeat,
        auto_repeat_index,
    ))
}

/// `none | <track-list> | <auto-track-list> | subgrid <line-name-list>? | masonry`.
fn template_value(items: &[&CV], cx: &ParseCtx) -> Option<Option<Arc<SpecGridTemplate>>> {
    match items {
        [v] if v.is_ident("none") || v.is_ident("masonry") => Some(None),
        // `subgrid` (with line names) is not supported: behaves as `none`.
        [first, rest @ ..] if first.is_ident("subgrid") => rest
            .iter()
            .all(|v| is_line_names(v) || v.is_function("repeat"))
            .then_some(None),
        _ => Some(Some(Arc::new(track_list(items, cx)?))),
    }
}

/// `grid-template-columns` / `grid-template-rows`.
pub(super) fn grid_template_tracks(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let [id] = ids else {
        return None;
    };
    push(out, *id, Spec::GridTemplate(template_value(items, cx)?));
    Some(())
}

type Templates = (Option<Arc<SpecGridTemplate>>, Option<Arc<SpecGridTemplate>>);

/// `[ <line-names>? <string> <track-size>? <line-names>? ]+ [ / <explicit-track-list> ]?`:
/// one row per string (`auto` when it has no size).
fn areas_template(items: &[&CV], cx: &ParseCtx) -> Option<Templates> {
    let (left, right) = match items.iter().position(|v| v.is_delim('/')) {
        Some(i) => (items.get(..i)?, Some(items.get(i + 1..)?)),
        None => (items, None),
    };
    let mut rows: Vec<SpecTrack> = Vec::new();
    let mut open_row = false;
    for v in left {
        if is_line_names(v) {
            continue;
        }
        if matches!(v, CV::Token(Token::String(_))) {
            if open_row && rows.len() < MAX_GRID_TRACKS {
                rows.push(SpecTrack::Auto);
            }
            open_row = true;
            continue;
        }
        if !open_row {
            return None;
        }
        let t = track_size(v, cx)?;
        if rows.len() < MAX_GRID_TRACKS {
            rows.push(t);
        }
        open_row = false;
    }
    if open_row && rows.len() < MAX_GRID_TRACKS {
        rows.push(SpecTrack::Auto);
    }
    if rows.is_empty() {
        return None;
    }
    let columns = match right {
        Some(right) => {
            let t = track_list(right, cx)?;
            if t.auto_repeat.is_some() {
                return None; // `<explicit-track-list>`
            }
            Some(Arc::new(t))
        }
        None => None,
    };
    Some((
        Some(Arc::new(SpecGridTemplate::new(rows, None, 0))),
        columns,
    ))
}

/// The `grid-template` shorthand value: (rows, columns).
fn template_shorthand(items: &[&CV], cx: &ParseCtx) -> Option<Templates> {
    if let [v] = items {
        if v.is_ident("none") {
            return Some((None, None));
        }
    }
    if items
        .iter()
        .any(|v| matches!(v, CV::Token(Token::String(_))))
    {
        return areas_template(items, cx);
    }
    let slash = items.iter().position(|v| v.is_delim('/'))?;
    let rows = template_value(items.get(..slash)?, cx)?;
    let columns = template_value(items.get(slash + 1..)?, cx)?;
    Some((rows, columns))
}

/// `grid-template` (longhands: rows, columns).
pub(super) fn grid_template(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let [rows_id, columns_id] = ids else {
        return None;
    };
    let (rows, columns) = template_shorthand(items, cx)?;
    push(out, *rows_id, Spec::GridTemplate(rows));
    push(out, *columns_id, Spec::GridTemplate(columns));
    Some(())
}

/// `[ auto-flow && dense? ] <track-size>*` (the implicit tracks are ignored).
fn is_auto_flow_part(items: &[&CV], cx: &ParseCtx) -> bool {
    let (mut flow, mut dense) = (false, false);
    let mut rest = items;
    while let Some((v, tail)) = rest.split_first() {
        if v.is_ident("auto-flow") && !flow {
            flow = true;
        } else if v.is_ident("dense") && !dense {
            dense = true;
        } else {
            break;
        }
        rest = tail;
    }
    flow && rest.iter().all(|v| track_size(v, cx).is_some())
}

/// The `grid` shorthand (longhands: rows, columns): a `grid-template`
/// value, or an auto-flow axis (its template becomes `none`) and a
/// template for the other axis.
pub(super) fn grid(items: &[&CV], ids: &[LonghandId], cx: &ParseCtx, out: &mut Out) -> Option<()> {
    let [rows_id, columns_id] = ids else {
        return None;
    };
    let (rows, columns) = if items.iter().any(|v| v.is_ident("auto-flow")) {
        let slash = items.iter().position(|v| v.is_delim('/'))?;
        let (left, right) = (items.get(..slash)?, items.get(slash + 1..)?);
        if is_auto_flow_part(left, cx) {
            (None, template_value(right, cx)?)
        } else if is_auto_flow_part(right, cx) {
            (template_value(left, cx)?, None)
        } else {
            return None;
        }
    } else {
        template_shorthand(items, cx)?
    };
    push(out, *rows_id, Spec::GridTemplate(rows));
    push(out, *columns_id, Spec::GridTemplate(columns));
    Some(())
}

/// A `<custom-ident>` usable as a grid line name.
fn is_line_name(v: &CV) -> bool {
    v.ident().is_some_and(|k| {
        ![
            "span", "auto", "inherit", "initial", "unset", "default", "revert",
        ]
        .iter()
        .any(|r| k.eq_ignore_ascii_case(r))
    })
}

/// `<grid-line>`: `auto | <custom-ident> | [<integer> && <custom-ident>?] |
/// [span && [<integer> || <custom-ident>]]` (named lines → `Auto`).
fn grid_line(items: &[&CV]) -> Option<GridLine> {
    if let [v] = items {
        if v.is_ident("auto") {
            return Some(GridLine::Auto);
        }
    }
    let (mut span, mut number, mut named) = (false, None, false);
    for v in items {
        if v.is_ident("span") && !span {
            span = true;
        } else if let Some(n) = integer(v).filter(|_| number.is_none()) {
            number = Some(n);
        } else if is_line_name(v) && !named {
            named = true;
        } else {
            return None;
        }
    }
    match (span, number, named) {
        (_, Some(0), _) | (true, Some(i32::MIN..=-1), _) | (_, None, false) => None,
        (_, _, true) => Some(GridLine::Auto),
        (true, Some(n), false) => Some(GridLine::Span(n.clamp(1, MAX_GRID_LINE).unsigned_abs())),
        (false, Some(n), false) => Some(GridLine::Line(n.clamp(-MAX_GRID_LINE, MAX_GRID_LINE))),
    }
}

/// `grid-row-start` & co. (one longhand), `grid-row` / `grid-column`
/// (start, end) and `grid-area` (row-start, column-start, row-end,
/// column-end): `<grid-line> [ / <grid-line> ]*`; omitted lines are `auto`.
pub(super) fn grid_placement(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let parts: Vec<&[&CV]> = items.split(|v| v.is_delim('/')).collect();
    if parts.len() > ids.len() {
        return None;
    }
    let lines = parts
        .iter()
        .map(|p| grid_line(p))
        .collect::<Option<Vec<_>>>()?;
    for (i, id) in ids.iter().enumerate() {
        let line = lines.get(i).copied().unwrap_or(GridLine::Auto);
        push(out, *id, Spec::GridLine(line));
    }
    Some(())
}

/// A length that does not depend on the element (only px and percentages).
fn is_context_free(e: &MathExpr) -> bool {
    match e {
        MathExpr::Linear(l) => {
            *l == LinearLen {
                px: l.px,
                percent: l.percent,
                ..LinearLen::ZERO
            }
        }
        _ => false,
    }
}

impl SpecTrack {
    fn context_free(&self) -> bool {
        match self {
            SpecTrack::Length(e) | SpecTrack::FitContent(e) => is_context_free(e),
            SpecTrack::MinMax(min, max) => min.context_free() && max.context_free(),
            SpecTrack::Fr(_) | SpecTrack::Auto | SpecTrack::MinContent | SpecTrack::MaxContent => {
                true
            }
        }
    }

    fn compute(&self, r: &Resolver) -> GridTrack {
        let lp = |e: &MathExpr| non_negative(e.eval(r));
        match self {
            SpecTrack::Length(e) => GridTrack::Length(lp(e)),
            SpecTrack::Fr(f) => GridTrack::Fr(crate::values::sane(*f).max(0.0)),
            SpecTrack::Auto => GridTrack::Auto,
            SpecTrack::MinContent => GridTrack::MinContent,
            SpecTrack::MaxContent => GridTrack::MaxContent,
            SpecTrack::FitContent(e) => GridTrack::FitContent(lp(e)),
            SpecTrack::MinMax(min, max) => {
                GridTrack::MinMax(Box::new(min.compute(r)), Box::new(max.compute(r)))
            }
        }
    }
}

/// Clamps a `calc()` result for a track size (negative values are invalid).
fn non_negative(mut v: LengthPercentage) -> LengthPercentage {
    if v.percent == 0.0 {
        v.px = v.px.max(0.0);
    } else if v.px == 0.0 {
        v.percent = v.percent.max(0.0);
    }
    v
}

impl SpecGridTemplate {
    fn new(
        tracks: Vec<SpecTrack>,
        auto_repeat: Option<(bool, Vec<SpecTrack>)>,
        auto_repeat_index: usize,
    ) -> Self {
        let mut t = SpecGridTemplate {
            tracks,
            auto_repeat,
            auto_repeat_index,
            computed: None,
        };
        let context_free = t.tracks.iter().all(SpecTrack::context_free)
            && t.auto_repeat
                .as_ref()
                .is_none_or(|(_, tracks)| tracks.iter().all(SpecTrack::context_free));
        if context_free {
            // Any resolver gives the same result.
            let r = Resolver::new(16.0, 16.0, false, (0.0, 0.0));
            t.computed = Some(Arc::new(t.compute(&r)));
        }
        t
    }

    /// Number of tracks stored (explicit + auto-repeated).
    pub(crate) fn track_count(&self) -> usize {
        self.tracks.len() + self.auto_repeat.as_ref().map_or(0, |(_, t)| t.len())
    }

    /// The computed template for an element whose units resolve with `r`.
    pub(crate) fn compute(&self, r: &Resolver) -> GridTemplate {
        GridTemplate {
            tracks: self.tracks.iter().map(|t| t.compute(r)).collect(),
            auto_repeat: self.auto_repeat.as_ref().map(|(fill, tracks)| AutoRepeat {
                fill: *fill,
                tracks: tracks.iter().map(|t| t.compute(r)).collect(),
            }),
            auto_repeat_index: self.auto_repeat_index.min(self.tracks.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, spec};
    use super::*;

    fn px(v: f32) -> GridTrack {
        GridTrack::Length(LengthPercentage::px(v))
    }

    fn template(name: &str, css: &str, id: LonghandId) -> Option<Option<GridTemplate>> {
        match spec(name, css, id)? {
            Spec::GridTemplate(t) => Some(t.map(|t| {
                let r = Resolver::new(10.0, 20.0, false, (1000.0, 500.0));
                t.compute(&r)
            })),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn columns(css: &str) -> Option<Option<GridTemplate>> {
        template(
            "grid-template-columns",
            css,
            LonghandId::GridTemplateColumns,
        )
    }

    fn tracks(css: &str) -> Vec<GridTrack> {
        columns(css)
            .unwrap_or_else(|| panic!("invalid: {css}"))
            .unwrap_or_else(|| panic!("none: {css}"))
            .tracks
    }

    fn fr(f: f32) -> GridTrack {
        GridTrack::Fr(f)
    }

    fn minmax(a: GridTrack, b: GridTrack) -> GridTrack {
        GridTrack::MinMax(Box::new(a), Box::new(b))
    }

    #[test]
    fn track_sizes() {
        assert_eq!(
            tracks("1fr 200px auto 10%"),
            vec![
                fr(1.0),
                px(200.0),
                GridTrack::Auto,
                GridTrack::Length(LengthPercentage::percent(10.0))
            ]
        );
        assert_eq!(
            tracks("min-content max-content 2em calc(1rem + 10%)"),
            vec![
                GridTrack::MinContent,
                GridTrack::MaxContent,
                px(20.0),
                GridTrack::Length(LengthPercentage {
                    px: 20.0,
                    percent: 10.0
                })
            ]
        );
        assert_eq!(
            tracks("minmax(100px, 1fr) fit-content(40%) minmax(auto, max-content)"),
            vec![
                minmax(px(100.0), fr(1.0)),
                GridTrack::FitContent(LengthPercentage::percent(40.0)),
                minmax(GridTrack::Auto, GridTrack::MaxContent),
            ]
        );
        // Negative calc() results are clamped.
        assert_eq!(tracks("calc(10px - 20px)"), vec![px(0.0)]);
        assert_eq!(tracks("0"), vec![px(0.0)]);
        for bad in [
            "-10px",
            "-1fr",
            "minmax(1fr, 100px)",
            "minmax(100px)",
            "minmax(1px, 2px, 3px)",
            "fit-content(1fr)",
            "fit-content(auto)",
            "bogus",
            "10",
            "[a]",
            "1px / 2px",
            "\"a\"",
            "minmax(minmax(1px, 2px), 1fr)",
        ] {
            assert_eq!(columns(bad), None, "{bad}");
        }
    }

    #[test]
    fn keywords_and_line_names() {
        assert_eq!(columns("none"), Some(None));
        assert_eq!(columns("subgrid"), Some(None));
        assert_eq!(columns("subgrid [a] [b c]"), Some(None));
        assert_eq!(columns("masonry"), Some(None));
        assert_eq!(columns("none 1fr"), None);
        assert_eq!(
            tracks("[full-start] minmax(1em, 1fr) [main-start] 100px [main-end full-end]"),
            vec![minmax(px(10.0), fr(1.0)), px(100.0)]
        );
    }

    #[test]
    fn repeat_expansion() {
        assert_eq!(tracks("repeat(3, 1fr)"), vec![fr(1.0); 3]);
        assert_eq!(
            tracks("[a] 100px repeat(2, 1fr [b] 2fr) auto"),
            vec![
                px(100.0),
                fr(1.0),
                fr(2.0),
                fr(1.0),
                fr(2.0),
                GridTrack::Auto
            ]
        );
        assert_eq!(tracks("repeat(calc(1 + 1), 5px)"), vec![px(5.0); 2]);
        // Capped at MAX_GRID_TRACKS.
        assert_eq!(tracks("repeat(1000000000, 1fr)").len(), MAX_GRID_TRACKS);
        assert_eq!(tracks("repeat(999, 1px 2px) 3px").len(), MAX_GRID_TRACKS);
        assert_eq!(
            tracks("repeat(2147483647, 1px 2px 3px)").len(),
            MAX_GRID_TRACKS
        );
        for bad in [
            "repeat(0, 1fr)",
            "repeat(-1, 1fr)",
            "repeat(1.5, 1fr)",
            "repeat(2)",
            "repeat(2, )",
            "repeat(2, [a])",
            "repeat(2, repeat(2, 1fr))",
            "repeat(auto, 1fr)",
            "repeat(2 3, 1fr)",
        ] {
            assert_eq!(columns(bad), None, "{bad}");
        }
    }

    #[test]
    fn auto_repeat() {
        let t = columns("repeat(auto-fill, minmax(100px, 1fr))")
            .unwrap()
            .unwrap();
        assert!(t.tracks.is_empty());
        assert_eq!(
            t.auto_repeat,
            Some(AutoRepeat {
                fill: true,
                tracks: vec![minmax(px(100.0), fr(1.0))]
            })
        );
        assert_eq!(t.auto_repeat_index, 0);
        let t = columns("100px [a] repeat(auto-fit, [b] 5em 10px) minmax(auto, 50px)")
            .unwrap()
            .unwrap();
        assert_eq!(t.tracks, vec![px(100.0), minmax(GridTrack::Auto, px(50.0))]);
        assert_eq!(t.auto_repeat_index, 1);
        assert_eq!(
            t.auto_repeat,
            Some(AutoRepeat {
                fill: false,
                tracks: vec![px(50.0), px(10.0)]
            })
        );
        for bad in [
            // Only fixed sizes in and next to an auto-repeat, and only one.
            "repeat(auto-fill, 1fr)",
            "repeat(auto-fill, auto)",
            "repeat(auto-fit, minmax(auto, 1fr))",
            "1fr repeat(auto-fill, 100px)",
            "repeat(auto-fill, 10px) repeat(auto-fit, 10px)",
            "repeat(auto-fill, 10px) repeat(2, auto)",
        ] {
            assert_eq!(columns(bad), None, "{bad}");
        }
        assert!(columns("repeat(auto-fill, 10px) repeat(2, minmax(1px, auto))").is_some());
    }

    #[test]
    fn context_free_templates_are_shared() {
        let shared = |css: &str| match spec(
            "grid-template-columns",
            css,
            LonghandId::GridTemplateColumns,
        ) {
            Some(Spec::GridTemplate(Some(t))) => t.computed.is_some(),
            other => panic!("unexpected {other:?}"),
        };
        assert!(shared("10px 5% minmax(10px, 1fr) repeat(auto-fill, 2px)"));
        assert!(shared(
            "1fr minmax(auto, 1fr) auto fit-content(10px) max-content"
        ));
        assert!(!shared("1fr 1em"));
        assert!(!shared("fit-content(10vw)"));
        assert!(!shared("repeat(auto-fill, minmax(1px, 1rem))"));
        assert!(!shared("min(10px, 5%)"));
    }

    #[test]
    fn grid_template_shorthand() {
        let both = |name: &str, css: &str| {
            Some((
                template(name, css, LonghandId::GridTemplateRows)?,
                template(name, css, LonghandId::GridTemplateColumns)?,
            ))
        };
        let (rows, cols) = both("grid-template", "auto 1fr / 100px 1fr").unwrap();
        assert_eq!(rows.unwrap().tracks, vec![GridTrack::Auto, fr(1.0)]);
        assert_eq!(cols.unwrap().tracks, vec![px(100.0), fr(1.0)]);
        assert_eq!(both("grid-template", "none"), Some((None, None)));
        let (rows, cols) = both("grid-template", "none / repeat(2, 1fr)").unwrap();
        assert_eq!(rows, None);
        assert_eq!(cols.unwrap().tracks.len(), 2);
        // Areas: one row per string.
        let (rows, cols) = both(
            "grid-template",
            "[top] \"a a\" 40px [mid] \"b c\" \"d d\" 1fr / 1fr 2fr",
        )
        .unwrap();
        assert_eq!(
            rows.unwrap().tracks,
            vec![px(40.0), GridTrack::Auto, fr(1.0)]
        );
        assert_eq!(cols.unwrap().tracks, vec![fr(1.0), fr(2.0)]);
        let (rows, cols) = both("grid-template", "\"a\" \"b\"").unwrap();
        assert_eq!(rows.unwrap().tracks.len(), 2);
        assert_eq!(cols, None);
        for bad in [
            "auto 1fr",
            "1fr / 1fr / 1fr",
            "/ 1fr",
            "1fr /",
            "\"a\" / repeat(auto-fill, 10px)",
            "1fr \"a\"",
            "\"a\" 1fr 2fr",
        ] {
            assert_eq!(both("grid-template", bad), None, "{bad}");
        }
        // `grid`: templates, or one auto-flow axis.
        let (rows, cols) = both("grid", "auto-flow / 1fr 1fr 1fr").unwrap();
        assert_eq!(rows, None);
        assert_eq!(cols.unwrap().tracks.len(), 3);
        let (rows, cols) = both("grid", "100px 1fr / auto-flow dense 50px").unwrap();
        assert_eq!(rows.unwrap().tracks.len(), 2);
        assert_eq!(cols, None);
        let (rows, cols) = both("grid", "dense auto-flow minmax(1px, auto) / 10px").unwrap();
        assert_eq!(rows, None);
        assert_eq!(cols.unwrap().tracks, vec![px(10.0)]);
        let (rows, cols) = both("grid", "auto 1fr / 1fr").unwrap();
        assert_eq!(rows.unwrap().tracks.len(), 2);
        assert_eq!(cols.unwrap().tracks.len(), 1);
        for bad in [
            "auto-flow",
            "auto-flow / auto-flow",
            "auto-flow auto-flow / 1fr",
            "auto-flow bogus / 1fr",
            "dense / 1fr",
        ] {
            assert_eq!(both("grid", bad), None, "{bad}");
        }
    }

    fn line(name: &str, css: &str, id: LonghandId) -> Option<GridLine> {
        match spec(name, css, id)? {
            Spec::GridLine(l) => Some(l),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn placement() {
        let start = |css: &str| line("grid-column-start", css, LonghandId::GridColumnStart);
        assert_eq!(start("auto"), Some(GridLine::Auto));
        assert_eq!(start("2"), Some(GridLine::Line(2)));
        assert_eq!(start("-1"), Some(GridLine::Line(-1)));
        assert_eq!(start("span 3"), Some(GridLine::Span(3)));
        assert_eq!(start("3 span"), Some(GridLine::Span(3)));
        assert_eq!(start("main-start"), Some(GridLine::Auto));
        assert_eq!(start("2 main"), Some(GridLine::Auto));
        assert_eq!(start("span main"), Some(GridLine::Auto));
        assert_eq!(start("99999999"), Some(GridLine::Line(MAX_GRID_LINE)));
        assert_eq!(start("-99999999"), Some(GridLine::Line(-MAX_GRID_LINE)));
        assert_eq!(start("span 99999999"), Some(GridLine::Span(10_000)));
        for bad in [
            "0",
            "span 0",
            "span -1",
            "span",
            "1.5",
            "1 2",
            "span span 2",
            "a b",
            "1 / 2",
            "10px",
        ] {
            assert_eq!(start(bad), None, "{bad}");
        }
        let col = |css: &str| {
            Some((
                line("grid-column", css, LonghandId::GridColumnStart)?,
                line("grid-column", css, LonghandId::GridColumnEnd)?,
            ))
        };
        assert_eq!(col("1 / -1"), Some((GridLine::Line(1), GridLine::Line(-1))));
        assert_eq!(
            col("2 / span 3"),
            Some((GridLine::Line(2), GridLine::Span(3)))
        );
        assert_eq!(col("span 2"), Some((GridLine::Span(2), GridLine::Auto)));
        assert_eq!(col("1"), Some((GridLine::Line(1), GridLine::Auto)));
        assert_eq!(col("auto"), Some((GridLine::Auto, GridLine::Auto)));
        assert_eq!(col("a / b"), Some((GridLine::Auto, GridLine::Auto)));
        assert_eq!(col("1 / 2 / 3"), None);
        assert_eq!(col("1 /"), None);
        let row_end = |css: &str| line("grid-row", css, LonghandId::GridRowEnd);
        assert_eq!(row_end("1 / 3"), Some(GridLine::Line(3)));
        // grid-area: row-start / column-start / row-end / column-end.
        let area = |css: &str| {
            Some([
                line("grid-area", css, LonghandId::GridRowStart)?,
                line("grid-area", css, LonghandId::GridColumnStart)?,
                line("grid-area", css, LonghandId::GridRowEnd)?,
                line("grid-area", css, LonghandId::GridColumnEnd)?,
            ])
        };
        assert_eq!(
            area("1 / 2 / 3 / 4"),
            Some([
                GridLine::Line(1),
                GridLine::Line(2),
                GridLine::Line(3),
                GridLine::Line(4)
            ])
        );
        assert_eq!(
            area("2 / 3"),
            Some([
                GridLine::Line(2),
                GridLine::Line(3),
                GridLine::Auto,
                GridLine::Auto
            ])
        );
        assert_eq!(area("header"), Some([GridLine::Auto; 4]));
        assert_eq!(area("1 / 2 / 3 / 4 / 5"), None);
        assert!(parse("grid-row-end", "span 2").is_some());
    }
}
