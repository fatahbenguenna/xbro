//! Serialization of component values back to CSS text (CSS Syntax §9 and
//! CSSOM "serialize an identifier" / "serialize a string").

use std::fmt::Write;

use super::ComponentValue;
use crate::tokenizer::Token;

/// Serializes `values`, inserting `/**/` between tokens that would otherwise
/// be re-tokenized differently (§9, "serialization" table).
pub(crate) fn serialize_list(values: &[ComponentValue], out: &mut String) {
    let mut prev: Option<&ComponentValue> = None;
    for value in values {
        if let Some(prev) = prev {
            if needs_separator(prev, value) {
                out.push_str("/**/");
            }
        }
        serialize_value(value, out);
        prev = Some(value);
    }
}

pub(crate) fn serialize_value(value: &ComponentValue, out: &mut String) {
    match value {
        ComponentValue::Token(token) => serialize_token(token, out),
        ComponentValue::Function { name, args } => {
            serialize_ident(name, out);
            out.push('(');
            serialize_list(args, out);
            out.push(')');
        }
        ComponentValue::Block { open, content } => {
            let close = match open {
                '[' => ']',
                '{' => '}',
                _ => ')',
            };
            out.push(*open);
            serialize_list(content, out);
            out.push(close);
        }
    }
}

fn serialize_token(token: &Token, out: &mut String) {
    match token {
        Token::Ident(s) => serialize_ident(s, out),
        Token::Function(name) => {
            serialize_ident(name, out);
            out.push('(');
        }
        Token::AtKeyword(name) => {
            out.push('@');
            serialize_ident(name, out);
        }
        Token::Hash { value, is_id } => {
            out.push('#');
            if *is_id {
                serialize_ident(value, out);
            } else {
                serialize_name(value, out);
            }
        }
        Token::String(s) => serialize_string(s, out),
        // A quote followed by a newline re-tokenizes as a bad string.
        Token::BadString => out.push_str("\"\n"),
        Token::Url(url) => serialize_url(url, out),
        // `url(()` re-tokenizes as a bad url.
        Token::BadUrl => out.push_str("url(()"),
        Token::Delim('\\') => out.push_str("\\\n"),
        Token::Delim(c) => out.push(*c),
        Token::Number { value, is_integer } => serialize_number(*value, *is_integer, out),
        Token::Percentage(value) => {
            serialize_number(*value, true, out);
            out.push('%');
        }
        Token::Dimension { value, unit } => {
            serialize_number(*value, true, out);
            serialize_unit(unit, out);
        }
        Token::Whitespace => out.push(' '),
        Token::Cdo => out.push_str("<!--"),
        Token::Cdc => out.push_str("-->"),
        Token::Colon => out.push(':'),
        Token::Semicolon => out.push(';'),
        Token::Comma => out.push(','),
        Token::OpenSquare => out.push('['),
        Token::CloseSquare => out.push(']'),
        Token::OpenParen => out.push('('),
        Token::CloseParen => out.push(')'),
        Token::OpenCurly => out.push('{'),
        Token::CloseCurly => out.push('}'),
    }
}

/// Writes a number so that it re-tokenizes to the same value and type.
/// `integer_form` allows whole values to be written without a fraction.
fn serialize_number(value: f32, integer_form: bool, out: &mut String) {
    if !value.is_finite() {
        // Not produced by the tokenizer; keep the output tokenizable.
        let clamped = if value.is_nan() {
            0.0
        } else {
            value.clamp(-f32::MAX, f32::MAX)
        };
        serialize_number(clamped, integer_form, out);
        return;
    }
    let start = out.len();
    let _ = write!(out, "{value}");
    if !integer_form && !out[start..].contains('.') {
        out.push_str(".0");
    }
}

fn hex_escape(c: char, out: &mut String) {
    let _ = write!(out, "\\{:x} ", c as u32);
}

/// CSSOM "serialize an identifier".
pub(crate) fn serialize_ident(s: &str, out: &mut String) {
    let first = s.chars().next();
    let single = s.chars().nth(1).is_none();
    for (index, c) in s.chars().enumerate() {
        match c {
            '\0' => out.push('\u{FFFD}'),
            '\u{1}'..='\u{1F}' | '\u{7F}' => hex_escape(c, out),
            c if c.is_ascii_digit() && (index == 0 || (index == 1 && first == Some('-'))) => {
                hex_escape(c, out)
            }
            '-' if index == 0 && single => out.push_str("\\-"),
            c if c as u32 >= 0x80 || c == '-' || c == '_' || c.is_ascii_alphanumeric() => {
                out.push(c)
            }
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
}

/// Serializes a hash name that is not a valid identifier (e.g. `#123`).
fn serialize_name(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\0' => out.push('\u{FFFD}'),
            '\u{1}'..='\u{1F}' | '\u{7F}' => hex_escape(c, out),
            c if c as u32 >= 0x80 || c == '-' || c == '_' || c.is_ascii_alphanumeric() => {
                out.push(c)
            }
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
}

/// Serializes a dimension unit; a unit that would be read back as an
/// exponent (`e3`, `e-1`) has its `e` escaped.
fn serialize_unit(unit: &str, out: &mut String) {
    let mut chars = unit.chars();
    if let (Some(e @ ('e' | 'E')), Some(next)) = (chars.next(), chars.next()) {
        if next.is_ascii_digit() || next == '+' || next == '-' {
            hex_escape(e, out);
            // The rest continues an identifier: digits and '-' need no escaping.
            serialize_name(&unit[1..], out);
            return;
        }
    }
    serialize_ident(unit, out);
}

/// CSSOM "serialize a string" (always double quotes).
pub(crate) fn serialize_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\0' => out.push('\u{FFFD}'),
            '\u{1}'..='\u{1F}' | '\u{7F}' => hex_escape(c, out),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Serializes an unquoted url token.
fn serialize_url(url: &str, out: &mut String) {
    out.push_str("url(");
    for c in url.chars() {
        match c {
            '\0' => out.push('\u{FFFD}'),
            '\u{1}'..='\u{20}' | '\u{7F}' => hex_escape(c, out),
            '"' | '\'' | '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(')');
}

/// Token classes relevant to the §9 serialization table.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Kind {
    Ident,
    Function,
    Url,
    AtKeyword,
    Hash,
    Number,
    Percentage,
    Dimension,
    Cdc,
    OpenParen,
    Delim(char),
    Other,
}

fn first_kind(value: &ComponentValue) -> Kind {
    match value {
        ComponentValue::Token(token) => token_kind(token),
        ComponentValue::Function { .. } => Kind::Function,
        ComponentValue::Block { open: '(', .. } => Kind::OpenParen,
        ComponentValue::Block { .. } => Kind::Other,
    }
}

fn last_kind(value: &ComponentValue) -> Kind {
    match value {
        ComponentValue::Token(token) => token_kind(token),
        // Functions and blocks end with a closing character.
        _ => Kind::Other,
    }
}

fn token_kind(token: &Token) -> Kind {
    match token {
        Token::Ident(_) => Kind::Ident,
        Token::Function(_) => Kind::Function,
        Token::Url(_) | Token::BadUrl => Kind::Url,
        Token::AtKeyword(_) => Kind::AtKeyword,
        Token::Hash { .. } => Kind::Hash,
        Token::Number { .. } => Kind::Number,
        Token::Percentage(_) => Kind::Percentage,
        Token::Dimension { .. } => Kind::Dimension,
        Token::Cdc => Kind::Cdc,
        Token::OpenParen => Kind::OpenParen,
        Token::Delim(c) => Kind::Delim(*c),
        _ => Kind::Other,
    }
}

/// True for an identifier token that serializes as exactly `--`: followed by
/// `>` it would be read back as a CDC token.
fn is_double_dash_ident(value: &ComponentValue) -> bool {
    matches!(value, ComponentValue::Token(Token::Ident(s)) if s == "--")
}

fn needs_separator(prev: &ComponentValue, next: &ComponentValue) -> bool {
    use Kind::*;
    let a = last_kind(prev);
    let b = first_kind(next);
    let identish = matches!(b, Ident | Function | Url | Delim('-'));
    let numeric = matches!(b, Number | Percentage | Dimension);
    match a {
        // `--` + `>` would become `-->` (CDC).
        Ident if b == Delim('>') => is_double_dash_ident(prev),
        Ident => identish || numeric || matches!(b, Cdc | OpenParen),
        AtKeyword | Hash | Dimension => identish || numeric || b == Cdc,
        Delim('#') => identish || numeric || b == Cdc,
        // `-` + `-->` would start the identifier `---` (not covered by the
        // §9 table, but the tokenizer reads it that way).
        Delim('-') => identish || numeric || b == Cdc,
        Number => matches!(b, Ident | Function | Url | Cdc) || numeric || b == Delim('%'),
        Delim('@') => matches!(b, Ident | Function | Url | Delim('-') | Dimension | Cdc),
        Delim('.') | Delim('+') => numeric,
        Delim('/') => b == Delim('*'),
        // `<` + `!` + `--` would become `<!--` (CDO).
        Delim('<') => b == Delim('!'),
        _ => false,
    }
}
