//! CSS tokenizer (CSS Syntax Module Level 3, §4).
//!
//! The input is preprocessed (§3.3: CR, FF and CRLF become LF, NUL becomes
//! U+FFFD), then consumed one token at a time. Every step consumes at least one
//! code point and looks ahead a bounded number of code points, so tokenizing
//! is linear in the input length.

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
    Tokenizer::new(input).collect()
}

/// A streaming CSS tokenizer; iterate it to get the tokens of the input.
#[derive(Clone, Debug)]
pub struct Tokenizer {
    /// Preprocessed input code points.
    chars: Vec<char>,
    /// Index of the next input code point.
    pos: usize,
}

/// `newline` after preprocessing is only LF.
fn is_newline(c: char) -> bool {
    c == '\n'
}

fn is_whitespace(c: char) -> bool {
    matches!(c, '\n' | '\t' | ' ')
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c as u32 >= 0x80
}

fn is_ident_char(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit() || c == '-'
}

fn is_non_printable(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{B}' | '\u{E}'..='\u{1F}' | '\u{7F}')
}

impl Tokenizer {
    /// Creates a tokenizer, preprocessing the input (§3.3).
    pub fn new(input: &str) -> Self {
        let mut chars = Vec::with_capacity(input.len());
        let mut iter = input.chars().peekable();
        while let Some(c) = iter.next() {
            match c {
                '\r' => {
                    if iter.peek() == Some(&'\n') {
                        iter.next();
                    }
                    chars.push('\n');
                }
                '\u{C}' => chars.push('\n'),
                '\0' => chars.push('\u{FFFD}'),
                c => chars.push(c),
            }
        }
        Tokenizer { chars, pos: 0 }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn peek_is(&self, offset: usize, f: impl Fn(char) -> bool) -> bool {
        self.peek(offset).is_some_and(f)
    }

    /// §4.3.8: do the code points at `offset` and `offset + 1` form a valid escape?
    fn is_valid_escape(&self, offset: usize) -> bool {
        self.peek(offset) == Some('\\') && !self.peek_is(offset + 1, is_newline)
    }

    /// §4.3.9: would the code points at `offset..offset + 3` start an ident sequence?
    fn would_start_ident(&self, offset: usize) -> bool {
        match self.peek(offset) {
            Some('-') => {
                self.peek_is(offset + 1, |c| is_ident_start(c) || c == '-')
                    || self.is_valid_escape(offset + 1)
            }
            Some('\\') => self.is_valid_escape(offset),
            Some(c) => is_ident_start(c),
            None => false,
        }
    }

    /// §4.3.10: would the code points at `offset..offset + 3` start a number?
    fn starts_number(&self, offset: usize) -> bool {
        match self.peek(offset) {
            Some('+' | '-') => {
                self.peek_is(offset + 1, |c| c.is_ascii_digit())
                    || (self.peek(offset + 1) == Some('.')
                        && self.peek_is(offset + 2, |c| c.is_ascii_digit()))
            }
            Some('.') => self.peek_is(offset + 1, |c| c.is_ascii_digit()),
            Some(c) => c.is_ascii_digit(),
            None => false,
        }
    }

    /// §4.3.2: consumes comments (an unterminated comment runs to the end).
    fn consume_comments(&mut self) {
        while self.peek(0) == Some('/') && self.peek(1) == Some('*') {
            self.pos += 2;
            loop {
                match self.peek(0) {
                    None => return,
                    Some('*') if self.peek(1) == Some('/') => {
                        self.pos += 2;
                        break;
                    }
                    Some(_) => self.pos += 1,
                }
            }
        }
    }

    /// §4.3.1: consumes and returns the next token, or `None` at the end.
    pub fn next_token(&mut self) -> Option<Token> {
        self.consume_comments();
        let c = self.peek(0)?;
        let token = match c {
            c if is_whitespace(c) => {
                while self.peek_is(0, is_whitespace) {
                    self.pos += 1;
                }
                Token::Whitespace
            }
            '"' | '\'' => {
                self.pos += 1;
                self.consume_string(c)
            }
            '#' => {
                if self.peek_is(1, is_ident_char) || self.is_valid_escape(1) {
                    let is_id = self.would_start_ident(1);
                    self.pos += 1;
                    let value = self.consume_ident_sequence();
                    Token::Hash { value, is_id }
                } else {
                    self.pos += 1;
                    Token::Delim('#')
                }
            }
            '(' => self.single(Token::OpenParen),
            ')' => self.single(Token::CloseParen),
            ',' => self.single(Token::Comma),
            ':' => self.single(Token::Colon),
            ';' => self.single(Token::Semicolon),
            '[' => self.single(Token::OpenSquare),
            ']' => self.single(Token::CloseSquare),
            '{' => self.single(Token::OpenCurly),
            '}' => self.single(Token::CloseCurly),
            '+' | '.' => {
                if self.starts_number(0) {
                    self.consume_numeric()
                } else {
                    self.single(Token::Delim(c))
                }
            }
            '-' => {
                if self.starts_number(0) {
                    self.consume_numeric()
                } else if self.peek(1) == Some('-') && self.peek(2) == Some('>') {
                    self.pos += 3;
                    Token::Cdc
                } else if self.would_start_ident(0) {
                    self.consume_ident_like()
                } else {
                    self.single(Token::Delim('-'))
                }
            }
            '<' => {
                if self.peek(1) == Some('!')
                    && self.peek(2) == Some('-')
                    && self.peek(3) == Some('-')
                {
                    self.pos += 4;
                    Token::Cdo
                } else {
                    self.single(Token::Delim('<'))
                }
            }
            '@' => {
                if self.would_start_ident(1) {
                    self.pos += 1;
                    Token::AtKeyword(self.consume_ident_sequence())
                } else {
                    self.single(Token::Delim('@'))
                }
            }
            '\\' => {
                if self.is_valid_escape(0) {
                    self.consume_ident_like()
                } else {
                    // Parse error: a backslash followed by a newline.
                    self.single(Token::Delim('\\'))
                }
            }
            c if c.is_ascii_digit() => self.consume_numeric(),
            c if is_ident_start(c) => self.consume_ident_like(),
            c => self.single(Token::Delim(c)),
        };
        Some(token)
    }

    fn single(&mut self, token: Token) -> Token {
        self.pos += 1;
        token
    }

    /// §4.3.7: consumes an escaped code point; the backslash is already consumed.
    fn consume_escape(&mut self) -> char {
        match self.peek(0) {
            None => '\u{FFFD}',
            Some(c) if c.is_ascii_hexdigit() => {
                let mut value: u32 = 0;
                let mut count = 0;
                while count < 6 {
                    match self.peek(0).and_then(|h| h.to_digit(16)) {
                        Some(d) => {
                            value = value * 16 + d;
                            self.pos += 1;
                            count += 1;
                        }
                        None => break,
                    }
                }
                if self.peek_is(0, is_whitespace) {
                    self.pos += 1;
                }
                if value == 0 {
                    '\u{FFFD}'
                } else {
                    // Surrogates and values above U+10FFFF are rejected by from_u32.
                    char::from_u32(value).unwrap_or('\u{FFFD}')
                }
            }
            Some(c) => {
                self.pos += 1;
                c
            }
        }
    }

    /// §4.3.11: consumes an ident sequence starting at the current position.
    fn consume_ident_sequence(&mut self) -> String {
        let mut s = String::new();
        loop {
            match self.peek(0) {
                Some(c) if is_ident_char(c) => {
                    s.push(c);
                    self.pos += 1;
                }
                Some('\\') if self.is_valid_escape(0) => {
                    self.pos += 1;
                    s.push(self.consume_escape());
                }
                _ => return s,
            }
        }
    }

    /// §4.3.12: consumes a number, returning its value and whether it is an integer.
    fn consume_number(&mut self) -> (f32, bool) {
        let mut repr = String::new();
        let mut is_integer = true;
        if let Some(sign @ ('+' | '-')) = self.peek(0) {
            repr.push(sign);
            self.pos += 1;
        }
        self.consume_digits(&mut repr);
        if self.peek(0) == Some('.') && self.peek_is(1, |c| c.is_ascii_digit()) {
            repr.push('.');
            self.pos += 1;
            self.consume_digits(&mut repr);
            is_integer = false;
        }
        if matches!(self.peek(0), Some('e' | 'E')) {
            let digit_at = |t: &Self, o: usize| t.peek_is(o, |c| c.is_ascii_digit());
            let exponent_len = if digit_at(self, 1) {
                Some(1)
            } else if matches!(self.peek(1), Some('+' | '-')) && digit_at(self, 2) {
                Some(2)
            } else {
                None
            };
            if let Some(len) = exponent_len {
                repr.push('e');
                if len == 2 {
                    if let Some(sign) = self.peek(1) {
                        repr.push(sign);
                    }
                }
                self.pos += len;
                self.consume_digits(&mut repr);
                is_integer = false;
            }
        }
        let value: f64 = repr.parse().unwrap_or(0.0);
        let value = (value as f32).clamp(-f32::MAX, f32::MAX);
        (value, is_integer)
    }

    fn consume_digits(&mut self, repr: &mut String) {
        while let Some(c) = self.peek(0).filter(char::is_ascii_digit) {
            repr.push(c);
            self.pos += 1;
        }
    }

    /// §4.3.3: consumes a numeric token (number, percentage or dimension).
    fn consume_numeric(&mut self) -> Token {
        let (value, is_integer) = self.consume_number();
        if self.would_start_ident(0) {
            let unit = self.consume_ident_sequence().to_ascii_lowercase();
            Token::Dimension { value, unit }
        } else if self.peek(0) == Some('%') {
            self.pos += 1;
            Token::Percentage(value)
        } else {
            Token::Number { value, is_integer }
        }
    }

    /// §4.3.4: consumes an ident, function or url token.
    fn consume_ident_like(&mut self) -> Token {
        let name = self.consume_ident_sequence();
        if name.eq_ignore_ascii_case("url") && self.peek(0) == Some('(') {
            self.pos += 1;
            while self.peek_is(0, is_whitespace) && self.peek_is(1, is_whitespace) {
                self.pos += 1;
            }
            let quote = |c: char| c == '"' || c == '\'';
            if self.peek_is(0, quote) || (self.peek_is(0, is_whitespace) && self.peek_is(1, quote))
            {
                Token::Function(name)
            } else {
                self.consume_url()
            }
        } else if self.peek(0) == Some('(') {
            self.pos += 1;
            Token::Function(name)
        } else {
            Token::Ident(name)
        }
    }

    /// §4.3.5: consumes a string token; the opening quote is already consumed.
    fn consume_string(&mut self, quote: char) -> Token {
        let mut s = String::new();
        loop {
            match self.peek(0) {
                None => return Token::String(s),
                Some(c) if c == quote => {
                    self.pos += 1;
                    return Token::String(s);
                }
                // Parse error: the newline is not consumed.
                Some('\n') => return Token::BadString,
                Some('\\') => match self.peek(1) {
                    None => self.pos += 1,
                    Some('\n') => self.pos += 2,
                    Some(_) => {
                        self.pos += 1;
                        s.push(self.consume_escape());
                    }
                },
                Some(c) => {
                    s.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// §4.3.6: consumes an unquoted url token; `url(` is already consumed.
    fn consume_url(&mut self) -> Token {
        let mut value = String::new();
        while self.peek_is(0, is_whitespace) {
            self.pos += 1;
        }
        loop {
            let Some(c) = self.peek(0) else {
                return Token::Url(value);
            };
            self.pos += 1;
            match c {
                ')' => return Token::Url(value),
                c if is_whitespace(c) => {
                    while self.peek_is(0, is_whitespace) {
                        self.pos += 1;
                    }
                    return match self.peek(0) {
                        None => Token::Url(value),
                        Some(')') => {
                            self.pos += 1;
                            Token::Url(value)
                        }
                        Some(_) => {
                            self.consume_bad_url_remnants();
                            Token::BadUrl
                        }
                    };
                }
                '"' | '\'' | '(' => {
                    self.consume_bad_url_remnants();
                    return Token::BadUrl;
                }
                c if is_non_printable(c) => {
                    self.consume_bad_url_remnants();
                    return Token::BadUrl;
                }
                '\\' => {
                    if self.peek_is(0, is_newline) {
                        self.consume_bad_url_remnants();
                        return Token::BadUrl;
                    }
                    value.push(self.consume_escape());
                }
                c => value.push(c),
            }
        }
    }

    /// §4.3.14: consumes the rest of a bad url, up to and including `)`.
    fn consume_bad_url_remnants(&mut self) {
        while let Some(c) = self.peek(0) {
            if c == ')' {
                self.pos += 1;
                return;
            }
            if self.is_valid_escape(0) {
                self.pos += 1;
                self.consume_escape();
            } else {
                self.pos += 1;
            }
        }
    }
}

impl Iterator for Tokenizer {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        self.next_token()
    }
}

#[cfg(test)]
mod tests;
