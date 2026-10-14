//! Layout for xbro: builds a box tree from the DOM + computed styles and
//! positions every box and line fragment in absolute document coordinates
//! (CSS px, origin at the top-left of the initial containing block).
//!
//! Supported formatting: block flow (with margin collapsing), inline flow
//! (line breaking, `text-align`, `vertical-align`, `white-space`), floats and
//! `clear`, relative/absolute/fixed positioning, list markers, flexbox, tables
//! (auto layout, `colspan`/`rowspan`), a grid approximation, replaced elements
//! (images, inline SVG, form controls) and `::before`/`::after` content.
//!
//! # Pipeline
//!
//! 1. **Box tree** (`box_tree`): the DOM + [`StyleMap`] become a tree of
//!    boxes (anonymous blocks around inline runs, block-in-inline splitting,
//!    `display: contents`, generated content, table fix-ups, replaced
//!    elements and form controls with their text).
//! 2. **Layout** (`block`, `inline`, `float`, `position`, `replaced`,
//!    `list`, `flex`, `table`): one pass over the box tree with per-pass
//!    caches (fonts, word widths, min/max-content widths), producing the
//!    [`LayoutTree`]. Absolutely positioned boxes are laid out when their
//!    containing block is done; `relative` offsets move finished subtrees.
//! 3. **Queries** (`query`, `dump`): hit testing, node rectangles, text
//!    fragments, document size and text dumps.
//!
//! # Approximations
//!
//! * Flexbox follows CSS Flexbox 1 §9 with simplifications (see the
//!   `flex` module): baseline alignment only in rows, no `visibility:
//!   collapse`.
//! * Grid containers are approximated by equal-width columns filled row
//!   by row (see the `grid` module): track sizes and explicit placement are
//!   ignored; with an unknown column count it is guessed from the items.
//! * Tables (see the `table` module): `table-layout: fixed` is only read
//!   from the inline `style` attribute (not in `ComputedStyle`),
//!   `caption-side` is ignored (captions on top), collapsed borders are
//!   approximated by overlapping cell borders.
//! * An `<img>` without a natural size but with a non-empty `alt` is a
//!   [`BoxKind::Image`] box sized like an inline-block around its alt text,
//!   which is laid out as its `lines` (unless `width`/`height` are given).
//! * `position: sticky` behaves as `relative` without offsets. A positioned
//!   inline element is the containing block (the bounding box of the
//!   padding boxes of its first and last fragments) of the absolutely
//!   positioned boxes in its inline content; those inside atomic inlines
//!   or block-level parts of a split inline use the nearest positioned
//!   block instead. The `relative` offsets of inline elements move their
//!   fragments.
//! * Absolutely positioned boxes become children of their containing
//!   block's box (of the block container holding the lines for an inline
//!   containing block, or of the root box for the initial containing block
//!   and the viewport); `content: url()` images are node-less
//!   [`BoxKind::Image`] boxes whose style's `content` holds the URL.
//! * `aspect-ratio` sizes non-replaced boxes with an auto width or height
//!   (the content still fits with `min-height: auto`).
//! * Left-to-right horizontal text only; one glyph per character.

pub mod geom;

mod block;
mod box_tree;
mod cx;
mod dump;
mod flex;
mod float;
mod grid;
mod inline;
mod intrinsic;
mod list;
mod position;
mod query;
mod replaced;
mod sizing;
mod table;
mod text;
mod tree;

use std::sync::Arc;

pub use geom::{Edges, Point, Rect, Size};
use xbro_dom::{Document, NodeId};
use xbro_style::{ComputedStyle, StyleMap};
use xbro_text::{FontId, FontSystem, ShapedGlyph};

/// Natural sizes of replaced content, provided by the engine (which loads images).
pub trait ReplacedSizes: Sync {
    /// Natural size in CSS px of the image for `<img>`, `<input type=image>`,
    /// `<svg>` (inline SVG rendered by the engine), `<video poster>`,
    /// `<object>`/`<embed>` images. `None` when not (yet) loaded or broken.
    fn node_size(&self, node: NodeId) -> Option<Size>;
    /// Natural size of an image referenced by URL (`list-style-image`, `content: url()`).
    fn url_size(&self, url: &str) -> Option<Size>;
}

/// A [`ReplacedSizes`] with no images at all.
pub struct NoImages;

impl ReplacedSizes for NoImages {
    fn node_size(&self, _: NodeId) -> Option<Size> {
        None
    }
    fn url_size(&self, _: &str) -> Option<Size> {
        None
    }
}

/// Everything layout needs.
pub struct LayoutContext<'a> {
    pub document: &'a Document,
    pub styles: &'a StyleMap,
    pub fonts: &'a FontSystem,
    pub replaced: &'a dyn ReplacedSizes,
    /// Size of the viewport (initial containing block) in CSS px.
    pub viewport: Size,
}

/// Kind of form control, for painting control-specific decorations.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlKind {
    /// `<input>` of a textual type (text, search, email, url, password (shown as
    /// bullets), number, tel, date…). Its value — or its placeholder, painted with
    /// a grey color — is laid out as the box's line content.
    TextField,
    /// `<input type=submit|reset|button>`: label laid out as line content.
    /// (`<button>` elements are ordinary inline-blocks with their children.)
    Button,
    Checkbox {
        checked: bool,
    },
    Radio {
        checked: bool,
    },
    /// `<select>`: the selected option's text is laid out as line content; paint adds an arrow.
    Select,
    /// `<textarea>`: its text content laid out with `pre-wrap`.
    TextArea,
    /// `<input type=range>`: a track and thumb (value 0..=1).
    Range {
        value: f32,
    },
    /// `<input type=color>`.
    Color(xbro_style::Color),
    /// `<progress>` / `<meter>` (value 0..=1).
    Progress {
        value: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum BoxKind {
    /// Block container: block, list-item, inline-block, table-caption, flex/grid
    /// items, anonymous block boxes, block-level `::before`/`::after`. Its content is
    /// either `children` (block formatting) or `lines` (inline formatting).
    Block,
    /// Flex container (also used for the grid approximation); items are `children`.
    Flex,
    /// Table wrapper box: `children` are the caption (if any) and row groups / rows.
    Table,
    TableRowGroup,
    TableRow,
    /// Table cell (a block container, like `Block`).
    TableCell,
    /// Replaced element painted with the image the engine associated with `node`.
    /// If no image is available, paint draws nothing (the alt text, if any, is laid
    /// out as `lines` instead).
    Image,
    /// `<iframe>`, `<video>` without poster, `<canvas>`, `<embed>`/`<object>`
    /// without image: an empty box of the right size.
    Placeholder,
    /// Form control. Text (value/placeholder/label/selected option) is in `lines`.
    Control(ControlKind),
}

/// Which pseudo-element generated a box, if any.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Pseudo {
    Before,
    After,
}

/// A laid-out box. All rects are absolute document coordinates, except
/// inside `position: fixed` subtrees where they are viewport coordinates
/// (the painter must not apply the scroll offset to them; see [`LayoutBox::is_fixed`]).
#[derive(Clone, Debug)]
pub struct LayoutBox {
    /// Generating element; `None` for anonymous boxes. For `::before`/`::after`
    /// boxes this is the originating element and `pseudo` is set.
    pub node: Option<NodeId>,
    pub pseudo: Option<Pseudo>,
    pub style: Arc<ComputedStyle>,
    pub kind: BoxKind,
    /// Content box.
    pub content: Rect,
    /// Used padding, border and margin widths.
    pub padding: Edges,
    pub border: Edges,
    pub margin: Edges,
    /// Child boxes in tree order: block-level children, floats, absolutely
    /// positioned boxes, flex items, table parts.
    pub children: Vec<LayoutBox>,
    /// Line boxes when this box establishes an inline formatting context.
    pub lines: Vec<LineBox>,
    /// List item marker, if this is a `list-item` with a marker.
    pub marker: Option<ListMarker>,
}

impl LayoutBox {
    pub fn padding_box(&self) -> Rect {
        self.content.outset(self.padding)
    }

    pub fn border_box(&self) -> Rect {
        self.padding_box().outset(self.border)
    }

    pub fn margin_box(&self) -> Rect {
        self.border_box().outset(self.margin)
    }

    /// True for `position: fixed` boxes (coordinates relative to the viewport).
    pub fn is_fixed(&self) -> bool {
        self.style.position == xbro_style::Position::Fixed
    }
}

/// One line of an inline formatting context.
#[derive(Clone, Debug)]
pub struct LineBox {
    /// The line box (full available width of the line, after float shortening).
    pub rect: Rect,
    /// Absolute y of the line's baseline.
    pub baseline: f32,
    /// Fragments in paint order. An [`InlineBoxFragment`] always precedes the
    /// fragments it contains so backgrounds are painted under text.
    pub fragments: Vec<Fragment>,
}

#[derive(Clone, Debug)]
pub enum Fragment {
    Text(TextFragment),
    /// Background/border/padding area of an inline element on this line.
    InlineBox(InlineBoxFragment),
    /// Atomic inline: inline-block, inline-flex/table, inline replaced element,
    /// inline form control. Fully positioned.
    Atomic(LayoutBox),
}

/// A run of text with a single style on a single line.
#[derive(Clone, Debug)]
pub struct TextFragment {
    /// Source DOM node: the text node; or the element for generated text
    /// (`::before`/`::after`, control values, alt text, list markers).
    pub node: NodeId,
    /// Box from `baseline - ascent` to `baseline + descent`, `width` = advance width.
    pub rect: Rect,
    /// Absolute y of the baseline.
    pub baseline: f32,
    /// Displayed text (after white-space processing and text-transform).
    pub text: String,
    pub font: FontId,
    pub font_size: f32,
    /// Shaped glyphs; `x` is relative to `rect.x`.
    pub glyphs: Vec<ShapedGlyph>,
    /// Style of the text (color, `decorations_in_effect`, `decoration_color`,
    /// visibility). Placeholder text uses a style whose color is grey.
    pub style: Arc<ComputedStyle>,
}

/// Portion of an inline element (e.g. `<span style="background:…">`, `<a>`) on one line.
#[derive(Clone, Debug)]
pub struct InlineBoxFragment {
    pub node: NodeId,
    pub style: Arc<ComputedStyle>,
    /// Border box of this portion: spans the element's content on this line
    /// vertically from `baseline - ascent - padding.top - border.top` to
    /// `baseline + descent + padding.bottom + border.bottom` (it does not affect line height).
    pub rect: Rect,
    /// Used border/padding; left values are 0 except on the first fragment of
    /// the element, right values are 0 except on its last fragment.
    pub border: Edges,
    pub padding: Edges,
}

#[derive(Clone, Debug)]
pub enum MarkerContent {
    Disc,
    Circle,
    Square,
    /// Ordinal markers ("1.", "iv.", "b.") and `::marker`-like strings.
    Text(TextFragment),
    /// `list-style-image` URL.
    Image(String),
}

#[derive(Clone, Debug)]
pub struct ListMarker {
    /// Where the marker is painted (outside markers sit left of the first line).
    pub rect: Rect,
    pub color: xbro_style::Color,
    pub content: MarkerContent,
}

/// Result of a hit test.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HitResult {
    /// Deepest DOM node under the point: a text node for text, otherwise the
    /// element whose box contains the point.
    pub node: NodeId,
}

/// The laid-out document.
#[derive(Clone, Debug)]
pub struct LayoutTree {
    /// Box of the root element (`<html>`), or `None` for an empty document.
    pub root: Option<LayoutBox>,
    /// Viewport used for this layout.
    pub viewport: Size,
    /// Scrollable size of the document: at least the viewport, extended to
    /// include all in-flow and positioned content (not `fixed` boxes).
    pub document_size: Size,
}

impl LayoutTree {
    /// Topmost node at `point` (document coordinates). `scroll` is the current
    /// scroll offset, needed to hit-test `position: fixed` content.
    /// Follows paint order: positioned boxes above in-flow content, later
    /// siblings above earlier ones, `visibility: hidden` boxes are skipped.
    /// A point on the canvas (inside the document size) that hits no box
    /// gives the root element.
    pub fn hit_test(&self, point: Point, scroll: Point) -> Option<HitResult> {
        query::hit_test(self, point, scroll)
    }

    /// Border box of the first box or fragment generated by `node` (or one of its
    /// descendants for elements that only generate inline fragments). Used to
    /// scroll to `#fragment` anchors.
    pub fn node_rect(&self, node: NodeId) -> Option<Rect> {
        query::node_rect(self, node)
    }

    /// All text fragments in paint order (for find-in-page).
    pub fn text_fragments(&self) -> Vec<&TextFragment> {
        query::text_fragments(self)
    }

    /// Human-readable dump of the box tree (`xbro --dump-layout`).
    pub fn dump(&self, doc: &Document) -> String {
        dump::dump(self, doc)
    }

    /// Compact outline of the box tree: one line per box (`tag` name, or
    /// `anon`/`::before`… for generated boxes, and its border box rounded
    /// to whole px), plus one line per line box with its text. Handy to
    /// compare layouts in tests.
    pub fn outline(&self, doc: &Document) -> String {
        dump::outline(self, doc)
    }
}

/// Lays out the whole document.
///
/// The root element's box is placed at the document origin with the
/// viewport width as its containing block width; the result's
/// `document_size` is the viewport extended to the scrollable overflow of
/// every box that is not `position: fixed`.
pub fn layout(ctx: &LayoutContext<'_>) -> LayoutTree {
    let viewport = Size::new(
        sizing::sane_size(ctx.viewport.width),
        sizing::sane_size(ctx.viewport.height),
    );
    let Some(root_node) = box_tree::build(ctx.document, ctx.styles, ctx.replaced) else {
        return LayoutTree {
            root: None,
            viewport,
            document_size: viewport,
        };
    };
    let mut cx = cx::LayoutCx::new(ctx.document, ctx.fonts, ctx.replaced, viewport);
    let root = cx.layout_root(&root_node);
    let document_size = query::document_size(&root, viewport);
    LayoutTree {
        root: Some(root),
        viewport,
        document_size,
    }
}
