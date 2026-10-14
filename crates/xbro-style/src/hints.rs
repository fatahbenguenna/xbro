//! HTML presentational hints (HTML Rendering §15): attributes such as
//! `bgcolor`, `width`, `align` or `<font color>` mapped to declarations that
//! the cascade treats as author rules with zero specificity, preceding all
//! author style sheets.

use std::sync::Arc;

use xbro_css::Color;
use xbro_dom::{Document, ElementData, NodeId};

use crate::computed::*;
use crate::properties::{
    LonghandId as L, PropertyDeclaration, Spec, SpecFontSize, SpecLpa, SpecVerticalAlign,
    WsCollapse,
};
use crate::values::{MathExpr, SpecColor};

/// Document-level inputs for hints.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct HintContext {
    /// `<body link>` color, applied to unvisited links.
    pub link_color: Option<Color>,
}

impl HintContext {
    pub fn new(doc: &Document) -> Self {
        let link_color = doc
            .body()
            .and_then(|b| doc.attr(b, "link"))
            .and_then(Color::parse_legacy);
        HintContext { link_color }
    }
}

fn push(out: &mut Vec<PropertyDeclaration>, id: L, spec: Spec) {
    out.push(PropertyDeclaration::Value(id, spec));
}

fn lpa(e: MathExpr) -> Spec {
    Spec::Lpa(SpecLpa::Lp(e))
}

fn skip_ws(s: &str) -> &str {
    s.trim_start_matches(|c: char| c.is_ascii_whitespace())
}

/// Leading ASCII digits of `s` as a number (saturating), and the rest.
fn digits(s: &str) -> Option<(f32, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let value = s
        .get(..end)?
        .bytes()
        .fold(0f32, |acc, b| (acc * 10.0 + f32::from(b - b'0')).min(1.0e7));
    Some((value, s.get(end..).unwrap_or("")))
}

/// HTML "rules for parsing non-negative integers".
pub(crate) fn non_negative_integer(s: &str) -> Option<f32> {
    let s = skip_ws(s);
    let s = s.strip_prefix('+').unwrap_or(s);
    digits(s).map(|(v, _)| v)
}

/// HTML "rules for parsing dimension values": a number of px, or a percentage.
pub(crate) fn dimension(s: &str) -> Option<MathExpr> {
    let s = skip_ws(s);
    let s = s.strip_prefix('+').unwrap_or(s);
    let (mut value, mut rest) = digits(s)?;
    if let Some(frac) = rest.strip_prefix('.') {
        let end = frac
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(frac.len());
        let mut scale = 0.1f32;
        for b in frac.get(..end).unwrap_or("").bytes().take(8) {
            value += f32::from(b - b'0') * scale;
            scale /= 10.0;
        }
        rest = frac.get(end..).unwrap_or("");
    }
    Some(if rest.starts_with('%') {
        MathExpr::percent(value)
    } else {
        MathExpr::px(value)
    })
}

/// Dimension values where zero is ignored (table and cell widths, ...).
fn non_zero_dimension(s: &str) -> Option<MathExpr> {
    dimension(s).filter(|e| *e != MathExpr::ZERO && *e != MathExpr::percent(0.0))
}

fn color_attr(e: &ElementData, name: &str) -> Option<Color> {
    e.attr(name).and_then(Color::parse_legacy)
}

fn bgcolor(e: &ElementData, out: &mut Vec<PropertyDeclaration>) {
    if let Some(c) = color_attr(e, "bgcolor") {
        push(out, L::BackgroundColor, Spec::Color(SpecColor::Color(c)));
    }
}

fn background(e: &ElementData, out: &mut Vec<PropertyDeclaration>) {
    if let Some(url) = e
        .attr("background")
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        push(
            out,
            L::BackgroundImage,
            Spec::BgImage(Some(crate::properties::SpecImage::Url(url.to_string()))),
        );
    }
}

fn text_align(e: &ElementData, out: &mut Vec<PropertyDeclaration>) {
    let Some(align) = e.attr("align").map(str::trim) else {
        return;
    };
    let value = match align.to_ascii_lowercase().as_str() {
        "left" => TextAlign::Left,
        "right" => TextAlign::Right,
        "center" | "middle" => TextAlign::Center,
        "justify" => TextAlign::Justify,
        _ => return,
    };
    push(
        out,
        L::TextAlign,
        Spec::TextAlign(crate::properties::SpecTextAlign::Align(value)),
    );
}

fn valign(e: &ElementData, out: &mut Vec<PropertyDeclaration>) {
    let Some(v) = e.attr("valign").map(str::trim) else {
        return;
    };
    let value = match v.to_ascii_lowercase().as_str() {
        "top" => VerticalAlign::Top,
        "middle" | "center" => VerticalAlign::Middle,
        "bottom" => VerticalAlign::Bottom,
        "baseline" => VerticalAlign::Baseline,
        _ => return,
    };
    push(
        out,
        L::VerticalAlign,
        Spec::VerticalAlign(SpecVerticalAlign::Keyword(value)),
    );
}

fn size_attrs(e: &ElementData, non_zero: bool, out: &mut Vec<PropertyDeclaration>) {
    let parse = if non_zero {
        non_zero_dimension
    } else {
        dimension
    };
    if let Some(w) = e.attr("width").and_then(parse) {
        push(out, L::Width, lpa(w));
    }
    if let Some(h) = e.attr("height").and_then(parse) {
        push(out, L::Height, lpa(h));
    }
}

/// `align` on replaced content: floats and vertical alignment.
fn replaced_align(e: &ElementData, out: &mut Vec<PropertyDeclaration>) {
    let Some(a) = e.attr("align").map(str::trim) else {
        return;
    };
    let va = match a.to_ascii_lowercase().as_str() {
        "left" => return push(out, L::Float, Spec::Float(Float::Left)),
        "right" => return push(out, L::Float, Spec::Float(Float::Right)),
        "top" => VerticalAlign::Top,
        "middle" | "center" | "absmiddle" | "abscenter" => VerticalAlign::Middle,
        "bottom" | "absbottom" => VerticalAlign::Bottom,
        "baseline" => VerticalAlign::Baseline,
        "texttop" => VerticalAlign::TextTop,
        _ => return,
    };
    push(
        out,
        L::VerticalAlign,
        Spec::VerticalAlign(SpecVerticalAlign::Keyword(va)),
    );
}

fn space_attrs(e: &ElementData, out: &mut Vec<PropertyDeclaration>) {
    if let Some(h) = e.attr("hspace").and_then(dimension) {
        push(out, L::MarginLeft, lpa(h.clone()));
        push(out, L::MarginRight, lpa(h));
    }
    if let Some(v) = e.attr("vspace").and_then(dimension) {
        push(out, L::MarginTop, lpa(v.clone()));
        push(out, L::MarginBottom, lpa(v));
    }
}

fn all_borders(
    out: &mut Vec<PropertyDeclaration>,
    width: f32,
    style: BorderStyle,
    color: Option<Color>,
) {
    for (w, s, c) in [
        (L::BorderTopWidth, L::BorderTopStyle, L::BorderTopColor),
        (
            L::BorderRightWidth,
            L::BorderRightStyle,
            L::BorderRightColor,
        ),
        (
            L::BorderBottomWidth,
            L::BorderBottomStyle,
            L::BorderBottomColor,
        ),
        (L::BorderLeftWidth, L::BorderLeftStyle, L::BorderLeftColor),
    ] {
        push(out, w, Spec::Length(MathExpr::px(width)));
        push(out, s, Spec::BorderStyle(style));
        if let Some(color) = color {
            push(out, c, Spec::Color(SpecColor::Color(color)));
        }
    }
}

/// `<img border>` / `<object border>`.
fn replaced_border(e: &ElementData, out: &mut Vec<PropertyDeclaration>) {
    if let Some(w) = e.attr("border").and_then(non_negative_integer) {
        all_borders(out, w, BorderStyle::Solid, None);
    }
}

/// Width of a table's `border` attribute (empty or unparsable → 1).
fn table_border_width(table: &ElementData) -> Option<f32> {
    table
        .attr("border")
        .map(|v| non_negative_integer(v).unwrap_or(1.0))
}

const GRAY: Color = Color::rgb(128, 128, 128);

fn nearest_table(doc: &Document, node: NodeId) -> Option<&ElementData> {
    let mut current = doc.parent_element(node);
    let mut guard = 0;
    while let Some(p) = current {
        guard += 1;
        if guard > 512 {
            return None;
        }
        let e = doc.element(p)?;
        if e.name == "table" {
            return Some(e);
        }
        current = doc.parent_element(p);
    }
    None
}

/// Legacy `<font size>`: 1..7, or ±N relative to 3.
pub(crate) fn legacy_font_size(s: &str) -> Option<f32> {
    let s = skip_ws(s);
    let (mode, rest) = match s.as_bytes().first() {
        Some(b'+') => (1, s.get(1..).unwrap_or("")),
        Some(b'-') => (-1, s.get(1..).unwrap_or("")),
        _ => (0, s),
    };
    let (value, _) = digits(rest)?;
    let size = match mode {
        1 => 3.0 + value,
        -1 => 3.0 - value,
        _ => value,
    };
    Some(match size.clamp(1.0, 7.0) as u8 {
        1 => 10.0,
        2 => 13.0,
        3 => 16.0,
        4 => 18.0,
        5 => 24.0,
        6 => 32.0,
        _ => 48.0,
    })
}

fn list_type(value: &str, ordered: bool, unordered: bool) -> Option<ListStyleType> {
    let v = value.trim();
    if ordered {
        // Case-sensitive per HTML.
        match v {
            "1" => return Some(ListStyleType::Decimal),
            "a" => return Some(ListStyleType::LowerAlpha),
            "A" => return Some(ListStyleType::UpperAlpha),
            "i" => return Some(ListStyleType::LowerRoman),
            "I" => return Some(ListStyleType::UpperRoman),
            _ => {}
        }
    }
    if unordered {
        match v.to_ascii_lowercase().as_str() {
            "disc" => return Some(ListStyleType::Disc),
            "circle" => return Some(ListStyleType::Circle),
            "square" => return Some(ListStyleType::Square),
            "none" => return Some(ListStyleType::None),
            _ => {}
        }
    }
    None
}

/// Appends the presentational hints of element `node` to `out`.
pub(crate) fn collect(
    doc: &Document,
    node: NodeId,
    e: &ElementData,
    cx: &HintContext,
    out: &mut Vec<PropertyDeclaration>,
) {
    // `embed[hidden]` stays inline with a zero size (UA sheet) instead.
    if e.name != "embed"
        && e.attr("hidden")
            .is_some_and(|v| !v.trim().eq_ignore_ascii_case("until-found"))
    {
        push(out, L::Display, Spec::Display(Display::None));
    }
    let is_image_input = || {
        e.name == "input"
            && e.attr("type")
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("image"))
    };
    match e.name.as_str() {
        "body" => {
            bgcolor(e, out);
            background(e, out);
            if let Some(c) = color_attr(e, "text") {
                push(out, L::Color, Spec::Color(SpecColor::Color(c)));
            }
            for (attr, ids) in [
                ("marginwidth", &[L::MarginLeft, L::MarginRight][..]),
                ("marginheight", &[L::MarginTop, L::MarginBottom][..]),
                ("leftmargin", &[L::MarginLeft][..]),
                ("rightmargin", &[L::MarginRight][..]),
                ("topmargin", &[L::MarginTop][..]),
                ("bottommargin", &[L::MarginBottom][..]),
            ] {
                if let Some(v) = e.attr(attr).and_then(non_negative_integer) {
                    for id in ids {
                        push(out, *id, lpa(MathExpr::px(v)));
                    }
                }
            }
        }
        "table" => {
            bgcolor(e, out);
            background(e, out);
            size_attrs(e, true, out);
            if let Some(a) = e.attr("align").map(str::trim) {
                match a.to_ascii_lowercase().as_str() {
                    "left" => push(out, L::Float, Spec::Float(Float::Left)),
                    "right" => push(out, L::Float, Spec::Float(Float::Right)),
                    "center" => {
                        push(out, L::MarginLeft, Spec::Lpa(SpecLpa::Auto));
                        push(out, L::MarginRight, Spec::Lpa(SpecLpa::Auto));
                    }
                    _ => {}
                }
            }
            let border_color = color_attr(e, "bordercolor");
            if let Some(w) = table_border_width(e).filter(|w| *w > 0.0) {
                all_borders(out, w, BorderStyle::Outset, border_color);
            } else if let Some(c) = border_color {
                for id in [
                    L::BorderTopColor,
                    L::BorderRightColor,
                    L::BorderBottomColor,
                    L::BorderLeftColor,
                ] {
                    push(out, id, Spec::Color(SpecColor::Color(c)));
                }
            }
            if let Some(s) = e.attr("cellspacing").and_then(non_negative_integer) {
                push(
                    out,
                    L::BorderSpacing,
                    Spec::BorderSpacing(MathExpr::px(s), MathExpr::px(s)),
                );
            }
        }
        "thead" | "tbody" | "tfoot" | "tr" => {
            bgcolor(e, out);
            background(e, out);
            text_align(e, out);
            valign(e, out);
            if e.name == "tr" {
                if let Some(h) = e.attr("height").and_then(non_zero_dimension) {
                    push(out, L::Height, lpa(h));
                }
            }
        }
        "td" | "th" => {
            bgcolor(e, out);
            background(e, out);
            text_align(e, out);
            valign(e, out);
            size_attrs(e, true, out);
            if e.has_attr("nowrap") {
                push(
                    out,
                    L::WhiteSpaceCollapse,
                    Spec::WsCollapse(WsCollapse::Collapse),
                );
                push(out, L::TextWrapMode, Spec::Bool(true));
            }
            if let Some(table) = nearest_table(doc, node) {
                if table_border_width(table).is_some_and(|w| w > 0.0) {
                    let color = color_attr(table, "bordercolor").unwrap_or(GRAY);
                    all_borders(out, 1.0, BorderStyle::Inset, Some(color));
                }
                if let Some(p) = table.attr("cellpadding").and_then(non_negative_integer) {
                    for id in [
                        L::PaddingTop,
                        L::PaddingRight,
                        L::PaddingBottom,
                        L::PaddingLeft,
                    ] {
                        push(out, id, Spec::Lp(MathExpr::px(p)));
                    }
                }
            }
        }
        "col" | "colgroup" => {
            if let Some(w) = e.attr("width").and_then(non_zero_dimension) {
                push(out, L::Width, lpa(w));
            }
            valign(e, out);
        }
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "caption" | "legend" => {
            text_align(e, out);
        }
        "img" | "object" | "embed" => {
            size_attrs(e, false, out);
            replaced_align(e, out);
            space_attrs(e, out);
            if e.name != "embed" {
                replaced_border(e, out);
            }
        }
        "iframe" => {
            size_attrs(e, false, out);
            replaced_align(e, out);
            space_attrs(e, out);
        }
        "video" | "canvas" | "applet" => size_attrs(e, false, out),
        "marquee" => {
            size_attrs(e, false, out);
            bgcolor(e, out);
            space_attrs(e, out);
        }
        "input" if is_image_input() => {
            size_attrs(e, false, out);
            replaced_align(e, out);
            space_attrs(e, out);
            replaced_border(e, out);
        }
        "hr" => hr(e, out),
        "font" => {
            if let Some(c) = color_attr(e, "color") {
                push(out, L::Color, Spec::Color(SpecColor::Color(c)));
            }
            if let Some(face) = e.attr("face") {
                let families: Vec<String> = face
                    .split(',')
                    .map(|f| f.trim().to_lowercase())
                    .filter(|f| !f.is_empty())
                    .collect();
                if !families.is_empty() {
                    push(out, L::FontFamily, Spec::FontFamily(Arc::new(families)));
                }
            }
            if let Some(px) = e.attr("size").and_then(legacy_font_size) {
                push(out, L::FontSize, Spec::FontSize(SpecFontSize::Px(px)));
            }
        }
        "ol" | "ul" | "li" => {
            let ordered = e.name != "ul";
            let unordered = e.name != "ol";
            if let Some(t) = e
                .attr("type")
                .and_then(|v| list_type(v, ordered, unordered))
            {
                push(out, L::ListStyleType, Spec::ListStyleType(t));
            }
        }
        "br" => {
            if let Some(c) = e.attr("clear") {
                let clear = match c.trim().to_ascii_lowercase().as_str() {
                    "left" => Some(Clear::Left),
                    "right" => Some(Clear::Right),
                    "all" | "both" => Some(Clear::Both),
                    _ => None,
                };
                if let Some(clear) = clear {
                    push(out, L::Clear, Spec::Clear(clear));
                }
            }
        }
        "a" => {
            if let Some(c) = cx.link_color.filter(|_| e.has_attr("href")) {
                push(out, L::Color, Spec::Color(SpecColor::Color(c)));
            }
        }
        _ => {}
    }
}

/// `<hr>` attributes (HTML §15.3.10).
fn hr(e: &ElementData, out: &mut Vec<PropertyDeclaration>) {
    if let Some(a) = e.attr("align") {
        let margins = match a.trim().to_ascii_lowercase().as_str() {
            "left" => Some((SpecLpa::Lp(MathExpr::ZERO), SpecLpa::Auto)),
            "right" => Some((SpecLpa::Auto, SpecLpa::Lp(MathExpr::ZERO))),
            "center" => Some((SpecLpa::Auto, SpecLpa::Auto)),
            _ => None,
        };
        if let Some((left, right)) = margins {
            push(out, L::MarginLeft, Spec::Lpa(left));
            push(out, L::MarginRight, Spec::Lpa(right));
        }
    }
    let color = color_attr(e, "color");
    let solid = color.is_some() || e.has_attr("noshade");
    if let Some(c) = color {
        push(out, L::Color, Spec::Color(SpecColor::Color(c)));
    }
    if solid {
        for id in [
            L::BorderTopStyle,
            L::BorderRightStyle,
            L::BorderBottomStyle,
            L::BorderLeftStyle,
        ] {
            push(out, id, Spec::BorderStyle(BorderStyle::Solid));
        }
    }
    if let Some(size) = e.attr("size").and_then(non_negative_integer) {
        if solid {
            for id in [
                L::BorderTopWidth,
                L::BorderRightWidth,
                L::BorderBottomWidth,
                L::BorderLeftWidth,
            ] {
                push(out, id, Spec::Length(MathExpr::px(size / 2.0)));
            }
        } else if size == 1.0 {
            push(out, L::BorderBottomWidth, Spec::Length(MathExpr::ZERO));
        } else if size > 1.0 {
            push(out, L::Height, lpa(MathExpr::px(size - 2.0)));
        }
    }
    if let Some(w) = e.attr("width").and_then(dimension) {
        push(out, L::Width, lpa(w));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_microsyntaxes() {
        assert_eq!(dimension(" 50%"), Some(MathExpr::percent(50.0)));
        assert_eq!(dimension("120px"), Some(MathExpr::px(120.0)));
        assert_eq!(dimension("12.5"), Some(MathExpr::px(12.5)));
        assert_eq!(dimension("x12"), None);
        assert_eq!(dimension(""), None);
        assert_eq!(non_zero_dimension("0"), None);
        assert_eq!(non_negative_integer(" +7abc"), Some(7.0));
        assert_eq!(non_negative_integer("-1"), None);
        assert!(non_negative_integer("99999999999999999999").is_some_and(|v| v.is_finite()));
    }

    #[test]
    fn font_sizes() {
        assert_eq!(legacy_font_size("1"), Some(10.0));
        assert_eq!(legacy_font_size("2"), Some(13.0));
        assert_eq!(legacy_font_size("3"), Some(16.0));
        assert_eq!(legacy_font_size("4"), Some(18.0));
        assert_eq!(legacy_font_size("5"), Some(24.0));
        assert_eq!(legacy_font_size("6"), Some(32.0));
        assert_eq!(legacy_font_size("7"), Some(48.0));
        assert_eq!(legacy_font_size("+2"), Some(24.0));
        assert_eq!(legacy_font_size("-1"), Some(13.0));
        assert_eq!(legacy_font_size("-10"), Some(10.0));
        assert_eq!(legacy_font_size("12"), Some(48.0));
        assert_eq!(legacy_font_size("big"), None);
    }

    #[test]
    fn list_types() {
        assert_eq!(list_type("A", true, false), Some(ListStyleType::UpperAlpha));
        assert_eq!(list_type("I", true, true), Some(ListStyleType::UpperRoman));
        assert_eq!(
            list_type("SQUARE", false, true),
            Some(ListStyleType::Square)
        );
        assert_eq!(list_type("square", true, false), None);
        assert_eq!(list_type("x", true, true), None);
    }
}
