//! Value-level parsing helpers shared by the property parsers: lengths,
//! percentages, numbers, colors and keywords.

pub(crate) mod calc;
pub(crate) mod length;

use xbro_css::{Color, ComponentValue as CV, LengthUnit, Token};

pub(crate) use calc::{is_math_function, parse_math_function, Calculated, MathExpr};
pub(crate) use length::{sane, LinearLen, Resolver};

/// Parsing options that depend on the document / property.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct ParseCtx {
    /// Quirks-mode "unitless length quirk": non-zero unitless numbers are px.
    pub unitless_quirk: bool,
}

/// A color, or `currentcolor` (resolved per element).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum SpecColor {
    Color(Color),
    CurrentColor,
}

impl SpecColor {
    pub fn resolve(self, current: Color) -> Color {
        match self {
            SpecColor::Color(c) => c,
            SpecColor::CurrentColor => current,
        }
    }
}

/// Parses one component as a color (or `currentcolor`).
pub(crate) fn color(v: &CV) -> Option<SpecColor> {
    if v.is_ident("currentcolor") {
        return Some(SpecColor::CurrentColor);
    }
    Color::parse(std::slice::from_ref(v)).map(SpecColor::Color)
}

/// Looks `v` up (ASCII case-insensitively) in a keyword table.
pub(crate) fn keyword<T: Copy>(v: &CV, table: &[(&str, T)]) -> Option<T> {
    let ident = v.ident()?;
    table
        .iter()
        .find(|(k, _)| ident.eq_ignore_ascii_case(k))
        .map(|(_, t)| *t)
}

/// The only item of `items`.
pub(crate) fn single<'a>(items: &[&'a CV]) -> Option<&'a CV> {
    match items {
        [one] => Some(*one),
        _ => None,
    }
}

/// Splits significant items on comma tokens.
pub(crate) fn split_on_commas<'a, 'b>(items: &'b [&'a CV]) -> Vec<&'b [&'a CV]> {
    items.split(|v| v.is_comma()).collect()
}

/// Rules for a length/percentage component.
#[derive(Copy, Clone, Debug)]
pub(crate) struct LengthRules {
    pub negative: bool,
    pub percent: bool,
}

pub(crate) const ANY_LP: LengthRules = LengthRules {
    negative: true,
    percent: true,
};
pub(crate) const NON_NEG_LP: LengthRules = LengthRules {
    negative: false,
    percent: true,
};
pub(crate) const NON_NEG_LENGTH: LengthRules = LengthRules {
    negative: false,
    percent: false,
};

/// Parses a length / percentage / math function from one component.
///
/// Unitless `0` is accepted; other unitless numbers only with the quirk.
/// Negative literals are rejected unless allowed (math results are clamped
/// later instead, as the spec requires).
pub(crate) fn length(v: &CV, rules: LengthRules, cx: &ParseCtx) -> Option<MathExpr> {
    match v {
        CV::Token(Token::Dimension { value, unit }) => {
            // `1\%` is a dimension whose unit is "%", not a percentage.
            let unit = LengthUnit::from_name(unit).filter(|u| *u != LengthUnit::Percent)?;
            if (!rules.negative && *value < 0.0) || !value.is_finite() {
                return None;
            }
            Some(MathExpr::Linear(LinearLen::from_unit(*value, unit)))
        }
        CV::Token(Token::Percentage(p)) => {
            if !rules.percent || (!rules.negative && *p < 0.0) || !p.is_finite() {
                return None;
            }
            Some(MathExpr::percent(*p))
        }
        CV::Token(Token::Number { value, .. }) => {
            if *value == 0.0 {
                Some(MathExpr::ZERO)
            } else if cx.unitless_quirk && (rules.negative || *value > 0.0) && value.is_finite() {
                Some(MathExpr::px(*value))
            } else {
                None
            }
        }
        CV::Function { name, args } if is_math_function(name) => {
            match parse_math_function(name, args)? {
                Calculated::Length(e) if rules.percent || !e.has_percent() => Some(e),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A number token or a number-valued math function.
pub(crate) fn number(v: &CV) -> Option<f32> {
    match v {
        CV::Token(Token::Number { value, .. }) if value.is_finite() => Some(*value),
        CV::Function { name, args } if is_math_function(name) => {
            match parse_math_function(name, args)? {
                Calculated::Number(n) if n.is_finite() => Some(n),
                _ => None,
            }
        }
        _ => None,
    }
}

/// An integer token, or a math function (rounded to the nearest integer).
pub(crate) fn integer(v: &CV) -> Option<i32> {
    match v {
        CV::Token(Token::Number {
            value,
            is_integer: true,
        }) => Some(clamp_i32(*value)),
        CV::Function { name, args } if is_math_function(name) => {
            match parse_math_function(name, args)? {
                Calculated::Number(n) if n.is_finite() => Some(clamp_i32(n.round())),
                _ => None,
            }
        }
        _ => None,
    }
}

fn clamp_i32(v: f32) -> i32 {
    // `as` saturates for floats; NaN becomes 0.
    v as i32
}

/// A non-negative number or a percentage (`50%` → 0.5).
pub(crate) fn number_or_percentage(v: &CV) -> Option<f32> {
    match v {
        CV::Token(Token::Percentage(p)) if p.is_finite() => Some(*p / 100.0),
        _ => number(v),
    }
}

/// An angle in degrees (`deg`, `rad`, `grad`, `turn`; unitless 0 accepted).
pub(crate) fn angle_degrees(v: &CV) -> Option<f32> {
    match v {
        CV::Token(Token::Dimension { value, unit }) => {
            let factor = match unit.to_ascii_lowercase().as_str() {
                "deg" => 1.0,
                "rad" => 180.0 / std::f32::consts::PI,
                "grad" => 0.9,
                "turn" => 360.0,
                _ => return None,
            };
            let deg = value * factor;
            deg.is_finite().then_some(deg)
        }
        CV::Token(Token::Number { value, .. }) if *value == 0.0 => Some(0.0),
        _ => None,
    }
}

/// URL of a `url()` token/function or `src()`.
pub(crate) fn url(v: &CV) -> Option<String> {
    xbro_css::values::url(v)
}

/// The first URL of an `image-set()` / `-webkit-image-set()` (string or `url()` items).
pub(crate) fn image_set_url(v: &CV) -> Option<String> {
    let CV::Function { name, args } = v else {
        return None;
    };
    let lower = name.to_ascii_lowercase();
    if lower != "image-set" && lower != "-webkit-image-set" {
        return None;
    }
    args.iter().find_map(|a| match a {
        CV::Token(Token::String(s)) => Some(s.clone()),
        other => url(other),
    })
}

/// True when `values` contains `var()` (or `env()`) anywhere.
pub(crate) fn contains_var(values: &[CV]) -> bool {
    values.iter().any(|v| match v {
        CV::Function { name, args } => {
            name.eq_ignore_ascii_case("var")
                || name.eq_ignore_ascii_case("env")
                || contains_var(args)
        }
        CV::Block { content, .. } => contains_var(content),
        CV::Token(_) => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_css::parse_component_values;

    fn cv(css: &str) -> CV {
        let mut v = parse_component_values(css);
        assert_eq!(v.len(), 1, "{css}");
        v.remove(0)
    }

    #[test]
    fn length_rules() {
        let cx = ParseCtx::default();
        assert_eq!(length(&cv("10px"), ANY_LP, &cx), Some(MathExpr::px(10.0)));
        assert_eq!(length(&cv("0"), NON_NEG_LENGTH, &cx), Some(MathExpr::ZERO));
        assert_eq!(length(&cv("5"), ANY_LP, &cx), None);
        assert_eq!(length(&cv("-5px"), NON_NEG_LP, &cx), None);
        assert_eq!(length(&cv("50%"), NON_NEG_LENGTH, &cx), None);
        assert_eq!(length(&cv("10deg"), ANY_LP, &cx), None);
        assert_eq!(length(&cv("calc(50% + 1px)"), NON_NEG_LENGTH, &cx), None);
        // An escaped `%` makes a dimension with an invalid unit.
        assert_eq!(length(&cv("1\\%"), NON_NEG_LENGTH, &cx), None);
        assert_eq!(length(&cv("1\\%"), ANY_LP, &cx), None);
        assert_eq!(length(&cv("calc(1\\% + 1px)"), ANY_LP, &cx), None);
        assert!(length(&cv("calc(-5px)"), NON_NEG_LP, &cx).is_some());
        let quirk = ParseCtx {
            unitless_quirk: true,
        };
        assert_eq!(length(&cv("5"), ANY_LP, &quirk), Some(MathExpr::px(5.0)));
        assert_eq!(length(&cv("-5"), NON_NEG_LP, &quirk), None);
    }

    #[test]
    fn numbers_and_angles() {
        assert_eq!(number(&cv("1.5")), Some(1.5));
        assert_eq!(number(&cv("calc(1 + 1)")), Some(2.0));
        assert_eq!(integer(&cv("3")), Some(3));
        assert_eq!(integer(&cv("3.5")), None);
        assert_eq!(integer(&cv("calc(2.6)")), Some(3));
        assert_eq!(integer(&cv("1e20")), None);
        assert_eq!(number_or_percentage(&cv("50%")), Some(0.5));
        assert_eq!(angle_degrees(&cv("0.5turn")), Some(180.0));
        assert_eq!(angle_degrees(&cv("100grad")), Some(90.0));
        assert!((angle_degrees(&cv("3.14159265rad")).unwrap() - 180.0).abs() < 1e-3);
        assert_eq!(angle_degrees(&cv("10px")), None);
    }

    #[test]
    fn colors_and_urls() {
        assert_eq!(
            color(&cv("red")),
            Some(SpecColor::Color(Color::rgb(255, 0, 0)))
        );
        assert_eq!(color(&cv("currentColor")), Some(SpecColor::CurrentColor));
        assert_eq!(color(&cv("bogus")), None);
        assert_eq!(url(&cv("url(a.png)")).as_deref(), Some("a.png"));
        assert_eq!(
            image_set_url(&cv("image-set(\"a.png\" 1x, \"b.png\" 2x)")).as_deref(),
            Some("a.png")
        );
        assert_eq!(
            image_set_url(&cv("-webkit-image-set(url(c.png) 1x)")).as_deref(),
            Some("c.png")
        );
    }

    #[test]
    fn var_detection() {
        assert!(contains_var(&parse_component_values("1px var(--a)")));
        assert!(contains_var(&parse_component_values(
            "calc(1px + var(--a))"
        )));
        assert!(!contains_var(&parse_component_values("calc(1px + 2px)")));
    }
}
