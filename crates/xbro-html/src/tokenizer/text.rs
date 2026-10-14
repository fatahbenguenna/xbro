//! Character data states: data, RCDATA, RAWTEXT, script data and PLAINTEXT.

use super::scanner::Scanner;
use super::{is_tag_whitespace, Token};
use crate::entities;

/// What a `<` in the data state starts.
enum LessThan {
    /// The `<` is plain text (e.g. `a < b`, `<` at EOF).
    Literal,
    /// Markup (tag, comment, doctype, bogus comment...) starts here.
    Markup,
    /// `</>`: ignored entirely; skip this many bytes.
    Skip(usize),
}

impl<'s> Scanner<'s> {
    /// Data state: returns a coalesced text run, or the markup token found at
    /// the current position.
    pub(super) fn data(&mut self, cdata_allowed: bool) -> Token {
        let mut text = String::new();
        while let Some(b) = self.peek() {
            match b {
                b'<' => match self.classify_less_than() {
                    LessThan::Literal => {
                        text.push('<');
                        self.pos += 1;
                    }
                    LessThan::Skip(n) => self.pos += n,
                    LessThan::Markup => {
                        if !text.is_empty() {
                            // Leave the markup for the next call so that the
                            // tree builder can react to the text first.
                            return Token::Text(text);
                        }
                        if let Some(token) = self.markup(cdata_allowed) {
                            return token;
                        }
                    }
                },
                b'&' => self.char_ref(&mut text, false),
                _ => {
                    let end = self
                        .b
                        .get(self.pos..)
                        .and_then(|rest| rest.iter().position(|&c| c == b'<' || c == b'&'))
                        .map_or(self.len(), |p| p + self.pos);
                    text.push_str(self.slice(self.pos, end));
                    self.pos = end;
                }
            }
        }
        if text.is_empty() {
            Token::Eof
        } else {
            Token::Text(text)
        }
    }

    fn classify_less_than(&self) -> LessThan {
        match self.peek_at(1) {
            Some(c) if c.is_ascii_alphabetic() => LessThan::Markup,
            Some(b'!') | Some(b'?') => LessThan::Markup,
            Some(b'/') => match self.peek_at(2) {
                Some(b'>') => LessThan::Skip(3),
                // `</` at EOF is emitted as text.
                None => LessThan::Literal,
                // `</x` is an end tag, anything else a bogus comment.
                Some(_) => LessThan::Markup,
            },
            _ => LessThan::Literal,
        }
    }

    /// Dispatches on the markup at `<`. Returns `None` when nothing is
    /// emitted (e.g. a tag cut off by the end of the input).
    fn markup(&mut self, cdata_allowed: bool) -> Option<Token> {
        match self.peek_at(1) {
            Some(b'!') => self.markup_declaration(cdata_allowed),
            Some(b'?') => {
                // `<?xml ...>`: bogus comment that includes the `?`.
                let start = self.pos + 1;
                Some(self.bogus_comment(start))
            }
            Some(b'/') => {
                if self.peek_at(2).is_some_and(|c| c.is_ascii_alphabetic()) {
                    self.tag(true)
                } else {
                    let start = self.pos + 2;
                    Some(self.bogus_comment(start))
                }
            }
            _ => self.tag(false),
        }
    }

    /// Decodes the character reference at `&` into `out`.
    pub(super) fn char_ref(&mut self, out: &mut String, in_attribute: bool) {
        let after = self.rest_from(self.pos + 1);
        match entities::match_reference(after, in_attribute) {
            Some((consumed, decoded)) => {
                decoded.push_to(out);
                self.pos += 1 + consumed;
            }
            None => {
                out.push('&');
                self.pos += 1;
            }
        }
    }

    /// RCDATA (`decode == true`) and RAWTEXT states: text until the
    /// appropriate end tag `</end_name`.
    pub(super) fn raw_text(&mut self, end_name: &str, decode: bool) -> Token {
        let start = self.pos;
        match self.find_appropriate_end_tag(start, end_name) {
            Some(p) if p == start => self.tag(true).unwrap_or(Token::Eof),
            Some(p) => {
                self.pos = p;
                Token::Text(self.text_between(start, p, decode))
            }
            None => {
                self.pos = self.len();
                if start >= self.len() {
                    Token::Eof
                } else {
                    Token::Text(self.text_between(start, self.len(), decode))
                }
            }
        }
    }

    fn text_between(&self, start: usize, end: usize, decode: bool) -> String {
        let raw = self.slice(start, end);
        if decode && raw.contains('&') {
            entities::decode(raw, false)
        } else {
            raw.to_string()
        }
    }

    /// Script data state, including the `<!--` escaped and double-escaped
    /// states: inside `<!-- <script> ... -->`, a `</script>` does not end
    /// the script.
    pub(super) fn script_data(&mut self, end_name: &str) -> Token {
        let start = self.pos;
        match self.find_script_end(start, end_name) {
            Some(p) if p == start => self.tag(true).unwrap_or(Token::Eof),
            Some(p) => {
                self.pos = p;
                Token::Text(self.slice(start, p).to_string())
            }
            None => {
                self.pos = self.len();
                if start >= self.len() {
                    Token::Eof
                } else {
                    Token::Text(self.slice(start, self.len()).to_string())
                }
            }
        }
    }

    /// PLAINTEXT state: everything up to EOF.
    pub(super) fn plaintext(&mut self) -> Token {
        let start = self.pos;
        self.pos = self.len();
        if start >= self.len() {
            Token::Eof
        } else {
            Token::Text(self.slice(start, self.len()).to_string())
        }
    }

    /// True when an appropriate end tag `</name` followed by whitespace, `/`
    /// or `>` starts at `index`.
    fn is_appropriate_end_tag(&self, index: usize, name: &str) -> bool {
        !name.is_empty()
            && self.starts_with_at(index, b"</")
            && self.starts_with_ci_at(index + 2, name.as_bytes())
            && self
                .byte_at(index + 2 + name.len())
                .is_some_and(|c| is_tag_whitespace(c) || c == b'/' || c == b'>')
    }

    fn find_appropriate_end_tag(&self, from: usize, name: &str) -> Option<usize> {
        if name.is_empty() {
            return None;
        }
        let mut i = from;
        loop {
            let p = self.find_seq(i, b"</")?;
            if self.is_appropriate_end_tag(p, name) {
                return Some(p);
            }
            i = p + 2;
        }
    }

    /// Reads ASCII letters at `index`; returns the end index.
    fn letters_end(&self, index: usize) -> usize {
        let mut i = index;
        while self.byte_at(i).is_some_and(|c| c.is_ascii_alphabetic()) {
            i += 1;
        }
        i
    }

    /// True when the letters in `start..end` spell `script` and are followed
    /// by whitespace, `/` or `>` (the double-escape start/end condition).
    fn is_script_word(&self, start: usize, end: usize) -> bool {
        self.slice(start, end).eq_ignore_ascii_case("script")
            && self
                .byte_at(end)
                .is_some_and(|c| is_tag_whitespace(c) || c == b'/' || c == b'>')
    }

    fn find_script_end(&self, from: usize, name: &str) -> Option<usize> {
        #[derive(PartialEq)]
        enum State {
            Normal,
            Escaped,
            DoubleEscaped,
        }
        let mut state = State::Normal;
        // Number of consecutive dashes seen in the (double) escaped states.
        let mut dashes = 0usize;
        let mut i = from;
        loop {
            match state {
                State::Normal => {
                    let p = self.find_byte(i, b'<')?;
                    if self.is_appropriate_end_tag(p, name) {
                        return Some(p);
                    }
                    if self.starts_with_at(p, b"<!--") {
                        state = State::Escaped;
                        // `<!--` leaves us in the "escaped dash dash" state.
                        dashes = 2;
                        i = p + 4;
                    } else {
                        i = p + 1;
                    }
                }
                State::Escaped | State::DoubleEscaped => {
                    let c = self.byte_at(i)?;
                    match c {
                        b'-' => {
                            dashes += 1;
                            i += 1;
                        }
                        b'>' => {
                            if dashes >= 2 {
                                state = State::Normal;
                            }
                            dashes = 0;
                            i += 1;
                        }
                        b'<' => {
                            dashes = 0;
                            if state == State::Escaped {
                                if self.is_appropriate_end_tag(i, name) {
                                    return Some(i);
                                }
                                let end = self.letters_end(i + 1);
                                if end > i + 1 && self.is_script_word(i + 1, end) {
                                    state = State::DoubleEscaped;
                                    i = end;
                                } else {
                                    i += 1;
                                }
                            } else if self.byte_at(i + 1) == Some(b'/') {
                                let end = self.letters_end(i + 2);
                                if end > i + 2 && self.is_script_word(i + 2, end) {
                                    state = State::Escaped;
                                    i = end;
                                } else {
                                    i += 1;
                                }
                            } else {
                                i += 1;
                            }
                        }
                        _ => {
                            dashes = 0;
                            i += 1;
                        }
                    }
                }
            }
        }
    }
}
