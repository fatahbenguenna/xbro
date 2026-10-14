//! Tree construction: turns tokens into a [`Document`].
//!
//! This follows the WHATWG tree construction algorithm with scripting
//! disabled: insertion modes, the stack of open elements, the list of active
//! formatting elements (with the adoption agency algorithm), foster
//! parenting, foreign content (SVG/MathML) and the raw text content models.
//!
//! Robustness measures on top of the spec:
//! * the DOM depth is capped at [`MAX_DEPTH`] elements: deeper elements are
//!   appended to the deepest allowed ancestor instead (flattening);
//! * the stack of open elements is kept below [`MAX_OPEN_ELEMENTS`] by
//!   implicitly closing ordinary elements when it is full, which keeps every
//!   stack walk bounded (no quadratic behaviour on pathological nesting);
//! * the list of active formatting elements is capped per marker section;
//! * the number of elements created by "reconstruct the active formatting
//!   elements" is budgeted in proportion to the input size, so that a few
//!   bytes per paragraph cannot clone dozens of formatting elements each
//!   (which would make the DOM, and memory, grow far faster than the input);
//! * reprocessing a token is bounded.

mod body;
mod depth;
mod foreign;
mod modes;
mod stack;
mod table;
mod tags;

use std::collections::{HashMap, HashSet};

use xbro_dom::{Attribute, Document, NodeId};

use crate::tokenizer::{TextMode, Token, Tokenizer};
use tags::{Ns, Tag};

/// Maximum element nesting depth in the produced DOM (`<html>` is depth 1).
pub const MAX_DEPTH: usize = 256;

/// Soft limit on the size of the stack of open elements.
const MAX_OPEN_ELEMENTS: usize = 2 * MAX_DEPTH;

/// Maximum number of entries after the last marker in the list of active
/// formatting elements.
const MAX_ACTIVE_FORMATTING: usize = 64;

/// Upper bound on the number of times a single token can be reprocessed.
const MAX_REPROCESS: usize = 512;

/// Elements that "reconstruct the active formatting elements" may create
/// regardless of the input size.
const RECONSTRUCT_BUDGET_BASE: usize = 16 * 1024;

/// On top of [`RECONSTRUCT_BUDGET_BASE`], reconstruction may create one
/// element per `RECONSTRUCT_BUDGET_DIVISOR` bytes of input. Real pages stay
/// far below this; an amplification attack (dozens of distinct formatting
/// elements re-opened by every `<p>x`) is held to O(input) nodes.
const RECONSTRUCT_BUDGET_DIVISOR: usize = 2;

/// Insertion modes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    InSelectInTable,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

/// An entry of the stack of open elements.
#[derive(Copy, Clone, Debug)]
struct OpenElement {
    node: NodeId,
    tag: Tag,
    ns: Ns,
    /// SVG `foreignObject`/`desc`/`title`, or MathML `annotation-xml` with
    /// an HTML encoding.
    html_integration_point: bool,
}

impl OpenElement {
    fn is_html(&self, tag: Tag) -> bool {
        self.ns == Ns::Html && self.tag == tag
    }

    fn is_mathml_text_integration_point(&self) -> bool {
        self.ns == Ns::MathMl
            && matches!(self.tag, Tag::Mi | Tag::Mo | Tag::Mn | Tag::Ms | Tag::Mtext)
    }

    fn is_special(&self) -> bool {
        match self.ns {
            Ns::Html => self.tag.is_special_html(),
            Ns::MathMl => matches!(
                self.tag,
                Tag::Mi | Tag::Mo | Tag::Mn | Tag::Ms | Tag::Mtext | Tag::AnnotationXml
            ),
            Ns::Svg => matches!(self.tag, Tag::ForeignObject | Tag::Desc | Tag::Title),
        }
    }
}

/// An entry of the list of active formatting elements.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Formatting {
    Marker,
    Element(NodeId, Tag),
}

/// HTML tree builder. Create one with [`TreeBuilder::new`] and consume it
/// with [`TreeBuilder::parse`].
pub struct TreeBuilder {
    doc: Document,
    mode: Mode,
    original_mode: Mode,
    template_modes: Vec<Mode>,
    open: Vec<OpenElement>,
    active: Vec<Formatting>,
    head: Option<NodeId>,
    form: Option<NodeId>,
    frameset_ok: bool,
    foster_parenting: bool,
    /// Drop a leading LF from the next token (after `<pre>`, `<listing>`, `<textarea>`).
    skip_newline: bool,
    /// Content model change for the tokenizer, applied after the current token.
    text_mode_request: Option<(TextMode, String)>,
    /// Number of HTML elements of each tag on the stack of open elements.
    counts: [u32; Tag::COUNT],
    /// Number of SVG/MathML elements on the stack of open elements, by
    /// ASCII-lowercased name (lets unmatched foreign end tags skip the walk).
    foreign_names: HashMap<String, u32>,
    /// Per node: is it on the stack of open elements?
    on_stack: Vec<bool>,
    /// Per node: DOM depth recorded at insertion time (the root is 0).
    depth: Vec<u16>,
    stopped: bool,
    /// Remaining number of elements that reconstruction may still create.
    reconstruct_budget: usize,
    /// Attribute names of the `<html>`/`<body>` elements that stray start
    /// tags merged attributes into (see `merge_attributes`).
    merged_attr_names: HashMap<NodeId, HashSet<String>>,
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder {
    /// Creates a tree builder that will fill a fresh, empty [`Document`].
    pub fn new() -> Self {
        Self {
            doc: Document::new(),
            mode: Mode::Initial,
            original_mode: Mode::InBody,
            template_modes: Vec::new(),
            open: Vec::new(),
            active: Vec::new(),
            head: None,
            form: None,
            frameset_ok: true,
            foster_parenting: false,
            skip_newline: false,
            text_mode_request: None,
            counts: [0; Tag::COUNT],
            foreign_names: HashMap::new(),
            on_stack: vec![false],
            depth: vec![0],
            stopped: false,
            reconstruct_budget: RECONSTRUCT_BUDGET_BASE,
            merged_attr_names: HashMap::new(),
        }
    }

    /// Parses a complete document. The result always contains
    /// `html > head` and `html > body` (or `frameset`). A single leading
    /// U+FEFF is ignored.
    pub fn parse(mut self, input: &str) -> Document {
        // A leading byte-order mark is not content (decoders strip it, but
        // callers may hand us a string that still starts with one).
        let input = input.strip_prefix('\u{FEFF}').unwrap_or(input);
        self.reconstruct_budget =
            RECONSTRUCT_BUDGET_BASE.saturating_add(input.len() / RECONSTRUCT_BUDGET_DIVISOR);
        let mut tokenizer = Tokenizer::new(input);
        loop {
            tokenizer.set_cdata_allowed(self.adjusted_current_is_foreign());
            let token = tokenizer.next_token();
            let is_eof = token == Token::Eof;
            self.process(token);
            if let Some((mode, end_tag)) = self.text_mode_request.take() {
                tokenizer.set_mode(mode, &end_tag);
            }
            if is_eof || self.stopped {
                break;
            }
        }
        self.finish()
    }

    /// Processes one token, reprocessing it as long as a mode asks for it.
    fn process(&mut self, mut token: Token) {
        if self.skip_newline {
            self.skip_newline = false;
            if let Token::Text(text) = &mut token {
                if text.starts_with('\n') {
                    text.remove(0);
                    if text.is_empty() {
                        return;
                    }
                }
            }
        }
        for _ in 0..MAX_REPROCESS {
            match self.dispatch(token) {
                Some(again) => token = again,
                None => return,
            }
        }
    }

    /// Tree construction dispatcher: HTML rules or foreign content rules.
    fn dispatch(&mut self, token: Token) -> Option<Token> {
        if self.use_html_rules(&token) {
            self.process_in(self.mode, token)
        } else {
            self.foreign_content(token)
        }
    }

    /// Processes `token` using the rules of insertion mode `mode`. Returns
    /// the token when it must be reprocessed in the (new) current mode.
    fn process_in(&mut self, mode: Mode, token: Token) -> Option<Token> {
        match mode {
            Mode::Initial => self.initial(token),
            Mode::BeforeHtml => self.before_html(token),
            Mode::BeforeHead => self.before_head(token),
            Mode::InHead => self.in_head(token),
            Mode::InHeadNoscript => self.in_head_noscript(token),
            Mode::AfterHead => self.after_head(token),
            Mode::InBody => self.in_body(token),
            Mode::Text => self.text(token),
            Mode::InTable => self.in_table(token),
            Mode::InCaption => self.in_caption(token),
            Mode::InColumnGroup => self.in_column_group(token),
            Mode::InTableBody => self.in_table_body(token),
            Mode::InRow => self.in_row(token),
            Mode::InCell => self.in_cell(token),
            Mode::InSelect => self.in_select(token),
            Mode::InSelectInTable => self.in_select_in_table(token),
            Mode::InTemplate => self.in_template(token),
            Mode::AfterBody => self.after_body(token),
            Mode::InFrameset => self.in_frameset(token),
            Mode::AfterFrameset => self.after_frameset(token),
            Mode::AfterAfterBody => self.after_after_body(token),
            Mode::AfterAfterFrameset => self.after_after_frameset(token),
        }
    }

    /// Asks the tokenizer to switch content model after the current token.
    fn request_text_mode(&mut self, mode: TextMode, end_tag: &str) {
        self.text_mode_request = Some((mode, end_tag.to_string()));
    }

    /// The generic raw text / RCDATA element parsing algorithms.
    fn parse_text_element(&mut self, name: &str, attrs: Vec<Attribute>, mode: TextMode) {
        self.insert_html(name, attrs);
        self.request_text_mode(mode, name);
        self.original_mode = self.mode;
        self.mode = Mode::Text;
    }

    /// Final fix-ups: guarantees `html > head, body` and the depth limit.
    fn finish(mut self) -> Document {
        let root = self.doc.root();
        let html = match self.doc.document_element() {
            Some(html) => html,
            None => {
                let html = self.doc.create_element("html", Vec::new());
                self.doc.append_child(root, html);
                html
            }
        };
        if self.doc.head().is_none() {
            let head = self.doc.create_element("head", Vec::new());
            match self.doc.children(html).first().copied() {
                Some(first) => self.doc.insert_before(html, head, first),
                None => self.doc.append_child(html, head),
            }
        }
        if self.doc.body().is_none() {
            let body = self.doc.create_element("body", Vec::new());
            self.doc.append_child(html, body);
        }
        depth::enforce_depth_limit(&mut self.doc, MAX_DEPTH);
        self.doc
    }
}

/// Whitespace characters as defined by the tree construction rules.
fn is_html_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' ')
}

fn is_all_whitespace(text: &str) -> bool {
    text.chars().all(is_html_whitespace)
}

/// Splits `text` into its leading whitespace and the rest.
fn split_leading_whitespace(text: &str) -> (&str, &str) {
    let rest = text.trim_start_matches(is_html_whitespace);
    text.split_at(text.len() - rest.len())
}

pub(crate) fn decode_bytes(bytes: &[u8], transport_charset: Option<&str>) -> String {
    crate::decode::decode_bytes(bytes, transport_charset)
}

#[cfg(test)]
mod tests;
