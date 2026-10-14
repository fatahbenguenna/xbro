//! `@supports` condition evaluation (CSS Conditional Rules Level 3/4).

use super::ComponentValue as CV;
use crate::color::Color;
use crate::selector;
use crate::tokenizer::Token;
use crate::values::trim;

/// Properties xbro's style system understands (lowercase).
const SUPPORTED_PROPERTIES: &[&str] = &[
    "align-content",
    "align-items",
    "align-self",
    "aspect-ratio",
    "background",
    "background-attachment",
    "background-clip",
    "background-color",
    "background-image",
    "background-origin",
    "background-position",
    "background-position-x",
    "background-position-y",
    "background-repeat",
    "background-size",
    "block-size",
    "border",
    "border-block",
    "border-block-color",
    "border-block-end",
    "border-block-end-color",
    "border-block-end-style",
    "border-block-end-width",
    "border-block-start",
    "border-block-start-color",
    "border-block-start-style",
    "border-block-start-width",
    "border-block-style",
    "border-block-width",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-end-end-radius",
    "border-end-start-radius",
    "border-inline",
    "border-inline-color",
    "border-inline-end",
    "border-inline-end-color",
    "border-inline-end-style",
    "border-inline-end-width",
    "border-inline-start",
    "border-inline-start-color",
    "border-inline-start-style",
    "border-inline-start-width",
    "border-inline-style",
    "border-inline-width",
    "border-left",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-spacing",
    "border-start-end-radius",
    "border-start-start-radius",
    "border-style",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "bottom",
    "box-sizing",
    "caption-side",
    "clear",
    "color",
    "column-gap",
    "content",
    "counter-increment",
    "counter-reset",
    "counter-set",
    "direction",
    "display",
    "empty-cells",
    "flex",
    "flex-basis",
    "flex-direction",
    "flex-flow",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "float",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "gap",
    "grid",
    "grid-area",
    "grid-auto-columns",
    "grid-auto-flow",
    "grid-auto-rows",
    "grid-column",
    "grid-column-end",
    "grid-column-gap",
    "grid-column-start",
    "grid-gap",
    "grid-row",
    "grid-row-end",
    "grid-row-gap",
    "grid-row-start",
    "grid-template",
    "grid-template-areas",
    "grid-template-columns",
    "grid-template-rows",
    "height",
    "inline-size",
    "inset",
    "inset-block",
    "inset-block-end",
    "inset-block-start",
    "inset-inline",
    "inset-inline-end",
    "inset-inline-start",
    "justify-content",
    "justify-items",
    "justify-self",
    "left",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "margin",
    "margin-block",
    "margin-block-end",
    "margin-block-start",
    "margin-bottom",
    "margin-inline",
    "margin-inline-end",
    "margin-inline-start",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-block-size",
    "max-height",
    "max-inline-size",
    "max-width",
    "min-block-size",
    "min-height",
    "min-inline-size",
    "min-width",
    "opacity",
    "order",
    "outline",
    "outline-color",
    "outline-offset",
    "outline-style",
    "outline-width",
    "overflow",
    "overflow-wrap",
    "overflow-x",
    "overflow-y",
    "padding",
    "padding-block",
    "padding-block-end",
    "padding-block-start",
    "padding-bottom",
    "padding-inline",
    "padding-inline-end",
    "padding-inline-start",
    "padding-left",
    "padding-right",
    "padding-top",
    "place-content",
    "place-items",
    "place-self",
    "position",
    "quotes",
    "right",
    "row-gap",
    "tab-size",
    "table-layout",
    "text-align",
    "text-decoration",
    "text-decoration-color",
    "text-decoration-line",
    "text-decoration-style",
    "text-indent",
    "text-transform",
    "text-wrap",
    "top",
    "unicode-bidi",
    "vertical-align",
    "visibility",
    "white-space",
    "white-space-collapse",
    "width",
    "word-break",
    "word-spacing",
    "word-wrap",
    "z-index",
];

/// Properties whose value is a single color.
const COLOR_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "border-color",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "outline-color",
    "text-decoration-color",
];

const DISPLAY_KEYWORDS: &[&str] = &[
    "none",
    "block",
    "inline",
    "inline-block",
    "flex",
    "inline-flex",
    "grid",
    "inline-grid",
    "flow",
    "flow-root",
    "list-item",
    "contents",
    "table",
    "inline-table",
    "table-row",
    "table-cell",
    "table-column",
    "table-column-group",
    "table-row-group",
    "table-header-group",
    "table-footer-group",
    "table-caption",
];

const CSS_WIDE_KEYWORDS: &[&str] = &["inherit", "initial", "unset", "revert", "revert-layer"];

/// True for properties xbro's style system understands (ASCII
/// case-insensitive): box model, colors, fonts, text, flex, grid, background,
/// border, position, display, overflow, opacity... Custom properties (`--x`)
/// are supported too. `transform`, `filter`, animations, etc. are not.
pub fn is_supported_property(name: &str) -> bool {
    if name.starts_with("--") {
        return true;
    }
    SUPPORTED_PROPERTIES
        .iter()
        .any(|p| p.eq_ignore_ascii_case(name))
}

/// Does xbro support `name: value`? Checks the property, and for a few
/// commonly feature-tested properties (colors, `display`) the value too.
fn is_supported_declaration(name: &str, value: &[CV]) -> bool {
    let value = trim(value);
    if !is_supported_property(name) {
        return false;
    }
    if name.starts_with("--") {
        return true;
    }
    if value.is_empty()
        || value
            .iter()
            .any(|v| matches!(v, CV::Token(Token::BadString | Token::BadUrl)))
    {
        return false;
    }
    if let [single] = value {
        if let Some(keyword) = single.ident() {
            if CSS_WIDE_KEYWORDS
                .iter()
                .any(|k| k.eq_ignore_ascii_case(keyword))
            {
                return true;
            }
        }
        if single.is_function("var") {
            return true;
        }
    }
    let lower = name.to_ascii_lowercase();
    if COLOR_PROPERTIES.contains(&lower.as_str()) {
        return value.len() == 1
            && (value[0].is_ident("currentcolor") || Color::parse(value).is_some());
    }
    if lower == "display" {
        return value.iter().filter(|v| !v.is_whitespace()).all(|v| {
            v.ident()
                .is_some_and(|k| DISPLAY_KEYWORDS.iter().any(|d| d.eq_ignore_ascii_case(k)))
        });
    }
    true
}

/// Evaluates an `@supports` prelude. Invalid conditions evaluate to false.
///
/// Supported: `(property: value)`, `not`, `and`, `or`, nested parentheses and
/// `selector(<complex-selector>)` (true when the selector parses). Other
/// functions (`font-tech()`, `font-format()`...) evaluate to false.
pub fn evaluate_supports(prelude: &[CV]) -> bool {
    let items: Vec<&CV> = prelude.iter().filter(|v| !v.is_whitespace()).collect();
    condition(&items, 0).unwrap_or(false)
}

/// Evaluates the argument of `@import ... supports(...)`, which is either a
/// condition or a bare declaration.
pub(crate) fn evaluate_import_supports(args: &[CV]) -> bool {
    let items: Vec<&CV> = args.iter().filter(|v| !v.is_whitespace()).collect();
    if let Some(result) = condition(&items, 0) {
        return result;
    }
    declaration(trim(args)).unwrap_or(false)
}

/// `<supports-condition>`; `None` on a syntax error.
fn condition(items: &[&CV], depth: usize) -> Option<bool> {
    if depth > super::MAX_NESTING_DEPTH {
        return None;
    }
    let (first, rest) = items.split_first()?;
    if first.is_ident("not") {
        return match rest {
            [operand] => in_parens(operand, depth).map(|b| !b),
            _ => None,
        };
    }
    let mut result = in_parens(first, depth)?;
    let mut operator: Option<bool> = None; // Some(true) = and, Some(false) = or
    let mut rest = rest;
    while let [keyword, operand, tail @ ..] = rest {
        let is_and = if keyword.is_ident("and") {
            true
        } else if keyword.is_ident("or") {
            false
        } else {
            return None;
        };
        if operator.is_some_and(|op| op != is_and) {
            return None;
        }
        operator = Some(is_and);
        let value = in_parens(operand, depth)?;
        result = if is_and {
            result && value
        } else {
            result || value
        };
        rest = tail;
    }
    if rest.is_empty() {
        Some(result)
    } else {
        None
    }
}

/// `<supports-in-parens>`; `None` on a syntax error.
fn in_parens(item: &CV, depth: usize) -> Option<bool> {
    match item {
        CV::Block { open: '(', content } => {
            let inner: Vec<&CV> = content.iter().filter(|v| !v.is_whitespace()).collect();
            let starts_condition = inner.first().is_some_and(|v| {
                v.is_ident("not") || matches!(v, CV::Block { open: '(', .. } | CV::Function { .. })
            });
            if starts_condition {
                if let Some(result) = condition(&inner, depth + 1) {
                    return Some(result);
                }
            }
            // `( <declaration> )`, otherwise general-enclosed (false).
            Some(declaration(trim(content)).unwrap_or(false))
        }
        CV::Function { name, args } => {
            if name.eq_ignore_ascii_case("selector") {
                Some(selector::parse_selector_list_from(args).is_some_and(|list| list.len() == 1))
            } else {
                // general-enclosed: font-tech(), font-format(), ...
                Some(false)
            }
        }
        _ => None,
    }
}

/// `property: value` (without parentheses); `None` if it is not a declaration.
fn declaration(items: &[CV]) -> Option<bool> {
    let (first, rest) = items.split_first()?;
    let name = first.ident()?;
    let rest = trim(rest);
    let (colon, value) = rest.split_first()?;
    if !matches!(colon, CV::Token(Token::Colon)) {
        return None;
    }
    Some(is_supported_declaration(name, value))
}
