//! Borders, border radii and outlines.

use xbro_css::ComponentValue as CV;

use super::{only, per_item, push, LonghandId, Out, Spec};
use crate::computed::BorderStyle;
use crate::values::{
    color, keyword, length, single, MathExpr, ParseCtx, SpecColor, NON_NEG_LENGTH, NON_NEG_LP,
};

/// `thin` / `medium` / `thick` or a non-negative length.
pub(super) fn line_width(v: &CV, cx: &ParseCtx) -> Option<MathExpr> {
    if let Some(px) = keyword(v, &[("thin", 1.0), ("medium", 3.0), ("thick", 5.0)]) {
        return Some(MathExpr::px(px));
    }
    length(v, NON_NEG_LENGTH, cx)
}

pub(super) fn line_style(v: &CV) -> Option<BorderStyle> {
    keyword(
        v,
        &[
            ("none", BorderStyle::None),
            ("hidden", BorderStyle::Hidden),
            ("solid", BorderStyle::Solid),
            ("dotted", BorderStyle::Dotted),
            ("dashed", BorderStyle::Dashed),
            ("double", BorderStyle::Double),
            ("groove", BorderStyle::Groove),
            ("ridge", BorderStyle::Ridge),
            ("inset", BorderStyle::Inset),
            ("outset", BorderStyle::Outset),
        ],
    )
}

pub(super) fn border_width(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    per_item(items, ids, out, |v| line_width(v, cx).map(Spec::Length))
}

pub(super) fn border_style(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    per_item(items, ids, out, |v| line_style(v).map(Spec::BorderStyle))
}

pub(super) fn border_color(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    per_item(items, ids, out, |v| color(v).map(Spec::Color))
}

/// `<line-width> || <line-style> || <color>`; missing parts are initial
/// (`medium`, `none`, `currentcolor`).
fn width_style_color(
    items: &[&CV],
    cx: &ParseCtx,
    style_of: impl Fn(&CV) -> Option<BorderStyle>,
) -> Option<(MathExpr, BorderStyle, SpecColor)> {
    if items.is_empty() || items.len() > 3 {
        return None;
    }
    let (mut w, mut s, mut c) = (None, None, None);
    for v in items {
        if w.is_none() {
            if let Some(x) = line_width(v, cx) {
                w = Some(x);
                continue;
            }
        }
        if s.is_none() {
            if let Some(x) = style_of(v) {
                s = Some(x);
                continue;
            }
        }
        if c.is_none() {
            if let Some(x) = color(v) {
                c = Some(x);
                continue;
            }
        }
        return None;
    }
    Some((
        w.unwrap_or(MathExpr::px(3.0)),
        s.unwrap_or(BorderStyle::None),
        c.unwrap_or(SpecColor::CurrentColor),
    ))
}

/// `border`, `border-top`, `border-inline`, ...: longhands come in
/// (width, style, color) triples, one per side.
pub(super) fn border_shorthand(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let (w, s, c) = width_style_color(items, cx, line_style)?;
    if ids.len() % 3 != 0 {
        return None;
    }
    for side in ids.chunks(3) {
        let [wid, sid, cid] = side else {
            return None;
        };
        push(out, *wid, Spec::Length(w.clone()));
        push(out, *sid, Spec::BorderStyle(s));
        push(out, *cid, Spec::Color(c));
    }
    Some(())
}

/// `border-radius`: 1–4 horizontal radii; the `/ vertical` part is ignored.
pub(super) fn border_radius(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let horizontal: &[&CV] = match items.iter().position(|v| v.is_delim('/')) {
        Some(slash) => {
            let vertical = items.get(slash + 1..)?;
            if vertical.is_empty() || vertical.len() > 4 {
                return None;
            }
            // The vertical radii must still be valid.
            for v in vertical {
                length(v, NON_NEG_LP, cx)?;
            }
            items.get(..slash)?
        }
        None => items,
    };
    per_item(horizontal, ids, out, |v| {
        length(v, NON_NEG_LP, cx).map(Spec::Lp)
    })
}

/// `border-top-left-radius: <h> <v>?` (the vertical radius is ignored).
pub(super) fn corner_radius(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let (h, rest) = items.split_first()?;
    if rest.len() > 1 {
        return None;
    }
    let h = length(h, NON_NEG_LP, cx)?;
    if let Some(v) = rest.first() {
        length(v, NON_NEG_LP, cx)?;
    }
    push(out, only(ids)?, Spec::Lp(h));
    Some(())
}

/// Outline styles: border styles (except `hidden`) plus `auto` (≈ solid).
fn outline_line_style(v: &CV) -> Option<BorderStyle> {
    if v.is_ident("auto") {
        return Some(BorderStyle::Solid);
    }
    line_style(v).filter(|s| *s != BorderStyle::Hidden)
}

/// Outline colors also accept `invert` (≈ currentcolor).
fn outline_color_value(v: &CV) -> Option<SpecColor> {
    if v.is_ident("invert") {
        return Some(SpecColor::CurrentColor);
    }
    color(v)
}

pub(super) fn outline(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let mut items: Vec<&CV> = items.to_vec();
    // `invert` is only valid as the color.
    let mut invert = false;
    if let Some(pos) = items.iter().position(|v| v.is_ident("invert")) {
        items.remove(pos);
        invert = true;
    }
    let (w, s, c) = if items.is_empty() && invert {
        (
            MathExpr::px(3.0),
            BorderStyle::None,
            SpecColor::CurrentColor,
        )
    } else {
        width_style_color(&items, cx, outline_line_style)?
    };
    let [wid, sid, cid] = ids else {
        return None;
    };
    push(out, *wid, Spec::Length(w));
    push(out, *sid, Spec::BorderStyle(s));
    push(out, *cid, Spec::Color(c));
    Some(())
}

pub(super) fn outline_width(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::Length(line_width(single(items)?, cx)?),
    );
    Some(())
}

pub(super) fn outline_style(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::BorderStyle(outline_line_style(single(items)?)?),
    );
    Some(())
}

pub(super) fn outline_color(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::Color(outline_color_value(single(items)?)?),
    );
    Some(())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, spec};
    use super::*;
    use xbro_css::Color;

    #[test]
    fn border_shorthand_any_order() {
        for css in ["1px solid red", "solid red 1px", "red 1px solid"] {
            assert_eq!(
                spec("border", css, LonghandId::BorderLeftWidth),
                Some(Spec::Length(MathExpr::px(1.0))),
                "{css}"
            );
            assert_eq!(
                spec("border", css, LonghandId::BorderTopColor),
                Some(Spec::Color(SpecColor::Color(Color::rgb(255, 0, 0))))
            );
        }
        // Missing parts reset to initial values.
        assert_eq!(
            spec("border-top", "solid", LonghandId::BorderTopWidth),
            Some(Spec::Length(MathExpr::px(3.0)))
        );
        assert_eq!(
            spec("border-top", "solid", LonghandId::BorderTopColor),
            Some(Spec::Color(SpecColor::CurrentColor))
        );
        assert!(parse("border", "1px 2px solid").is_none());
        assert!(parse("border", "solid dashed").is_none());
        assert!(parse("border-top", "1px solid red blue").is_none());
        assert_eq!(parse("border", "none").map(|d| d.len()), Some(12));
        assert_eq!(
            parse("border-inline", "thick double").map(|d| d.len()),
            Some(6)
        );
    }

    #[test]
    fn border_lists() {
        assert_eq!(
            spec(
                "border-width",
                "thin medium thick",
                LonghandId::BorderLeftWidth
            ),
            Some(Spec::Length(MathExpr::px(3.0)))
        );
        assert_eq!(
            spec(
                "border-width",
                "thin medium thick",
                LonghandId::BorderBottomWidth
            ),
            Some(Spec::Length(MathExpr::px(5.0)))
        );
        assert_eq!(
            spec("border-style", "solid dotted", LonghandId::BorderLeftStyle),
            Some(Spec::BorderStyle(BorderStyle::Dotted))
        );
        assert_eq!(
            spec(
                "border-color",
                "red currentcolor",
                LonghandId::BorderRightColor
            ),
            Some(Spec::Color(SpecColor::CurrentColor))
        );
        assert!(parse("border-width", "10%").is_none());
        assert!(parse("border-width", "-1px").is_none());
        assert_eq!(
            spec(
                "border-block-width",
                "1px 2px",
                LonghandId::BorderBottomWidth
            ),
            Some(Spec::Length(MathExpr::px(2.0)))
        );
    }

    #[test]
    fn radii() {
        assert_eq!(
            spec(
                "border-radius",
                "1px 2px / 5px",
                LonghandId::BorderBottomLeftRadius
            ),
            Some(Spec::Lp(MathExpr::px(2.0)))
        );
        assert_eq!(
            spec("border-radius", "50%", LonghandId::BorderTopRightRadius),
            Some(Spec::Lp(MathExpr::percent(50.0)))
        );
        assert!(parse("border-radius", "1px /").is_none());
        assert!(parse("border-radius", "-1px").is_none());
        assert_eq!(
            spec(
                "border-top-left-radius",
                "4px 8px",
                LonghandId::BorderTopLeftRadius
            ),
            Some(Spec::Lp(MathExpr::px(4.0)))
        );
        assert_eq!(
            spec(
                "border-end-start-radius",
                "3px",
                LonghandId::BorderBottomLeftRadius
            ),
            Some(Spec::Lp(MathExpr::px(3.0)))
        );
    }

    #[test]
    fn outlines() {
        assert_eq!(
            spec("outline", "0", LonghandId::OutlineWidth),
            Some(Spec::Length(MathExpr::ZERO))
        );
        assert_eq!(
            spec("outline", "none", LonghandId::OutlineStyle),
            Some(Spec::BorderStyle(BorderStyle::None))
        );
        assert_eq!(
            spec("outline", "2px auto blue", LonghandId::OutlineStyle),
            Some(Spec::BorderStyle(BorderStyle::Solid))
        );
        assert_eq!(
            spec("outline", "thin dotted invert", LonghandId::OutlineColor),
            Some(Spec::Color(SpecColor::CurrentColor))
        );
        assert!(parse("outline-style", "hidden").is_none());
        assert_eq!(
            spec("outline-width", "thick", LonghandId::OutlineWidth),
            Some(Spec::Length(MathExpr::px(5.0)))
        );
    }
}
