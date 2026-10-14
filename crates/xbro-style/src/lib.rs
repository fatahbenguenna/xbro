//! Style system for xbro: selector matching against the DOM, the cascade
//! (origins, specificity, `!important`, inline styles), inheritance, CSS
//! custom properties (`var()`), `calc()`, shorthands and HTML presentational
//! hints, producing a [`ComputedStyle`] per rendered node.
//!
//! Approximations (see the module docs for details): logical properties are
//! mapped for horizontal left-to-right text; `revert` rolls back to the
//! user-agent value (there is no user origin);
//! percentages inside `min()`/`max()`/`clamp()` that mix with lengths are
//! resolved against the viewport width; gradient stop positions given as
//! lengths are spread like omitted ones; radial and conic gradients paint
//! their first color as the background color; intrinsic sizes
//! (`min-content`, `fit-content`, ...) compute to `auto`.
//!
//! Hostile pages are bounded: values copied into every element they apply
//! to are size-limited (`content` ≤ 256 items / 64 KiB of text, gradients
//! ≤ 256 stops, `calc()` trees ≤ 256 nodes; larger ones are invalid
//! declarations), repeated declarations in a block are deduplicated, and
//! custom-property work is memoized across elements and charged to a
//! per-document budget (see the `vars` module).

mod bloom;
pub mod cascade;
mod compute;
pub mod computed;
mod dom;
mod hints;
pub mod properties;
mod rules;
pub mod ua;
mod values;
mod vars;

use std::collections::HashMap;
use std::sync::Arc;

pub use computed::*;
use xbro_css::{MediaContext, Stylesheet};
use xbro_dom::{Document, NodeId};

/// Inputs to style computation besides the document and author stylesheets.
#[derive(Clone, Debug, Default)]
pub struct StyleContext {
    /// Viewport and device information (also used to resolve `vw`/`vh`).
    pub media: MediaContext,
    /// Element under the mouse pointer: it and its ancestors match `:hover`.
    pub hover: Option<NodeId>,
    /// Focused element (`:focus`, and its ancestors match `:focus-within`).
    pub focus: Option<NodeId>,
    /// Element being pressed (`:active`, also applied to its ancestors).
    pub active: Option<NodeId>,
}

/// Computed styles for a document, indexed by [`NodeId`].
#[derive(Clone, Debug, Default)]
pub struct StyleMap {
    /// `styles[node.index()]`: `Some` for every element that generates boxes
    /// (display is not `none` for it and all ancestors) and for every text node
    /// whose parent element has a style (text nodes share their parent's `Arc`).
    /// `None` for nodes in `display: none` subtrees, comments, doctypes and
    /// non-rendered elements (`head`, `script`, `style`, …, via the UA sheet).
    pub styles: Vec<Option<Arc<ComputedStyle>>>,
    /// `::before` styles, only for elements whose `::before` has `content`
    /// other than `normal`/`none` and display other than `none`.
    pub before: HashMap<NodeId, Arc<ComputedStyle>>,
    /// `::after` styles, same rules as `before`.
    pub after: HashMap<NodeId, Arc<ComputedStyle>>,
}

impl StyleMap {
    /// Style of a rendered element or text node (`None` if not rendered).
    pub fn get(&self, node: NodeId) -> Option<&Arc<ComputedStyle>> {
        self.styles.get(node.index()).and_then(|s| s.as_ref())
    }

    /// Style of the `::before` pseudo-element of `node`, if it generates a box.
    pub fn before(&self, node: NodeId) -> Option<&Arc<ComputedStyle>> {
        self.before.get(&node)
    }

    /// Style of the `::after` pseudo-element of `node`, if it generates a box.
    pub fn after(&self, node: NodeId) -> Option<&Arc<ComputedStyle>> {
        self.after.get(&node)
    }
}

/// Computes styles for every rendered node of `doc`.
///
/// Cascade order (lowest to highest precedence): user-agent normal, author
/// normal (sheets in the given order, then `style=""` attributes), author
/// `!important`, user-agent `!important`. HTML presentational hints
/// (`bgcolor`, `width`, `align`, `<font color>`, …) are treated as author rules
/// with zero specificity that precede all author sheets.
///
/// `author_sheets` are in document order; `@media` and `@supports` rules are
/// evaluated against `ctx.media`. `@import` rules are ignored here (the engine
/// inlines imported sheets before calling this).
pub fn compute_styles(
    doc: &Document,
    author_sheets: &[Stylesheet],
    ctx: &StyleContext,
) -> StyleMap {
    cascade::compute(doc, author_sheets, ctx)
}
