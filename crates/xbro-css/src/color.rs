//! RGBA colors (non-premultiplied, 8 bits per channel).

use crate::parser::ComponentValue;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

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
        let _ = s;
        todo!("xbro-css: implement Color::parse_str")
    }

    /// Parses a color from component values (a single ident, hash token or
    /// `rgb()/rgba()/hsl()/hsla()/hwb()` function; leading/trailing whitespace ignored).
    /// Returns `None` for `currentcolor` and anything that is not a color.
    pub fn parse(values: &[ComponentValue]) -> Option<Color> {
        let _ = values;
        todo!("xbro-css: implement Color::parse")
    }

    /// Looks up one of the 148 CSS named colors (plus `transparent`), case-insensitively.
    pub fn named(name: &str) -> Option<Color> {
        let _ = name;
        todo!("xbro-css: implement named colors")
    }

    /// Parses legacy HTML color attribute values (`bgcolor="red"`, `color="#abc"`,
    /// `"ff0000"` without `#`), following the "rules for parsing a legacy color value".
    pub fn parse_legacy(s: &str) -> Option<Color> {
        let _ = s;
        todo!("xbro-css: implement legacy color parsing")
    }
}
