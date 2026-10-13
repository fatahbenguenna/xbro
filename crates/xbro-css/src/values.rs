//! Generic value helpers shared by property parsers in `xbro-style`.

use crate::parser::ComponentValue;
use crate::tokenizer::Token;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum LengthUnit {
    Px,
    Em,
    Rem,
    Ex,
    Ch,
    Pt,
    Pc,
    In,
    Cm,
    Mm,
    Q,
    Vw,
    Vh,
    Vmin,
    Vmax,
    Percent,
}

impl LengthUnit {
    /// Maps a lowercased unit name (`"px"`, `"%"`, `"dvh"` → Vh, `"svw"` → Vw, ...) to a unit.
    pub fn from_name(unit: &str) -> Option<Self> {
        let _ = unit;
        todo!("xbro-css: implement LengthUnit::from_name")
    }
}

/// A length or percentage as written (not yet resolved to px).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Length {
    pub value: f32,
    pub unit: LengthUnit,
}

impl Length {
    pub const fn px(value: f32) -> Self {
        Self {
            value,
            unit: LengthUnit::Px,
        }
    }

    /// Parses a single dimension/percentage token, or a unitless `0`.
    /// (`calc()` is handled by `xbro-style`, which knows the resolution context.)
    pub fn parse(value: &ComponentValue) -> Option<Length> {
        let _ = value;
        todo!("xbro-css: implement Length::parse")
    }
}

/// Removes leading and trailing whitespace component values.
pub fn trim(values: &[ComponentValue]) -> &[ComponentValue] {
    let start = values
        .iter()
        .position(|v| !v.is_whitespace())
        .unwrap_or(values.len());
    let end = values
        .iter()
        .rposition(|v| !v.is_whitespace())
        .map_or(start, |e| e + 1);
    &values[start..end.max(start)]
}

/// Splits on top-level whitespace, dropping empty groups (`1px solid red` → 3 items).
/// Functions and blocks stay whole.
pub fn split_whitespace(values: &[ComponentValue]) -> Vec<&[ComponentValue]> {
    values
        .split(|v| v.is_whitespace())
        .filter(|g| !g.is_empty())
        .collect()
}

/// Splits on top-level commas, trimming whitespace around each item.
pub fn split_commas(values: &[ComponentValue]) -> Vec<&[ComponentValue]> {
    values.split(|v| v.is_comma()).map(trim).collect()
}

/// Number value of a `Number` token.
pub fn number(value: &ComponentValue) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Number { value, .. }) => Some(*value),
        _ => None,
    }
}

/// String value of a `String` token.
pub fn string(value: &ComponentValue) -> Option<&str> {
    match value {
        ComponentValue::Token(Token::String(s)) => Some(s),
        _ => None,
    }
}

/// URL of a `url(x)` token or `url("x")` / `src("x")` function.
pub fn url(value: &ComponentValue) -> Option<String> {
    let _ = value;
    todo!("xbro-css: implement values::url")
}
