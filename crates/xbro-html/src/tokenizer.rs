//! HTML tokenizer (WHATWG "tokenization" state machine, simplified).

use xbro_dom::Attribute;

/// A token emitted by the [`Tokenizer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Doctype {
        name: String,
        /// True when the doctype forces quirks mode (missing/odd doctype).
        force_quirks: bool,
    },
    StartTag {
        /// Lowercased tag name.
        name: String,
        /// Attributes with lowercased names; duplicates removed (first wins);
        /// character references decoded in values.
        attrs: Vec<Attribute>,
        self_closing: bool,
    },
    EndTag {
        name: String,
    },
    Comment(String),
    /// A run of character data (character references already decoded).
    Text(String),
    Eof,
}

/// Content model the tokenizer is switched to by the tree builder after
/// certain start tags (e.g. `<script>`, `<style>`, `<title>`, `<textarea>`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextMode {
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
    input: &'a str,
    pos: usize,
    mode: TextMode,
    /// Tag name that ends the current RcData/RawText/ScriptData section.
    last_start_tag: String,
    pending: std::collections::VecDeque<Token>,
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            mode: TextMode::Data,
            last_start_tag: String::new(),
            pending: Default::default(),
        }
    }

    /// Switches the content model; `end_tag` is the element name whose end tag
    /// returns the tokenizer to [`TextMode::Data`].
    pub fn set_mode(&mut self, mode: TextMode, end_tag: &str) {
        self.mode = mode;
        self.last_start_tag = end_tag.to_string();
    }

    /// Returns the next token; returns [`Token::Eof`] forever once input is exhausted.
    pub fn next_token(&mut self) -> Token {
        let _ = (&self.input, self.pos, &self.pending);
        todo!("xbro-html: implement tokenizer")
    }
}
