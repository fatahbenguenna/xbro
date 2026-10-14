//! Box generation, positioning, sizes, margins, paddings, overflow.

use xbro_css::{ComponentValue as CV, Token};

use super::{only, per_item, push, LonghandId, Out, Spec, SpecLpa, SpecMax};
use crate::computed::*;
use crate::values::{
    integer, keyword, length, number_or_percentage, single, ParseCtx, ANY_LP, NON_NEG_LP,
};

pub(super) fn display(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let words = items
        .iter()
        .map(|v| v.ident().map(str::to_ascii_lowercase))
        .collect::<Option<Vec<_>>>()?;
    let d = match words.as_slice() {
        [one] => display_keyword(one)?,
        [a, b] => display_pair(a, b).or_else(|| display_pair(b, a))?,
        [a, b, c] => display_triple([a, b, c])?,
        _ => return None,
    };
    push(out, only(ids)?, Spec::Display(d));
    Some(())
}

/// Single-keyword `display` values (including legacy / vendor ones).
///
/// `-webkit-box` is mostly used for `-webkit-line-clamp` with
/// `-webkit-box-orient: vertical`, which a row flex container would break, so
/// it maps to `block` (and `-webkit-inline-box` to `inline-block`).
pub(super) fn display_keyword(k: &str) -> Option<Display> {
    Some(match k {
        "none" => Display::None,
        "contents" => Display::Contents,
        "block" | "flow" | "flow-root" | "run-in" | "compact" | "-webkit-box" | "-moz-box" => {
            Display::Block
        }
        "inline"
        | "ruby"
        | "ruby-base"
        | "ruby-text"
        | "ruby-base-container"
        | "ruby-text-container"
        | "math"
        | "inline-list-item" => Display::Inline,
        "inline-block" | "-webkit-inline-box" | "-moz-inline-box" | "-moz-inline-block" => {
            Display::InlineBlock
        }
        "list-item" => Display::ListItem,
        "flex" | "-webkit-flex" | "-ms-flexbox" => Display::Flex,
        "inline-flex" | "-webkit-inline-flex" | "-ms-inline-flexbox" => Display::InlineFlex,
        "grid" | "-ms-grid" => Display::Grid,
        "inline-grid" | "-ms-inline-grid" => Display::InlineGrid,
        "table" => Display::Table,
        "inline-table" => Display::InlineTable,
        "table-row-group" => Display::TableRowGroup,
        "table-header-group" => Display::TableHeaderGroup,
        "table-footer-group" => Display::TableFooterGroup,
        "table-row" => Display::TableRow,
        "table-cell" => Display::TableCell,
        "table-column" => Display::TableColumn,
        "table-column-group" => Display::TableColumnGroup,
        "table-caption" => Display::TableCaption,
        _ => return None,
    })
}

/// Two-value syntax: `<outer> <inner>` (either order is tried by the caller).
fn display_pair(outer: &str, inner: &str) -> Option<Display> {
    let block = match outer {
        "block" | "run-in" => true,
        "inline" => false,
        "list-item" => {
            return matches!(inner, "block" | "flow" | "flow-root" | "run-in")
                .then_some(Display::ListItem)
        }
        _ => return None,
    };
    Some(match (block, inner) {
        (true, "flow" | "flow-root" | "ruby" | "math") => Display::Block,
        (false, "flow" | "ruby" | "math") => Display::Inline,
        (false, "flow-root") => Display::InlineBlock,
        (true, "table") => Display::Table,
        (false, "table") => Display::InlineTable,
        (true, "flex") => Display::Flex,
        (false, "flex") => Display::InlineFlex,
        (true, "grid") => Display::Grid,
        (false, "grid") => Display::InlineGrid,
        // `inline list-item` has no xbro equivalent: approximated as inline.
        (false, "list-item") => Display::Inline,
        _ => return None,
    })
}

/// `block flow list-item` and permutations.
fn display_triple(words: [&String; 3]) -> Option<Display> {
    if !words.iter().any(|w| w.as_str() == "list-item") {
        return None;
    }
    let rest: Vec<&str> = words
        .iter()
        .map(|w| w.as_str())
        .filter(|w| *w != "list-item")
        .collect();
    let [a, b] = rest.as_slice() else {
        return None;
    };
    let valid = |outer: &str, inner: &str| {
        matches!(outer, "block" | "inline" | "run-in") && matches!(inner, "flow" | "flow-root")
    };
    if valid(a, b) || valid(b, a) {
        Some(if rest.contains(&"inline") {
            Display::Inline
        } else {
            Display::ListItem
        })
    } else {
        None
    }
}

pub(super) fn position(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let p = keyword(
        single(items)?,
        &[
            ("static", Position::Static),
            ("relative", Position::Relative),
            ("absolute", Position::Absolute),
            ("fixed", Position::Fixed),
            ("sticky", Position::Sticky),
            ("-webkit-sticky", Position::Sticky),
        ],
    )?;
    push(out, only(ids)?, Spec::Position(p));
    Some(())
}

/// `auto` or a length/percentage with the given rules.
fn lpa(v: &CV, negative: bool, cx: &ParseCtx) -> Option<SpecLpa> {
    if v.is_ident("auto") {
        return Some(SpecLpa::Auto);
    }
    length(v, if negative { ANY_LP } else { NON_NEG_LP }, cx).map(SpecLpa::Lp)
}

/// `top` / `inset` / `inset-inline` / ... (1, 2 or 4 longhands).
pub(super) fn inset(items: &[&CV], ids: &[LonghandId], cx: &ParseCtx, out: &mut Out) -> Option<()> {
    per_item(items, ids, out, |v| lpa(v, true, cx).map(Spec::Lpa))
}

pub(super) fn margin(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    per_item(items, ids, out, |v| lpa(v, true, cx).map(Spec::Lpa))
}

pub(super) fn padding(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    per_item(items, ids, out, |v| length(v, NON_NEG_LP, cx).map(Spec::Lp))
}

/// Intrinsic sizing keywords, approximated as `auto` (or `none` for max sizes).
fn is_intrinsic_keyword(v: &CV) -> bool {
    if let CV::Function { name, .. } = v {
        return name.eq_ignore_ascii_case("fit-content");
    }
    v.ident().is_some_and(|k| {
        [
            "min-content",
            "max-content",
            "fit-content",
            "-webkit-min-content",
            "-webkit-max-content",
            "-webkit-fit-content",
            "-moz-min-content",
            "-moz-max-content",
            "-moz-fit-content",
            "-webkit-fill-available",
            "-moz-available",
            "fill-available",
            "stretch",
            "intrinsic",
            "min-intrinsic",
        ]
        .iter()
        .any(|w| k.eq_ignore_ascii_case(w))
    })
}

/// `width`, `height`, `min-width`, `min-height` and their logical aliases.
pub(super) fn size(items: &[&CV], ids: &[LonghandId], cx: &ParseCtx, out: &mut Out) -> Option<()> {
    let v = single(items)?;
    let spec = if is_intrinsic_keyword(v) {
        SpecLpa::Auto
    } else {
        lpa(v, false, cx)?
    };
    push(out, only(ids)?, Spec::Lpa(spec));
    Some(())
}

pub(super) fn max_size(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let v = single(items)?;
    let spec = if v.is_ident("none") || is_intrinsic_keyword(v) {
        SpecMax::None
    } else {
        SpecMax::Lp(length(v, NON_NEG_LP, cx)?)
    };
    push(out, only(ids)?, Spec::Max(spec));
    Some(())
}

pub(super) fn z_index(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let v = single(items)?;
    let z = if v.is_ident("auto") {
        None
    } else {
        Some(integer(v)?)
    };
    push(out, only(ids)?, Spec::ZIndex(z));
    Some(())
}

pub(super) fn float(items: &[&CV], ids: &[LonghandId], _: &ParseCtx, out: &mut Out) -> Option<()> {
    let f = keyword(
        single(items)?,
        &[
            ("none", Float::None),
            ("left", Float::Left),
            ("right", Float::Right),
            ("inline-start", Float::Left),
            ("inline-end", Float::Right),
        ],
    )?;
    push(out, only(ids)?, Spec::Float(f));
    Some(())
}

pub(super) fn clear(items: &[&CV], ids: &[LonghandId], _: &ParseCtx, out: &mut Out) -> Option<()> {
    let c = keyword(
        single(items)?,
        &[
            ("none", Clear::None),
            ("left", Clear::Left),
            ("right", Clear::Right),
            ("both", Clear::Both),
            ("inline-start", Clear::Left),
            ("inline-end", Clear::Right),
        ],
    )?;
    push(out, only(ids)?, Spec::Clear(c));
    Some(())
}

pub(super) fn box_sizing(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let b = keyword(
        single(items)?,
        &[
            ("content-box", BoxSizing::ContentBox),
            ("border-box", BoxSizing::BorderBox),
        ],
    )?;
    push(out, only(ids)?, Spec::BoxSizing(b));
    Some(())
}

fn overflow_keyword(v: &CV) -> Option<Overflow> {
    keyword(
        v,
        &[
            ("visible", Overflow::Visible),
            ("hidden", Overflow::Hidden),
            ("clip", Overflow::Clip),
            ("scroll", Overflow::Scroll),
            ("auto", Overflow::Auto),
            ("overlay", Overflow::Auto),
            ("-webkit-paged-x", Overflow::Auto),
            ("-webkit-paged-y", Overflow::Auto),
        ],
    )
}

/// `overflow` (`<x> <y>?`) and `overflow-x` / `overflow-y`.
pub(super) fn overflow(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    per_item(items, ids, out, |v| overflow_keyword(v).map(Spec::Overflow))
}

/// `auto || <ratio>`.
pub(super) fn aspect_ratio(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let mut rest: Vec<&CV> = items.to_vec();
    if rest.first().is_some_and(|v| v.is_ident("auto")) {
        rest.remove(0);
    } else if rest.last().is_some_and(|v| v.is_ident("auto")) {
        rest.pop();
    }
    let ratio = match rest.as_slice() {
        [] => None,
        [w] => ratio_part(w)?.into(),
        [w, slash, h] if slash.is_delim('/') => {
            let (w, h) = (ratio_part(w)?, ratio_part(h)?);
            (h > 0.0).then(|| w / h)
        }
        _ => return None,
    };
    // Degenerate ratios (0 or infinite) behave as `auto`.
    let ratio = ratio.filter(|r| *r > 0.0 && r.is_finite());
    push(out, only(ids)?, Spec::AspectRatio(ratio));
    Some(())
}

fn ratio_part(v: &CV) -> Option<f32> {
    match v {
        CV::Token(Token::Number { value, .. }) if *value >= 0.0 => Some(*value),
        _ => crate::values::number(v).filter(|n| *n >= 0.0),
    }
}

pub(super) fn visibility(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let v = keyword(
        single(items)?,
        &[
            ("visible", Visibility::Visible),
            ("hidden", Visibility::Hidden),
            ("collapse", Visibility::Collapse),
        ],
    )?;
    push(out, only(ids)?, Spec::Visibility(v));
    Some(())
}

pub(super) fn opacity(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let o = number_or_percentage(single(items)?)?;
    push(out, only(ids)?, Spec::Number(o.clamp(0.0, 1.0)));
    Some(())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, spec};
    use super::*;
    use crate::values::MathExpr;

    fn display_of(v: &str) -> Option<Display> {
        match spec("display", v, LonghandId::Display)? {
            Spec::Display(d) => Some(d),
            _ => None,
        }
    }

    #[test]
    fn display_values() {
        let cases = [
            ("block", Display::Block),
            ("inline", Display::Inline),
            ("inline-block", Display::InlineBlock),
            ("flow-root", Display::Block),
            ("list-item", Display::ListItem),
            ("flex", Display::Flex),
            ("inline-flex", Display::InlineFlex),
            ("grid", Display::Grid),
            ("inline-grid", Display::InlineGrid),
            ("contents", Display::Contents),
            ("none", Display::None),
            ("table", Display::Table),
            ("inline-table", Display::InlineTable),
            ("table-row-group", Display::TableRowGroup),
            ("table-header-group", Display::TableHeaderGroup),
            ("table-footer-group", Display::TableFooterGroup),
            ("table-row", Display::TableRow),
            ("table-cell", Display::TableCell),
            ("table-column", Display::TableColumn),
            ("table-column-group", Display::TableColumnGroup),
            ("table-caption", Display::TableCaption),
            ("run-in", Display::Block),
            ("-webkit-box", Display::Block),
            ("-webkit-inline-box", Display::InlineBlock),
            ("-webkit-flex", Display::Flex),
            ("inline flex", Display::InlineFlex),
            ("flex inline", Display::InlineFlex),
            ("block flow-root", Display::Block),
            ("inline flow-root", Display::InlineBlock),
            ("block flex", Display::Flex),
            ("inline grid", Display::InlineGrid),
            ("block table", Display::Table),
            ("inline table", Display::InlineTable),
            ("inline flow", Display::Inline),
            ("block list-item", Display::ListItem),
            ("list-item block", Display::ListItem),
            ("block flow list-item", Display::ListItem),
            ("ruby", Display::Inline),
            ("BLOCK", Display::Block),
        ];
        for (css, expected) in cases {
            assert_eq!(display_of(css), Some(expected), "{css}");
        }
        for bad in [
            "blok",
            "block block",
            "inline inline",
            "1",
            "block flex grid",
        ] {
            assert_eq!(display_of(bad), None, "{bad}");
        }
    }

    #[test]
    fn sizes() {
        assert_eq!(
            spec("width", "50%", LonghandId::Width),
            Some(Spec::Lpa(SpecLpa::Lp(MathExpr::percent(50.0))))
        );
        assert_eq!(
            spec("width", "max-content", LonghandId::Width),
            Some(Spec::Lpa(SpecLpa::Auto))
        );
        assert_eq!(
            spec("width", "fit-content(100px)", LonghandId::Width),
            Some(Spec::Lpa(SpecLpa::Auto))
        );
        assert!(parse("width", "-10px").is_none());
        assert!(parse("width", "10").is_none());
        assert!(parse("width", "none").is_none());
        assert_eq!(
            spec("max-width", "none", LonghandId::MaxWidth),
            Some(Spec::Max(SpecMax::None))
        );
        assert_eq!(
            spec("max-inline-size", "10px", LonghandId::MaxWidth),
            Some(Spec::Max(SpecMax::Lp(MathExpr::px(10.0))))
        );
        assert_eq!(
            spec("min-block-size", "auto", LonghandId::MinHeight),
            Some(Spec::Lpa(SpecLpa::Auto))
        );
    }

    #[test]
    fn side_shorthands() {
        let d = parse("margin", "1px 2px 3px").unwrap();
        let px = |v: f32| Spec::Lpa(SpecLpa::Lp(MathExpr::px(v)));
        let got: Vec<_> = d
            .iter()
            .map(|d| match d {
                super::super::PropertyDeclaration::Value(id, s) => (*id, s.clone()),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (LonghandId::MarginTop, px(1.0)),
                (LonghandId::MarginRight, px(2.0)),
                (LonghandId::MarginBottom, px(3.0)),
                (LonghandId::MarginLeft, px(2.0)),
            ]
        );
        assert!(parse("margin", "1px 2px 3px 4px 5px").is_none());
        assert!(parse("padding", "1px -2px").is_none());
        assert_eq!(
            spec("margin-inline", "auto 5px", LonghandId::MarginRight),
            Some(px(5.0))
        );
        assert_eq!(
            spec("margin-block", "7px", LonghandId::MarginBottom),
            Some(px(7.0))
        );
        assert_eq!(spec("inset", "1px 2px", LonghandId::Left), Some(px(2.0)));
        assert_eq!(
            spec("inset-inline-start", "3px", LonghandId::Left),
            Some(px(3.0))
        );
        assert_eq!(
            spec("padding-block-end", "4px", LonghandId::PaddingBottom),
            Some(Spec::Lp(MathExpr::px(4.0)))
        );
    }

    #[test]
    fn misc_box_properties() {
        assert_eq!(
            spec("z-index", "auto", LonghandId::ZIndex),
            Some(Spec::ZIndex(None))
        );
        assert_eq!(
            spec("z-index", "-3", LonghandId::ZIndex),
            Some(Spec::ZIndex(Some(-3)))
        );
        assert!(parse("z-index", "1.5").is_none());
        assert_eq!(
            spec("float", "inline-start", LonghandId::Float),
            Some(Spec::Float(Float::Left))
        );
        assert_eq!(
            spec("clear", "inline-end", LonghandId::Clear),
            Some(Spec::Clear(Clear::Right))
        );
        assert_eq!(
            spec("overflow", "hidden auto", LonghandId::OverflowY),
            Some(Spec::Overflow(Overflow::Auto))
        );
        assert_eq!(
            spec("overflow", "overlay", LonghandId::OverflowX),
            Some(Spec::Overflow(Overflow::Auto))
        );
        assert_eq!(
            spec("aspect-ratio", "16 / 9", LonghandId::AspectRatio),
            Some(Spec::AspectRatio(Some(16.0 / 9.0)))
        );
        assert_eq!(
            spec("aspect-ratio", "auto 2", LonghandId::AspectRatio),
            Some(Spec::AspectRatio(Some(2.0)))
        );
        assert_eq!(
            spec("aspect-ratio", "1 / 0", LonghandId::AspectRatio),
            Some(Spec::AspectRatio(None))
        );
        assert!(parse("aspect-ratio", "-1").is_none());
        assert_eq!(
            spec("opacity", "50%", LonghandId::Opacity),
            Some(Spec::Number(0.5))
        );
        assert_eq!(
            spec("opacity", "3", LonghandId::Opacity),
            Some(Spec::Number(1.0))
        );
        assert_eq!(
            spec("position", "-webkit-sticky", LonghandId::Position),
            Some(Spec::Position(Position::Sticky))
        );
    }
}
