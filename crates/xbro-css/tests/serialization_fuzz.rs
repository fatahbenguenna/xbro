//! Property tests: `to_css` output re-tokenizes to the same component values.

mod support;

use support::Rng;
use xbro_css::parser::{parse_component_values, to_css, ComponentValue as CV};
use xbro_css::tokenizer::Token;

fn number(value: f32) -> Token {
    Token::Number {
        value,
        is_integer: value.fract() == 0.0 && value.abs() < 1e7,
    }
}

/// A token the tokenizer could have produced.
fn random_token(rng: &mut Rng) -> Token {
    const IDENTS: &[&str] = &[
        "a", "--", "-a", "e", "e5", "a-", "x1", "\u{e9}", "-", "-1", "1a", "a b", "n-1", "url", "_",
    ];
    const HASHES: &[(&str, bool)] = &[
        ("a", true),
        ("1a", false),
        ("-", false),
        ("--", true),
        ("-a", true),
        ("-1", false),
        ("a b", true),
        ("\u{e9}", true),
    ];
    const DELIMS: &[char] = &[
        '-', '+', '.', '#', '@', '<', '!', '/', '*', '%', '>', '\\', '&', '~', '|', '=', '$', '^',
        '?', '`',
    ];
    const NUMBERS: &[f32] = &[0.0, 1.0, -1.0, 1.5, -0.5, 1e20, 1e-7, 12.0, 0.25];
    const UNITS: &[&str] = &["px", "e", "e3", "e-1", "-x", "--", "n-1", "\u{e9}", "x y"];
    match rng.below(20) {
        0 | 1 => Token::Ident(rng.pick(IDENTS).to_string()),
        2 => Token::AtKeyword(rng.pick(IDENTS).to_string()),
        3 => {
            let (value, is_id) = *rng.pick(HASHES);
            Token::Hash {
                value: value.to_string(),
                is_id,
            }
        }
        4 => Token::String(rng.pick(&["", "a\"b", "x\\y", "\n", "\u{1}"]).to_string()),
        5 => Token::BadString,
        6 => Token::Url(rng.pick(&["", "a b", "x(y)", "q'\"", "\u{7f}"]).to_string()),
        7 => Token::BadUrl,
        8..=11 => Token::Delim(*rng.pick(DELIMS)),
        12 => number(*rng.pick(NUMBERS)),
        13 => Token::Percentage(*rng.pick(NUMBERS)),
        14 => Token::Dimension {
            value: *rng.pick(NUMBERS),
            unit: rng.pick(UNITS).to_string(),
        },
        15 => Token::Whitespace,
        16 => Token::Cdo,
        17 => Token::Cdc,
        18 => rng
            .pick(&[Token::Colon, Token::Semicolon, Token::Comma])
            .clone(),
        _ => Token::Delim('-'),
    }
}

fn random_values(rng: &mut Rng, depth: usize) -> Vec<CV> {
    let len = rng.below(8);
    (0..len)
        .map(|_| {
            if depth < 3 && rng.chance(15) {
                let content = random_values(rng, depth + 1);
                match rng.below(4) {
                    0 => CV::Function {
                        name: rng.pick(&["f", "-x", "rgb", "e1"]).to_string(),
                        args: content,
                    },
                    n => CV::Block {
                        open: ['(', '[', '{'][n - 1],
                        content,
                    },
                }
            } else {
                CV::Token(random_token(rng))
            }
        })
        .collect()
}

/// What the tokenizer gives back for `values`: a whitespace run is one token,
/// and bad strings / stray backslashes are serialized with a newline after
/// them, which reads back as whitespace.
fn normalize(values: &[CV]) -> Vec<CV> {
    let mut out: Vec<CV> = Vec::new();
    for value in values {
        let value = match value {
            CV::Function { name, args } => CV::Function {
                name: name.clone(),
                args: normalize(args),
            },
            CV::Block { open, content } => CV::Block {
                open: *open,
                content: normalize(content),
            },
            other => other.clone(),
        };
        let trailing_ws = matches!(
            value,
            CV::Token(Token::BadString) | CV::Token(Token::Delim('\\'))
        );
        if !(value.is_whitespace() && out.last().is_some_and(CV::is_whitespace)) {
            out.push(value);
        }
        if trailing_ws && !out.last().is_some_and(CV::is_whitespace) {
            out.push(CV::Token(Token::Whitespace));
        }
    }
    // Whitespace following a newline-terminated token merges with it.
    out.dedup_by(|b, a| a.is_whitespace() && b.is_whitespace());
    out
}

#[test]
fn random_token_sequences_round_trip() {
    let mut rng = Rng::new(0x5eed);
    for case in 0..20_000 {
        let values = random_values(&mut rng, 0);
        let css = to_css(&values);
        let expected = normalize(&values);
        let reparsed = parse_component_values(&css);
        assert_eq!(
            normalize(&reparsed),
            expected,
            "case {case}: {values:?} serialized as {css:?}"
        );
    }
}

#[test]
fn known_merging_pairs_are_separated() {
    let t = |v: Vec<Token>| v.into_iter().map(CV::Token).collect::<Vec<_>>();
    let cases = [
        // `--` followed by `>` is a CDC token.
        t(vec![Token::Ident("--".into()), Token::Delim('>')]),
        // `-` followed by `-->` starts the identifier `---`.
        t(vec![Token::Delim('-'), Token::Cdc]),
        // `<` `!` `--x` would form a CDO token.
        t(vec![
            Token::Delim('<'),
            Token::Delim('!'),
            Token::Ident("--x".into()),
        ]),
        t(vec![number(1.0), Token::Delim('-'), Token::Cdc]),
    ];
    for values in cases {
        let css = to_css(&values);
        assert_eq!(
            parse_component_values(&css),
            values,
            "serialized as {css:?}"
        );
    }
}

#[test]
fn real_css_round_trips_through_to_css() {
    // Serializing parsed CSS and parsing it again is a fixed point.
    let inputs = [
        "a{b:c}--><!-- x",
        "1--->x",
        "@m\\31 x #-- u+1-2 1e3e3 1\\65 3",
        "url( a\\)b ) url(\"x\") f(1 2)[3]{4}",
        "-\\-> <\\!-- \\--> a--b",
        "+.5 -.5e-3px .1. 1.e2",
    ];
    for css in inputs {
        let values = parse_component_values(css);
        let serialized = to_css(&values);
        assert_eq!(
            normalize(&parse_component_values(&serialized)),
            normalize(&values),
            "{css:?} serialized as {serialized:?}"
        );
    }
}
