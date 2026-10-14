//! Media queries (Media Queries Level 4, subset).
//!
//! Queries are evaluated with three-valued logic: unknown features and
//! `<general-enclosed>` productions are "unknown", which `not` keeps unknown
//! and which makes the query false at the top level.

use crate::parser::ComponentValue;
use crate::tokenizer::Token;
use crate::values::{split_commas, trim};

/// Environment media queries are evaluated against.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MediaContext {
    /// Viewport width in CSS px.
    pub width: f32,
    /// Viewport height in CSS px.
    pub height: f32,
    /// Device pixels per CSS px.
    pub resolution: f32,
    /// Prefer dark color scheme (xbro defaults to light).
    pub dark: bool,
}

impl Default for MediaContext {
    fn default() -> Self {
        Self {
            width: 1024.0,
            height: 768.0,
            resolution: 1.0,
            dark: false,
        }
    }
}

/// A comma-separated list of media queries, kept as component values and
/// evaluated on demand (the viewport may change between layouts).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MediaQueryList {
    pub values: Vec<ComponentValue>,
}

impl MediaQueryList {
    pub fn new(values: Vec<ComponentValue>) -> Self {
        Self { values }
    }

    pub fn parse_str(s: &str) -> Self {
        Self::new(crate::parser::parse_component_values(s))
    }

    /// Evaluates the list for a screen device. An empty list matches.
    ///
    /// Supported: media types `all`, `screen` (true), `print`/`speech`/others
    /// (false); `not` / `only`; `and`; features `width`, `min-width`,
    /// `max-width`, `height`, `min-height`, `max-height`, range syntax
    /// (`(width >= 600px)`, `(400px < width < 800px)`), `orientation`,
    /// `prefers-color-scheme`, `prefers-reduced-motion` (no-preference),
    /// `hover` (hover), `pointer` (fine), `any-hover`, `any-pointer`,
    /// `resolution`/`min-resolution`/`max-resolution` (dppx, dpi, x),
    /// `-webkit-min-device-pixel-ratio`, `aspect-ratio`, `color` (true).
    /// Lengths accept px, em/rem (16px), pt, etc. An unknown feature makes that
    /// query false (but other comma-separated queries are still evaluated).
    ///
    /// Conditions also support `or`, `not (feature)` and nested parentheses
    /// (with Kleene logic: `(unknown) or (width > 0)` is true).
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        let values = trim(&self.values);
        if values.is_empty() {
            return true;
        }
        split_commas(values)
            .into_iter()
            .any(|query| evaluate_query(query, ctx))
    }
}

/// Three-valued logic result.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Kleene {
    True,
    False,
    Unknown,
}

impl Kleene {
    fn from_bool(b: bool) -> Self {
        if b {
            Kleene::True
        } else {
            Kleene::False
        }
    }

    fn negate(self) -> Self {
        match self {
            Kleene::True => Kleene::False,
            Kleene::False => Kleene::True,
            Kleene::Unknown => Kleene::Unknown,
        }
    }

    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Kleene::False, _) | (_, Kleene::False) => Kleene::False,
            (Kleene::True, Kleene::True) => Kleene::True,
            _ => Kleene::Unknown,
        }
    }

    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Kleene::True, _) | (_, Kleene::True) => Kleene::True,
            (Kleene::False, Kleene::False) => Kleene::False,
            _ => Kleene::Unknown,
        }
    }
}

const RESERVED_TYPES: &[&str] = &["not", "and", "or", "only", "layer"];

/// One `<media-query>`; malformed queries are false ("not all").
fn evaluate_query(query: &[ComponentValue], ctx: &MediaContext) -> bool {
    let items: Vec<&ComponentValue> = query.iter().filter(|v| !v.is_whitespace()).collect();
    let Some(first) = items.first() else {
        return false;
    };
    let Some(word) = first.ident() else {
        return condition(&items, true, ctx, 0) == Some(Kleene::True);
    };
    let (negated, type_index) = if word.eq_ignore_ascii_case("not") {
        if items.get(1).and_then(|v| v.ident()).is_none() {
            // `not (feature)`: a media condition.
            return condition(&items, true, ctx, 0) == Some(Kleene::True);
        }
        (true, 1)
    } else if word.eq_ignore_ascii_case("only") {
        (false, 1)
    } else {
        (false, 0)
    };
    let Some(media_type) = items.get(type_index).and_then(|v| v.ident()) else {
        return false;
    };
    if RESERVED_TYPES
        .iter()
        .any(|r| r.eq_ignore_ascii_case(media_type))
    {
        return false;
    }
    let type_matches =
        media_type.eq_ignore_ascii_case("all") || media_type.eq_ignore_ascii_case("screen");
    let mut result = Kleene::from_bool(type_matches);
    let rest = &items[type_index + 1..];
    if !rest.is_empty() {
        if !rest[0].is_ident("and") {
            return false;
        }
        match condition(&rest[1..], false, ctx, 0) {
            Some(c) => result = result.and(c),
            None => return false,
        }
    }
    if negated {
        result = result.negate();
    }
    result == Kleene::True
}

/// `<media-condition>` (or `<media-condition-without-or>`); `None` on a syntax error.
fn condition(
    items: &[&ComponentValue],
    allow_or: bool,
    ctx: &MediaContext,
    depth: usize,
) -> Option<Kleene> {
    if depth > crate::parser::MAX_NESTING_DEPTH {
        return None;
    }
    let (first, rest) = items.split_first()?;
    if first.is_ident("not") {
        return match rest {
            [operand] => in_parens(operand, ctx, depth).map(Kleene::negate),
            _ => None,
        };
    }
    let mut result = in_parens(first, ctx, depth)?;
    let mut operator: Option<bool> = None; // Some(true) = and, Some(false) = or
    let mut rest = rest;
    while let [keyword, operand, tail @ ..] = rest {
        let is_and = if keyword.is_ident("and") {
            true
        } else if keyword.is_ident("or") && allow_or {
            false
        } else {
            return None;
        };
        if operator.is_some_and(|op| op != is_and) {
            return None;
        }
        operator = Some(is_and);
        let value = in_parens(operand, ctx, depth)?;
        result = if is_and {
            result.and(value)
        } else {
            result.or(value)
        };
        rest = tail;
    }
    rest.is_empty().then_some(result)
}

/// `<media-in-parens>`; `None` on a syntax error.
fn in_parens(item: &ComponentValue, ctx: &MediaContext, depth: usize) -> Option<Kleene> {
    match item {
        ComponentValue::Block { open: '(', content } => {
            let inner: Vec<&ComponentValue> =
                content.iter().filter(|v| !v.is_whitespace()).collect();
            let nested = inner.first().is_some_and(|v| {
                v.is_ident("not") || matches!(v, ComponentValue::Block { open: '(', .. })
            });
            if nested {
                // A nested condition, or general-enclosed (unknown).
                return Some(condition(&inner, true, ctx, depth + 1).unwrap_or(Kleene::Unknown));
            }
            Some(feature(trim(content), ctx).unwrap_or(Kleene::Unknown))
        }
        // `<general-enclosed>`
        ComponentValue::Function { .. } => Some(Kleene::Unknown),
        _ => None,
    }
}

/// A parsed `<mf-value>`.
#[derive(Clone, Debug)]
enum Value {
    /// A plain number (also the numerator of a ratio).
    Number(f64),
    /// A `<ratio>` (`16/9`), as numerator and denominator.
    Ratio(f64, f64),
    /// A length in px.
    Length(f64),
    /// A resolution in dppx.
    Resolution(f64),
    Ident(String),
}

/// Comparison operators of the range syntax.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Op {
    /// The operator with its operands swapped (`a < b` ⇔ `b > a`).
    fn flip(self) -> Op {
        match self {
            Op::Lt => Op::Gt,
            Op::Le => Op::Ge,
            Op::Gt => Op::Lt,
            Op::Ge => Op::Le,
            Op::Eq => Op::Eq,
        }
    }

    fn holds(self, ordering: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::*;
        match self {
            Op::Lt => ordering == Less,
            Op::Le => ordering != Greater,
            Op::Gt => ordering == Greater,
            Op::Ge => ordering != Less,
            Op::Eq => ordering == Equal,
        }
    }
}

/// How a feature's values are typed and compared.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Kind {
    Length,
    Ratio,
    Resolution,
    Integer,
    Number,
    Discrete,
}

/// Known features: name, kind, and whether `min-`/`max-` prefixes apply.
fn feature_kind(name: &str) -> Option<Kind> {
    Some(match name {
        "width" | "height" | "device-width" | "device-height" => Kind::Length,
        "aspect-ratio" | "device-aspect-ratio" => Kind::Ratio,
        "resolution" => Kind::Resolution,
        "color" | "color-index" | "monochrome" | "grid" => Kind::Integer,
        "-webkit-device-pixel-ratio" | "-moz-device-pixel-ratio" => Kind::Number,
        "orientation"
        | "prefers-color-scheme"
        | "prefers-reduced-motion"
        | "prefers-contrast"
        | "prefers-reduced-transparency"
        | "prefers-reduced-data"
        | "forced-colors"
        | "inverted-colors"
        | "hover"
        | "any-hover"
        | "pointer"
        | "any-pointer"
        | "scan"
        | "update"
        | "overflow-block"
        | "overflow-inline"
        | "color-gamut"
        | "display-mode"
        | "scripting"
        | "dynamic-range"
        | "video-dynamic-range"
        | "-webkit-transform-3d" => Kind::Discrete,
        _ => return None,
    })
}

/// The context's value for a numeric feature.
fn numeric_value(name: &str, ctx: &MediaContext) -> Value {
    match name {
        "width" | "device-width" => Value::Length(f64::from(ctx.width)),
        "height" | "device-height" => Value::Length(f64::from(ctx.height)),
        "aspect-ratio" | "device-aspect-ratio" => {
            Value::Ratio(f64::from(ctx.width), f64::from(ctx.height))
        }
        "resolution" => Value::Resolution(f64::from(ctx.resolution)),
        "-webkit-device-pixel-ratio" | "-moz-device-pixel-ratio" => {
            Value::Number(f64::from(ctx.resolution))
        }
        "color" => Value::Number(8.0),
        // color-index, monochrome, grid
        _ => Value::Number(0.0),
    }
}

/// Does the discrete feature `name` have the value `value`? `None` for an
/// unknown value. Boolean context (`value == None`) is true unless the
/// feature's value is `none` / `no-preference` / 0.
fn discrete_matches(name: &str, value: Option<&str>, ctx: &MediaContext) -> Option<bool> {
    let current: &str = match name {
        "orientation" => {
            if ctx.height >= ctx.width {
                "portrait"
            } else {
                "landscape"
            }
        }
        "prefers-color-scheme" => {
            if ctx.dark {
                "dark"
            } else {
                "light"
            }
        }
        "prefers-reduced-motion"
        | "prefers-contrast"
        | "prefers-reduced-transparency"
        | "prefers-reduced-data" => "no-preference",
        "forced-colors" | "inverted-colors" | "scripting" => "none",
        "hover" | "any-hover" => "hover",
        "pointer" | "any-pointer" => "fine",
        "update" => "fast",
        "overflow-block" | "overflow-inline" => "scroll",
        "color-gamut" => "srgb",
        "display-mode" => "browser",
        "dynamic-range" | "video-dynamic-range" => "standard",
        // Not a TV; no 3D transforms.
        _ => "",
    };
    let known_values: &[&str] = match name {
        "orientation" => &["portrait", "landscape"],
        "prefers-color-scheme" => &["light", "dark"],
        "prefers-reduced-motion" | "prefers-reduced-transparency" => &["no-preference", "reduce"],
        "prefers-contrast" => &["no-preference", "more", "less", "custom"],
        "prefers-reduced-data" => &["no-preference", "reduce"],
        "forced-colors" => &["none", "active"],
        "inverted-colors" => &["none", "inverted"],
        "scripting" => &["none", "initial-only", "enabled"],
        "hover" | "any-hover" => &["none", "hover"],
        "pointer" | "any-pointer" => &["none", "coarse", "fine"],
        "update" => &["none", "slow", "fast"],
        "overflow-block" => &["none", "scroll", "paged", "optional-paged"],
        "overflow-inline" => &["none", "scroll"],
        // srgb ⊂ p3 ⊂ rec2020: only srgb matches.
        "color-gamut" => &["srgb", "p3", "rec2020"],
        "display-mode" => &[
            "browser",
            "fullscreen",
            "standalone",
            "minimal-ui",
            "picture-in-picture",
            "window-controls-overlay",
        ],
        "dynamic-range" | "video-dynamic-range" => &["standard", "high"],
        "scan" => &["interlace", "progressive"],
        _ => &[],
    };
    match value {
        None => Some(!matches!(current, "" | "none" | "no-preference")),
        Some(v) => {
            let v = v.to_ascii_lowercase();
            if !known_values.contains(&v.as_str()) {
                return None;
            }
            Some(v == current)
        }
    }
}

/// Converts a length to px (em/rem = 16px, viewport units from `ctx`).
fn length_px(value: f32, unit: &str, ctx: &MediaContext) -> Option<f64> {
    let v = f64::from(value);
    let vw = f64::from(ctx.width) / 100.0;
    let vh = f64::from(ctx.height) / 100.0;
    Some(match unit {
        "px" => v,
        "em" | "rem" => v * 16.0,
        "ex" | "rex" | "ch" | "rch" => v * 8.0,
        "pt" => v * 96.0 / 72.0,
        "pc" => v * 16.0,
        "in" => v * 96.0,
        "cm" => v * 96.0 / 2.54,
        "mm" => v * 96.0 / 25.4,
        "q" => v * 96.0 / 101.6,
        "vw" | "svw" | "lvw" | "dvw" | "vi" => v * vw,
        "vh" | "svh" | "lvh" | "dvh" | "vb" => v * vh,
        "vmin" | "svmin" | "lvmin" | "dvmin" => v * vw.min(vh),
        "vmax" | "svmax" | "lvmax" | "dvmax" => v * vw.max(vh),
        _ => return None,
    })
}

fn resolution_dppx(value: f32, unit: &str) -> Option<f64> {
    let v = f64::from(value);
    Some(match unit {
        "dppx" | "x" => v,
        "dpi" => v / 96.0,
        "dpcm" => v * 2.54 / 96.0,
        _ => return None,
    })
}

/// Parses an `<mf-value>` (a number, ratio, dimension or identifier).
fn parse_value(items: &[&ComponentValue], ctx: &MediaContext) -> Option<Value> {
    match items {
        [ComponentValue::Token(Token::Number { value, .. })] => {
            Some(Value::Number(f64::from(*value)))
        }
        [ComponentValue::Token(Token::Dimension { value, unit })] => length_px(*value, unit, ctx)
            .map(Value::Length)
            .or_else(|| resolution_dppx(*value, unit).map(Value::Resolution)),
        [ComponentValue::Token(Token::Ident(name))] => Some(Value::Ident(name.clone())),
        [ComponentValue::Token(Token::Number { value: a, .. }), slash, ComponentValue::Token(Token::Number { value: b, .. })]
            if slash.is_delim('/') =>
        {
            Some(Value::Ratio(f64::from(*a), f64::from(*b)))
        }
        _ => None,
    }
}

/// Compares the context's value with a query value, for a feature of `kind`.
fn compare(kind: Kind, actual: &Value, wanted: &Value) -> Option<std::cmp::Ordering> {
    match (kind, actual, wanted) {
        (Kind::Length, Value::Length(a), Value::Length(b)) => a.partial_cmp(b),
        // A unitless zero is a valid length.
        (Kind::Length, Value::Length(a), Value::Number(b)) if *b == 0.0 => a.partial_cmp(b),
        (Kind::Ratio, Value::Ratio(an, ad), wanted) => {
            let (bn, bd) = match wanted {
                Value::Ratio(n, d) => (*n, *d),
                Value::Number(n) => (*n, 1.0),
                _ => return None,
            };
            if bn < 0.0 || bd < 0.0 || (bn == 0.0 && bd == 0.0) {
                return None;
            }
            // an/ad vs bn/bd, without dividing.
            (an * bd).partial_cmp(&(bn * ad))
        }
        (Kind::Resolution, Value::Resolution(a), Value::Resolution(b)) => a.partial_cmp(b),
        (Kind::Resolution, Value::Resolution(_), Value::Ident(i))
            if i.eq_ignore_ascii_case("infinite") =>
        {
            Some(std::cmp::Ordering::Less)
        }
        (Kind::Integer, Value::Number(a), Value::Number(b)) if b.fract() == 0.0 => a.partial_cmp(b),
        (Kind::Number, Value::Number(a), Value::Number(b)) => a.partial_cmp(b),
        _ => None,
    }
}

fn is_zero(value: &Value) -> bool {
    match value {
        Value::Number(v) | Value::Length(v) | Value::Resolution(v) => *v == 0.0,
        Value::Ratio(n, _) => *n == 0.0,
        Value::Ident(_) => false,
    }
}

/// Evaluates a `<media-feature>` (the content of its parentheses). `None`
/// means unknown (unknown feature, invalid value or syntax).
fn feature(content: &[ComponentValue], ctx: &MediaContext) -> Option<Kleene> {
    // Boolean context: `(name)`.
    if let [ComponentValue::Token(Token::Ident(name))] = content {
        let name = name.to_ascii_lowercase();
        let kind = feature_kind(&name)?;
        return Some(Kleene::from_bool(if kind == Kind::Discrete {
            discrete_matches(&name, None, ctx)?
        } else {
            !is_zero(&numeric_value(&name, ctx))
        }));
    }
    // Plain: `name: value`.
    if let Some(colon) = content
        .iter()
        .position(|v| matches!(v, ComponentValue::Token(Token::Colon)))
    {
        let [ComponentValue::Token(Token::Ident(name))] = trim(&content[..colon]) else {
            return None;
        };
        let value: Vec<&ComponentValue> = content[colon + 1..]
            .iter()
            .filter(|v| !v.is_whitespace())
            .collect();
        return plain_feature(&name.to_ascii_lowercase(), &value, ctx);
    }
    range_feature(content, ctx)
}

fn plain_feature(name: &str, value: &[&ComponentValue], ctx: &MediaContext) -> Option<Kleene> {
    let (base, op) = if let Some(base) = name.strip_prefix("min-") {
        (base, Op::Ge)
    } else if let Some(base) = name.strip_prefix("max-") {
        (base, Op::Le)
    } else if let Some(base) = name.strip_prefix("-webkit-min-") {
        (base, Op::Ge)
    } else if let Some(base) = name.strip_prefix("-webkit-max-") {
        (base, Op::Le)
    } else if let Some(base) = name.strip_prefix("min--moz-") {
        (base, Op::Ge)
    } else if let Some(base) = name.strip_prefix("max--moz-") {
        (base, Op::Le)
    } else {
        (name, Op::Eq)
    };
    let base = match base {
        "device-pixel-ratio" if name.contains("moz") => "-moz-device-pixel-ratio",
        "device-pixel-ratio" => "-webkit-device-pixel-ratio",
        other => other,
    };
    let kind = feature_kind(base)?;
    if kind == Kind::Discrete {
        if op != Op::Eq {
            return None;
        }
        let [ComponentValue::Token(Token::Ident(v))] = value else {
            return None;
        };
        return discrete_matches(base, Some(v), ctx).map(Kleene::from_bool);
    }
    let wanted = parse_value(value, ctx)?;
    let ordering = compare(kind, &numeric_value(base, ctx), &wanted)?;
    Some(Kleene::from_bool(op.holds(ordering)))
}

/// Splits range-syntax content at its comparison operators.
fn range_feature(content: &[ComponentValue], ctx: &MediaContext) -> Option<Kleene> {
    let mut parts: Vec<Vec<&ComponentValue>> = vec![Vec::new()];
    let mut ops: Vec<Op> = Vec::new();
    let mut i = 0;
    while let Some(item) = content.get(i) {
        let op = match item {
            ComponentValue::Token(Token::Delim(c @ ('<' | '>'))) => {
                let or_equal = content.get(i + 1).is_some_and(|v| v.is_delim('='));
                if or_equal {
                    i += 1;
                }
                Some(match (c, or_equal) {
                    ('<', false) => Op::Lt,
                    ('<', true) => Op::Le,
                    ('>', false) => Op::Gt,
                    _ => Op::Ge,
                })
            }
            ComponentValue::Token(Token::Delim('=')) => Some(Op::Eq),
            _ => None,
        };
        match op {
            Some(op) => {
                ops.push(op);
                parts.push(Vec::new());
            }
            None => {
                if !item.is_whitespace() {
                    if let Some(part) = parts.last_mut() {
                        part.push(item);
                    }
                }
            }
        }
        i += 1;
    }
    let feature_name = |part: &[&ComponentValue]| -> Option<String> {
        match part {
            [v] => {
                let name = v.ident()?.to_ascii_lowercase();
                let kind = feature_kind(&name)?;
                (kind != Kind::Discrete).then_some(name)
            }
            _ => None,
        }
    };
    match (parts.as_slice(), ops.as_slice()) {
        ([left, right], [op]) => {
            let (name, value, op) = if let Some(name) = feature_name(left) {
                (name, right, *op)
            } else {
                (feature_name(right)?, left, op.flip())
            };
            let kind = feature_kind(&name)?;
            let wanted = parse_value(value, ctx)?;
            let ordering = compare(kind, &numeric_value(&name, ctx), &wanted)?;
            Some(Kleene::from_bool(op.holds(ordering)))
        }
        ([low, middle, high], [op1, op2]) => {
            let ascending = matches!(op1, Op::Lt | Op::Le) && matches!(op2, Op::Lt | Op::Le);
            let descending = matches!(op1, Op::Gt | Op::Ge) && matches!(op2, Op::Gt | Op::Ge);
            if !ascending && !descending {
                return None;
            }
            let name = feature_name(middle)?;
            let kind = feature_kind(&name)?;
            let actual = numeric_value(&name, ctx);
            let low_value = parse_value(low, ctx)?;
            let high_value = parse_value(high, ctx)?;
            // `low op1 name` ⇔ `name flip(op1) low`; `name op2 high`.
            let first = op1.flip().holds(compare(kind, &actual, &low_value)?);
            let second = op2.holds(compare(kind, &actual, &high_value)?);
            Some(Kleene::from_bool(first && second))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
