//! Backgrounds: color, image (url / gradients), repeat, size, position.
//!
//! Only one image layer is kept by xbro: the color comes from the last layer
//! of the `background` shorthand, everything else from the first one.

use xbro_css::ComponentValue as CV;

use super::{only, push, LonghandId, Out, Spec, SpecBgSize, SpecImage};
use crate::computed::BackgroundRepeat;
use crate::values::{
    angle_degrees, color, image_set_url, keyword, length, split_on_commas, url, LinearLen,
    MathExpr, ParseCtx, SpecColor, ANY_LP, NON_NEG_LP,
};

pub(super) fn background_color(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let [v] = items else {
        return None;
    };
    push(out, only(ids)?, Spec::Color(color(v)?));
    Some(())
}

/// Parses each comma-separated layer with `f` (all must be valid) and
/// returns the first layer's value.
fn first_layer<T>(items: &[&CV], f: impl Fn(&[&CV]) -> Option<T>) -> Option<T> {
    let mut first = None;
    for layer in split_on_commas(items) {
        let v = f(layer)?;
        first.get_or_insert(v);
    }
    first
}

pub(super) fn background_image(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let img = first_layer(items, |layer| match layer {
        [v] => image(v),
        _ => None,
    })?;
    push(out, only(ids)?, Spec::BgImage(img));
    Some(())
}

pub(super) fn background_repeat(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let r = first_layer(items, repeat)?;
    push(out, only(ids)?, Spec::BgRepeat(r));
    Some(())
}

pub(super) fn background_size(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let s = first_layer(items, |layer| size(layer, cx))?;
    push(out, only(ids)?, Spec::BgSize(s));
    Some(())
}

pub(super) fn background_position(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let (x, y) = first_layer(items, |layer| position(layer, cx))?;
    let [xid, yid] = ids else {
        return None;
    };
    push(out, *xid, Spec::Lp(x));
    push(out, *yid, Spec::Lp(y));
    Some(())
}

/// `background-position-x` / `-y`: `center | [keyword]? <length-percentage>?`.
pub(super) fn background_position_axis(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let id = only(ids)?;
    let horizontal = id == LonghandId::BackgroundPositionX;
    let v = first_layer(items, |layer| {
        let (kw, offset) = match layer {
            [k, o] => (Some(*k), Some(*o)),
            [single] => match length(single, ANY_LP, cx) {
                Some(lp) => return Some(lp),
                None => (Some(*single), None),
            },
            _ => return None,
        };
        let k = kw?.ident()?.to_ascii_lowercase();
        let (base, sign) = match (k.as_str(), horizontal) {
            ("center", _) if offset.is_none() => return Some(MathExpr::percent(50.0)),
            ("left" | "x-start", true) | ("top" | "y-start", false) => (0.0, 1.0),
            ("right" | "x-end", true) | ("bottom" | "y-end", false) => (100.0, -1.0),
            _ => return None,
        };
        let offset = match offset {
            Some(o) => length(o, ANY_LP, cx)?,
            None => MathExpr::ZERO,
        };
        Some(MathExpr::percent(base).plus(offset.times(sign)))
    })?;
    push(out, id, Spec::Lp(v));
    Some(())
}

/// `background` shorthand.
pub(super) fn background(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let layers = split_on_commas(items);
    let count = layers.len();
    let mut first: Option<Layer> = None;
    let mut last_color = None;
    for (i, items) in layers.into_iter().enumerate() {
        let l = layer(items, cx, i + 1 == count)?;
        if i + 1 == count {
            last_color = l.color;
        }
        first.get_or_insert(l);
    }
    let first = first?;
    let [cid, iid, rid, sid, xid, yid] = ids else {
        return None;
    };
    let (x, y) = first.position.unwrap_or((MathExpr::ZERO, MathExpr::ZERO));
    push(
        out,
        *cid,
        Spec::Color(last_color.unwrap_or(SpecColor::Color(xbro_css::Color::TRANSPARENT))),
    );
    push(out, *iid, Spec::BgImage(first.image.unwrap_or(None)));
    push(
        out,
        *rid,
        Spec::BgRepeat(first.repeat.unwrap_or(BackgroundRepeat::Repeat)),
    );
    push(
        out,
        *sid,
        Spec::BgSize(first.size.unwrap_or(SpecBgSize::Auto)),
    );
    push(out, *xid, Spec::Lp(x));
    push(out, *yid, Spec::Lp(y));
    Some(())
}

#[derive(Default)]
struct Layer {
    color: Option<SpecColor>,
    image: Option<Option<SpecImage>>,
    repeat: Option<BackgroundRepeat>,
    size: Option<SpecBgSize>,
    position: Option<(MathExpr, MathExpr)>,
}

fn is_repeat_keyword(v: &CV) -> bool {
    [
        "repeat",
        "repeat-x",
        "repeat-y",
        "no-repeat",
        "space",
        "round",
    ]
    .iter()
    .any(|k| v.is_ident(k))
}

fn is_position_token(v: &CV, cx: &ParseCtx) -> bool {
    ["left", "right", "top", "bottom", "center"]
        .iter()
        .any(|k| v.is_ident(k))
        || length(v, ANY_LP, cx).is_some()
}

fn layer(items: &[&CV], cx: &ParseCtx, is_last: bool) -> Option<Layer> {
    let mut l = Layer::default();
    let (mut attachment, mut boxes) = (false, 0);
    let mut i = 0;
    while let Some(v) = items.get(i) {
        if l.image.is_none() {
            if let Some(img) = image(v) {
                l.image = Some(img);
                i += 1;
                continue;
            }
        }
        if l.repeat.is_none() && is_repeat_keyword(v) {
            let n = if items.get(i + 1).is_some_and(|n| is_repeat_keyword(n)) {
                2
            } else {
                1
            };
            l.repeat = Some(repeat(items.get(i..i + n)?)?);
            i += n;
            continue;
        }
        if l.position.is_none() && is_position_token(v, cx) {
            let run = items
                .get(i..)
                .unwrap_or(&[])
                .iter()
                .take(4)
                .take_while(|t| is_position_token(t, cx))
                .count();
            let (n, pos) = (1..=run)
                .rev()
                .find_map(|n| Some((n, position(items.get(i..i + n)?, cx)?)))?;
            l.position = Some(pos);
            i += n;
            if items.get(i).is_some_and(|s| s.is_delim('/')) {
                i += 1;
                let two = items
                    .get(i..i + 2)
                    .and_then(|s| size(s, cx))
                    .map(|s| (2, s));
                let (n, s) = two.or_else(|| Some((1, size(items.get(i..i + 1)?, cx)?)))?;
                l.size = Some(s);
                i += n;
            }
            continue;
        }
        if !attachment && ["scroll", "fixed", "local"].iter().any(|k| v.is_ident(k)) {
            attachment = true;
            i += 1;
            continue;
        }
        if boxes < 2
            && ["border-box", "padding-box", "content-box", "text"]
                .iter()
                .any(|k| v.is_ident(k))
        {
            boxes += 1;
            i += 1;
            continue;
        }
        if is_last && l.color.is_none() {
            if let Some(c) = color(v) {
                l.color = Some(c);
                i += 1;
                continue;
            }
        }
        return None;
    }
    Some(l)
}

/// One `<bg-image>`: `Some(None)` for `none`.
pub(super) fn image(v: &CV) -> Option<Option<SpecImage>> {
    if v.is_ident("none") {
        return Some(None);
    }
    if let Some(u) = url(v).or_else(|| image_set_url(v)) {
        return Some(Some(SpecImage::Url(u)));
    }
    let CV::Function { name, args } = v else {
        return None;
    };
    let lower = name.to_ascii_lowercase();
    let unprefixed = lower
        .strip_prefix("-webkit-")
        .or_else(|| lower.strip_prefix("-moz-"))
        .or_else(|| lower.strip_prefix("-o-"))
        .unwrap_or(&lower);
    let legacy = unprefixed.len() != lower.len();
    match unprefixed {
        "linear-gradient" | "repeating-linear-gradient" => linear_gradient(args, legacy).map(Some),
        "radial-gradient"
        | "repeating-radial-gradient"
        | "conic-gradient"
        | "repeating-conic-gradient"
        | "gradient" => first_color(args).map(|c| Some(SpecImage::Approx(c))),
        _ => None,
    }
}

/// First color found among the arguments (searching `from()`/`color-stop()` too).
fn first_color(args: &[CV]) -> Option<SpecColor> {
    args.iter().find_map(|a| match a {
        CV::Function { name, args }
            if name.eq_ignore_ascii_case("from") || name.eq_ignore_ascii_case("color-stop") =>
        {
            first_color(args)
        }
        other => color(other),
    })
}

/// Gradient direction in degrees from `to <side> [<side>]` (or legacy `<side>`).
fn side_angle(words: &[&CV], legacy: bool) -> Option<f32> {
    let mut x = 0i32; // -1 left, 1 right
    let mut y = 0i32; // -1 top, 1 bottom
    for w in words {
        let k = w.ident()?.to_ascii_lowercase();
        match k.as_str() {
            "left" if x == 0 => x = -1,
            "right" if x == 0 => x = 1,
            "top" if y == 0 => y = -1,
            "bottom" if y == 0 => y = 1,
            "center" if legacy => {}
            _ => return None,
        }
    }
    if legacy {
        // Legacy syntax names the starting point.
        x = -x;
        y = -y;
    }
    Some(match (x, y) {
        (0, -1) => 0.0,
        (1, -1) => 45.0,
        (1, 0) => 90.0,
        (1, 1) => 135.0,
        (0, 1) | (0, 0) => 180.0,
        (-1, 1) => 225.0,
        (-1, 0) => 270.0,
        (-1, -1) => 315.0,
        _ => return None,
    })
}

/// Most color stops accepted in a gradient (stops are copied into every
/// element the gradient applies to; larger gradients are invalid).
const MAX_STOPS: usize = 256;

fn linear_gradient(args: &[CV], legacy: bool) -> Option<SpecImage> {
    let items: Vec<&CV> = args.iter().filter(|v| !v.is_whitespace()).collect();
    let parts = split_on_commas(&items);
    let (mut angle, mut stop_parts) = (180.0, &parts[..]);
    if let Some((first, rest)) = parts.split_first() {
        let angle_value = match first {
            [a] => angle_degrees(a).map(|deg| if legacy { 90.0 - deg } else { deg }),
            _ => None,
        };
        let direction = angle_value.or_else(|| match first {
            [to, sides @ ..] if !legacy && to.is_ident("to") && !sides.is_empty() => {
                side_angle(sides, false)
            }
            sides if legacy => side_angle(sides, true),
            _ => None,
        });
        if let Some(deg) = direction {
            angle = deg.rem_euclid(360.0);
            stop_parts = rest;
        }
    }
    let mut stops = Vec::new();
    for part in stop_parts {
        match part {
            // Color hint (bare position): ignored.
            [p] if color(p).is_none() && is_stop_position(p) => {}
            [c, positions @ ..] if positions.len() <= 2 => {
                let c = color(c)?;
                if positions.is_empty() {
                    stops.push((c, None));
                }
                for p in positions {
                    if !is_stop_position(p) {
                        return None;
                    }
                    stops.push((c, stop_position(p)));
                }
            }
            _ => return None,
        }
    }
    (stops.len() >= 2 && stops.len() <= MAX_STOPS).then_some(SpecImage::LinearGradient {
        angle_deg: angle,
        stops,
    })
}

fn is_stop_position(v: &CV) -> bool {
    length(v, ANY_LP, &ParseCtx::default()).is_some()
}

/// Position as a fraction of the gradient line: percentages are exact,
/// zero is 0, other lengths are unknown without the box size (`None`, so
/// they are spread evenly like omitted positions — approximation).
fn stop_position(v: &CV) -> Option<f32> {
    match length(v, ANY_LP, &ParseCtx::default())? {
        MathExpr::Linear(l) if l == LinearLen::percent(l.percent) => Some(l.percent / 100.0),
        _ => None,
    }
}

fn repeat(items: &[&CV]) -> Option<BackgroundRepeat> {
    let axis = |v: &CV| -> Option<bool> {
        // true = repeats along the axis.
        keyword(
            v,
            &[
                ("repeat", true),
                ("space", true),
                ("round", true),
                ("no-repeat", false),
            ],
        )
    };
    match items {
        [v] => {
            if v.is_ident("repeat-x") {
                return Some(BackgroundRepeat::RepeatX);
            }
            if v.is_ident("repeat-y") {
                return Some(BackgroundRepeat::RepeatY);
            }
            let r = axis(v)?;
            Some(if r {
                BackgroundRepeat::Repeat
            } else {
                BackgroundRepeat::NoRepeat
            })
        }
        [x, y] => Some(match (axis(x)?, axis(y)?) {
            (true, true) => BackgroundRepeat::Repeat,
            (true, false) => BackgroundRepeat::RepeatX,
            (false, true) => BackgroundRepeat::RepeatY,
            (false, false) => BackgroundRepeat::NoRepeat,
        }),
        _ => None,
    }
}

fn size(items: &[&CV], cx: &ParseCtx) -> Option<SpecBgSize> {
    let one = |v: &CV| -> Option<Option<MathExpr>> {
        if v.is_ident("auto") {
            Some(None)
        } else {
            length(v, NON_NEG_LP, cx).map(Some)
        }
    };
    match items {
        [v] if v.is_ident("cover") => Some(SpecBgSize::Cover),
        [v] if v.is_ident("contain") => Some(SpecBgSize::Contain),
        [v] => match one(v)? {
            None => Some(SpecBgSize::Auto),
            w => Some(SpecBgSize::Explicit(w, None)),
        },
        [a, b] => match (one(a)?, one(b)?) {
            (None, None) => Some(SpecBgSize::Auto),
            (w, h) => Some(SpecBgSize::Explicit(w, h)),
        },
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    X,
    Y,
    Either,
}

enum PosItem {
    Keyword(Axis, f32),
    Lp(MathExpr),
}

/// `<bg-position>` (1–4 values) → (x, y).
pub(super) fn position(items: &[&CV], cx: &ParseCtx) -> Option<(MathExpr, MathExpr)> {
    let parsed = items
        .iter()
        .map(|v| {
            if let Some((axis, pct)) = keyword(
                v,
                &[
                    ("left", (Axis::X, 0.0)),
                    ("right", (Axis::X, 100.0)),
                    ("top", (Axis::Y, 0.0)),
                    ("bottom", (Axis::Y, 100.0)),
                    ("center", (Axis::Either, 50.0)),
                ],
            ) {
                Some(PosItem::Keyword(axis, pct))
            } else {
                length(v, ANY_LP, cx).map(PosItem::Lp)
            }
        })
        .collect::<Option<Vec<_>>>()?;
    let center = || MathExpr::percent(50.0);
    match parsed.as_slice() {
        [PosItem::Lp(x)] => Some((x.clone(), center())),
        [PosItem::Keyword(axis, p)] => Some(match axis {
            Axis::Y => (center(), MathExpr::percent(*p)),
            _ => (MathExpr::percent(*p), center()),
        }),
        [a, b] => {
            let x_ok = |i: &PosItem| {
                matches!(
                    i,
                    PosItem::Lp(_) | PosItem::Keyword(Axis::X | Axis::Either, _)
                )
            };
            let y_ok = |i: &PosItem| {
                matches!(
                    i,
                    PosItem::Lp(_) | PosItem::Keyword(Axis::Y | Axis::Either, _)
                )
            };
            let value = |i: &PosItem| match i {
                PosItem::Lp(e) => e.clone(),
                PosItem::Keyword(_, p) => MathExpr::percent(*p),
            };
            if x_ok(a) && y_ok(b) {
                Some((value(a), value(b)))
            } else if matches!((a, b), (PosItem::Keyword(..), PosItem::Keyword(..)))
                && x_ok(b)
                && y_ok(a)
            {
                Some((value(b), value(a)))
            } else {
                None
            }
        }
        [_, _, _] | [_, _, _, _] => four_value_position(&parsed),
        _ => None,
    }
}

/// 3/4-value syntax: `<keyword> <offset>?` pairs for each axis.
fn four_value_position(items: &[PosItem]) -> Option<(MathExpr, MathExpr)> {
    let mut groups: Vec<(Axis, MathExpr)> = Vec::new();
    let mut i = 0;
    while let Some(item) = items.get(i) {
        let PosItem::Keyword(axis, pct) = item else {
            return None;
        };
        let offset = match items.get(i + 1) {
            Some(PosItem::Lp(o)) => {
                if *axis == Axis::Either {
                    return None; // `center` takes no offset
                }
                i += 1;
                Some(o.clone())
            }
            _ => None,
        };
        i += 1;
        let value = match offset {
            None => MathExpr::percent(*pct),
            Some(o) if *pct == 0.0 => MathExpr::ZERO.plus(o),
            Some(o) => MathExpr::percent(100.0).plus(o.times(-1.0)),
        };
        groups.push((*axis, value));
    }
    let [(a1, v1), (a2, v2)] = <[(Axis, MathExpr); 2]>::try_from(groups).ok()?;
    match (a1, a2) {
        (Axis::X, Axis::Y | Axis::Either) | (Axis::Either, Axis::Y) => Some((v1, v2)),
        (Axis::Y, Axis::X | Axis::Either) | (Axis::Either, Axis::X) => Some((v2, v1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, spec};
    use super::*;
    use xbro_css::Color;

    fn img(css: &str) -> Option<SpecImage> {
        match spec("background-image", css, LonghandId::BackgroundImage)? {
            Spec::BgImage(i) => i,
            _ => None,
        }
    }

    fn red() -> SpecColor {
        SpecColor::Color(Color::rgb(255, 0, 0))
    }
    fn blue() -> SpecColor {
        SpecColor::Color(Color::rgb(0, 0, 255))
    }

    #[test]
    fn images() {
        assert_eq!(img("url(a.png)"), Some(SpecImage::Url("a.png".into())));
        assert_eq!(
            img("url(a.png), url(b.png)"),
            Some(SpecImage::Url("a.png".into()))
        );
        assert_eq!(
            img("image-set('x.png' 1x)"),
            Some(SpecImage::Url("x.png".into()))
        );
        assert_eq!(img("none"), None);
        assert!(parse("background-image", "nonsense").is_none());
        assert!(parse("background-image", "url(a), 5px").is_none());
        assert_eq!(
            img("radial-gradient(circle at center, red, blue)"),
            Some(SpecImage::Approx(red()))
        );
        assert_eq!(
            img("conic-gradient(from 0deg, blue, red)"),
            Some(SpecImage::Approx(blue()))
        );
        assert_eq!(
            img("-webkit-gradient(linear, left top, left bottom, from(red), to(blue))"),
            Some(SpecImage::Approx(red()))
        );
    }

    #[test]
    fn linear_gradients() {
        let lg = |angle: f32, stops: Vec<(SpecColor, Option<f32>)>| {
            Some(SpecImage::LinearGradient {
                angle_deg: angle,
                stops,
            })
        };
        assert_eq!(
            img("linear-gradient(red, blue)"),
            lg(180.0, vec![(red(), None), (blue(), None)])
        );
        assert_eq!(
            img("linear-gradient(to right, red 10%, blue 90%)"),
            lg(90.0, vec![(red(), Some(0.1)), (blue(), Some(0.9))])
        );
        assert_eq!(
            img("linear-gradient(to bottom left, red, blue)"),
            lg(225.0, vec![(red(), None), (blue(), None)])
        );
        assert_eq!(
            img("linear-gradient(0.25turn, red, 30%, blue)"),
            lg(90.0, vec![(red(), None), (blue(), None)])
        );
        assert_eq!(
            img("linear-gradient(45deg, red 0 50%, blue 50px)"),
            lg(
                45.0,
                vec![(red(), Some(0.0)), (red(), Some(0.5)), (blue(), None)]
            )
        );
        assert_eq!(
            img("-webkit-linear-gradient(top, red, blue)"),
            lg(180.0, vec![(red(), None), (blue(), None)])
        );
        assert_eq!(
            img("-webkit-linear-gradient(0deg, red, blue)"),
            lg(90.0, vec![(red(), None), (blue(), None)])
        );
        assert_eq!(
            img("repeating-linear-gradient(-90deg, red, blue)"),
            lg(270.0, vec![(red(), None), (blue(), None)])
        );
        assert_eq!(img("linear-gradient(red)"), None);
        let many = |n: usize| format!("linear-gradient({})", vec!["red"; n].join(", "));
        assert!(matches!(
            img(&many(MAX_STOPS)),
            Some(SpecImage::LinearGradient { ref stops, .. }) if stops.len() == MAX_STOPS
        ));
        assert!(parse("background-image", &many(MAX_STOPS + 1)).is_none());
        assert!(parse("background-image", "linear-gradient(to middle, red, blue)").is_none());
        assert!(parse("background-image", "linear-gradient(red, nope)").is_none());
    }

    #[test]
    fn positions() {
        let pos = |css: &str| match (
            spec("background-position", css, LonghandId::BackgroundPositionX),
            spec("background-position", css, LonghandId::BackgroundPositionY),
        ) {
            (Some(Spec::Lp(x)), Some(Spec::Lp(y))) => Some((x, y)),
            _ => None,
        };
        let p = MathExpr::percent;
        assert_eq!(pos("center"), Some((p(50.0), p(50.0))));
        assert_eq!(pos("top"), Some((p(50.0), p(0.0))));
        assert_eq!(pos("right"), Some((p(100.0), p(50.0))));
        assert_eq!(pos("10px"), Some((MathExpr::px(10.0), p(50.0))));
        assert_eq!(pos("top left"), Some((p(0.0), p(0.0))));
        assert_eq!(pos("left bottom"), Some((p(0.0), p(100.0))));
        assert_eq!(pos("25% 75%"), Some((p(25.0), p(75.0))));
        assert_eq!(pos("center 10px"), Some((p(50.0), MathExpr::px(10.0))));
        assert_eq!(
            pos("right 10px bottom 20px"),
            Some((
                p(100.0).plus(MathExpr::px(-10.0)),
                p(100.0).plus(MathExpr::px(-20.0))
            ))
        );
        assert_eq!(
            pos("bottom 5px left"),
            Some((p(0.0), p(100.0).plus(MathExpr::px(-5.0))))
        );
        assert_eq!(pos("left 10px top"), Some((MathExpr::px(10.0), p(0.0))));
        assert_eq!(pos("10px left"), None);
        assert_eq!(pos("top 10px"), None);
        assert_eq!(pos("left right"), None);
        assert_eq!(pos("center 5px top"), None);
        assert_eq!(
            spec(
                "background-position-x",
                "right 10px",
                LonghandId::BackgroundPositionX
            ),
            Some(Spec::Lp(p(100.0).plus(MathExpr::px(-10.0))))
        );
        assert_eq!(
            spec(
                "background-position-y",
                "center",
                LonghandId::BackgroundPositionY
            ),
            Some(Spec::Lp(p(50.0)))
        );
        assert!(parse("background-position-y", "left").is_none());
    }

    #[test]
    fn repeat_and_size() {
        let rep = |css: &str| spec("background-repeat", css, LonghandId::BackgroundRepeat);
        assert_eq!(
            rep("repeat-x"),
            Some(Spec::BgRepeat(BackgroundRepeat::RepeatX))
        );
        assert_eq!(
            rep("no-repeat repeat"),
            Some(Spec::BgRepeat(BackgroundRepeat::RepeatY))
        );
        assert_eq!(rep("space"), Some(Spec::BgRepeat(BackgroundRepeat::Repeat)));
        assert_eq!(
            rep("no-repeat, repeat"),
            Some(Spec::BgRepeat(BackgroundRepeat::NoRepeat))
        );
        assert_eq!(rep("repeat-x repeat"), None);
        let size = |css: &str| spec("background-size", css, LonghandId::BackgroundSize);
        assert_eq!(size("cover"), Some(Spec::BgSize(SpecBgSize::Cover)));
        assert_eq!(size("auto"), Some(Spec::BgSize(SpecBgSize::Auto)));
        assert_eq!(
            size("50% auto"),
            Some(Spec::BgSize(SpecBgSize::Explicit(
                Some(MathExpr::percent(50.0)),
                None
            )))
        );
        assert_eq!(
            size("10px"),
            Some(Spec::BgSize(SpecBgSize::Explicit(
                Some(MathExpr::px(10.0)),
                None
            )))
        );
        assert_eq!(size("-1px"), None);
    }

    #[test]
    fn shorthand() {
        let css = "url(a.png) no-repeat right 5px top / 10px auto fixed, #00f";
        assert_eq!(
            spec("background", css, LonghandId::BackgroundColor),
            Some(Spec::Color(blue()))
        );
        assert_eq!(
            spec("background", css, LonghandId::BackgroundImage),
            Some(Spec::BgImage(Some(SpecImage::Url("a.png".into()))))
        );
        assert_eq!(
            spec("background", css, LonghandId::BackgroundRepeat),
            Some(Spec::BgRepeat(BackgroundRepeat::NoRepeat))
        );
        assert_eq!(
            spec("background", css, LonghandId::BackgroundSize),
            Some(Spec::BgSize(SpecBgSize::Explicit(
                Some(MathExpr::px(10.0)),
                None
            )))
        );
        assert_eq!(
            spec("background", css, LonghandId::BackgroundPositionX),
            Some(Spec::Lp(MathExpr::percent(100.0).plus(MathExpr::px(-5.0))))
        );
        assert_eq!(
            spec("background", "red", LonghandId::BackgroundImage),
            Some(Spec::BgImage(None))
        );
        assert_eq!(
            spec("background", "none", LonghandId::BackgroundColor),
            Some(Spec::Color(SpecColor::Color(Color::TRANSPARENT)))
        );
        assert_eq!(
            spec("background", "center / cover", LonghandId::BackgroundSize),
            Some(Spec::BgSize(SpecBgSize::Cover))
        );
        assert_eq!(
            spec(
                "background",
                "linear-gradient(red, blue) padding-box",
                LonghandId::BackgroundColor
            ),
            Some(Spec::Color(SpecColor::Color(Color::TRANSPARENT)))
        );
        // Color only allowed in the last layer; junk rejected.
        assert!(parse("background", "red, url(a.png)").is_none());
        assert!(parse("background", "red blue").is_none());
        assert!(parse("background", "url(a) url(b)").is_none());
    }
}
