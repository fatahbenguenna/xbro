//! Tag open, tag name, attribute and self-closing states.

use std::collections::HashSet;

use xbro_dom::Attribute;

use super::scanner::Scanner;
use super::{is_tag_whitespace, Token};
use crate::entities;

/// Above this many attributes, duplicate detection switches from a linear
/// scan to a hash set (keeps pathological tags linear).
const LINEAR_DEDUP_LIMIT: usize = 16;

/// Collects attributes, dropping duplicates (the first occurrence wins).
#[derive(Default)]
struct AttributeList {
    attrs: Vec<Attribute>,
    names: Option<HashSet<String>>,
}

impl AttributeList {
    fn push(&mut self, name: String, value: String) {
        if let Some(names) = &mut self.names {
            if !names.insert(name.clone()) {
                return;
            }
        } else {
            if self.attrs.iter().any(|a| a.name == name) {
                return;
            }
            if self.attrs.len() >= LINEAR_DEDUP_LIMIT {
                let mut names: HashSet<String> =
                    self.attrs.iter().map(|a| a.name.clone()).collect();
                names.insert(name.clone());
                self.names = Some(names);
            }
        }
        self.attrs.push(Attribute { name, value });
    }
}

impl<'s> Scanner<'s> {
    /// Parses a start (`end == false`, at `<x`) or end (`end == true`, at
    /// `</x`) tag. Returns `None` when the input ends inside the tag (the
    /// tag is then dropped, as the spec requires); the position is then at
    /// the end of the input.
    pub(super) fn tag(&mut self, end: bool) -> Option<Token> {
        self.pos += if end { 2 } else { 1 };
        let name_start = self.pos;
        while let Some(c) = self.peek() {
            if is_tag_whitespace(c) || c == b'/' || c == b'>' {
                break;
            }
            self.pos += 1;
        }
        if self.at_end() {
            return None;
        }
        let name = self.slice(name_start, self.pos).to_ascii_lowercase();
        let mut attrs = AttributeList::default();
        let mut self_closing = false;
        loop {
            // Before attribute name.
            self.skip_whitespace();
            match self.peek() {
                None => return None,
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(b'/') => {
                    // Self-closing start tag state.
                    self.pos += 1;
                    match self.peek() {
                        None => return None,
                        Some(b'>') => {
                            self.pos += 1;
                            self_closing = true;
                            break;
                        }
                        // Unexpected solidus: reconsume in "before attribute name".
                        Some(_) => continue,
                    }
                }
                Some(_) => {
                    let (attr_name, value) = self.attribute()?;
                    attrs.push(attr_name, value);
                }
            }
        }
        Some(if end {
            Token::EndTag { name }
        } else {
            Token::StartTag {
                name,
                attrs: attrs.attrs,
                self_closing,
            }
        })
    }

    /// Attribute name and value states. Starts on the first character of
    /// the name (which may be `=`). Returns `None` at EOF.
    fn attribute(&mut self) -> Option<(String, String)> {
        let name_start = self.pos;
        // The first character always belongs to the name, even `=`.
        self.pos += 1;
        while let Some(c) = self.peek() {
            if is_tag_whitespace(c) || c == b'/' || c == b'>' || c == b'=' {
                break;
            }
            self.pos += 1;
        }
        let name = self.slice(name_start, self.pos).to_ascii_lowercase();
        // After attribute name.
        self.skip_whitespace();
        match self.peek() {
            None => {
                self.pos = self.len();
                None
            }
            Some(b'=') => {
                self.pos += 1;
                let value = self.attribute_value()?;
                Some((name, value))
            }
            // No value: `/`, `>` or the next attribute are handled by the caller.
            Some(_) => Some((name, String::new())),
        }
    }

    /// Before attribute value and the quoted/unquoted value states.
    fn attribute_value(&mut self) -> Option<String> {
        self.skip_whitespace();
        match self.peek() {
            None => {
                self.pos = self.len();
                None
            }
            Some(quote @ (b'"' | b'\'')) => {
                let start = self.pos + 1;
                match self.find_byte(start, quote) {
                    Some(end) => {
                        self.pos = end + 1;
                        Some(decode_attribute(self.slice(start, end)))
                    }
                    None => {
                        self.pos = self.len();
                        None
                    }
                }
            }
            // Missing value: `<a href=>`.
            Some(b'>') => Some(String::new()),
            Some(_) => {
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if is_tag_whitespace(c) || c == b'>' {
                        break;
                    }
                    self.pos += 1;
                }
                if self.at_end() {
                    return None;
                }
                Some(decode_attribute(self.slice(start, self.pos)))
            }
        }
    }
}

fn decode_attribute(raw: &str) -> String {
    if raw.contains('&') {
        entities::decode(raw, true)
    } else {
        raw.to_string()
    }
}
