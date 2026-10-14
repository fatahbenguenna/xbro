//! Color functions: `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`,
//! `oklch()`, `color()`, `color-mix()` and `light-dark()`.

use super::{parse_value, spaces, Color};
use crate::parser::ComponentValue as CV;
use crate::tokenizer::Token;
use crate::values::{split_commas, split_whitespace, trim};

/// A raw color function argument.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Comp {
    Number(f64),
    Percent(f64),
    /// An angle, in degrees.
    Angle(f64),
    /// The `none` keyword.
    None,
}

fn component(value: &CV) -> Option<Comp> {
    match value {
        CV::Token(Token::Number { value, .. }) => Some(Comp::Number(f64::from(*value))),
        CV::Token(Token::Percentage(p)) => Some(Comp::Percent(f64::from(*p))),
        CV::Token(Token::Dimension { value, unit }) => {
            let v = f64::from(*value);
            let degrees = match unit.as_str() {
                "deg" => v,
                "grad" => v * 0.9,
                "rad" => v.to_degrees(),
                "turn" => v * 360.0,
                _ => return None,
            };
            Some(Comp::Angle(degrees))
        }
        CV::Token(Token::Ident(s)) if s.eq_ignore_ascii_case("none") => Some(Comp::None),
        _ => None,
    }
}

/// Parsed arguments of a color function.
struct Args {
    comps: [Comp; 3],
    alpha: Option<Comp>,
    /// Comma-separated legacy syntax.
    legacy: bool,
}

/// Parses `c1 c2 c3 [/ alpha]` or the legacy `c1, c2, c3[, alpha]`.
fn parse_args(args: &[CV]) -> Option<Args> {
    let args = trim(args);
    if args.iter().any(CV::is_comma) {
        let parts = split_commas(args);
        if parts.len() != 3 && parts.len() != 4 {
            return None;
        }
        let single = |part: &[CV]| match part {
            [value] => component(value),
            _ => None,
        };
        let comps = [
            single(parts.first()?)?,
            single(parts.get(1)?)?,
            single(parts.get(2)?)?,
        ];
        let alpha = match parts.get(3) {
            Some(part) => Some(single(part)?),
            None => None,
        };
        return Some(Args {
            comps,
            alpha,
            legacy: true,
        });
    }
    let (main, alpha) = match args.iter().position(|v| v.is_delim('/')) {
        Some(slash) => (&args[..slash], Some(trim(&args[slash + 1..]))),
        None => (args, None),
    };
    let groups = split_whitespace(main);
    let single = |i: usize| match groups.get(i)? {
        [value] => component(value),
        _ => None,
    };
    if groups.len() != 3 {
        return None;
    }
    let comps = [single(0)?, single(1)?, single(2)?];
    let alpha = match alpha {
        Some([value]) => Some(component(value)?),
        Some(_) => return None,
        None => None,
    };
    Some(Args {
        comps,
        alpha,
        legacy: false,
    })
}

/// Alpha in 0..=1 (`None` = opaque).
fn alpha_value(alpha: Option<Comp>) -> Option<f64> {
    match alpha {
        None => Some(1.0),
        Some(Comp::Number(n)) => Some(n.clamp(0.0, 1.0)),
        Some(Comp::Percent(p)) => Some((p / 100.0).clamp(0.0, 1.0)),
        Some(Comp::None) => Some(0.0),
        Some(Comp::Angle(_)) => None,
    }
}

/// Converts a 0..=1 channel to 8 bits, clamping and rounding half up.
fn unit_to_u8(v: f64) -> u8 {
    if v.is_nan() {
        return 0;
    }
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn from_unit_rgb(rgb: [f64; 3], alpha: f64) -> Color {
    Color::rgba(
        unit_to_u8(rgb[0]),
        unit_to_u8(rgb[1]),
        unit_to_u8(rgb[2]),
        unit_to_u8(alpha),
    )
}

/// Parses a color function by (case-insensitive) name.
pub(super) fn parse_function(name: &str, args: &[CV], depth: usize) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "rgb" | "rgba" => rgb(args),
        "hsl" | "hsla" => hsl(args),
        "hwb" => hwb(args),
        "lab" => lab_like(args, false, false),
        "lch" => lab_like(args, false, true),
        "oklab" => lab_like(args, true, false),
        "oklch" => lab_like(args, true, true),
        "color" => color_function(args),
        "color-mix" => color_mix(args, depth),
        "light-dark" => light_dark(args, depth),
        _ => None,
    }
}

fn rgb(args: &[CV]) -> Option<Color> {
    let args = parse_args(args)?;
    if args.legacy {
        let all_numbers = args.comps.iter().all(|c| matches!(c, Comp::Number(_)));
        let all_percents = args.comps.iter().all(|c| matches!(c, Comp::Percent(_)));
        if !(all_numbers || all_percents) || args.alpha == Some(Comp::None) {
            return None;
        }
    }
    let channel = |c: Comp| -> Option<u8> {
        let v = match c {
            Comp::Number(n) => n,
            Comp::Percent(p) => p / 100.0 * 255.0,
            Comp::None => 0.0,
            Comp::Angle(_) => return None,
        };
        Some(v.clamp(0.0, 255.0).round() as u8)
    };
    let [r, g, b] = args.comps;
    Some(Color::rgba(
        channel(r)?,
        channel(g)?,
        channel(b)?,
        unit_to_u8(alpha_value(args.alpha)?),
    ))
}

/// Hue in degrees, normalized to 0..360.
fn hue(c: Comp) -> Option<f64> {
    let degrees = match c {
        Comp::Number(n) | Comp::Angle(n) => n,
        Comp::None => 0.0,
        Comp::Percent(_) => return None,
    };
    Some(degrees.rem_euclid(360.0))
}

/// A percentage-like component (`50%`, or `50` in modern syntax) as 0..=1.
fn fraction(c: Comp, legacy: bool) -> Option<f64> {
    let v = match c {
        Comp::Percent(p) => p,
        Comp::Number(n) if !legacy => n,
        Comp::None if !legacy => 0.0,
        _ => return None,
    };
    Some((v / 100.0).clamp(0.0, 1.0))
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let f = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn hsl(args: &[CV]) -> Option<Color> {
    let args = parse_args(args)?;
    if args.legacy && args.comps[0] == Comp::None {
        return None;
    }
    let h = hue(args.comps[0])?;
    let s = fraction(args.comps[1], args.legacy)?;
    let l = fraction(args.comps[2], args.legacy)?;
    if args.legacy && args.alpha == Some(Comp::None) {
        return None;
    }
    Some(from_unit_rgb(hsl_to_rgb(h, s, l), alpha_value(args.alpha)?))
}

fn hwb(args: &[CV]) -> Option<Color> {
    let args = parse_args(args)?;
    if args.legacy {
        return None;
    }
    let h = hue(args.comps[0])?;
    let w = fraction(args.comps[1], false)?;
    let b = fraction(args.comps[2], false)?;
    let rgb = if w + b >= 1.0 {
        let gray = w / (w + b);
        [gray; 3]
    } else {
        hsl_to_rgb(h, 1.0, 0.5).map(|c| c * (1.0 - w - b) + w)
    };
    Some(from_unit_rgb(rgb, alpha_value(args.alpha)?))
}

/// A number, or a percentage scaled so that 100% is `full`.
fn scaled(c: Comp, full: f64) -> Option<f64> {
    match c {
        Comp::Number(n) => Some(n),
        Comp::Percent(p) => Some(p / 100.0 * full),
        Comp::None => Some(0.0),
        Comp::Angle(_) => None,
    }
}

/// `lab()`, `lch()`, `oklab()` and `oklch()`.
fn lab_like(args: &[CV], ok: bool, polar: bool) -> Option<Color> {
    let args = parse_args(args)?;
    if args.legacy {
        return None;
    }
    let [c0, c1, c2] = args.comps;
    let (l_full, ab_full, chroma_full) = if ok {
        (1.0, 0.4, 0.4)
    } else {
        (100.0, 125.0, 150.0)
    };
    let l = scaled(c0, l_full)?.clamp(0.0, l_full);
    let rgb = if polar {
        let c = scaled(c1, chroma_full)?.max(0.0);
        let h = hue(c2)?;
        if ok {
            spaces::oklch_to_srgb(l, c, h)
        } else {
            spaces::lch_to_srgb(l, c, h)
        }
    } else {
        let a = scaled(c1, ab_full)?;
        let b = scaled(c2, ab_full)?;
        if ok {
            spaces::oklab_to_srgb(l, a, b)
        } else {
            spaces::lab_to_srgb(l, a, b)
        }
    };
    Some(from_unit_rgb(rgb, alpha_value(args.alpha)?))
}

/// `color(<space> c1 c2 c3 [/ alpha])`.
fn color_function(args: &[CV]) -> Option<Color> {
    let args = trim(args);
    let (space, rest) = args.split_first()?;
    let space = space.ident()?.to_ascii_lowercase();
    let parsed = parse_args(rest)?;
    if parsed.legacy {
        return None;
    }
    let mut c = [0.0; 3];
    for (out, comp) in c.iter_mut().zip(parsed.comps) {
        *out = scaled(comp, 1.0)?;
    }
    let rgb = spaces::predefined_to_srgb(&space, c)?;
    Some(from_unit_rgb(rgb, alpha_value(parsed.alpha)?))
}

const MIX_SPACES: &[&str] = &[
    "srgb",
    "srgb-linear",
    "display-p3",
    "a98-rgb",
    "prophoto-rgb",
    "rec2020",
    "lab",
    "oklab",
    "xyz",
    "xyz-d50",
    "xyz-d65",
    "hsl",
    "hwb",
    "lch",
    "oklch",
];

/// One `<color> <percentage>?` argument of `color-mix()`.
fn mix_item(part: &[CV], depth: usize) -> Option<(Color, Option<f64>)> {
    let items: Vec<&CV> = part.iter().filter(|v| !v.is_whitespace()).collect();
    let percent = |v: &CV| match v {
        CV::Token(Token::Percentage(p)) if (0.0..=100.0).contains(p) => Some(f64::from(*p)),
        _ => None,
    };
    match items.as_slice() {
        [color] => Some((parse_value(color, depth + 1)?, None)),
        [a, b] => match percent(a) {
            Some(p) => Some((parse_value(b, depth + 1)?, Some(p))),
            None => Some((parse_value(a, depth + 1)?, Some(percent(b)?))),
        },
        _ => None,
    }
}

/// `color-mix(in <space> [<hue-method> hue]?, <color> <p>?, <color> <p>?)`,
/// approximated by mixing (premultiplied) in sRGB whatever the space.
fn color_mix(args: &[CV], depth: usize) -> Option<Color> {
    let parts = split_commas(trim(args));
    let [method, first, second] = parts.as_slice() else {
        return None;
    };
    let method: Vec<&CV> = method.iter().filter(|v| !v.is_whitespace()).collect();
    match method.as_slice() {
        [kw, space, hue_method @ ..] if kw.is_ident("in") => {
            let space = space.ident()?.to_ascii_lowercase();
            if !MIX_SPACES.contains(&space.as_str()) {
                return None;
            }
            match hue_method {
                [] => {}
                [m, h]
                    if h.is_ident("hue")
                        && ["shorter", "longer", "increasing", "decreasing"]
                            .iter()
                            .any(|k| m.is_ident(k)) => {}
                _ => return None,
            }
        }
        _ => return None,
    }
    let (c1, p1) = mix_item(first, depth)?;
    let (c2, p2) = mix_item(second, depth)?;
    let (p1, p2) = match (p1, p2) {
        (None, None) => (50.0, 50.0),
        (Some(p), None) => (p, 100.0 - p),
        (None, Some(p)) => (100.0 - p, p),
        (Some(a), Some(b)) => (a, b),
    };
    let sum = p1 + p2;
    if sum <= 0.0 {
        return None;
    }
    let alpha_multiplier = if sum < 100.0 { sum / 100.0 } else { 1.0 };
    let (w1, w2) = (p1 / sum, p2 / sum);
    let a1 = f64::from(c1.a) / 255.0;
    let a2 = f64::from(c2.a) / 255.0;
    let alpha = a1 * w1 + a2 * w2;
    let channel = |x: u8, y: u8| -> f64 {
        let (x, y) = (f64::from(x) / 255.0, f64::from(y) / 255.0);
        if alpha > 0.0 {
            (x * a1 * w1 + y * a2 * w2) / alpha
        } else {
            x * w1 + y * w2
        }
    };
    Some(from_unit_rgb(
        [
            channel(c1.r, c2.r),
            channel(c1.g, c2.g),
            channel(c1.b, c2.b),
        ],
        alpha * alpha_multiplier,
    ))
}

/// `light-dark(<light>, <dark>)`: xbro uses the light color scheme.
fn light_dark(args: &[CV], depth: usize) -> Option<Color> {
    let parts = split_commas(trim(args));
    match parts.as_slice() {
        [[light], [dark]] => {
            parse_value(dark, depth + 1)?;
            parse_value(light, depth + 1)
        }
        _ => None,
    }
}
