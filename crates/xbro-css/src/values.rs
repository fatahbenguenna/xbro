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
    ///
    /// The comparison is ASCII case-insensitive. Approximations: the small /
    /// large / dynamic viewport units map to the plain viewport units, logical
    /// viewport units (`vi`, `vb`) to `vw` / `vh`, container query units to
    /// viewport units, `ic` to `em`, and root-relative font units (`rex`,
    /// `rch`, `ric`) to their element-relative counterparts (`ric` → `rem`).
    /// Resolutions (`x`, `dppx`, `dpi`) and other dimensions are not lengths.
    pub fn from_name(unit: &str) -> Option<Self> {
        if unit.len() > 5 {
            return None;
        }
        let unit = unit.to_ascii_lowercase();
        Some(match unit.as_str() {
            "px" => LengthUnit::Px,
            "em" | "ic" => LengthUnit::Em,
            "rem" | "ric" => LengthUnit::Rem,
            "ex" | "rex" => LengthUnit::Ex,
            "ch" | "rch" => LengthUnit::Ch,
            "pt" => LengthUnit::Pt,
            "pc" => LengthUnit::Pc,
            "in" => LengthUnit::In,
            "cm" => LengthUnit::Cm,
            "mm" => LengthUnit::Mm,
            "q" => LengthUnit::Q,
            "vw" | "svw" | "lvw" | "dvw" | "vi" | "svi" | "lvi" | "dvi" | "cqw" | "cqi" => {
                LengthUnit::Vw
            }
            "vh" | "svh" | "lvh" | "dvh" | "vb" | "svb" | "lvb" | "dvb" | "cqh" | "cqb" => {
                LengthUnit::Vh
            }
            "vmin" | "svmin" | "lvmin" | "dvmin" | "cqmin" => LengthUnit::Vmin,
            "vmax" | "svmax" | "lvmax" | "dvmax" | "cqmax" => LengthUnit::Vmax,
            "%" => LengthUnit::Percent,
            _ => return None,
        })
    }

    /// True for units with a fixed size in px (`px`, `pt`, `pc`, `in`, `cm`, `mm`, `q`).
    pub fn is_absolute(self) -> bool {
        matches!(
            self,
            LengthUnit::Px
                | LengthUnit::Pt
                | LengthUnit::Pc
                | LengthUnit::In
                | LengthUnit::Cm
                | LengthUnit::Mm
                | LengthUnit::Q
        )
    }

    /// Size of one unit in px for absolute units (`None` otherwise).
    pub fn absolute_px(self) -> Option<f32> {
        Some(match self {
            LengthUnit::Px => 1.0,
            LengthUnit::Pt => 96.0 / 72.0,
            LengthUnit::Pc => 16.0,
            LengthUnit::In => 96.0,
            LengthUnit::Cm => 96.0 / 2.54,
            LengthUnit::Mm => 96.0 / 25.4,
            LengthUnit::Q => 96.0 / 101.6,
            _ => return None,
        })
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
        match value {
            ComponentValue::Token(Token::Dimension { value, unit }) => Some(Length {
                value: *value,
                unit: LengthUnit::from_name(unit).filter(|u| *u != LengthUnit::Percent)?,
            }),
            ComponentValue::Token(Token::Percentage(value)) => Some(Length {
                value: *value,
                unit: LengthUnit::Percent,
            }),
            ComponentValue::Token(Token::Number { value, .. }) if *value == 0.0 => {
                Some(Length::px(0.0))
            }
            _ => None,
        }
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
    match value {
        ComponentValue::Token(Token::Url(url)) => Some(url.clone()),
        ComponentValue::Function { name, args }
            if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("src") =>
        {
            // `url("x" <modifiers>*)`: the string comes first.
            match trim(args).first()? {
                ComponentValue::Token(Token::String(s)) => Some(s.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Identifier of an `Ident` token (shortcut for [`ComponentValue::ident`]).
pub fn ident(value: &ComponentValue) -> Option<&str> {
    value.ident()
}

/// `(value, unit)` of a `Dimension` token (unit lowercased).
pub fn dimension(value: &ComponentValue) -> Option<(f32, &str)> {
    match value {
        ComponentValue::Token(Token::Dimension { value, unit }) => Some((*value, unit)),
        _ => None,
    }
}

/// Value of a `Percentage` token (e.g. `50` for `50%`).
pub fn percentage(value: &ComponentValue) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Percentage(p)) => Some(*p),
        _ => None,
    }
}

/// Integer value of a `Number` token whose source had no fraction or exponent.
pub fn integer(value: &ComponentValue) -> Option<i32> {
    match value {
        ComponentValue::Token(Token::Number {
            value,
            is_integer: true,
        }) => Some(value.clamp(i32::MIN as f32, i32::MAX as f32) as i32),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
