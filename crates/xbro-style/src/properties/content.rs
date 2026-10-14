//! Generated content, list styles and the cursor.

use xbro_css::{ComponentValue as CV, Token};

use super::{only, push, LonghandId, Out, Spec, SpecContent, SpecContentItem};
use crate::computed::{Cursor, ListStylePosition, ListStyleType};
use crate::values::{image_set_url, keyword, single, split_on_commas, url, ParseCtx};

/// Largest `content` value accepted (items / bytes of text). The value is
/// copied into every pseudo-element it applies to, so a huge one would cost
/// elements × size; such declarations are rejected as invalid.
const MAX_CONTENT_ITEMS: usize = 256;
const MAX_CONTENT_TEXT: usize = 64 * 1024;

/// `content: normal | none | [<string> | attr() | url() | counter() | quotes]+ [/ alt]?`.
pub(super) fn content(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let id = only(ids)?;
    if let [v] = items {
        if v.is_ident("normal") || v.is_ident("none") {
            push(out, id, Spec::Content(SpecContent::Normal));
            return Some(());
        }
    }
    let mut result = Vec::new();
    for (i, v) in items.iter().enumerate() {
        if v.is_delim('/') {
            // Alternative text follows: must be strings / attr(); ignored.
            if i == 0 || items.len() == i + 1 {
                return None;
            }
            break;
        }
        match v {
            CV::Token(Token::String(s)) => result.push(SpecContentItem::Text(s.clone())),
            CV::Function { name, args } if name.eq_ignore_ascii_case("attr") => {
                result.push(attr(args)?);
            }
            CV::Function { name, .. }
                if name.eq_ignore_ascii_case("counter")
                    || name.eq_ignore_ascii_case("counters") =>
            {
                result.push(SpecContentItem::Text(String::new()));
            }
            CV::Function { name, .. } if name.to_ascii_lowercase().contains("gradient") => {}
            _ => {
                if let Some(u) = url(v).or_else(|| image_set_url(v)) {
                    result.push(SpecContentItem::Image(u));
                    continue;
                }
                let k = v.ident()?.to_ascii_lowercase();
                match k.as_str() {
                    "open-quote" => result.push(SpecContentItem::OpenQuote),
                    "close-quote" => result.push(SpecContentItem::CloseQuote),
                    "no-open-quote" | "no-close-quote" => {}
                    _ => return None,
                }
            }
        }
    }
    let text: usize = result
        .iter()
        .map(|item| match item {
            SpecContentItem::Text(t) | SpecContentItem::Image(t) => t.len(),
            SpecContentItem::Attr { name, fallback } => name.len() + fallback.len(),
            SpecContentItem::OpenQuote | SpecContentItem::CloseQuote => 0,
        })
        .sum();
    if result.len() > MAX_CONTENT_ITEMS || text > MAX_CONTENT_TEXT {
        return None;
    }
    push(out, id, Spec::Content(SpecContent::Items(result)));
    Some(())
}

/// `attr(name [type]? [, fallback]?)`.
fn attr(args: &[CV]) -> Option<SpecContentItem> {
    let items: Vec<&CV> = args.iter().filter(|v| !v.is_whitespace()).collect();
    let parts = split_on_commas(&items);
    let (first, fallback) = match parts.as_slice() {
        [first] => (*first, None),
        [first, fb] => (*first, Some(*fb)),
        _ => return None,
    };
    let name = first.first()?.ident()?.to_ascii_lowercase();
    let fallback = match fallback {
        None => String::new(),
        Some([CV::Token(Token::String(s))]) => s.clone(),
        Some(_) => return None,
    };
    Some(SpecContentItem::Attr { name, fallback })
}

/// `cursor: [<url> [<x> <y>]?,]* <keyword>`.
pub(super) fn cursor(items: &[&CV], ids: &[LonghandId], _: &ParseCtx, out: &mut Out) -> Option<()> {
    let parts = split_on_commas(items);
    let (last, images) = parts.split_last()?;
    for img in images {
        let (u, rest) = img.split_first()?;
        url(u).or_else(|| image_set_url(u))?;
        if !(rest.is_empty() || rest.len() == 2) {
            return None;
        }
    }
    let c = keyword(
        single(last)?,
        &[
            ("auto", Cursor::Auto),
            ("default", Cursor::Default),
            ("none", Cursor::Default),
            ("context-menu", Cursor::Default),
            ("copy", Cursor::Default),
            ("alias", Cursor::Default),
            ("pointer", Cursor::Pointer),
            ("hand", Cursor::Pointer),
            ("zoom-in", Cursor::Pointer),
            ("zoom-out", Cursor::Pointer),
            ("text", Cursor::Text),
            ("vertical-text", Cursor::Text),
            ("wait", Cursor::Wait),
            ("progress", Cursor::Progress),
            ("help", Cursor::Help),
            ("move", Cursor::Move),
            ("all-scroll", Cursor::Move),
            ("col-resize", Cursor::Move),
            ("row-resize", Cursor::Move),
            ("n-resize", Cursor::Move),
            ("e-resize", Cursor::Move),
            ("s-resize", Cursor::Move),
            ("w-resize", Cursor::Move),
            ("ne-resize", Cursor::Move),
            ("nw-resize", Cursor::Move),
            ("se-resize", Cursor::Move),
            ("sw-resize", Cursor::Move),
            ("ew-resize", Cursor::Move),
            ("ns-resize", Cursor::Move),
            ("nesw-resize", Cursor::Move),
            ("nwse-resize", Cursor::Move),
            ("not-allowed", Cursor::NotAllowed),
            ("no-drop", Cursor::NotAllowed),
            ("crosshair", Cursor::Crosshair),
            ("cell", Cursor::Crosshair),
            ("grab", Cursor::Grab),
            ("grabbing", Cursor::Grab),
            ("-webkit-grab", Cursor::Grab),
            ("-webkit-grabbing", Cursor::Grab),
            ("-moz-grab", Cursor::Grab),
            ("-moz-grabbing", Cursor::Grab),
            ("-webkit-zoom-in", Cursor::Pointer),
            ("-webkit-zoom-out", Cursor::Pointer),
        ],
    )?;
    push(out, only(ids)?, Spec::Cursor(c));
    Some(())
}

/// Counter style names that are numeric (rendered as decimal by xbro).
const NUMERIC_STYLES: &[&str] = &[
    "arabic-indic",
    "armenian",
    "upper-armenian",
    "lower-armenian",
    "bengali",
    "cambodian",
    "khmer",
    "cjk-decimal",
    "devanagari",
    "georgian",
    "gujarati",
    "gurmukhi",
    "hebrew",
    "kannada",
    "lao",
    "malayalam",
    "mongolian",
    "myanmar",
    "oriya",
    "persian",
    "tamil",
    "telugu",
    "thai",
    "tibetan",
    "japanese-informal",
    "japanese-formal",
    "korean-hangul-formal",
    "korean-hanja-informal",
    "korean-hanja-formal",
    "simp-chinese-informal",
    "simp-chinese-formal",
    "trad-chinese-informal",
    "trad-chinese-formal",
    "cjk-ideographic",
    "ethiopic-numeric",
];

/// A `list-style-type` value. Unknown counter-style names fall back to
/// `decimal` when they look numeric and to `disc` otherwise; string markers
/// are approximated by `disc`.
fn list_type(v: &CV) -> Option<ListStyleType> {
    if matches!(v, CV::Token(Token::String(_))) {
        return Some(ListStyleType::Disc);
    }
    if let CV::Function { name, .. } = v {
        // `symbols()`: approximated.
        return name
            .eq_ignore_ascii_case("symbols")
            .then_some(ListStyleType::Disc);
    }
    let k = v.ident()?.to_ascii_lowercase();
    Some(match k.as_str() {
        "disc" => ListStyleType::Disc,
        "circle" => ListStyleType::Circle,
        "square" => ListStyleType::Square,
        "decimal" => ListStyleType::Decimal,
        "decimal-leading-zero" => ListStyleType::DecimalLeadingZero,
        "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
        "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
        "lower-roman" => ListStyleType::LowerRoman,
        "upper-roman" => ListStyleType::UpperRoman,
        "lower-greek" => ListStyleType::LowerGreek,
        "none" => ListStyleType::None,
        // CSS-wide keywords are handled earlier; `default` is reserved.
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" | "inside"
        | "outside" => return None,
        other if other.contains("decimal") || NUMERIC_STYLES.contains(&other) => {
            ListStyleType::Decimal
        }
        _ => ListStyleType::Disc,
    })
}

fn list_position(v: &CV) -> Option<ListStylePosition> {
    keyword(
        v,
        &[
            ("outside", ListStylePosition::Outside),
            ("inside", ListStylePosition::Inside),
        ],
    )
}

/// `none` → `Some(None)`, `url()` → `Some(Some(url))`.
fn list_image(v: &CV) -> Option<Option<String>> {
    if v.is_ident("none") {
        return Some(None);
    }
    url(v).or_else(|| image_set_url(v)).map(Some)
}

pub(super) fn list_style_type(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::ListStyleType(list_type(single(items)?)?),
    );
    Some(())
}

pub(super) fn list_style_position(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::ListStylePosition(list_position(single(items)?)?),
    );
    Some(())
}

pub(super) fn list_style_image(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::ListStyleImage(list_image(single(items)?)?),
    );
    Some(())
}

/// `list-style: <type> || <position> || <image>` (with `none` disambiguation).
pub(super) fn list_style(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let [type_id, pos_id, image_id] = ids else {
        return None;
    };
    let (mut ty, mut pos, mut image) = (None, None, None);
    let mut nones = 0;
    for v in items {
        if v.is_ident("none") {
            nones += 1;
            continue;
        }
        if pos.is_none() {
            if let Some(p) = list_position(v) {
                pos = Some(p);
                continue;
            }
        }
        if image.is_none() {
            if let Some(Some(u)) = list_image(v) {
                image = Some(Some(u));
                continue;
            }
        }
        if ty.is_none() {
            if let Some(t) = list_type(v) {
                ty = Some(t);
                continue;
            }
        }
        return None;
    }
    // Each `none` fills whichever of type / image is still unset.
    for _ in 0..nones {
        if ty.is_none() {
            ty = Some(ListStyleType::None);
        } else if image.is_none() {
            image = Some(None);
        } else {
            return None;
        }
    }
    push(
        out,
        *type_id,
        Spec::ListStyleType(ty.unwrap_or(ListStyleType::Disc)),
    );
    push(
        out,
        *pos_id,
        Spec::ListStylePosition(pos.unwrap_or_default()),
    );
    push(out, *image_id, Spec::ListStyleImage(image.unwrap_or(None)));
    Some(())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, spec};
    use super::*;

    fn content_of(css: &str) -> Option<SpecContent> {
        match spec("content", css, LonghandId::Content)? {
            Spec::Content(c) => Some(c),
            _ => None,
        }
    }

    #[test]
    fn content_values() {
        assert_eq!(content_of("none"), Some(SpecContent::Normal));
        assert_eq!(content_of("normal"), Some(SpecContent::Normal));
        assert_eq!(
            content_of("\"(\" attr(href) \")\""),
            Some(SpecContent::Items(vec![
                SpecContentItem::Text("(".into()),
                SpecContentItem::Attr {
                    name: "href".into(),
                    fallback: String::new()
                },
                SpecContentItem::Text(")".into()),
            ]))
        );
        assert_eq!(
            content_of("open-quote counter(x) close-quote no-open-quote"),
            Some(SpecContent::Items(vec![
                SpecContentItem::OpenQuote,
                SpecContentItem::Text(String::new()),
                SpecContentItem::CloseQuote,
            ]))
        );
        assert_eq!(
            content_of("url(i.png) / \"alt\""),
            Some(SpecContent::Items(vec![SpecContentItem::Image(
                "i.png".into()
            )]))
        );
        assert_eq!(
            content_of("attr(data-x, \"fb\")"),
            Some(SpecContent::Items(vec![SpecContentItem::Attr {
                name: "data-x".into(),
                fallback: "fb".into()
            }]))
        );
        assert_eq!(
            content_of("\"\""),
            Some(SpecContent::Items(vec![SpecContentItem::Text(
                String::new()
            )]))
        );
        assert!(parse("content", "bogus").is_none());
        assert!(parse("content", "12px").is_none());
    }

    #[test]
    fn huge_content_values_are_invalid() {
        let items = |n: usize| vec!["\"x\""; n].join(" ");
        assert!(parse("content", &items(MAX_CONTENT_ITEMS)).is_some());
        assert!(parse("content", &items(MAX_CONTENT_ITEMS + 1)).is_none());
        let text = |n: usize| format!("\"{}\"", "y".repeat(n));
        assert!(parse("content", &text(MAX_CONTENT_TEXT)).is_some());
        assert!(parse("content", &text(MAX_CONTENT_TEXT + 1)).is_none());
    }

    #[test]
    fn cursors() {
        assert_eq!(
            spec(
                "cursor",
                "url(a.cur) 4 4, url(b.png), pointer",
                LonghandId::Cursor
            ),
            Some(Spec::Cursor(Cursor::Pointer))
        );
        assert_eq!(
            spec("cursor", "ns-resize", LonghandId::Cursor),
            Some(Spec::Cursor(Cursor::Move))
        );
        assert!(parse("cursor", "url(a.cur)").is_none());
        assert!(parse("cursor", "frobnicate").is_none());
    }

    #[test]
    fn list_styles() {
        let ty = |css: &str| spec("list-style-type", css, LonghandId::ListStyleType);
        assert_eq!(
            ty("lower-latin"),
            Some(Spec::ListStyleType(ListStyleType::LowerAlpha))
        );
        assert_eq!(
            ty("armenian"),
            Some(Spec::ListStyleType(ListStyleType::Decimal))
        );
        assert_eq!(
            ty("disclosure-open"),
            Some(Spec::ListStyleType(ListStyleType::Disc))
        );
        assert_eq!(ty("\"-\""), Some(Spec::ListStyleType(ListStyleType::Disc)));
        let ls = |css: &str, id| spec("list-style", css, id);
        assert_eq!(
            ls("none", LonghandId::ListStyleType),
            Some(Spec::ListStyleType(ListStyleType::None))
        );
        assert_eq!(
            ls("none", LonghandId::ListStyleImage),
            Some(Spec::ListStyleImage(None))
        );
        assert_eq!(
            ls("square inside", LonghandId::ListStylePosition),
            Some(Spec::ListStylePosition(ListStylePosition::Inside))
        );
        assert_eq!(
            ls("url(b.png) none", LonghandId::ListStyleType),
            Some(Spec::ListStyleType(ListStyleType::None))
        );
        assert_eq!(
            ls("url(b.png)", LonghandId::ListStyleImage),
            Some(Spec::ListStyleImage(Some("b.png".into())))
        );
        assert!(parse("list-style", "disc none none").is_none());
        assert!(parse("list-style", "inside outside").is_none());
    }
}
