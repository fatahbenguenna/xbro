//! CSS tokenizer (CSS Syntax Module Level 3, §4).

/// A CSS token. Identifiers keep their original case (callers compare
/// ASCII case-insensitively where the spec says so); escapes are resolved.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Ident(String),
    /// `name(` — the name without the parenthesis.
    Function(String),
    /// `@name` — the name without `@`.
    AtKeyword(String),
    /// `#name` — `is_id` is true when the name would be a valid identifier.
    Hash {
        value: String,
        is_id: bool,
    },
    String(String),
    BadString,
    /// `url(unquoted)`; a quoted `url("x")` is a Function("url") followed by a String.
    Url(String),
    BadUrl,
    Delim(char),
    Number {
        value: f32,
        is_integer: bool,
    },
    Percentage(f32),
    /// Unit is lowercased.
    Dimension {
        value: f32,
        unit: String,
    },
    Whitespace,
    /// `<!--`
    Cdo,
    /// `-->`
    Cdc,
    Colon,
    Semicolon,
    Comma,
    OpenSquare,
    CloseSquare,
    OpenParen,
    CloseParen,
    OpenCurly,
    CloseCurly,
}

/// Tokenizes a whole string (comments are dropped).
pub fn tokenize(input: &str) -> Vec<Token> {
    let _ = input;
    todo!("xbro-css: implement tokenizer")
}
