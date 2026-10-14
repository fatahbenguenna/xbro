//! HTML tokenizer (WHATWG "tokenization" state machine, simplified).
//!
//! The input is normalised once up front (CRLF/CR → LF, NUL → U+FFFD). The
//! tokenizer then produces coalesced [`Token::Text`] runs and tag, comment and
//! doctype tokens. The tree builder drives the content model: it switches the
//! tokenizer to RCDATA/RAWTEXT/script/PLAINTEXT after the corresponding start
//! tags ([`Tokenizer::set_mode`]) and tells it whether `<![CDATA[` sections are
//! allowed ([`Tokenizer::set_cdata_allowed`], foreign content only).
//!
//! The tokenizer never reads ahead of the token it returns, so mode changes
//! requested by the tree builder after a token always apply to the next one.

mod markup;
mod scanner;
mod tag;
mod text;

use std::borrow::Cow;

use xbro_dom::Attribute;

use scanner::Scanner;

/// A token emitted by the [`Tokenizer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// `<!DOCTYPE name ...>`; the name is lowercased.
    Doctype {
        name: String,
        /// True when the doctype forces quirks mode (missing/odd doctype).
        ///
        /// This already includes the WHATWG quirks-mode checks on the name
        /// and on the public/system identifiers (legacy DTDs), so the tree
        /// builder only has to copy it to [`xbro_dom::Document::quirks_mode`].
        force_quirks: bool,
    },
    /// `<name attr=value ...>` (or `<name/>` when `self_closing`).
    StartTag {
        /// Lowercased tag name.
        name: String,
        /// Attributes with lowercased names; duplicates removed (first wins);
        /// character references decoded in values.
        attrs: Vec<Attribute>,
        self_closing: bool,
    },
    /// `</name>`; attributes and the self-closing flag of end tags are dropped.
    EndTag {
        /// Lowercased tag name.
        name: String,
    },
    /// `<!-- data -->`, or a bogus comment (`<?xml ...>`, `</ >`, `<!x>`).
    Comment(String),
    /// A run of character data (character references already decoded).
    Text(String),
    /// End of input.
    Eof,
}

/// Content model the tokenizer is switched to by the tree builder after
/// certain start tags (e.g. `<script>`, `<style>`, `<title>`, `<textarea>`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextMode {
    /// Normal markup.
    Data,
    /// `<title>`, `<textarea>`: character references decoded, no tags until the matching end tag.
    RcData,
    /// `<style>`, `<xmp>`, `<iframe>`, `<noembed>`, `<noframes>`: raw text until the matching end tag.
    RawText,
    /// `<script>`: raw text until `</script>`.
    ScriptData,
    /// `<plaintext>`: everything until EOF is text.
    PlainText,
}

/// Streaming tokenizer over a `&str`.
pub struct Tokenizer<'a> {
    /// Normalised input (borrowed when no normalisation was needed).
    input: Cow<'a, str>,
    pos: usize,
    mode: TextMode,
    /// Tag name that ends the current RcData/RawText/ScriptData section.
    last_start_tag: String,
    /// Whether `<![CDATA[` starts a CDATA section (foreign content).
    cdata_allowed: bool,
}

impl<'a> Tokenizer<'a> {
    /// Creates a tokenizer in the [`TextMode::Data`] state. CRLF/CR are
    /// normalised to LF and NUL characters replaced with U+FFFD up front.
    pub fn new(input: &'a str) -> Self {
        Self {
            input: normalize_newlines_and_nulls(input),
            pos: 0,
            mode: TextMode::Data,
            last_start_tag: String::new(),
            cdata_allowed: false,
        }
    }

    /// Switches the content model; `end_tag` is the element name whose end tag
    /// returns the tokenizer to [`TextMode::Data`] (matched ASCII
    /// case-insensitively; an empty name never matches).
    pub fn set_mode(&mut self, mode: TextMode, end_tag: &str) {
        self.mode = mode;
        self.last_start_tag = end_tag.to_ascii_lowercase();
    }

    /// Current content model.
    pub fn mode(&self) -> TextMode {
        self.mode
    }

    /// Tells the tokenizer whether `<![CDATA[ ... ]]>` sections are allowed.
    ///
    /// The tree builder enables this while the adjusted current node is an
    /// SVG or MathML element; elsewhere `<![CDATA[` starts a bogus comment.
    pub fn set_cdata_allowed(&mut self, allowed: bool) {
        self.cdata_allowed = allowed;
    }

    /// Returns the next token; returns [`Token::Eof`] forever once input is exhausted.
    pub fn next_token(&mut self) -> Token {
        let mut sc = Scanner::new(&self.input, self.pos);
        let token = match self.mode {
            TextMode::Data => sc.data(self.cdata_allowed),
            TextMode::RcData => sc.raw_text(&self.last_start_tag, true),
            TextMode::RawText => sc.raw_text(&self.last_start_tag, false),
            TextMode::ScriptData => sc.script_data(&self.last_start_tag),
            TextMode::PlainText => sc.plaintext(),
        };
        self.pos = sc.pos;
        if matches!(token, Token::EndTag { .. })
            && matches!(
                self.mode,
                TextMode::RcData | TextMode::RawText | TextMode::ScriptData
            )
        {
            // The appropriate end tag closes the raw text section.
            self.mode = TextMode::Data;
        }
        token
    }
}

/// Normalises CRLF and CR to LF and replaces NUL characters with U+FFFD.
fn normalize_newlines_and_nulls(input: &str) -> Cow<'_, str> {
    let bytes = input.as_bytes();
    if !bytes.iter().any(|&b| b == b'\r' || b == 0) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len() + 8);
    let mut last = 0;
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'\r' => {
                out.push_str(input.get(last..i).unwrap_or(""));
                out.push('\n');
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                last = i + 1;
            }
            0 => {
                out.push_str(input.get(last..i).unwrap_or(""));
                out.push('\u{FFFD}');
                last = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push_str(input.get(last..).unwrap_or(""));
    Cow::Owned(out)
}

/// HTML whitespace as far as the tokenizer is concerned (CR is normalised away).
pub(crate) fn is_tag_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b' ' | b'\r')
}

#[cfg(test)]
mod tests;
