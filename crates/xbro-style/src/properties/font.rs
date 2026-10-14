//! Fonts: `font-family`, `font-size`, `font-weight`, `font-style`,
//! `line-height` and the `font` shorthand.

use std::sync::Arc;

use xbro_css::{ComponentValue as CV, Token};

use super::{only, push, LonghandId, Out, Spec, SpecFontSize, SpecFontWeight, SpecLineHeight};
use crate::computed::FontStyle;
use crate::values::{keyword, length, number, single, split_on_commas, ParseCtx, NON_NEG_LP};

/// Absolute font-size keywords in px.
const SIZE_KEYWORDS: &[(&str, f32)] = &[
    ("xx-small", 9.0),
    ("x-small", 10.0),
    ("small", 13.0),
    ("medium", 16.0),
    ("large", 18.0),
    ("x-large", 24.0),
    ("xx-large", 32.0),
    ("xxx-large", 48.0),
    ("-webkit-xxx-large", 48.0),
];

/// `font-family` list: strings or identifier sequences, lowercased.
pub(crate) fn family_list(items: &[&CV]) -> Option<Arc<Vec<String>>> {
    let mut families = Vec::new();
    for part in split_on_commas(items) {
        let name = match part {
            [CV::Token(Token::String(s))] => s.trim().to_lowercase(),
            idents if !idents.is_empty() => {
                let words = idents
                    .iter()
                    .map(|v| v.ident())
                    .collect::<Option<Vec<_>>>()?;
                // CSS-wide keywords and `default` cannot be unquoted family names.
                if let [w] = words.as_slice() {
                    if [
                        "inherit",
                        "initial",
                        "unset",
                        "revert",
                        "revert-layer",
                        "default",
                    ]
                    .iter()
                    .any(|k| w.eq_ignore_ascii_case(k))
                    {
                        return None;
                    }
                }
                words.join(" ").to_lowercase()
            }
            _ => return None,
        };
        if name.is_empty() {
            return None;
        }
        families.push(name);
    }
    (!families.is_empty()).then(|| Arc::new(families))
}

pub(super) fn font_family(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(out, only(ids)?, Spec::FontFamily(family_list(items)?));
    Some(())
}

fn size_value(v: &CV, cx: &ParseCtx) -> Option<SpecFontSize> {
    if let Some(px) = keyword(v, SIZE_KEYWORDS) {
        return Some(SpecFontSize::Px(px));
    }
    if v.is_ident("smaller") {
        return Some(SpecFontSize::Smaller);
    }
    if v.is_ident("larger") {
        return Some(SpecFontSize::Larger);
    }
    length(v, NON_NEG_LP, cx).map(SpecFontSize::Length)
}

pub(super) fn font_size(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::FontSize(size_value(single(items)?, cx)?),
    );
    Some(())
}

fn weight_value(v: &CV) -> Option<SpecFontWeight> {
    if let Some(w) = keyword(
        v,
        &[
            ("normal", SpecFontWeight::Absolute(400.0)),
            ("bold", SpecFontWeight::Absolute(700.0)),
            ("bolder", SpecFontWeight::Bolder),
            ("lighter", SpecFontWeight::Lighter),
        ],
    ) {
        return Some(w);
    }
    match v {
        CV::Token(Token::Number { value, .. }) => (1.0..=1000.0)
            .contains(value)
            .then_some(SpecFontWeight::Absolute(*value)),
        // calc() results are clamped to the valid range.
        _ => number(v).map(|n| SpecFontWeight::Absolute(n.clamp(1.0, 1000.0))),
    }
}

pub(super) fn font_weight(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::FontWeight(weight_value(single(items)?)?),
    );
    Some(())
}

fn style_keyword(v: &CV) -> Option<FontStyle> {
    keyword(
        v,
        &[
            ("normal", FontStyle::Normal),
            ("italic", FontStyle::Italic),
            ("oblique", FontStyle::Oblique),
        ],
    )
}

fn is_angle(v: &CV) -> bool {
    crate::values::angle_degrees(v).is_some()
}

pub(super) fn font_style(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let style = match items {
        [v] => style_keyword(v)?,
        // `oblique <angle>`
        [v, a] if v.is_ident("oblique") && is_angle(a) => FontStyle::Oblique,
        _ => return None,
    };
    push(out, only(ids)?, Spec::FontStyle(style));
    Some(())
}

fn line_height_value(v: &CV, cx: &ParseCtx) -> Option<SpecLineHeight> {
    if v.is_ident("normal") {
        return Some(SpecLineHeight::Normal);
    }
    if let Some(n) = number(v) {
        return (n >= 0.0).then_some(SpecLineHeight::Number(n));
    }
    length(v, NON_NEG_LP, cx).map(SpecLineHeight::Length)
}

pub(super) fn line_height(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::LineHeight(line_height_value(single(items)?, cx)?),
    );
    Some(())
}

const STRETCH_KEYWORDS: &[&str] = &[
    "ultra-condensed",
    "extra-condensed",
    "condensed",
    "semi-condensed",
    "semi-expanded",
    "expanded",
    "extra-expanded",
    "ultra-expanded",
];

const SYSTEM_FONTS: &[&str] = &[
    "caption",
    "icon",
    "menu",
    "message-box",
    "small-caption",
    "status-bar",
    "-webkit-control",
    "-webkit-small-control",
    "-webkit-mini-control",
];

/// `font: [style || variant || weight || stretch]? size [/ line-height]? family`
/// or a system font keyword (≈ `13px sans-serif`).
pub(super) fn font(items: &[&CV], ids: &[LonghandId], _: &ParseCtx, out: &mut Out) -> Option<()> {
    let [style_id, weight_id, size_id, lh_id, family_id] = ids else {
        return None;
    };
    // The unitless length quirk never applies inside the `font` shorthand.
    let cx = ParseCtx::default();
    if let [v] = items {
        if SYSTEM_FONTS.iter().any(|k| v.is_ident(k)) {
            push(out, *style_id, Spec::FontStyle(FontStyle::Normal));
            push(
                out,
                *weight_id,
                Spec::FontWeight(SpecFontWeight::Absolute(400.0)),
            );
            push(out, *size_id, Spec::FontSize(SpecFontSize::Px(13.0)));
            push(out, *lh_id, Spec::LineHeight(SpecLineHeight::Normal));
            push(
                out,
                *family_id,
                Spec::FontFamily(Arc::new(vec!["sans-serif".into()])),
            );
            return Some(());
        }
    }
    let (mut style, mut weight) = (None, None);
    let (mut variant, mut stretch) = (false, false);
    let mut i = 0;
    let mut prefix_count = 0;
    while let Some(v) = items.get(i) {
        if prefix_count >= 4 {
            break;
        }
        if v.is_ident("normal") {
        } else if style.is_none() && style_keyword(v).is_some() {
            style = style_keyword(v);
            if v.is_ident("oblique") && items.get(i + 1).is_some_and(|a| is_angle(a)) {
                i += 1;
            }
        } else if !variant && v.is_ident("small-caps") {
            variant = true;
        } else if weight.is_none()
            && (matches!(v, CV::Token(Token::Number { .. })) || v.ident().is_some())
            && matches!(weight_value(v), Some(SpecFontWeight::Absolute(_)))
        {
            // Only absolute weights: `bolder` / `lighter` are invalid here.
            weight = weight_value(v);
        } else if !stretch && STRETCH_KEYWORDS.iter().any(|k| v.is_ident(k)) {
            stretch = true;
        } else {
            break;
        }
        prefix_count += 1;
        i += 1;
    }
    let size = size_value(items.get(i)?, &cx)?;
    i += 1;
    let mut lh = SpecLineHeight::Normal;
    if items.get(i).is_some_and(|v| v.is_delim('/')) {
        lh = line_height_value(items.get(i + 1)?, &cx)?;
        i += 2;
    }
    let family = family_list(items.get(i..)?)?;
    push(
        out,
        *style_id,
        Spec::FontStyle(style.unwrap_or(FontStyle::Normal)),
    );
    push(
        out,
        *weight_id,
        Spec::FontWeight(weight.unwrap_or(SpecFontWeight::Absolute(400.0))),
    );
    push(out, *size_id, Spec::FontSize(size));
    push(out, *lh_id, Spec::LineHeight(lh));
    push(out, *family_id, Spec::FontFamily(family));
    Some(())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, spec};
    use super::*;
    use crate::values::MathExpr;

    fn families(css: &str) -> Option<Vec<String>> {
        match spec("font-family", css, LonghandId::FontFamily)? {
            Spec::FontFamily(f) => Some((*f).clone()),
            _ => None,
        }
    }

    #[test]
    fn family_lists() {
        assert_eq!(
            families("\"Helvetica Neue\", Arial, Times New Roman, SANS-SERIF"),
            Some(vec![
                "helvetica neue".to_string(),
                "arial".into(),
                "times new roman".into(),
                "sans-serif".into()
            ])
        );
        assert_eq!(families("serif"), Some(vec!["serif".into()]));
        assert_eq!(families("a,,b"), None);
        assert_eq!(families("12px"), None);
        assert_eq!(families("default"), None);
        assert_eq!(families("Foo, inherit"), None);
    }

    #[test]
    fn sizes_weights_styles() {
        assert_eq!(
            spec("font-size", "x-large", LonghandId::FontSize),
            Some(Spec::FontSize(SpecFontSize::Px(24.0)))
        );
        assert_eq!(
            spec("font-size", "smaller", LonghandId::FontSize),
            Some(Spec::FontSize(SpecFontSize::Smaller))
        );
        assert!(parse("font-size", "-1px").is_none());
        assert_eq!(
            spec("font-weight", "bolder", LonghandId::FontWeight),
            Some(Spec::FontWeight(SpecFontWeight::Bolder))
        );
        assert_eq!(
            spec("font-weight", "1000", LonghandId::FontWeight),
            Some(Spec::FontWeight(SpecFontWeight::Absolute(1000.0)))
        );
        assert!(parse("font-weight", "0").is_none());
        assert!(parse("font-weight", "1001").is_none());
        assert_eq!(
            spec("font-weight", "calc(2000)", LonghandId::FontWeight),
            Some(Spec::FontWeight(SpecFontWeight::Absolute(1000.0)))
        );
        assert_eq!(
            spec("font-style", "oblique 10deg", LonghandId::FontStyle),
            Some(Spec::FontStyle(FontStyle::Oblique))
        );
        assert_eq!(
            spec("line-height", "1.5", LonghandId::LineHeight),
            Some(Spec::LineHeight(SpecLineHeight::Number(1.5)))
        );
        assert_eq!(
            spec("line-height", "120%", LonghandId::LineHeight),
            Some(Spec::LineHeight(SpecLineHeight::Length(MathExpr::percent(
                120.0
            ))))
        );
        assert!(parse("line-height", "-1").is_none());
    }

    #[test]
    fn shorthand() {
        let css = "italic small-caps bold 12px/1.5 \"Fira Sans\", serif";
        assert_eq!(
            spec("font", css, LonghandId::FontStyle),
            Some(Spec::FontStyle(FontStyle::Italic))
        );
        assert_eq!(
            spec("font", css, LonghandId::FontWeight),
            Some(Spec::FontWeight(SpecFontWeight::Absolute(700.0)))
        );
        assert_eq!(
            spec("font", css, LonghandId::FontSize),
            Some(Spec::FontSize(SpecFontSize::Length(MathExpr::px(12.0))))
        );
        assert_eq!(
            spec("font", css, LonghandId::LineHeight),
            Some(Spec::LineHeight(SpecLineHeight::Number(1.5)))
        );
        assert_eq!(
            spec("font", "600 1em sans-serif", LonghandId::FontWeight),
            Some(Spec::FontWeight(SpecFontWeight::Absolute(600.0)))
        );
        // Unspecified parts are reset.
        assert_eq!(
            spec("font", "13.333px sans-serif", LonghandId::LineHeight),
            Some(Spec::LineHeight(SpecLineHeight::Normal))
        );
        assert_eq!(
            spec("font", "menu", LonghandId::FontSize),
            Some(Spec::FontSize(SpecFontSize::Px(13.0)))
        );
        assert_eq!(
            spec("font", "condensed normal large serif", LonghandId::FontSize),
            Some(Spec::FontSize(SpecFontSize::Px(18.0)))
        );
        assert!(parse("font", "12px").is_none());
        assert!(parse("font", "bold serif").is_none());
        assert!(parse("font", "12px/ serif").is_none());
        assert!(parse("font", "bold bold 12px serif").is_none());
        // Relative weights are not allowed in the shorthand.
        assert!(parse("font", "bolder 12px serif").is_none());
        assert!(parse("font", "italic lighter 12px serif").is_none());
        assert!(parse("font-weight", "bolder").is_some());
    }
}
