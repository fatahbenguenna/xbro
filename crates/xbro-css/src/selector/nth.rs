//! The `An+B` microsyntax (CSS Syntax §6) used by `:nth-*()`.

use crate::parser::ComponentValue as CV;
use crate::tokenizer::Token;
use crate::values::trim;

fn clamp_i32(v: i64) -> i32 {
    // Lossless after the clamp.
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Integer value of a whole `f32` (tokens only carry `f32`).
fn integral(value: f32) -> Option<i64> {
    (value.is_finite() && value.fract() == 0.0).then_some(value as i64)
}

/// Parses the digits of `-123` style suffixes; saturates on overflow.
fn parse_digits(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(s.bytes().fold(0i64, |acc, b| {
        acc.saturating_mul(10).saturating_add(i64::from(b - b'0'))
    }))
}

/// Parses `An+B` (`odd`, `even`, `3`, `-n+3`, `2n+1`, `+5`, `2n - 1`, ...).
/// Returns `None` for anything else, including `of S` selectors.
pub(crate) fn parse_nth(args: &[CV]) -> Option<(i32, i32)> {
    let items = trim(args);
    let (first, rest) = items.split_first()?;
    let (a, b) = match first {
        CV::Token(Token::Ident(s)) if s.eq_ignore_ascii_case("odd") => {
            only_whitespace(rest)?;
            (2, 1)
        }
        CV::Token(Token::Ident(s)) if s.eq_ignore_ascii_case("even") => {
            only_whitespace(rest)?;
            (2, 0)
        }
        CV::Token(Token::Number {
            value,
            is_integer: true,
        }) => {
            only_whitespace(rest)?;
            (0, integral(*value)?)
        }
        CV::Token(Token::Dimension { value, unit }) => {
            let a = integral(*value)?;
            (a, n_suffix(unit, rest)?)
        }
        CV::Token(Token::Ident(s)) => match s.strip_prefix('-') {
            Some(after) => (-1, n_suffix(after, rest)?),
            None => (1, n_suffix(s, rest)?),
        },
        // `+n...`: the `+` must be immediately followed by the identifier.
        CV::Token(Token::Delim('+')) => match rest.split_first() {
            Some((CV::Token(Token::Ident(s)), rest)) if !s.starts_with('-') => {
                (1, n_suffix(s, rest)?)
            }
            _ => return None,
        },
        _ => return None,
    };
    Some((clamp_i32(a), clamp_i32(b)))
}

fn only_whitespace(rest: &[CV]) -> Option<()> {
    rest.iter().all(CV::is_whitespace).then_some(())
}

/// Parses what follows the `A` part: `s` is the remaining text of the token
/// that holds `n` (e.g. `n`, `n-`, `n-3`), `rest` the following values.
/// Returns `B`.
fn n_suffix(s: &str, rest: &[CV]) -> Option<i64> {
    let mut chars = s.chars();
    if !matches!(chars.next(), Some('n' | 'N')) {
        return None;
    }
    let after = chars.as_str();
    let mut i = 0;
    skip_whitespace(rest, &mut i);
    let b = if after.is_empty() {
        // `An`, `An+B`, `An + B`, `An -B`, ...
        match rest.get(i) {
            None => 0,
            Some(CV::Token(Token::Number {
                value,
                is_integer: true,
            })) => {
                // Must be a signed integer: without whitespace the sign is
                // certain (`2n+1`); with whitespace a sign is assumed since
                // tokens do not record it (`2n 1` is accepted as `2n+1`).
                i += 1;
                integral(*value)?
            }
            Some(sign @ CV::Token(Token::Delim('+' | '-'))) => {
                let negative = sign.is_delim('-');
                i += 1;
                skip_whitespace(rest, &mut i);
                let value = signless_integer(rest.get(i)?)?;
                i += 1;
                if negative {
                    -value
                } else {
                    value
                }
            }
            Some(_) => return None,
        }
    } else if after == "-" {
        // `An- B`
        let value = signless_integer(rest.get(i)?)?;
        i += 1;
        -value
    } else {
        // `An-B` in one token.
        let digits = after.strip_prefix('-')?;
        if i < rest.len() {
            return None;
        }
        -parse_digits(digits)?
    };
    skip_whitespace(rest, &mut i);
    (i == rest.len()).then_some(b)
}

fn signless_integer(value: &CV) -> Option<i64> {
    match value {
        CV::Token(Token::Number {
            value,
            is_integer: true,
        }) if *value >= 0.0 => integral(*value),
        _ => None,
    }
}

fn skip_whitespace(items: &[CV], i: &mut usize) {
    while items.get(*i).is_some_and(CV::is_whitespace) {
        *i += 1;
    }
}

/// Does the 1-based `index` match `An+B` (for some n ≥ 0)? Computed in
/// `i128`, so no input can overflow.
pub(crate) fn nth_matches(a: i32, b: i32, index: i64) -> bool {
    let (a, b, index) = (i128::from(a), i128::from(b), i128::from(index));
    if a == 0 {
        return index == b;
    }
    let diff = index - b;
    diff % a == 0 && diff / a >= 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_component_values;

    fn nth(s: &str) -> Option<(i32, i32)> {
        parse_nth(&parse_component_values(s))
    }

    #[test]
    fn keywords_and_integers() {
        assert_eq!(nth("odd"), Some((2, 1)));
        assert_eq!(nth(" EVEN "), Some((2, 0)));
        assert_eq!(nth("3"), Some((0, 3)));
        assert_eq!(nth("+5"), Some((0, 5)));
        assert_eq!(nth("-2"), Some((0, -2)));
        assert_eq!(nth("1.5"), None);
    }

    #[test]
    fn an_plus_b_forms() {
        assert_eq!(nth("n"), Some((1, 0)));
        assert_eq!(nth("N"), Some((1, 0)));
        assert_eq!(nth("-n"), Some((-1, 0)));
        assert_eq!(nth("+n"), Some((1, 0)));
        assert_eq!(nth("2n"), Some((2, 0)));
        assert_eq!(nth("2n+1"), Some((2, 1)));
        assert_eq!(nth("2N+1"), Some((2, 1)));
        assert_eq!(nth("2n-1"), Some((2, -1)));
        assert_eq!(nth("-n+3"), Some((-1, 3)));
        assert_eq!(nth("-n-3"), Some((-1, -3)));
        assert_eq!(nth("+n+3"), Some((1, 3)));
        assert_eq!(nth("n-3"), Some((1, -3)));
        assert_eq!(nth("2n + 1"), Some((2, 1)));
        assert_eq!(nth("2n - 1"), Some((2, -1)));
        assert_eq!(nth("2n +1"), Some((2, 1)));
        assert_eq!(nth("2n -1"), Some((2, -1)));
        assert_eq!(nth("2n- 1"), Some((2, -1)));
        assert_eq!(nth("n- 1"), Some((1, -1)));
        assert_eq!(nth("-2n+4"), Some((-2, 4)));
        assert_eq!(nth("0n+3"), Some((0, 3)));
    }

    #[test]
    fn invalid_forms() {
        assert_eq!(nth(""), None);
        assert_eq!(nth("- n"), None);
        assert_eq!(nth("+ n"), None);
        assert_eq!(nth("+-n"), None);
        assert_eq!(nth("2n + -1"), None);
        assert_eq!(nth("2n+"), None);
        assert_eq!(nth("2n 1 2"), None);
        assert_eq!(nth("n of .a"), None);
        assert_eq!(nth("2m+1"), None);
        assert_eq!(nth("2.5n"), None);
        assert_eq!(nth("2n-1a"), None);
        assert_eq!(nth("foo"), None);
        assert_eq!(nth("odd 1"), None);
    }

    #[test]
    fn huge_values_saturate() {
        assert_eq!(nth("99999999999"), Some((0, i32::MAX)));
        assert_eq!(nth("n-99999999999999999999999"), Some((1, i32::MIN)));
    }

    #[test]
    fn matching() {
        // odd
        assert!(nth_matches(2, 1, 1));
        assert!(!nth_matches(2, 1, 2));
        assert!(nth_matches(2, 1, 3));
        // -n+3: first three
        assert!(nth_matches(-1, 3, 1));
        assert!(nth_matches(-1, 3, 3));
        assert!(!nth_matches(-1, 3, 4));
        // exact index
        assert!(nth_matches(0, 4, 4));
        assert!(!nth_matches(0, 4, 5));
        // 3n-1: 2, 5, 8
        assert!(nth_matches(3, -1, 2));
        assert!(nth_matches(3, -1, 5));
        assert!(!nth_matches(3, -1, 3));
        // n+5: 5 and later
        assert!(!nth_matches(1, 5, 4));
        assert!(nth_matches(1, 5, 9));
        // Extreme values do not overflow.
        assert!(!nth_matches(i32::MIN, i32::MAX, 1));
        assert!(!nth_matches(-1, i32::MIN, 1));
        assert!(nth_matches(1, i32::MIN, i64::MAX));
        assert!(!nth_matches(-1, i32::MIN, i64::MAX));
        assert!(nth_matches(i32::MIN, 0, 0));
        assert!(!nth_matches(2, 1, i64::MIN));
    }
}
