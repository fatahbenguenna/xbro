//! Box tree construction: turns the DOM + [`StyleMap`] into a tree of
//! [`BoxNode`]s that layout walks.
//!
//! * `display: none` (no style) generates nothing; `display: contents`
//!   promotes the element's children (and its `::before`/`::after`).
//! * Block containers with both block-level and inline-level children wrap
//!   each run of inline-level content in an anonymous block (runs that are
//!   only collapsible whitespace are dropped).
//! * Block-level boxes inside inline elements split the inline: the inline
//!   content before and after goes into anonymous blocks around the block
//!   box (the inline's own decorations are lost for the block part, its
//!   text keeps its inherited style).
//! * Replaced elements (`img`, `svg`, `video`, `canvas`, `iframe`, `embed`,
//!   `object`, `audio`, `input`, `select`, `textarea`, `progress`, `meter`)
//!   are single atomic boxes; `<svg>` subtrees are never entered.
//! * Floats and absolutely positioned boxes stay in the child lists and are
//!   handled as out-of-flow by layout.
//! * Table parts are organised by [`crate::table`].
//! * The element whose `overflow` propagates to the viewport (the root
//!   element, or `<body>` when the root's is `visible`) gets a used
//!   `overflow: visible` (see [`viewport_overflow`]).
//! * `<button>` boxes are flagged ([`BoxNode::button`]): they establish a
//!   formatting context and are fit-content wide even when block-level.

use std::cell::Cell;
use std::sync::Arc;

use xbro_dom::{Document, NodeData, NodeId};
use xbro_style::{
    Color, ComputedStyle, Content, ContentItem, Display, Float, ListStylePosition, Overflow,
    StyleMap, WhiteSpace,
};

use crate::geom::Size;
use crate::intrinsic::Intrinsic;
use crate::{list, table, ControlKind, Pseudo, ReplacedSizes};

/// Defensive recursion limit (the parser already limits DOM depth to 256).
const MAX_BUILD_DEPTH: usize = 1024;

/// Colour of placeholder text in text fields.
pub(crate) const PLACEHOLDER_COLOR: Color = Color::rgb(0x75, 0x75, 0x75);

/// What a box is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    /// Block container: its children are block-level boxes, or inline-level
    /// content when [`BoxNode::inline_content`] is set.
    Block,
    /// Flex (or grid) container; children are flex items.
    Flex,
    /// Table wrapper; children are captions, row groups and rows.
    Table,
    TableRowGroup,
    TableRow,
    /// Table cell (a block container).
    TableCell,
    /// Non-atomic inline box (`span`, `a`, inline `::before`…).
    Inline,
    /// Raw text of a text node (or generated text); white-space processing
    /// happens during inline layout.
    Text(Box<str>),
    /// `<br>`.
    LineBreak,
    /// `<wbr>`.
    WordBreak,
    /// Replaced element or form control.
    Replaced(Replaced),
}

/// Replaced content.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Replaced {
    /// `<img>` / `<input type=image>`; the alt text (if any) is the single
    /// text child, laid out when the image has no natural size.
    Image,
    /// Inline `<svg>` (natural size from the engine or its attributes).
    Svg,
    /// `content: url(...)` image.
    UrlImage(String),
    /// `<iframe>`, `<video>`, `<canvas>`, `<embed>`, `<object>`, `<audio>`:
    /// an image when the engine has one for the node, else a placeholder.
    /// `keep_ratio`: the default size also defines an aspect ratio (video, canvas).
    Media { default: Size, keep_ratio: bool },
    /// Form control; its text (value, placeholder, label, selected option)
    /// is the text child.
    Control(Control),
}

/// A form control.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Control {
    pub kind: ControlKind,
    pub sizing: ControlSizing,
}

/// How a control's default content size is computed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ControlSizing {
    /// Text field: `size` characters of ~0.55em, one line high.
    TextField { size: f32 },
    /// Button-like: shrink-wrapped label, one line high.
    Button,
    /// Fixed default content size.
    Fixed(Size),
    /// `<select>`: widest option text + arrow.
    Select { options: Vec<String> },
    /// `<textarea>`: `cols` monospace advances × `rows` lines.
    TextArea { cols: f32, rows: f32 },
}

/// A table column (`<col>`, or a `<colgroup>` without `<col>` children):
/// how many grid columns it covers and the width hint it gives them.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct ColSpec {
    /// Number of columns (`span`, clamped to `1..=1000`).
    pub span: u32,
    /// The column's computed `width` (from CSS or the `width` attribute).
    pub width: xbro_style::LengthPercentageAuto,
}

/// Largest `span` of a column and number of columns described by `<col>`s.
const MAX_COL_SPAN: u32 = 1000;

/// A node of the box tree.
#[derive(Debug)]
pub(crate) struct BoxNode {
    /// Generating node (element or text node); `None` for anonymous boxes.
    pub node: Option<NodeId>,
    pub pseudo: Option<Pseudo>,
    pub style: Arc<ComputedStyle>,
    pub kind: Kind,
    /// Outer display is inline-level (atomic unless `kind` is `Inline`/`Text`/breaks).
    pub inline_level: bool,
    /// For block containers: children are inline-level content.
    pub inline_content: bool,
    /// For inline boxes: some descendant is block-level (needs splitting).
    pub has_block_desc: bool,
    /// Child of a flex (or grid) container: establishes a formatting context.
    pub flex_item: bool,
    pub children: Vec<BoxNode>,
    /// List item ordinal value (for `display: list-item`).
    pub ordinal: Option<i64>,
    /// Intrinsic (min/max-content) widths of the content box, cached for the
    /// duration of one layout pass.
    pub intrinsic: Cell<Option<Intrinsic>>,
    /// For tables: the columns declared by `<col>`/`<colgroup>` children
    /// (which generate no boxes), in order.
    pub cols: Vec<ColSpec>,
    /// Box of a `<button>` element: it establishes a formatting context
    /// and an auto width is its fit-content width even when block-level
    /// (HTML rendering, "button layout").
    pub button: bool,
}

impl BoxNode {
    pub(crate) fn new(
        node: Option<NodeId>,
        pseudo: Option<Pseudo>,
        style: Arc<ComputedStyle>,
        kind: Kind,
    ) -> Self {
        BoxNode {
            node,
            pseudo,
            style,
            kind,
            inline_level: false,
            inline_content: false,
            has_block_desc: false,
            flex_item: false,
            children: Vec::new(),
            ordinal: None,
            intrinsic: Cell::new(None),
            cols: Vec::new(),
            button: false,
        }
    }

    /// Anonymous block box containing inline-level content.
    pub(crate) fn anonymous_block(style: Arc<ComputedStyle>, children: Vec<BoxNode>) -> Self {
        let mut b = BoxNode::new(None, None, style, Kind::Block);
        b.inline_content = true;
        b.children = children;
        b
    }

    /// True for text, `<br>` and `<wbr>` boxes (which share their parent's style).
    pub(crate) fn is_text_like(&self) -> bool {
        matches!(self.kind, Kind::Text(_) | Kind::LineBreak | Kind::WordBreak)
    }

    /// Float or absolutely positioned box.
    pub(crate) fn is_out_of_flow(&self) -> bool {
        !self.is_text_like() && !matches!(self.kind, Kind::Inline) && self.style.is_out_of_flow()
    }

    pub(crate) fn is_float(&self) -> bool {
        !self.is_text_like() && self.style.float != Float::None && !self.is_absolute()
    }

    pub(crate) fn is_absolute(&self) -> bool {
        !self.is_text_like() && self.style.position.is_out_of_flow()
    }

    /// In-flow block-level box.
    pub(crate) fn is_block_level(&self) -> bool {
        !self.inline_level
            && !self.is_out_of_flow()
            && !self.is_text_like()
            && !matches!(self.kind, Kind::Inline)
    }

    /// Replaced element or control.
    pub(crate) fn is_replaced(&self) -> bool {
        matches!(self.kind, Kind::Replaced(_))
    }

    /// Box that only contains collapsible whitespace (droppable between blocks).
    fn is_collapsible_whitespace(&self) -> bool {
        match &self.kind {
            Kind::Text(t) => {
                matches!(
                    self.style.white_space,
                    WhiteSpace::Normal | WhiteSpace::Nowrap
                ) && t
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C'))
            }
            Kind::Inline => {
                let s = &self.style;
                let b = s.border_widths();
                s.padding.left.is_zero()
                    && s.padding.right.is_zero()
                    && b.left == 0.0
                    && b.right == 0.0
                    && s.margin.left.resolve(0.0).unwrap_or(0.0) == 0.0
                    && s.margin.right.resolve(0.0).unwrap_or(0.0) == 0.0
                    && self.children.iter().all(BoxNode::is_collapsible_whitespace)
            }
            _ => false,
        }
    }
}

/// `overflow-x` / `overflow-y` of the viewport.
pub(crate) type ViewportOverflow = (Overflow, Overflow);

/// Overflow propagation to the viewport (CSS Overflow 3 §3.3): the root
/// element's `overflow` applies to the viewport, or the `<body>`'s when the
/// root's is `visible`. Returns the element whose value propagates (its
/// used `overflow` becomes `visible`: it neither clips nor establishes a
/// formatting context because of it) and the viewport's values.
pub(crate) fn viewport_overflow(
    doc: &Document,
    styles: &StyleMap,
) -> (Option<NodeId>, ViewportOverflow) {
    let visible = (Overflow::Visible, Overflow::Visible);
    let Some(root) = doc.document_element() else {
        return (None, visible);
    };
    let of = |n: NodeId| styles.get(n).map(|s| (s.overflow_x, s.overflow_y));
    match of(root) {
        Some(o) if o != visible => (Some(root), o),
        Some(_) if doc.is_element_named(root, "html") => {
            let body = doc
                .element_children(root)
                .find(|&c| doc.is_element_named(c, "body"));
            match body.and_then(|b| of(b).map(|o| (b, o))) {
                Some((b, o)) if o != visible => (Some(b), o),
                _ => (None, visible),
            }
        }
        _ => (None, visible),
    }
}

/// Builds the box tree for `doc`. Returns `None` when the document element
/// is missing or not rendered. The element whose `overflow` propagates to
/// the viewport (see [`viewport_overflow`]) gets `overflow: visible`.
pub(crate) fn build(
    doc: &Document,
    styles: &StyleMap,
    replaced: &dyn ReplacedSizes,
) -> Option<BoxNode> {
    let root = doc.document_element()?;
    let root_style = styles.get(root)?.clone();
    let builder = Builder {
        doc,
        styles,
        replaced,
        propagated: viewport_overflow(doc, styles).0,
    };
    let mut out = Vec::new();
    builder.element(root, None, &mut out, 0);
    if out.len() == 1
        && out
            .first()
            .is_some_and(|b| !b.inline_level && !b.is_text_like())
    {
        return out.pop();
    }
    if out.is_empty() {
        return None;
    }
    // Odd roots (replaced or inline-level content): wrap in an anonymous block.
    let mut wrapper = BoxNode::new(Some(root), None, root_style.clone(), Kind::Block);
    let children = finish_block_children(out, &root_style, &mut wrapper);
    wrapper.children = children;
    Some(wrapper)
}

struct Builder<'a> {
    doc: &'a Document,
    styles: &'a StyleMap,
    replaced: &'a dyn ReplacedSizes,
    /// Element whose `overflow` was propagated to the viewport.
    propagated: Option<NodeId>,
}

impl Builder<'_> {
    /// Appends the boxes generated by element `el` to `out`.
    fn element(&self, el: NodeId, ordinal: Option<i64>, out: &mut Vec<BoxNode>, depth: usize) {
        if depth > MAX_BUILD_DEPTH {
            return;
        }
        let Some(style) = self.styles.get(el) else {
            return;
        };
        let Some(data) = self.doc.element(el) else {
            return;
        };
        if style.display == Display::None {
            return;
        }
        let visible_style;
        let style = if self.propagated == Some(el) {
            let mut s = (**style).clone();
            s.overflow_x = Overflow::Visible;
            s.overflow_y = Overflow::Visible;
            visible_style = Arc::new(s);
            &visible_style
        } else {
            style
        };
        let name = data.name.as_str();
        if let Some(replaced) = self.replaced_kind(el, name) {
            if style.display == Display::Contents {
                // `display: contents` computes to `none` on replaced elements.
                return;
            }
            out.push(self.replaced_box(el, style, replaced));
            return;
        }
        match name {
            "br" => {
                out.push(text_like(el, style, Kind::LineBreak));
                return;
            }
            "wbr" => {
                out.push(text_like(el, style, Kind::WordBreak));
                return;
            }
            _ => {}
        }
        if style.display == Display::Contents {
            self.pseudo(el, Pseudo::Before, out);
            self.children(el, out, depth + 1);
            self.pseudo(el, Pseudo::After, out);
            return;
        }
        let mut children = Vec::new();
        if style.display == Display::ListItem
            && style.list_style_position == ListStylePosition::Inside
        {
            self.inside_marker(el, style, ordinal.unwrap_or(1), &mut children);
        }
        self.pseudo(el, Pseudo::Before, &mut children);
        self.children(el, &mut children, depth + 1);
        self.pseudo(el, Pseudo::After, &mut children);
        if let Some(mut b) = make_box(Some(el), None, style.clone(), children, ordinal) {
            if matches!(b.kind, Kind::Table) {
                b.cols = self.table_columns(el);
            }
            b.button = name == "button" && matches!(b.kind, Kind::Block | Kind::Flex);
            out.push(b);
        }
    }

    /// Columns declared by the `display: table-column(-group)` children of
    /// table element `el`. A column group with `<col>` children is described
    /// by them (a `<col>` without width takes the group's); an empty group
    /// covers `span` columns.
    fn table_columns(&self, el: NodeId) -> Vec<ColSpec> {
        let mut out = Vec::new();
        let mut total = 0u32;
        let display = |n: NodeId| self.styles.get(n).map(|s| s.display);
        let span = |n: NodeId| {
            self.doc
                .attr(n, "span")
                .and_then(parse_int)
                .map_or(1, |v| v.clamp(1, i64::from(MAX_COL_SPAN)) as u32)
        };
        let mut push = |out: &mut Vec<ColSpec>, spec: ColSpec| {
            if total < MAX_COL_SPAN {
                let span = spec.span.min(MAX_COL_SPAN - total);
                total += span;
                out.push(ColSpec { span, ..spec });
            }
        };
        for c in self.doc.element_children(el) {
            let Some(style) = self.styles.get(c) else {
                continue;
            };
            match style.display {
                Display::TableColumn => push(
                    &mut out,
                    ColSpec {
                        span: span(c),
                        width: style.width,
                    },
                ),
                Display::TableColumnGroup => {
                    let cols: Vec<NodeId> = self
                        .doc
                        .element_children(c)
                        .filter(|&k| display(k) == Some(Display::TableColumn))
                        .collect();
                    if cols.is_empty() {
                        push(
                            &mut out,
                            ColSpec {
                                span: span(c),
                                width: style.width,
                            },
                        );
                    }
                    for k in cols {
                        let width = self
                            .styles
                            .get(k)
                            .map(|s| s.width)
                            .filter(|w| !w.is_auto())
                            .unwrap_or(style.width);
                        push(
                            &mut out,
                            ColSpec {
                                span: span(k),
                                width,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Inline content of an `inside` list marker: the `list-style-image` when
    /// its size is known, else the marker text followed by a space.
    fn inside_marker(
        &self,
        el: NodeId,
        style: &Arc<ComputedStyle>,
        ordinal: i64,
        out: &mut Vec<BoxNode>,
    ) {
        if let Some(url) = &style.list_style_image {
            if self.replaced.url_size(url).is_some() {
                let mut s = ComputedStyle::inherit_from(style);
                s.display = Display::Inline;
                let mut img = BoxNode::new(
                    None,
                    None,
                    Arc::new(s),
                    Kind::Replaced(Replaced::UrlImage(url.to_string())),
                );
                img.inline_level = true;
                out.push(img);
                out.push(BoxNode::new(
                    Some(el),
                    None,
                    style.clone(),
                    Kind::Text("\u{A0}".into()),
                ));
                return;
            }
        }
        if let Some(text) = list::inside_marker_text(style.list_style_type, ordinal) {
            out.push(BoxNode::new(
                Some(el),
                None,
                style.clone(),
                Kind::Text(text.into()),
            ));
        }
    }

    /// Appends the boxes of the children of `el`, computing list ordinals.
    fn children(&self, el: NodeId, out: &mut Vec<BoxNode>, depth: usize) {
        let mut counter: Option<(i64, i64)> = None;
        for &c in self.doc.children(el) {
            match &self.doc.node(c).data {
                NodeData::Text(t) => {
                    if t.is_empty() {
                        continue;
                    }
                    if let Some(style) = self.styles.get(c) {
                        out.push(BoxNode::new(
                            Some(c),
                            None,
                            style.clone(),
                            Kind::Text(t.as_str().into()),
                        ));
                    }
                }
                NodeData::Element(_) => {
                    let is_item = self
                        .styles
                        .get(c)
                        .is_some_and(|s| s.display == Display::ListItem);
                    let ordinal = if is_item {
                        let (value, step) = *counter.get_or_insert_with(|| self.list_start(el));
                        let value = self
                            .doc
                            .attr(c, "value")
                            .and_then(parse_int)
                            .unwrap_or(value);
                        counter = Some((value.saturating_add(step), step));
                        Some(value)
                    } else {
                        None
                    };
                    self.element(c, ordinal, out, depth);
                }
                _ => {}
            }
        }
    }

    /// First ordinal and step of the list items children of `el`.
    fn list_start(&self, el: NodeId) -> (i64, i64) {
        let is_ol = self.doc.is_element_named(el, "ol");
        let reversed = is_ol && self.doc.attr(el, "reversed").is_some();
        let start = if is_ol {
            self.doc.attr(el, "start").and_then(parse_int)
        } else {
            None
        };
        if reversed {
            let count = self
                .doc
                .children(el)
                .iter()
                .filter(|&&c| {
                    self.styles
                        .get(c)
                        .is_some_and(|s| s.display == Display::ListItem)
                        && self.doc.node(c).is_element()
                })
                .count();
            (start.unwrap_or(count as i64), -1)
        } else {
            (start.unwrap_or(1), 1)
        }
    }

    /// Appends the box of the `::before`/`::after` pseudo-element of `el`.
    fn pseudo(&self, el: NodeId, which: Pseudo, out: &mut Vec<BoxNode>) {
        let style = match which {
            Pseudo::Before => self.styles.before(el),
            Pseudo::After => self.styles.after(el),
        };
        let Some(style) = style else {
            return;
        };
        let Content::Items(items) = &style.content else {
            return;
        };
        let mut kids = Vec::new();
        for item in items {
            let text = match item {
                ContentItem::Text(s) => s.as_str(),
                ContentItem::OpenQuote => "\u{201C}",
                ContentItem::CloseQuote => "\u{201D}",
                ContentItem::Image(url) => {
                    // An anonymous inline replaced box inside the
                    // pseudo-element: inherited properties only (the
                    // pseudo-element's own width etc. size the wrapper).
                    // `content` carries the URL for the painter.
                    let mut s = ComputedStyle::inherit_from(style);
                    s.content = Content::Items(vec![ContentItem::Image(url.clone())]);
                    s.display = Display::Inline;
                    let mut img = BoxNode::new(
                        None,
                        Some(which),
                        Arc::new(s),
                        Kind::Replaced(Replaced::UrlImage(url.clone())),
                    );
                    img.inline_level = true;
                    kids.push(img);
                    continue;
                }
            };
            if !text.is_empty() {
                kids.push(BoxNode::new(
                    Some(el),
                    Some(which),
                    style.clone(),
                    Kind::Text(text.into()),
                ));
            }
        }
        if style.display == Display::Contents {
            out.extend(kids);
        } else if let Some(mut b) = make_box(Some(el), Some(which), style.clone(), kids, None) {
            b.pseudo = Some(which);
            out.push(b);
        }
    }

    /// Classifies replaced elements.
    fn replaced_kind(&self, el: NodeId, name: &str) -> Option<Replaced> {
        let media = |w: f32, h: f32, keep_ratio: bool| {
            Some(Replaced::Media {
                default: Size::new(w, h),
                keep_ratio,
            })
        };
        match name {
            "img" => Some(Replaced::Image),
            "svg" => Some(Replaced::Svg),
            "video" | "canvas" => media(300.0, 150.0, true),
            "iframe" | "embed" | "object" => media(300.0, 150.0, false),
            "audio" => media(300.0, 54.0, false),
            "input" => {
                let ty = self
                    .doc
                    .attr(el, "type")
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if ty == "image" {
                    Some(Replaced::Image)
                } else {
                    Some(Replaced::Control(self.input_control(el, &ty)))
                }
            }
            "select" => Some(Replaced::Control(Control {
                kind: ControlKind::Select,
                sizing: ControlSizing::Select {
                    options: self
                        .select_options(el)
                        .into_iter()
                        .map(|(t, _)| t)
                        .collect(),
                },
            })),
            "textarea" => {
                let cols = self
                    .doc
                    .attr(el, "cols")
                    .and_then(parse_positive)
                    .unwrap_or(20.0);
                let rows = self
                    .doc
                    .attr(el, "rows")
                    .and_then(parse_positive)
                    .unwrap_or(2.0);
                Some(Replaced::Control(Control {
                    kind: ControlKind::TextArea,
                    sizing: ControlSizing::TextArea {
                        cols: cols.min(10_000.0),
                        rows: rows.min(10_000.0),
                    },
                }))
            }
            "progress" => {
                let max = self
                    .doc
                    .attr(el, "max")
                    .and_then(parse_float)
                    .filter(|m| *m > 0.0)
                    .unwrap_or(1.0);
                let value = self
                    .doc
                    .attr(el, "value")
                    .and_then(parse_float)
                    .map(|v| (v / max).clamp(0.0, 1.0))
                    .unwrap_or(0.0);
                Some(Replaced::Control(Control {
                    kind: ControlKind::Progress { value },
                    sizing: ControlSizing::Fixed(Size::new(160.0, 16.0)),
                }))
            }
            "meter" => {
                let num = |n: &str, d: f32| self.doc.attr(el, n).and_then(parse_float).unwrap_or(d);
                let min = num("min", 0.0);
                let max = num("max", 1.0);
                let v = num("value", min);
                let value = if max > min {
                    ((v - min) / (max - min)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                Some(Replaced::Control(Control {
                    kind: ControlKind::Progress { value },
                    sizing: ControlSizing::Fixed(Size::new(80.0, 16.0)),
                }))
            }
            _ => None,
        }
    }

    fn input_control(&self, el: NodeId, ty: &str) -> Control {
        let checked = self.doc.attr(el, "checked").is_some();
        let fixed = |w, h| ControlSizing::Fixed(Size::new(w, h));
        match ty {
            "checkbox" => Control {
                kind: ControlKind::Checkbox { checked },
                sizing: fixed(13.0, 13.0),
            },
            "radio" => Control {
                kind: ControlKind::Radio { checked },
                sizing: fixed(13.0, 13.0),
            },
            "range" => {
                let num = |n: &str, d: f32| self.doc.attr(el, n).and_then(parse_float).unwrap_or(d);
                let min = num("min", 0.0);
                let max = num("max", 100.0);
                let v = num("value", (min + max) / 2.0);
                let value = if max > min {
                    ((v - min) / (max - min)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                Control {
                    kind: ControlKind::Range { value },
                    sizing: fixed(129.0, 16.0),
                }
            }
            "color" => {
                let color = self
                    .doc
                    .attr(el, "value")
                    .map(str::trim)
                    .and_then(|v| v.strip_prefix('#'))
                    .and_then(Color::from_hex)
                    .unwrap_or(Color::BLACK);
                Control {
                    kind: ControlKind::Color(color),
                    sizing: fixed(44.0, 23.0),
                }
            }
            "submit" | "reset" | "button" | "file" => Control {
                kind: ControlKind::Button,
                sizing: ControlSizing::Button,
            },
            "hidden" => Control {
                kind: ControlKind::TextField,
                sizing: fixed(0.0, 0.0),
            },
            _ => {
                let size = self
                    .doc
                    .attr(el, "size")
                    .and_then(parse_positive)
                    .unwrap_or(20.0)
                    .min(10_000.0);
                Control {
                    kind: ControlKind::TextField,
                    sizing: ControlSizing::TextField { size },
                }
            }
        }
    }

    /// `(text, selected)` of every `<option>` of a `<select>`.
    fn select_options(&self, el: NodeId) -> Vec<(String, bool)> {
        self.doc
            .descendants(el)
            .filter(|&n| self.doc.is_element_named(n, "option"))
            .take(10_000)
            .map(|o| {
                let label = self
                    .doc
                    .attr(o, "label")
                    .filter(|l| !l.trim().is_empty())
                    .map(collapse_ws)
                    .unwrap_or_else(|| collapse_ws(&self.doc.text_content(o)));
                (label, self.doc.attr(o, "selected").is_some())
            })
            .collect()
    }

    /// Builds the box of a replaced element, with its text child if any.
    fn replaced_box(&self, el: NodeId, style: &Arc<ComputedStyle>, r: Replaced) -> BoxNode {
        let mut text: Option<(String, Arc<ComputedStyle>)> = None;
        match &r {
            Replaced::Image => {
                if let Some(alt) = self.doc.attr(el, "alt").filter(|a| !a.trim().is_empty()) {
                    text = Some((alt.to_string(), style.clone()));
                }
            }
            Replaced::Control(c) => text = self.control_text(el, style, c),
            _ => {}
        }
        let mut b = BoxNode::new(Some(el), None, style.clone(), Kind::Replaced(r));
        b.inline_level = !style.display.is_block_level();
        if let Some((t, s)) = text {
            b.children
                .push(BoxNode::new(Some(el), None, s, Kind::Text(t.into())));
            b.inline_content = true;
        }
        b
    }

    /// Text shown inside a control and its style.
    fn control_text(
        &self,
        el: NodeId,
        style: &Arc<ComputedStyle>,
        c: &Control,
    ) -> Option<(String, Arc<ComputedStyle>)> {
        let single_line = |color: Option<Color>| {
            let mut s = (**style).clone();
            s.white_space = WhiteSpace::Pre;
            s.text_indent = xbro_style::LengthPercentage::ZERO;
            if let Some(color) = color {
                s.color = color;
            }
            Arc::new(s)
        };
        let attr = |n: &str| self.doc.attr(el, n);
        match &c.kind {
            ControlKind::TextField => {
                let ty = attr("type").unwrap_or("").trim().to_ascii_lowercase();
                if ty == "hidden" {
                    return None;
                }
                match attr("value").filter(|v| !v.is_empty()) {
                    Some(v) => {
                        let shown = if ty == "password" {
                            "\u{2022}".repeat(v.chars().count().min(100_000))
                        } else {
                            v.replace(['\n', '\r'], "")
                        };
                        Some((shown, single_line(None)))
                    }
                    None => attr("placeholder").filter(|p| !p.is_empty()).map(|p| {
                        (
                            p.replace(['\n', '\r'], ""),
                            single_line(Some(PLACEHOLDER_COLOR)),
                        )
                    }),
                }
            }
            ControlKind::Button => {
                let ty = attr("type").unwrap_or("").trim().to_ascii_lowercase();
                let label = match ty.as_str() {
                    "file" => "Choose File".to_string(),
                    _ => match attr("value") {
                        Some(v) => v.to_string(),
                        None => match ty.as_str() {
                            "submit" => "Submit".to_string(),
                            "reset" => "Reset".to_string(),
                            _ => String::new(),
                        },
                    },
                };
                (!label.is_empty()).then(|| (label, single_line(None)))
            }
            ControlKind::Select => {
                let options = self.select_options(el);
                let selected = options
                    .iter()
                    .find(|(_, sel)| *sel)
                    .or_else(|| options.first())
                    .map(|(t, _)| t.clone())
                    .unwrap_or_default();
                (!selected.is_empty()).then(|| (selected, single_line(None)))
            }
            ControlKind::TextArea => {
                let content = self.doc.text_content(el);
                if !content.is_empty() {
                    Some((content, style.clone()))
                } else {
                    attr("placeholder").filter(|p| !p.is_empty()).map(|p| {
                        let mut s = (**style).clone();
                        s.color = PLACEHOLDER_COLOR;
                        (p.to_string(), Arc::new(s))
                    })
                }
            }
            _ => None,
        }
    }
}

/// `<br>`/`<wbr>` box.
fn text_like(el: NodeId, style: &Arc<ComputedStyle>, kind: Kind) -> BoxNode {
    let mut b = BoxNode::new(Some(el), None, style.clone(), kind);
    b.inline_level = true;
    b
}

/// Creates the box for a (pseudo-)element from its display type and raw children.
fn make_box(
    node: Option<NodeId>,
    pseudo: Option<Pseudo>,
    style: Arc<ComputedStyle>,
    children: Vec<BoxNode>,
    ordinal: Option<i64>,
) -> Option<BoxNode> {
    let display = style.display;
    let mut b = match display {
        Display::None | Display::TableColumn | Display::TableColumnGroup => return None,
        Display::Inline | Display::Contents => {
            let mut b = BoxNode::new(node, pseudo, style.clone(), Kind::Inline);
            b.inline_level = true;
            b.has_block_desc = children.iter().any(|c| {
                c.is_block_level() || (matches!(c.kind, Kind::Inline) && c.has_block_desc)
            });
            b.children = children;
            return Some(b);
        }
        Display::Block | Display::ListItem | Display::InlineBlock | Display::TableCaption => {
            let mut b = BoxNode::new(node, pseudo, style.clone(), Kind::Block);
            b.inline_level = display == Display::InlineBlock;
            let kids = finish_block_children(children, &style, &mut b);
            b.children = kids;
            b
        }
        Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid => {
            let mut b = BoxNode::new(node, pseudo, style.clone(), Kind::Flex);
            b.inline_level = matches!(display, Display::InlineFlex | Display::InlineGrid);
            let flat = split_block_in_inline(children);
            b.children = wrap_inline_runs(flat, &style);
            for c in &mut b.children {
                c.flex_item = true;
            }
            b
        }
        Display::Table | Display::InlineTable => {
            let mut b = BoxNode::new(node, pseudo, style.clone(), Kind::Table);
            b.inline_level = display == Display::InlineTable;
            b.children = table::fixup_table_children(children, &style);
            b
        }
        Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup => {
            let mut b = BoxNode::new(node, pseudo, style.clone(), Kind::TableRowGroup);
            b.children = table::fixup_row_group_children(children, &style);
            b
        }
        Display::TableRow => {
            let mut b = BoxNode::new(node, pseudo, style.clone(), Kind::TableRow);
            b.children = table::fixup_row_children(children, &style);
            b
        }
        Display::TableCell => {
            let mut b = BoxNode::new(node, pseudo, style.clone(), Kind::TableCell);
            let kids = finish_block_children(children, &style, &mut b);
            b.children = kids;
            b
        }
    };
    if display == Display::ListItem {
        b.ordinal = Some(ordinal.unwrap_or(1));
    }
    Some(b)
}

/// Organises the raw children of a block container: splits inlines around
/// block-level descendants, wraps misparented table parts, and either keeps
/// inline content (setting `inline_content`) or wraps inline runs into
/// anonymous blocks.
pub(crate) fn finish_block_children(
    children: Vec<BoxNode>,
    style: &Arc<ComputedStyle>,
    container: &mut BoxNode,
) -> Vec<BoxNode> {
    let flat = split_block_in_inline(children);
    let flat = table::wrap_misparented(flat, style);
    if flat.iter().any(BoxNode::is_block_level) {
        container.inline_content = false;
        wrap_inline_runs(flat, style)
    } else {
        container.inline_content = true;
        flat
    }
}

/// Replaces inline boxes that contain block-level descendants by their parts.
fn split_block_in_inline(children: Vec<BoxNode>) -> Vec<BoxNode> {
    if !children
        .iter()
        .any(|c| matches!(c.kind, Kind::Inline) && c.has_block_desc)
    {
        return children;
    }
    let mut out = Vec::with_capacity(children.len() + 2);
    for c in children {
        if matches!(c.kind, Kind::Inline) && c.has_block_desc {
            split_inline(c, &mut out);
        } else {
            out.push(c);
        }
    }
    out
}

/// Splits one inline box around its block-level descendants.
fn split_inline(b: BoxNode, out: &mut Vec<BoxNode>) {
    let BoxNode {
        node,
        pseudo,
        style,
        children,
        ..
    } = b;
    let shell = |kids: Vec<BoxNode>| {
        let mut s = BoxNode::new(node, pseudo, style.clone(), Kind::Inline);
        s.inline_level = true;
        s.children = kids;
        s
    };
    let mut current: Vec<BoxNode> = Vec::new();
    for c in children {
        if matches!(c.kind, Kind::Inline) && c.has_block_desc {
            let mut parts = Vec::new();
            split_inline(c, &mut parts);
            for p in parts {
                if p.is_block_level() {
                    if !current.is_empty() {
                        out.push(shell(std::mem::take(&mut current)));
                    }
                    out.push(p);
                } else {
                    current.push(p);
                }
            }
        } else if c.is_block_level() {
            if !current.is_empty() {
                out.push(shell(std::mem::take(&mut current)));
            }
            out.push(c);
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        out.push(shell(current));
    }
}

/// Wraps every run of inline-level boxes into an anonymous block box
/// (runs of collapsible whitespace are dropped). Out-of-flow boxes join the
/// current run if there is one. `text-indent` only applies to an anonymous
/// block that comes before any block-level sibling (CSS 2.1 §16.1: the
/// first line of an anonymous block box is only indented if it is the
/// first child of its parent element).
pub(crate) fn wrap_inline_runs(
    flat: Vec<BoxNode>,
    parent_style: &Arc<ComputedStyle>,
) -> Vec<BoxNode> {
    let mut anon_style: Option<Arc<ComputedStyle>> = None;
    let mut unindented_style: Option<Arc<ComputedStyle>> = None;
    let mut seen_block = false;
    let mut out = Vec::with_capacity(flat.len());
    let mut run: Vec<BoxNode> = Vec::new();
    let mut flush = |run: &mut Vec<BoxNode>, out: &mut Vec<BoxNode>, seen_block: bool| {
        if run.is_empty() {
            return;
        }
        let droppable = run
            .iter()
            .all(|c| c.is_out_of_flow() || c.is_collapsible_whitespace());
        if droppable {
            out.extend(run.drain(..).filter(BoxNode::is_out_of_flow));
            return;
        }
        let style = if seen_block && !parent_style.text_indent.is_zero() {
            unindented_style
                .get_or_insert_with(|| {
                    let mut s = (*anonymous_style(parent_style)).clone();
                    s.text_indent = xbro_style::LengthPercentage::ZERO;
                    Arc::new(s)
                })
                .clone()
        } else {
            anon_style
                .get_or_insert_with(|| anonymous_style(parent_style))
                .clone()
        };
        out.push(BoxNode::anonymous_block(style, std::mem::take(run)));
    };
    for c in flat {
        if c.is_block_level() {
            flush(&mut run, &mut out, seen_block);
            seen_block = true;
            out.push(c);
        } else if c.is_out_of_flow() && run.is_empty() {
            out.push(c);
        } else {
            run.push(c);
        }
    }
    flush(&mut run, &mut out, seen_block);
    out
}

/// Style of anonymous block boxes inside `parent`.
pub(crate) fn anonymous_style(parent: &ComputedStyle) -> Arc<ComputedStyle> {
    let mut s = ComputedStyle::inherit_from(parent);
    s.display = Display::Block;
    Arc::new(s)
}

fn parse_int(s: &str) -> Option<i64> {
    let t = s.trim();
    let t = t.strip_prefix('+').unwrap_or(t);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let digits = digits.get(..end)?;
    if digits.is_empty() {
        return None;
    }
    let v: i64 = digits
        .bytes()
        .take(15)
        .fold(0i64, |acc, b| acc * 10 + i64::from(b - b'0'));
    Some(if neg { -v } else { v })
}

fn parse_float(s: &str) -> Option<f32> {
    s.trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(-1.0e9, 1.0e9))
}

fn parse_positive(s: &str) -> Option<f32> {
    parse_int(s).filter(|v| *v > 0).map(|v| v as f32)
}

/// Collapses whitespace runs to single spaces and trims.
fn collapse_ws(s: &str) -> String {
    s.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers() {
        assert_eq!(parse_int(" 12 "), Some(12));
        assert_eq!(parse_int("-3"), Some(-3));
        assert_eq!(parse_int("+7px"), Some(7));
        assert_eq!(parse_int("x"), None);
        assert_eq!(
            parse_int("99999999999999999999999"),
            Some(999_999_999_999_999)
        );
        assert_eq!(parse_positive("0"), None);
        assert_eq!(parse_float("NaN"), None);
        assert_eq!(collapse_ws("  a \n b "), "a b");
    }
}
