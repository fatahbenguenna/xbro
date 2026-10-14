//! Queries on a laid-out tree: hit testing, node rectangles, text
//! fragments and the scrollable document size.
//!
//! Hit testing walks boxes in the reverse of the painter's order (CSS 2.1
//! Appendix E, as `xbro-paint` implements it), so that the topmost painted
//! box wins:
//!
//! * stacking contexts are the root, positioned boxes with a `z-index`,
//!   flex items with a `z-index`, `fixed` and `sticky` boxes and boxes with
//!   `opacity < 1`;
//! * inside a stacking context: child stacking contexts with a positive
//!   `z-index` (highest first), then positioned descendants with `z-index:
//!   auto` and `z-index: 0` stacking contexts (later in document order
//!   first; a `z-index: auto` box is tested "atomically", its own
//!   positioned descendants belonging to the enclosing stacking context),
//!   then the in-flow content (inline content, then floats, then the
//!   backgrounds of blocks, later first), then child stacking contexts with
//!   a negative `z-index`, then the stacking context's own box.
//!
//! `overflow` other than `visible` clips descendants to the padding box
//! (absolutely positioned descendants only by the clips up to their
//! containing block); `visibility: hidden` boxes and fragments are
//! transparent to hits (their visible descendants are not). `position:
//! fixed` subtrees are tested in viewport coordinates and ignore the clips
//! of their ancestors.

use std::cell::OnceCell;
use std::collections::HashMap;

use xbro_dom::NodeId;
use xbro_style::{Float, Overflow, Position, Visibility};

use crate::geom::{Point, Rect, Size};
use crate::sizing::sane_size;
use crate::{
    BoxKind, Fragment, HitResult, LayoutBox, LayoutTree, ListMarker, MarkerContent, Pseudo,
    TextFragment,
};

/// Recursion guard (the DOM depth is bounded by the parser; anonymous boxes
/// add a few levels).
const MAX_DEPTH: usize = 2048;

fn visible(style: &xbro_style::ComputedStyle) -> bool {
    style.visibility == Visibility::Visible
}

fn clips(b: &LayoutBox) -> bool {
    b.style.overflow_x.clips() || b.style.overflow_y.clips()
}

/// Used opacity (as the painter computes it).
fn opacity(b: &LayoutBox) -> f32 {
    let o = b.style.opacity;
    if o.is_finite() {
        o.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// True when the box creates a stacking context (`parent_flex`: it is a
/// flex item).
fn creates_stacking_context(b: &LayoutBox, parent_flex: bool) -> bool {
    let s = &b.style;
    opacity(b) < 1.0
        || matches!(s.position, Position::Fixed | Position::Sticky)
        || (s.z_index.is_some() && (s.position.is_positioned() || parent_flex))
}

/// Painted in a positioned layer of its stacking context.
fn is_layered(b: &LayoutBox, parent_flex: bool) -> bool {
    b.style.position.is_positioned() || creates_stacking_context(b, parent_flex)
}

fn z_order(b: &LayoutBox, parent_flex: bool) -> i32 {
    if b.style.position.is_positioned() || parent_flex {
        b.style.z_index.unwrap_or(0)
    } else {
        0
    }
}

fn is_float(b: &LayoutBox) -> bool {
    b.style.float != Float::None && !b.style.position.is_out_of_flow()
}

fn intersect(clip: Option<Rect>, r: Rect) -> Option<Rect> {
    Some(match clip {
        Some(c) => c.intersection(&r).unwrap_or(Rect::new(r.x, r.y, 0.0, 0.0)),
        None => r,
    })
}

fn inside(clip: Option<Rect>, p: Point) -> bool {
    clip.is_none_or(|c| c.contains(p))
}

/// The clip applying to the content of `b`, inside `clip`.
fn inner_clip(b: &LayoutBox, clip: Option<Rect>) -> Option<Rect> {
    if clips(b) {
        intersect(clip, b.padding_box())
    } else {
        clip
    }
}

/// Document-order key of a layered box (see `xbro-paint`): its node index
/// and a rank (0 the element, 1 its `::before`, 2 its `::after`, keyed by
/// the last node of the element's subtree, 3 an anonymous box).
type DocOrder = (usize, u8);

/// A positioned layer or child stacking context of a stacking context.
struct Entry<'a> {
    b: &'a LayoutBox,
    /// Clip applying to the box.
    clip: Option<Rect>,
    /// Real stacking context (else a positioned `z-index: auto` box).
    real: bool,
    z: i32,
    order: DocOrder,
}

struct HitCx<'a> {
    root: &'a LayoutBox,
    /// Point in document coordinates.
    doc: Point,
    /// Point in viewport coordinates.
    view: Point,
    /// Last node index of each element's subtree (for `::after` ordering),
    /// computed on first use.
    ends: OnceCell<HashMap<usize, usize>>,
}

impl HitCx<'_> {
    fn point(&self, in_fixed: bool) -> Point {
        if in_fixed {
            self.view
        } else {
            self.doc
        }
    }

    fn doc_order(&self, c: &LayoutBox, parent: &LayoutBox) -> DocOrder {
        match (c.node, c.pseudo) {
            (Some(node), None) => (node.0, 0),
            (Some(node), Some(Pseudo::Before)) => (node.0, 1),
            (Some(node), Some(Pseudo::After)) => {
                let ends = self.ends.get_or_init(|| subtree_ends(self.root));
                let end = ends.get(&node.0).copied().unwrap_or(node.0);
                (end.max(node.0), 2)
            }
            (None, _) => (parent.node.map_or(0, |n| n.0), 3),
        }
    }
}

/// Last node index in the subtree of every element with a box.
fn subtree_ends(root: &LayoutBox) -> HashMap<usize, usize> {
    fn walk(b: &LayoutBox, ends: &mut HashMap<usize, usize>, depth: usize) -> usize {
        let mut max = b.node.map_or(0, |n| n.0);
        if depth > MAX_DEPTH {
            return max;
        }
        for c in &b.children {
            max = max.max(walk(c, ends, depth + 1));
        }
        for fragment in b.lines.iter().flat_map(|l| &l.fragments) {
            let end = match fragment {
                Fragment::Text(t) => t.node.0,
                Fragment::InlineBox(ib) => ib.node.0,
                Fragment::Atomic(a) => walk(a, ends, depth + 1),
            };
            max = max.max(end);
        }
        if let Some(ListMarker {
            content: MarkerContent::Text(t),
            ..
        }) = &b.marker
        {
            max = max.max(t.node.0);
        }
        if let (Some(node), None) = (b.node, b.pseudo) {
            let end = ends.entry(node.0).or_insert(0);
            *end = (*end).max(max);
        }
        max
    }
    let mut ends = HashMap::new();
    walk(root, &mut ends, 0);
    ends
}

pub(crate) fn hit_test(tree: &LayoutTree, point: Point, scroll: Point) -> Option<HitResult> {
    let root = tree.root.as_ref()?;
    if !(point.x.is_finite() && point.y.is_finite()) {
        return None;
    }
    let sx = if scroll.x.is_finite() { scroll.x } else { 0.0 };
    let sy = if scroll.y.is_finite() { scroll.y } else { 0.0 };
    let cx = HitCx {
        root,
        doc: point,
        view: Point::new(point.x - sx, point.y - sy),
        ends: OnceCell::new(),
    };
    hit_context(root, &cx, false, None, 0)
        .or_else(|| {
            // The canvas below/after the content belongs to the root element.
            let ds = tree.document_size;
            let on_canvas =
                point.x >= 0.0 && point.y >= 0.0 && point.x < ds.width && point.y < ds.height;
            on_canvas.then_some(root.node).flatten()
        })
        .map(|node| HitResult { node })
}

/// Hit tests a box creating a stacking context (or the root) whose
/// enclosing clip is `clip`.
fn hit_context(
    b: &LayoutBox,
    cx: &HitCx<'_>,
    in_fixed: bool,
    clip: Option<Rect>,
    depth: usize,
) -> Option<NodeId> {
    if depth > MAX_DEPTH {
        return None;
    }
    let entering_fixed = b.is_fixed() && !in_fixed;
    let in_fixed = in_fixed || b.is_fixed();
    // Fixed boxes escape the clips of their ancestors (other coordinate space).
    let clip = if entering_fixed { None } else { clip };
    let p = cx.point(in_fixed);
    let own = inner_clip(b, clip);
    let abs = if b.style.position.is_positioned() {
        own
    } else {
        clip
    };
    let mut negative = Vec::new();
    let mut zero = Vec::new();
    let mut positive = Vec::new();
    collect(
        b,
        cx,
        own,
        abs,
        &mut [&mut negative, &mut zero, &mut positive],
        depth + 1,
    );
    // Stable sorts: equal keys keep the box tree order.
    negative.sort_by_key(|e: &Entry<'_>| (e.z, e.order));
    zero.sort_by_key(|e: &Entry<'_>| e.order);
    positive.sort_by_key(|e: &Entry<'_>| (e.z, e.order));
    let entry_hit = |e: &Entry<'_>| {
        if e.real {
            hit_context(e.b, cx, in_fixed, e.clip, depth + 1)
        } else {
            hit_atomic(e.b, p, e.clip, depth + 1)
        }
    };
    if let Some(n) = positive
        .iter()
        .rev()
        .chain(zero.iter().rev())
        .find_map(entry_hit)
    {
        return Some(n);
    }
    if let Some(n) = hit_flow(b, p, own, depth + 1) {
        return Some(n);
    }
    if let Some(n) = negative.iter().rev().find_map(entry_hit) {
        return Some(n);
    }
    (visible(&b.style) && inside(clip, p) && b.border_box().contains(p))
        .then_some(b.node)
        .flatten()
}

/// Collects the positioned descendants and child stacking contexts of a
/// stacking context `b` (into `out`: negative, zero/auto and positive
/// `z-index`), with the clips that apply to them: all enclosing clips for
/// in-flow and relatively positioned boxes, those up to the containing
/// block for absolutely positioned boxes, none for fixed boxes.
fn collect<'a>(
    b: &'a LayoutBox,
    cx: &HitCx<'_>,
    clip: Option<Rect>,
    abs_clip: Option<Rect>,
    out: &mut [&mut Vec<Entry<'a>>; 3],
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let parent_flex = b.kind == BoxKind::Flex;
    let atomics = b
        .lines
        .iter()
        .flat_map(|l| &l.fragments)
        .filter_map(|f| match f {
            Fragment::Atomic(a) => Some(a),
            _ => None,
        });
    for c in b.children.iter().chain(atomics) {
        let c_clip = match c.style.position {
            Position::Absolute => abs_clip,
            Position::Fixed => None,
            _ => clip,
        };
        if creates_stacking_context(c, parent_flex) {
            let z = z_order(c, parent_flex);
            let entry = Entry {
                b: c,
                clip: c_clip,
                real: true,
                z,
                order: cx.doc_order(c, b),
            };
            let slot = match z {
                z if z < 0 => 0,
                0 => 1,
                _ => 2,
            };
            if let Some(v) = out.get_mut(slot) {
                v.push(entry);
            }
            continue;
        }
        let positioned = c.style.position.is_positioned();
        if positioned {
            if let Some(v) = out.get_mut(1) {
                v.push(Entry {
                    b: c,
                    clip: c_clip,
                    real: false,
                    z: 0,
                    order: cx.doc_order(c, b),
                });
            }
        }
        let inner = inner_clip(c, c_clip);
        let inner_abs = if positioned { inner } else { abs_clip };
        collect(c, cx, inner, inner_abs, out, depth + 1);
    }
}

/// Hit tests a box "atomically" (as if it created a stacking context,
/// its positioned descendants excluded): its in-flow content, then its own
/// border box.
fn hit_atomic(b: &LayoutBox, p: Point, clip: Option<Rect>, depth: usize) -> Option<NodeId> {
    if depth > MAX_DEPTH {
        return None;
    }
    if let Some(n) = hit_flow(b, p, inner_clip(b, clip), depth + 1) {
        return Some(n);
    }
    (visible(&b.style) && inside(clip, p) && b.border_box().contains(p))
        .then_some(b.node)
        .flatten()
}

/// The non-layered content of `b` (not its own box), in reverse paint
/// order: inline content (and flex items), then floats, then the
/// backgrounds of in-flow blocks.
fn hit_flow(b: &LayoutBox, p: Point, clip: Option<Rect>, depth: usize) -> Option<NodeId> {
    if depth > MAX_DEPTH || !inside(clip, p) {
        return None;
    }
    hit_inlines(b, p, clip, depth)
        .or_else(|| hit_floats(b, p, clip, depth))
        .or_else(|| hit_blocks(b, p, clip, depth))
}

/// Inline content of `b` and of its in-flow descendants, later first;
/// flex items atomically.
fn hit_inlines(b: &LayoutBox, p: Point, clip: Option<Rect>, depth: usize) -> Option<NodeId> {
    if depth > MAX_DEPTH {
        return None;
    }
    let flex = b.kind == BoxKind::Flex;
    for c in b.children.iter().rev() {
        if is_layered(c, flex) {
            continue;
        }
        if flex {
            if let Some(n) = hit_atomic(c, p, clip, depth + 1) {
                return Some(n);
            }
            continue;
        }
        if is_float(c) {
            continue;
        }
        if let Some(n) = hit_inlines(c, p, inner_clip(c, clip), depth + 1) {
            return Some(n);
        }
    }
    if !inside(clip, p) {
        return None;
    }
    for line in b.lines.iter().rev() {
        for f in line.fragments.iter().rev() {
            match f {
                Fragment::Text(t) => {
                    if visible(&t.style) && t.rect.contains(p) {
                        return Some(t.node);
                    }
                }
                Fragment::InlineBox(ib) => {
                    if visible(&ib.style) && ib.rect.contains(p) {
                        return Some(ib.node);
                    }
                }
                Fragment::Atomic(a) => {
                    if !is_layered(a, false) {
                        if let Some(n) = hit_atomic(a, p, clip, depth + 1) {
                            return Some(n);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Non-positioned floats among the in-flow descendants of `b`, later first.
fn hit_floats(b: &LayoutBox, p: Point, clip: Option<Rect>, depth: usize) -> Option<NodeId> {
    if depth > MAX_DEPTH || b.kind == BoxKind::Flex {
        return None;
    }
    for c in b.children.iter().rev() {
        if is_layered(c, false) {
            continue;
        }
        let n = if is_float(c) {
            hit_atomic(c, p, clip, depth + 1)
        } else {
            hit_floats(c, p, inner_clip(c, clip), depth + 1)
        };
        if n.is_some() {
            return n;
        }
    }
    None
}

/// Backgrounds of the in-flow block-level descendants of `b`, deepest and
/// later first.
fn hit_blocks(b: &LayoutBox, p: Point, clip: Option<Rect>, depth: usize) -> Option<NodeId> {
    if depth > MAX_DEPTH || b.kind == BoxKind::Flex {
        return None;
    }
    for c in b.children.iter().rev() {
        if is_layered(c, false) || is_float(c) {
            continue;
        }
        if let Some(n) = hit_blocks(c, p, inner_clip(c, clip), depth + 1) {
            return Some(n);
        }
        if visible(&c.style) && inside(clip, p) && c.border_box().contains(p) {
            if let Some(n) = c.node {
                return Some(n);
            }
        }
    }
    None
}

pub(crate) fn node_rect(tree: &LayoutTree, node: NodeId) -> Option<Rect> {
    find_rect(tree.root.as_ref()?, node, 0)
}

fn find_rect(b: &LayoutBox, node: NodeId, depth: usize) -> Option<Rect> {
    if depth > MAX_DEPTH {
        return None;
    }
    if b.node == Some(node) && b.pseudo.is_none() {
        return Some(b.border_box());
    }
    for line in &b.lines {
        for f in &line.fragments {
            match f {
                Fragment::Text(t) if t.node == node => return Some(t.rect),
                Fragment::InlineBox(ib) if ib.node == node => return Some(ib.rect),
                Fragment::Atomic(a) => {
                    if let Some(r) = find_rect(a, node, depth + 1) {
                        return Some(r);
                    }
                }
                _ => {}
            }
        }
    }
    b.children
        .iter()
        .find_map(|c| find_rect(c, node, depth + 1))
}

pub(crate) fn text_fragments(tree: &LayoutTree) -> Vec<&TextFragment> {
    let mut out = Vec::new();
    if let Some(root) = &tree.root {
        collect_text(root, &mut out, 0);
    }
    out
}

fn collect_text<'a>(b: &'a LayoutBox, out: &mut Vec<&'a TextFragment>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    // In-flow block children paint before the inline content of a box.
    for c in b.children.iter().filter(|c| !c.style.is_out_of_flow()) {
        collect_text(c, out, depth + 1);
    }
    for line in &b.lines {
        for f in &line.fragments {
            match f {
                Fragment::Text(t) => out.push(t),
                Fragment::Atomic(a) => collect_text(a, out, depth + 1),
                Fragment::InlineBox(_) => {}
            }
        }
    }
    for c in b.children.iter().filter(|c| c.style.is_out_of_flow()) {
        collect_text(c, out, depth + 1);
    }
}

/// The viewport size extended to the right/bottom extent of every box that
/// is not in a `position: fixed` subtree (clipped by `overflow`). An axis
/// in which the viewport's `overflow` (propagated from the root element
/// or `<body>`) is `hidden` or `clip` does not scroll: the document size is
/// the viewport's there.
pub(crate) fn document_size(
    root: &LayoutBox,
    viewport: Size,
    (overflow_x, overflow_y): (Overflow, Overflow),
) -> Size {
    let mut ext = (viewport.width, viewport.height);
    let m = root.margin_box();
    grow(&mut ext, m);
    extent(root, None, &mut ext, 0);
    let scrolls = |o: Overflow| !matches!(o, Overflow::Hidden | Overflow::Clip);
    if !scrolls(overflow_x) {
        ext.0 = viewport.width;
    }
    if !scrolls(overflow_y) {
        ext.1 = viewport.height;
    }
    Size::new(sane_size(ext.0), sane_size(ext.1))
}

fn grow(ext: &mut (f32, f32), r: Rect) {
    if r.width > 0.0 || r.height > 0.0 {
        ext.0 = ext.0.max(r.right());
        ext.1 = ext.1.max(r.bottom());
    }
}

fn clipped(r: Rect, clip: Option<Rect>) -> Option<Rect> {
    match clip {
        None => Some(r),
        Some(c) => {
            let x = r.x.max(c.x);
            let y = r.y.max(c.y);
            let right = r.right().min(c.right());
            let bottom = r.bottom().min(c.bottom());
            (right >= x && bottom >= y).then(|| Rect::new(x, y, right - x, bottom - y))
        }
    }
}

fn extent(b: &LayoutBox, clip: Option<Rect>, ext: &mut (f32, f32), depth: usize) {
    if depth > MAX_DEPTH || b.is_fixed() {
        return;
    }
    if let Some(r) = clipped(b.border_box(), clip) {
        grow(ext, r);
    }
    let inner = if clips(b) {
        intersect(clip, b.padding_box())
    } else {
        clip
    };
    for line in &b.lines {
        for f in &line.fragments {
            match f {
                Fragment::Text(t) => {
                    if let Some(r) = clipped(t.rect, inner) {
                        grow(ext, r);
                    }
                }
                Fragment::InlineBox(ib) => {
                    if let Some(r) = clipped(ib.rect, inner) {
                        grow(ext, r);
                    }
                }
                Fragment::Atomic(a) => extent(a, inner, ext, depth + 1),
            }
        }
    }
    for c in &b.children {
        extent(c, inner, ext, depth + 1);
    }
}
