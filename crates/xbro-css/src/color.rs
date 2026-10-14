//! RGBA colors (non-premultiplied, 8 bits per channel).
//!
//! Parsing covers CSS Color 4/5 syntaxes: named and system colors, hex
//! notations, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`,
//! `oklch()`, `color()`, `color-mix()` and `light-dark()`, plus the HTML
//! legacy color algorithm used by presentational attributes. Colors outside
//! the sRGB gamut are clamped.

mod functions;
mod legacy;
mod named;
mod spaces;

#[cfg(test)]
mod tests;

use crate::parser::ComponentValue;
use crate::tokenizer::Token;
use crate::values::trim;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Maximum nesting of color functions (`color-mix(in srgb, color-mix(...), ...)`).
const MAX_COLOR_DEPTH: usize = 8;

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn is_transparent(&self) -> bool {
        self.a == 0
    }

    /// Returns the color with its alpha multiplied by `opacity` (0..=1).
    pub fn with_opacity(self, opacity: f32) -> Self {
        let a = (self.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8;
        Self { a, ..self }
    }

    /// Parses a color keyword or hex notation from a single string, e.g.
    /// `"red"`, `"#f00"`, `"#ff000080"`, `"rgb(255 0 0 / 50%)"`.
    /// `currentcolor` is NOT handled here (it returns `None`): callers resolve it.
    pub fn parse_str(s: &str) -> Option<Color> {
        Color::parse(&crate::parser::parse_component_values(s))
    }

    /// Parses a color from component values (a single ident, hash token or
    /// `rgb()/rgba()/hsl()/hsla()/hwb()` function; leading/trailing whitespace ignored).
    /// Returns `None` for `currentcolor` and anything that is not a color.
    ///
    /// Also accepts system colors (`Canvas`, `LinkText`, ... with light-theme
    /// values), `lab()`, `lch()`, `oklab()`, `oklch()`, `color()`,
    /// `color-mix()` (mixed in sRGB) and `light-dark()` (light value).
    pub fn parse(values: &[ComponentValue]) -> Option<Color> {
        match trim(values) {
            [value] => parse_value(value, 0),
            _ => None,
        }
    }

    /// Looks up one of the 148 CSS named colors (plus `transparent`), case-insensitively.
    pub fn named(name: &str) -> Option<Color> {
        named::named(name)
    }

    /// Looks up a CSS system color (`Canvas`, `CanvasText`, `LinkText`,
    /// `ButtonFace`, `Highlight`, ..., and the deprecated ones such as
    /// `ButtonShadow` or `WindowText`), case-insensitively, with the values of
    /// a light theme.
    pub fn system(name: &str) -> Option<Color> {
        named::system(name)
    }

    /// Parses a hex color without the `#` (`rgb`, `rgba`, `rrggbb`, `rrggbbaa`).
    pub fn from_hex(hex: &str) -> Option<Color> {
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()?;
        let pair = |i: usize| {
            digits
                .get(i)
                .zip(digits.get(i + 1))
                .map(|(h, l)| h * 16 + l)
        };
        let single = |i: usize| digits.get(i).map(|d| d * 17);
        match digits.len() {
            3 => Some(Color::rgb(single(0)?, single(1)?, single(2)?)),
            4 => Some(Color::rgba(single(0)?, single(1)?, single(2)?, single(3)?)),
            6 => Some(Color::rgb(pair(0)?, pair(2)?, pair(4)?)),
            8 => Some(Color::rgba(pair(0)?, pair(2)?, pair(4)?, pair(6)?)),
            _ => None,
        }
    }

    /// Parses legacy HTML color attribute values (`bgcolor="red"`, `color="#abc"`,
    /// `"ff0000"` without `#`), following the "rules for parsing a legacy color value".
    pub fn parse_legacy(s: &str) -> Option<Color> {
        legacy::parse_legacy(s)
    }
}

/// Parses a single component value as a color.
pub(crate) fn parse_value(value: &ComponentValue, depth: usize) -> Option<Color> {
    match value {
        ComponentValue::Token(Token::Ident(name)) => {
            Color::named(name).or_else(|| Color::system(name))
        }
        ComponentValue::Token(Token::Hash { value, .. }) => Color::from_hex(value),
        ComponentValue::Function { name, args } if depth < MAX_COLOR_DEPTH => {
            functions::parse_function(name, args, depth)
        }
        _ => None,
    }
}
