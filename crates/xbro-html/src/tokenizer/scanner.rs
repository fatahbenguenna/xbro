//! A byte cursor over the normalised input, shared by the tokenizer states.
//!
//! All positions the scanner stops at are either the end of the input or an
//! ASCII byte, so slicing between two such positions always falls on UTF-8
//! character boundaries. Slicing still goes through [`Scanner::slice`], which
//! never panics.

pub(super) struct Scanner<'s> {
    pub(super) s: &'s str,
    pub(super) b: &'s [u8],
    pub(super) pos: usize,
}

impl<'s> Scanner<'s> {
    pub(super) fn new(s: &'s str, pos: usize) -> Self {
        Self {
            s,
            b: s.as_bytes(),
            pos: pos.min(s.len()),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.b.len()
    }

    pub(super) fn at_end(&self) -> bool {
        self.pos >= self.b.len()
    }

    pub(super) fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    pub(super) fn peek_at(&self, offset: usize) -> Option<u8> {
        self.b.get(self.pos + offset).copied()
    }

    pub(super) fn byte_at(&self, index: usize) -> Option<u8> {
        self.b.get(index).copied()
    }

    /// The text between two byte offsets (empty if they are not valid boundaries).
    pub(super) fn slice(&self, start: usize, end: usize) -> &'s str {
        self.s.get(start..end).unwrap_or("")
    }

    /// The rest of the input from `start`.
    pub(super) fn rest_from(&self, start: usize) -> &'s str {
        self.s.get(start..).unwrap_or("")
    }

    /// True when the input at `index` starts with `pat` (exact bytes).
    pub(super) fn starts_with_at(&self, index: usize, pat: &[u8]) -> bool {
        self.b
            .get(index..)
            .is_some_and(|rest| rest.starts_with(pat))
    }

    /// True when the input at `index` starts with `pat`, ASCII case-insensitively.
    pub(super) fn starts_with_ci_at(&self, index: usize, pat: &[u8]) -> bool {
        self.b
            .get(index..index + pat.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(pat))
    }

    /// Index of the first occurrence of `needle` at or after `from`.
    pub(super) fn find_byte(&self, from: usize, needle: u8) -> Option<usize> {
        self.b
            .get(from..)?
            .iter()
            .position(|&b| b == needle)
            .map(|p| p + from)
    }

    /// Index of the first occurrence of the byte sequence `needle` at or after `from`.
    pub(super) fn find_seq(&self, from: usize, needle: &[u8]) -> Option<usize> {
        let first = *needle.first()?;
        let mut i = from;
        loop {
            let p = self.find_byte(i, first)?;
            if self.starts_with_at(p, needle) {
                return Some(p);
            }
            i = p + 1;
        }
    }

    /// Skips tag whitespace.
    pub(super) fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(super::is_tag_whitespace) {
            self.pos += 1;
        }
    }
}
