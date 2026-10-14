//! `calc()`, `min()`, `max()` and `clamp()`.
//!
//! Expressions are parsed once per declaration. Sums and products by numbers
//! are folded into a single [`LinearLen`]; only `min()`/`max()`/`clamp()`
//! remain as tree nodes, evaluated per element once font sizes are known.
//! Number-only expressions fold to a constant.

use xbro_css::values::split_commas;
use xbro_css::{ComponentValue as CV, LengthUnit, Token};

use super::length::{sane, LinearLen, Resolver};
use crate::computed::LengthPercentage;

/// Maximum nesting of parentheses / math functions.
const MAX_DEPTH: usize = 32;

/// Maximum number of nodes of a length expression kept for per-element
/// evaluation (larger expressions are rejected): an expression is evaluated
/// for every element it applies to, so its size must stay small.
const MAX_NODES: usize = 256;

/// A length-valued math expression.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MathExpr {
    Linear(LinearLen),
    Sum(Vec<MathExpr>),
    Scale(Box<MathExpr>, f32),
    Min(Vec<MathExpr>),
    Max(Vec<MathExpr>),
    /// `clamp(min, value, max)`
    Clamp(Box<[MathExpr; 3]>),
}

/// Result of parsing a math function: a constant number or a length expression.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Calculated {
    Number(f32),
    Length(MathExpr),
}

impl MathExpr {
    pub const ZERO: Self = MathExpr::Linear(LinearLen::ZERO);

    pub const fn px(px: f32) -> Self {
        MathExpr::Linear(LinearLen::px(px))
    }

    pub const fn percent(p: f32) -> Self {
        MathExpr::Linear(LinearLen::percent(p))
    }

    /// Number of nodes of the expression tree.
    fn node_count(&self) -> usize {
        1 + match self {
            MathExpr::Linear(_) => 0,
            MathExpr::Sum(v) | MathExpr::Min(v) | MathExpr::Max(v) => {
                v.iter().map(MathExpr::node_count).sum()
            }
            MathExpr::Scale(e, _) => e.node_count(),
            MathExpr::Clamp(args) => args.iter().map(MathExpr::node_count).sum(),
        }
    }

    /// True when a percentage appears anywhere in the expression.
    pub fn has_percent(&self) -> bool {
        match self {
            MathExpr::Linear(l) => l.has_percent(),
            MathExpr::Sum(v) | MathExpr::Min(v) | MathExpr::Max(v) => {
                v.iter().any(MathExpr::has_percent)
            }
            MathExpr::Scale(e, _) => e.has_percent(),
            MathExpr::Clamp(args) => args.iter().any(MathExpr::has_percent),
        }
    }

    /// Adds two length expressions, folding linear terms.
    pub fn plus(self, other: MathExpr) -> MathExpr {
        match (self, other) {
            (MathExpr::Linear(a), MathExpr::Linear(b)) => MathExpr::Linear(a.add(b)),
            (MathExpr::Sum(mut a), MathExpr::Sum(b)) => {
                a.extend(b);
                MathExpr::Sum(a)
            }
            (MathExpr::Sum(mut a), b) => {
                a.push(b);
                MathExpr::Sum(a)
            }
            (a, MathExpr::Sum(mut b)) => {
                b.insert(0, a);
                MathExpr::Sum(b)
            }
            (a, b) => MathExpr::Sum(vec![a, b]),
        }
    }

    /// Multiplies by a constant, folding linear terms.
    pub fn times(self, k: f32) -> MathExpr {
        match self {
            MathExpr::Linear(l) => MathExpr::Linear(l.scale(k)),
            MathExpr::Scale(e, k2) => MathExpr::Scale(e, k * k2),
            e => MathExpr::Scale(Box::new(e), k),
        }
    }

    /// Evaluates to px + percent with `r`.
    ///
    /// `min()`/`max()`/`clamp()` are exact when all their arguments are pure
    /// lengths or all pure percentages; when they mix both, the percentages
    /// are resolved against the viewport width (approximation).
    pub fn eval(&self, r: &Resolver) -> LengthPercentage {
        match self {
            MathExpr::Linear(l) => l.resolve(r),
            MathExpr::Sum(terms) => terms.iter().fold(LengthPercentage::ZERO, |acc, t| {
                let v = t.eval(r);
                LengthPercentage {
                    px: sane(acc.px + v.px),
                    percent: sane(acc.percent + v.percent),
                }
            }),
            MathExpr::Scale(e, k) => {
                let v = e.eval(r);
                LengthPercentage {
                    px: sane(v.px * k),
                    percent: sane(v.percent * k),
                }
            }
            MathExpr::Min(args) => extremum(&evals(args, r), true, r),
            MathExpr::Max(args) => extremum(&evals(args, r), false, r),
            MathExpr::Clamp(args) => {
                let [lo, val, hi] = &**args;
                let upper = extremum(&[val.eval(r), hi.eval(r)], true, r);
                extremum(&[lo.eval(r), upper], false, r)
            }
        }
    }
}

fn evals(args: &[MathExpr], r: &Resolver) -> Vec<LengthPercentage> {
    args.iter().map(|a| a.eval(r)).collect()
}

/// Minimum (`min = true`) or maximum of evaluated arguments.
fn extremum(values: &[LengthPercentage], min: bool, r: &Resolver) -> LengthPercentage {
    let pick = |it: &mut dyn Iterator<Item = f32>| -> f32 {
        let first = it.next().unwrap_or(0.0);
        it.fold(first, |a, b| if min { a.min(b) } else { a.max(b) })
    };
    if values.iter().all(|v| v.percent == 0.0) {
        LengthPercentage::px(pick(&mut values.iter().map(|v| v.px)))
    } else if values.iter().all(|v| v.px == 0.0) {
        LengthPercentage::percent(pick(&mut values.iter().map(|v| v.percent)))
    } else {
        let basis = r.viewport_width;
        LengthPercentage::px(sane(pick(
            &mut values.iter().map(|v| v.px + v.percent * basis / 100.0),
        )))
    }
}

/// True for the function names handled by [`parse_math_function`].
pub(crate) fn is_math_function(name: &str) -> bool {
    ["calc", "-webkit-calc", "-moz-calc", "min", "max", "clamp"]
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
}

/// Parses `calc()` / `min()` / `max()` / `clamp()`; `None` when invalid
/// (unknown function, type mismatch, division by zero, ...) or when the
/// resulting length expression has more than [`MAX_NODES`] nodes.
pub(crate) fn parse_math_function(name: &str, args: &[CV]) -> Option<Calculated> {
    match function(name, args, 0)? {
        Calculated::Length(e) if e.node_count() > MAX_NODES => None,
        other => Some(other),
    }
}

fn significant(values: &[CV]) -> Vec<&CV> {
    values.iter().filter(|v| !v.is_whitespace()).collect()
}

fn function(name: &str, args: &[CV], depth: usize) -> Option<Calculated> {
    if depth > MAX_DEPTH {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "calc" | "-webkit-calc" | "-moz-calc" => sum(&significant(args), depth + 1),
        "min" | "max" => {
            let parts = split_commas(args);
            let values = parts
                .iter()
                .map(|p| sum(&significant(p), depth + 1))
                .collect::<Option<Vec<_>>>()?;
            min_max(values, lower == "min")
        }
        "clamp" => {
            let parts = split_commas(args);
            let [lo, val, hi] = parts.as_slice() else {
                return None;
            };
            let lo = sum(&significant(lo), depth + 1)?;
            let val = sum(&significant(val), depth + 1)?;
            let hi = sum(&significant(hi), depth + 1)?;
            match (lo, val, hi) {
                (Calculated::Number(a), Calculated::Number(b), Calculated::Number(c)) => {
                    Some(Calculated::Number(a.max(b.min(c))))
                }
                (Calculated::Length(a), Calculated::Length(b), Calculated::Length(c)) => {
                    Some(Calculated::Length(MathExpr::Clamp(Box::new([a, b, c]))))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn min_max(values: Vec<Calculated>, is_min: bool) -> Option<Calculated> {
    if values.is_empty() {
        return None;
    }
    if values.iter().all(|v| matches!(v, Calculated::Number(_))) {
        let nums = values.iter().filter_map(|v| match v {
            Calculated::Number(n) => Some(*n),
            Calculated::Length(_) => None,
        });
        let folded = nums.fold(None, |acc: Option<f32>, n| {
            Some(match acc {
                None => n,
                Some(a) if is_min => a.min(n),
                Some(a) => a.max(n),
            })
        })?;
        return Some(Calculated::Number(folded));
    }
    let lengths = values
        .into_iter()
        .map(|v| match v {
            Calculated::Length(e) => Some(e),
            Calculated::Number(_) => None,
        })
        .collect::<Option<Vec<_>>>()?;
    // Fold constant px-only arguments.
    let all_px = lengths
        .iter()
        .all(|e| matches!(e, MathExpr::Linear(l) if *l == LinearLen::px(l.px)));
    if all_px {
        let pxs = lengths.iter().filter_map(|e| match e {
            MathExpr::Linear(l) => Some(l.px),
            _ => None,
        });
        let folded = pxs.fold(None, |acc: Option<f32>, n| {
            Some(match acc {
                None => n,
                Some(a) if is_min => a.min(n),
                Some(a) => a.max(n),
            })
        })?;
        return Some(Calculated::Length(MathExpr::px(folded)));
    }
    Some(Calculated::Length(if is_min {
        MathExpr::Min(lengths)
    } else {
        MathExpr::Max(lengths)
    }))
}

/// `<calc-sum>`: products separated by `+` / `-`.
fn sum(items: &[&CV], depth: usize) -> Option<Calculated> {
    if depth > MAX_DEPTH || items.is_empty() {
        return None;
    }
    let mut acc: Option<Calculated> = None;
    let mut start = 0;
    let mut sign = 1.0f32;
    let apply = |acc: Option<Calculated>, slice: &[&CV], sign: f32| -> Option<Calculated> {
        let term = negate_if(product(slice, depth)?, sign < 0.0);
        Some(match acc {
            None => term,
            Some(a) => add(a, term)?,
        })
    };
    for (i, item) in items.iter().enumerate() {
        let next_sign = if item.is_delim('+') {
            1.0
        } else if item.is_delim('-') {
            -1.0
        } else {
            continue;
        };
        acc = Some(apply(acc, items.get(start..i)?, sign)?);
        sign = next_sign;
        start = i + 1;
    }
    apply(acc, items.get(start..)?, sign)
}

fn negate_if(v: Calculated, negate: bool) -> Calculated {
    if !negate {
        return v;
    }
    match v {
        Calculated::Number(n) => Calculated::Number(-n),
        Calculated::Length(e) => Calculated::Length(e.times(-1.0)),
    }
}

fn add(a: Calculated, b: Calculated) -> Option<Calculated> {
    match (a, b) {
        (Calculated::Number(x), Calculated::Number(y)) => Some(Calculated::Number(x + y)),
        (Calculated::Length(x), Calculated::Length(y)) => Some(Calculated::Length(x.plus(y))),
        _ => None,
    }
}

/// `<calc-product>`: values separated by `*` / `/`.
fn product(items: &[&CV], depth: usize) -> Option<Calculated> {
    let (first, mut rest) = items.split_first()?;
    let mut acc = value(first, depth)?;
    while let [op, operand, tail @ ..] = rest {
        let rhs = value(operand, depth)?;
        acc = if op.is_delim('*') {
            match (acc, rhs) {
                (Calculated::Number(a), Calculated::Number(b)) => Calculated::Number(a * b),
                (Calculated::Number(k), Calculated::Length(e))
                | (Calculated::Length(e), Calculated::Number(k)) => Calculated::Length(e.times(k)),
                _ => return None,
            }
        } else if op.is_delim('/') {
            let Calculated::Number(k) = rhs else {
                return None;
            };
            if k == 0.0 || !k.is_finite() {
                return None;
            }
            match acc {
                Calculated::Number(a) => Calculated::Number(a / k),
                Calculated::Length(e) => Calculated::Length(e.times(1.0 / k)),
            }
        } else {
            return None;
        };
        rest = tail;
    }
    rest.is_empty().then_some(acc)
}

/// `<calc-value>`.
fn value(item: &CV, depth: usize) -> Option<Calculated> {
    match item {
        CV::Token(Token::Number { value, .. }) => Some(Calculated::Number(*value)),
        CV::Token(Token::Percentage(p)) => Some(Calculated::Length(MathExpr::percent(*p))),
        CV::Token(Token::Dimension { value, unit }) => {
            let unit = LengthUnit::from_name(unit).filter(|u| *u != LengthUnit::Percent)?;
            Some(Calculated::Length(MathExpr::Linear(LinearLen::from_unit(
                *value, unit,
            ))))
        }
        CV::Block { open: '(', content } => sum(&significant(content), depth + 1),
        CV::Function { name, args } => function(name, args, depth + 1),
        CV::Token(Token::Ident(name)) => {
            if name.eq_ignore_ascii_case("pi") {
                Some(Calculated::Number(std::f32::consts::PI))
            } else if name.eq_ignore_ascii_case("e") {
                Some(Calculated::Number(std::f32::consts::E))
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_css::parse_component_values;

    fn parse(css: &str) -> Option<Calculated> {
        let values = parse_component_values(css);
        match values.as_slice() {
            [CV::Function { name, args }] => parse_math_function(name, args),
            _ => None,
        }
    }

    fn resolver() -> Resolver {
        Resolver::new(10.0, 16.0, false, (1000.0, 800.0))
    }

    fn eval(css: &str) -> LengthPercentage {
        match parse(css) {
            Some(Calculated::Length(e)) => e.eval(&resolver()),
            other => panic!("{css}: {other:?}"),
        }
    }

    #[test]
    fn linear_folding() {
        assert_eq!(
            parse("calc(10px + 2em)"),
            Some(Calculated::Length(MathExpr::Linear(LinearLen {
                px: 10.0,
                em: 2.0,
                ..LinearLen::ZERO
            })))
        );
        assert_eq!(
            eval("calc(100% - 20px)"),
            LengthPercentage {
                px: -20.0,
                percent: 100.0
            }
        );
        let third = eval("calc((100% - 30px) / 3)");
        assert!((third.px + 10.0).abs() < 1e-4 && (third.percent - 100.0 / 3.0).abs() < 1e-4);
        assert_eq!(eval("calc(2 * 1em + 1rem)").px, 36.0);
        assert_eq!(eval("calc(1px * 2 * 3)").px, 6.0);
        assert_eq!(eval("calc(calc(5px + 5px) * 2)").px, 20.0);
        assert_eq!(eval("-webkit-calc(1in - 6px)").px, 90.0);
        assert_eq!(eval("calc(10vw - 1vh)").px, 92.0);
    }

    #[test]
    fn numbers() {
        assert_eq!(parse("calc(1 + 2 * 3)"), Some(Calculated::Number(7.0)));
        assert_eq!(parse("calc((1 + 2) * 3)"), Some(Calculated::Number(9.0)));
        assert_eq!(parse("calc(6 / 4)"), Some(Calculated::Number(1.5)));
        assert_eq!(parse("min(3, 1, 2)"), Some(Calculated::Number(1.0)));
        assert_eq!(parse("clamp(1, 5, 3)"), Some(Calculated::Number(3.0)));
    }

    #[test]
    fn invalid_expressions() {
        for css in [
            "calc(1px * 2px)",
            "calc(1px / 0)",
            "calc(2 / 1px)",
            "calc(1px + 2)",
            "calc()",
            "calc(1px +)",
            "calc(+ 1px)",
            "calc(1px 2px)",
            "calc(10deg)",
            "calc(foo)",
            "min()",
            "clamp(1px, 2px)",
            "round(1px, 2px)",
            "max(1px, 2)",
        ] {
            assert_eq!(parse(css), None, "{css}");
        }
    }

    #[test]
    fn min_max_clamp() {
        assert_eq!(eval("min(10px, 2em, 30px)").px, 10.0);
        assert_eq!(eval("max(10px, 2em, 5px)").px, 20.0);
        assert_eq!(eval("min(50%, 20%)"), LengthPercentage::percent(20.0));
        // Mixed: percentages against the viewport width (1000px).
        assert_eq!(eval("min(50%, 600px)"), LengthPercentage::px(500.0));
        assert_eq!(eval("max(50%, 600px)"), LengthPercentage::px(600.0));
        assert_eq!(eval("clamp(1rem, 2.5vw, 2rem)").px, 25.0);
        assert_eq!(eval("clamp(1rem, 1vw, 2rem)").px, 16.0);
        assert_eq!(eval("clamp(1rem, 10vw, 2rem)").px, 32.0);
        assert_eq!(eval("calc(min(10px, 20px) + 5px)").px, 15.0);
        assert_eq!(eval("calc(max(1em, 5px) * 2)").px, 20.0);
    }

    #[test]
    fn huge_expressions_are_rejected() {
        // Terms that cannot be folded into one linear length.
        let term = "min(1em, 2px)";
        let small = format!("calc({})", vec![term; 20].join(" + "));
        assert_eq!(eval(&small).px, 40.0);
        let huge = format!("calc({})", vec![term; 5_000].join(" + "));
        assert_eq!(parse(&huge), None);
        // Foldable terms stay a single node, whatever their number.
        let linear = format!("calc({})", vec!["1px"; 5_000].join(" + "));
        assert_eq!(eval(&linear).px, 5_000.0);
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let mut css = String::new();
        for _ in 0..200 {
            css.push_str("calc(");
        }
        css.push_str("1px");
        for _ in 0..200 {
            css.push(')');
        }
        // Either rejected or evaluated, but never a stack overflow.
        let _ = parse(&css);
    }
}
