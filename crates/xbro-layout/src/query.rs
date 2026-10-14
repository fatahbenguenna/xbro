//! Queries on a laid-out tree: hit testing, node rectangles, text
//! fragments and the scrollable document size.
//!
//! Hit testing walks boxes in reverse paint order (a simplified CSS 2.1
//! Appendix E): inside each layer (the root, positioned boxes and other
//! stacking contexts) positioned descendants with `z-index >= 0` come first
//! (highest `z-index` first, later boxes first), then the inline content,
//! floats and in-flow blocks of the layer (later first), then descendants
//! with negative `z-index`, then the layer's own box. `overflow` other than
//! `visible` clips descendants to the padding box; `visibility: hidden`
//! boxes and fragments are transparent to hits (their visible descendants
//! are not). `position: fixed` subtrees are tested in viewport coordinates.

use xbro_dom::NodeId;
use xbro_style::{Float, Visibility};

use crate::geom::{Point, Rect, Size};
use crate::sizing::sane_size;
use crate::{BoxKind, Fragment, HitResult, LayoutBox, LayoutTree, TextFragment};

/// Recursion guard (the DOM depth is bounded by the parser; anonymous boxes
/// add a few levels).
const MAX_DEPTH: usize = 2048;

fn visible(style: &xbro_style::ComputedStyle) -> bool {
    style.visibility == Visibility::Visible
}

fn clips(b: &LayoutBox) -> bool {
    b.style.overflow_x.clips() || b.style.overflow_y.clips()
}

fn opacity_below_one(b: &LayoutBox) -> bool {
    b.style.opacity < 1.0
}

/// Painted in a layer of its own (positioned, or a stacking context).
fn is_layered(b: &LayoutBox, parent_flex: bool) -> bool {
    b.style.position.is_positioned()
        || opacity_below_one(b)
        || (parent_flex && b.style.z_index.is_some())
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

/// A layered descendant found while walking a layer's flow content.
struct Layer<'a> {
    b: &'a LayoutBox,
    z: i32,
    clip: Option<Rect>,
}

struct HitCx {
    /// Point in document coordinates.
    doc: Point,
    /// Point in viewport coordinates.
    view: Point,
}

pub(crate) fn hit_test(tree: &LayoutTree, point: Point, scroll: Point) -> Option<HitResult> {
    let root = tree.root.as_ref()?;
    if !(point.x.is_finite() && point.y.is_finite()) {
        return None;
    }
    let sx = if scroll.x.is_finite() { scroll.x } else { 0.0 };
    let sy = if scroll.y.is_finite() { scroll.y } else { 0.0 };
    let cx = HitCx {
        doc: point,
        view: Point::new(point.x - sx, point.y - sy),
    };
    hit_layer(root, &cx, false, None, 0)
        .or_else(|| {
            // The canvas below/after the content belongs to the root element.
            let ds = tree.document_size;
            let on_canvas =
                point.x >= 0.0 && point.y >= 0.0 && point.x < ds.width && point.y < ds.height;
            on_canvas.then_some(root.node).flatten()
        })
        .map(|node| HitResult { node })
}

/// Hit tests a layer rooted at `b`.
fn hit_layer(
    b: &LayoutBox,
    cx: &HitCx,
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
    let p = if in_fixed { cx.view } else { cx.doc };
    if !inside(clip, p) {
        return None;
    }
    let inner = if clips(b) {
        intersect(clip, b.padding_box())
    } else {
        clip
    };
    let mut layers = Vec::new();
    collect_layers(b, inner, &mut layers, depth + 1);
    // Stable sort: tree order kept within a z-index.
    layers.sort_by_key(|l| l.z);
    for l in layers.iter().rev().filter(|l| l.z >= 0) {
        if let Some(n) = hit_layer(l.b, cx, in_fixed, l.clip, depth + 1) {
            return Some(n);
        }
    }
    if let Some(n) = hit_flow_content(b, p, inner, depth + 1) {
        return Some(n);
    }
    for l in layers.iter().rev().filter(|l| l.z < 0) {
        if let Some(n) = hit_layer(l.b, cx, in_fixed, l.clip, depth + 1) {
            return Some(n);
        }
    }
    if visible(&b.style) && b.border_box().contains(p) {
        return b.node;
    }
    None
}

/// Collects the layered descendants of `b` reachable without crossing
/// another layer, with the clip that applies to them.
fn collect_layers<'a>(
    b: &'a LayoutBox,
    clip: Option<Rect>,
    out: &mut Vec<Layer<'a>>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let flex = b.kind == BoxKind::Flex;
    let visit = |c: &'a LayoutBox, parent_flex: bool, out: &mut Vec<Layer<'a>>| {
        if is_layered(c, parent_flex) {
            out.push(Layer {
                b: c,
                z: z_order(c, parent_flex),
                clip,
            });
        } else {
            let inner = if clips(c) {
                intersect(clip, c.padding_box())
            } else {
                clip
            };
            collect_layers(c, inner, out, depth + 1);
        }
    };
    for line in &b.lines {
        for f in &line.fragments {
            if let Fragment::Atomic(a) = f {
                visit(a, false, out);
            }
        }
    }
    for c in &b.children {
        visit(c, flex, out);
    }
}

/// Hit tests the non-layered content of `b` (not its own box).
fn hit_flow_content(b: &LayoutBox, p: Point, clip: Option<Rect>, depth: usize) -> Option<NodeId> {
    if depth > MAX_DEPTH || !inside(clip, p) {
        return None;
    }
    let flex = b.kind == BoxKind::Flex;
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
                        if let Some(n) = hit_flow_box(a, p, clip, depth + 1) {
                            return Some(n);
                        }
                    }
                }
            }
        }
    }
    let flow_children = |floats: bool| {
        b.children
            .iter()
            .rev()
            .filter(move |c| is_float(c) == floats && !is_layered(c, flex))
    };
    for c in flow_children(true).chain(flow_children(false)) {
        if let Some(n) = hit_flow_box(c, p, clip, depth + 1) {
            return Some(n);
        }
    }
    None
}

/// Hit tests a non-layered box: its content, then its own border box.
fn hit_flow_box(b: &LayoutBox, p: Point, clip: Option<Rect>, depth: usize) -> Option<NodeId> {
    let inner = if clips(b) {
        intersect(clip, b.padding_box())
    } else {
        clip
    };
    if let Some(n) = hit_flow_content(b, p, inner, depth + 1) {
        return Some(n);
    }
    if visible(&b.style) && inside(clip, p) && b.border_box().contains(p) {
        return b.node;
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
/// is not in a `position: fixed` subtree (clipped by `overflow`).
pub(crate) fn document_size(root: &LayoutBox, viewport: Size) -> Size {
    let mut ext = (viewport.width, viewport.height);
    let m = root.margin_box();
    grow(&mut ext, m);
    extent(root, None, &mut ext, 0);
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
