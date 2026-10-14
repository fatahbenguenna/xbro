//! Block formatting (CSS 2.1 §9.4.1, §10.3.3, §10.6.3, §8.3.1).
//!
//! Block-level boxes in normal flow are laid out by
//! [`LayoutCx::layout_flow_child`], which threads a [`Flow`] (current y,
//! pending collapsible margins and the float context of the enclosing block
//! formatting context) through the tree:
//!
//! * a box's top margin is added to the pending margins; it collapses with
//!   its first child's when the box has no top border/padding, no clearance
//!   and does not establish a BFC: the box's top is then wherever its first
//!   in-flow content resolves the pending margins;
//! * a box with `height: auto`, no bottom border/padding passes its last
//!   child's bottom margin through to its own bottom margin;
//! * empty boxes collapse through (their margins join the pending ones).
//!
//! Boxes that establish an independent formatting context (root, floats,
//! absolutely positioned boxes, inline-blocks, `overflow != visible`, flex
//! and table boxes, replaced elements) are laid out by
//! [`LayoutCx::layout_independent`] with a fresh float context; their auto
//! height includes their floats. In flow they are narrowed to avoid floats.

use xbro_style::{Clear, Display, Float, Overflow};

use crate::box_tree::{BoxNode, Kind, Replaced};
use crate::cx::LayoutCx;
use crate::float::{FloatContext, Side};
use crate::geom::{Edges, Point, Rect};
use crate::inline::{layout_ifc, AtomicBox};
use crate::sizing::{
    block_width, box_edges, clamp_size, height_limits, sane_size, shrink_to_fit,
    specified_height, specified_width, width_limits, BoxEdges, EPSILON,
};
use crate::{flex, table, BoxKind, LayoutBox, LineBox};

/// Containing block of a box: x and width of its content box (always
/// definite) and its height when definite.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Cb {
    pub x: f32,
    pub width: f32,
    pub height: Option<f32>,
}

/// Collapsible margins adjoining so far (CSS 2.1 §8.3.1).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct Margins {
    /// Largest positive margin.
    pos: f32,
    /// Most negative margin (≤ 0).
    neg: f32,
}

impl Margins {
    pub fn add(&mut self, m: f32) {
        if m > 0.0 {
            self.pos = self.pos.max(m);
        } else {
            self.neg = self.neg.min(m);
        }
    }

    /// Collapsed value: max positive + min negative.
    pub fn resolve(&self) -> f32 {
        self.pos + self.neg
    }
}

/// State of a block formatting context while laying out its in-flow boxes.
pub(crate) struct Flow<'f> {
    pub floats: &'f mut FloatContext,
    /// Bottom of the last in-flow content (before the pending margins).
    pub y: f32,
    pub margins: Margins,
}

impl Flow<'_> {
    /// Resolves the pending margins: the current y becomes final.
    pub fn resolve(&mut self) -> f32 {
        self.y += self.margins.resolve();
        self.margins = Margins::default();
        self.y
    }

    /// Where content would start if the pending margins were resolved now.
    pub fn hypothetical(&self) -> f32 {
        self.y + self.margins.resolve()
    }
}

/// Laid-out contents of a box (children and/or line boxes).
#[derive(Debug, Default)]
pub(crate) struct ContentLayout {
    pub children: Vec<LayoutBox>,
    pub lines: Vec<LineBox>,
    /// Auto content height.
    pub height: f32,
}

/// A float laid out but not yet placed.
#[derive(Debug)]
pub(crate) struct LaidFloat {
    pub lb: LayoutBox,
    /// Pending absolutely positioned boxes registered during its layout.
    pub pending: (usize, usize),
    pub side: Side,
    pub clear: Clear,
    /// Margin box origin used for the tentative layout.
    pub origin: Point,
    /// Margin box size.
    pub width: f32,
    pub height: f32,
}

impl LaidFloat {
    /// Width of the margin box.
    pub fn outer_width(&self) -> f32 {
        self.width
    }
}

/// How a BFC root in flow is sized horizontally.
#[derive(Copy, Clone, Debug)]
enum WidthMode {
    /// Fills the available space (block rules).
    Fill,
    /// Fixed content width.
    Fixed(f32),
    /// Shrink-to-fit between min- and max-content (auto-width tables).
    Shrink(f32, f32),
}

/// True when the box establishes an independent formatting context (or is
/// a replaced element), so it never collapses margins with its children and
/// avoids floats.
pub(crate) fn establishes_bfc(b: &BoxNode) -> bool {
    let s = &b.style;
    matches!(
        b.kind,
        Kind::Flex | Kind::Table | Kind::Replaced(_) | Kind::TableCell
    ) || s.overflow_x != Overflow::Visible
        || s.overflow_y != Overflow::Visible
        || s.float != Float::None
        || s.position.is_out_of_flow()
        || b.inline_level
        || matches!(s.display, Display::TableCaption)
}

/// The [`BoxKind`] of a laid-out box.
pub(crate) fn layout_kind(cx: &LayoutCx<'_>, b: &BoxNode) -> BoxKind {
    match &b.kind {
        Kind::Flex => BoxKind::Flex,
        Kind::Table => BoxKind::Table,
        Kind::TableRowGroup => BoxKind::TableRowGroup,
        Kind::TableRow => BoxKind::TableRow,
        Kind::TableCell => BoxKind::TableCell,
        Kind::Replaced(r) => match r {
            Replaced::Image | Replaced::Svg | Replaced::UrlImage(_) => BoxKind::Image,
            Replaced::Media { .. } => {
                let has_image = b.node.and_then(|n| cx.replaced.node_size(n)).is_some();
                if has_image {
                    BoxKind::Image
                } else {
                    BoxKind::Placeholder
                }
            }
            Replaced::Control(c) => BoxKind::Control(c.kind.clone()),
        },
        _ => BoxKind::Block,
    }
}

impl<'t> LayoutCx<'t> {
    /// Lays out the root element's box against the initial containing block.
    pub fn layout_root(&mut self, root: &'t BoxNode) -> LayoutBox {
        let vw = self.viewport.width;
        let vh = self.viewport.height;
        let cb = Cb {
            x: 0.0,
            width: vw,
            height: Some(vh),
        };
        let mut edges = box_edges(&root.style, vw);
        let (content_w, forced_h) = if root.is_replaced() {
            let (w, h) = self.replaced_size(root, &edges, cb);
            (w, h)
        } else {
            let spec = specified_width(&root.style, Some(vw), edges.pb_h());
            let (w, ml, mr) = block_width(&root.style, &edges, vw, vw, spec);
            edges.margin.left = ml;
            edges.margin.right = mr;
            (w, None)
        };
        let mut lb = self.layout_independent(
            root,
            edges.margin.left,
            edges.margin.top,
            content_w,
            &edges,
            cb,
            forced_h,
        );
        self.apply_relative(&mut lb, cb);
        // Boxes positioned against the initial containing block or the viewport.
        let icb = Rect::new(0.0, 0.0, vw, vh);
        for _ in 0..64 {
            let pending = self.take_pending(|_| true);
            if pending.is_empty() {
                break;
            }
            for p in pending {
                let abs = self.layout_abspos(p, icb);
                lb.children.push(abs);
            }
        }
        lb
    }

    /// Lays out an in-flow block-level box. Returns the box (relative offset
    /// applied) and the y of its border box top before that offset, or
    /// `None` when the box collapsed through (empty, margins joined the flow).
    pub fn layout_flow_child(
        &mut self,
        b: &'t BoxNode,
        cb: Cb,
        flow: &mut Flow<'_>,
    ) -> (LayoutBox, Option<f32>) {
        let style = &b.style;
        let mut edges = box_edges(style, cb.width);
        if establishes_bfc(b) {
            let (mut lb, top) = self.layout_flow_bfc(b, cb, flow, &edges);
            self.apply_relative(&mut lb, cb);
            return (lb, Some(top));
        }
        let spec_w = specified_width(style, Some(cb.width), edges.pb_h());
        let (content_w, ml, mr) = block_width(style, &edges, cb.width, cb.width, spec_w);
        edges.margin.left = ml;
        edges.margin.right = mr;

        // Clearance.
        let mut cleared = false;
        if style.clear != Clear::None {
            if let Some(cy) = flow.floats.clear_y(style.clear) {
                let mut m = flow.margins;
                m.add(edges.margin.top);
                if flow.y + m.resolve() < cy {
                    flow.y = cy;
                    flow.margins = Margins::default();
                    cleared = true;
                }
            }
        }
        let collapse_top = !cleared && edges.padding.top == 0.0 && edges.border.top == 0.0;
        let border_top = if cleared {
            Some(flow.y)
        } else {
            flow.margins.add(edges.margin.top);
            if collapse_top {
                None
            } else {
                Some(flow.resolve())
            }
        };

        let pb_v = edges.pb_v();
        let (min_h, max_h) = height_limits(style, cb.height, pb_v);
        let spec_h = specified_height(style, cb.height, pb_v).map(|h| clamp_size(h, min_h, max_h));
        let content_x = cb.x + ml + edges.border.left + edges.padding.left;
        if let Some(bt) = border_top {
            flow.y = bt + edges.border.top + edges.padding.top;
        }
        let child_cb = Cb {
            x: content_x,
            width: content_w,
            height: spec_h,
        };
        let frame = style.position.is_positioned().then(|| self.push_frame());
        let mut children = Vec::new();
        let mut lines = Vec::new();
        let first_top = if b.inline_content {
            let hyp = flow.hypothetical();
            let out = layout_ifc(self, b, child_cb, hyp, flow.floats);
            children = out.floats;
            let min_line = self.empty_marker_height(b);
            if !out.lines.is_empty() || min_line > 0.0 {
                flow.y = hyp + out.height.max(min_line);
                flow.margins = Margins::default();
                lines = out.lines;
                Some(hyp)
            } else {
                None
            }
        } else {
            self.layout_block_children(b, child_cb, flow, &mut children)
        };

        let collapse_bottom =
            spec_h.is_none() && edges.padding.bottom == 0.0 && edges.border.bottom == 0.0;
        let content_top = match border_top {
            Some(bt) => bt + edges.border.top + edges.padding.top,
            None => match first_top {
                Some(t) => t,
                None if collapse_bottom && min_h <= 0.0 => {
                    // Empty box: collapses through.
                    let hyp = flow.hypothetical();
                    flow.margins.add(edges.margin.bottom);
                    let mut lb = self.new_box(
                        b,
                        Rect::new(content_x, hyp, content_w, 0.0),
                        &edges,
                        children,
                        lines,
                    );
                    self.finish_box(b, &mut lb, frame);
                    self.apply_relative(&mut lb, cb);
                    return (lb, None);
                }
                None => flow.resolve(),
            },
        };
        let auto_bottom = if collapse_bottom {
            flow.y
        } else {
            flow.hypothetical()
        };
        let auto_h = sane_size(auto_bottom - content_top);
        let h = spec_h.unwrap_or_else(|| clamp_size(auto_h, min_h, max_h));
        if !collapse_bottom || (h - auto_h).abs() > EPSILON {
            flow.margins = Margins::default();
        }
        flow.y = content_top + h + edges.padding.bottom + edges.border.bottom;
        flow.margins.add(edges.margin.bottom);
        let border_top_y = content_top - edges.border.top - edges.padding.top;
        let mut lb = self.new_box(
            b,
            Rect::new(content_x, content_top, content_w, h),
            &edges,
            children,
            lines,
        );
        self.finish_box(b, &mut lb, frame);
        self.apply_relative(&mut lb, cb);
        (lb, Some(border_top_y))
    }

    /// Lays out the children of a block container whose children are
    /// block-level. Returns the border-box top of the first child that did
    /// not collapse through.
    pub fn layout_block_children(
        &mut self,
        b: &'t BoxNode,
        cb: Cb,
        flow: &mut Flow<'_>,
        out: &mut Vec<LayoutBox>,
    ) -> Option<f32> {
        let mut first_top = None;
        for c in &b.children {
            if c.is_float() {
                let laid = self.prepare_float(c, cb);
                let min_y = flow.hypothetical();
                let lb = self.place_float(laid, flow.floats, min_y, cb);
                out.push(lb);
            } else if c.is_absolute() {
                let y = flow.hypothetical();
                self.register_abs(c, Point::new(cb.x, y));
            } else if c.is_text_like() || matches!(c.kind, Kind::Inline) {
                // Box construction wraps inline content in anonymous blocks.
                continue;
            } else {
                let (lb, top) = self.layout_flow_child(c, cb, flow);
                if first_top.is_none() {
                    first_top = top;
                }
                out.push(lb);
            }
        }
        first_top
    }

    /// In-flow box establishing a BFC (or replaced): margins resolved before
    /// it, narrowed or moved down to avoid floats.
    fn layout_flow_bfc(
        &mut self,
        b: &'t BoxNode,
        cb: Cb,
        flow: &mut Flow<'_>,
        edges: &BoxEdges,
    ) -> (LayoutBox, f32) {
        let style = &b.style;
        flow.margins.add(edges.margin.top);
        let mut y = flow.hypothetical();
        if style.clear != Clear::None {
            if let Some(cy) = flow.floats.clear_y(style.clear) {
                y = y.max(cy);
            }
        }
        let pb_h = edges.pb_h();
        let (min_w, max_w) = width_limits(style, Some(cb.width), pb_h);
        let mut forced_h = None;
        let mode = match &b.kind {
            Kind::Replaced(_) => {
                let (w, h) = self.replaced_size(b, edges, cb);
                forced_h = h;
                WidthMode::Fixed(w)
            }
            Kind::Table => {
                let i = self.content_intrinsic(b);
                match specified_width(style, Some(cb.width), pb_h) {
                    Some(w) => WidthMode::Fixed(clamp_size(w.max(i.min), min_w, max_w)),
                    None => WidthMode::Shrink(i.min, i.max),
                }
            }
            _ => match specified_width(style, Some(cb.width), pb_h) {
                Some(w) => WidthMode::Fixed(clamp_size(w, min_w, max_w)),
                None => WidthMode::Fill,
            },
        };
        let est_h = forced_h
            .or_else(|| specified_height(style, cb.height, edges.pb_v()))
            .map_or(1.0, |h| h + edges.pb_v());
        let mut used = *edges;
        let mut content_w;
        let mut x;
        let mut guard = 0;
        loop {
            guard += 1;
            let narrowed = flow.floats.narrows(y, est_h, cb.x, cb.x + cb.width);
            if !narrowed {
                let spec = match mode {
                    WidthMode::Fill => None,
                    WidthMode::Fixed(w) => Some(w),
                    WidthMode::Shrink(lo, hi) => Some(clamp_size(
                        shrink_to_fit(lo, hi, cb.width - edges.margin.horizontal() - pb_h),
                        min_w,
                        max_w,
                    )),
                };
                let (w, ml, mr) = block_width(style, edges, cb.width, cb.width, spec);
                content_w = w;
                used.margin.left = ml;
                used.margin.right = mr;
                x = cb.x + ml;
                break;
            }
            let (l, r) = flow.floats.available(y, est_h, cb.x, cb.x + cb.width);
            let left_edge = (cb.x + edges.margin.left).max(l);
            let right_edge = (cb.x + cb.width - edges.margin.right).min(r);
            let space = right_edge - left_edge;
            let (w, fits) = match mode {
                WidthMode::Fill => {
                    let w = clamp_size(space - pb_h, min_w, max_w);
                    (w, true)
                }
                WidthMode::Fixed(w) => (w, w + pb_h <= space + EPSILON),
                WidthMode::Shrink(lo, hi) => {
                    let w = clamp_size(shrink_to_fit(lo, hi, space - pb_h), min_w, max_w);
                    (w, w + pb_h <= space + EPSILON)
                }
            };
            let next = if fits {
                None
            } else {
                flow.floats.next_bottom(y, est_h)
            };
            match next {
                Some(ny) if guard < 10_000 => y = ny,
                _ => {
                    content_w = w;
                    let slack = sane_size(space - w - pb_h);
                    x = match (edges.auto_left, edges.auto_right) {
                        (true, true) => left_edge + slack / 2.0,
                        (true, false) => left_edge + slack,
                        _ => left_edge,
                    };
                    used.margin.left = x - cb.x;
                    used.margin.right = sane_size(cb.x + cb.width - (x + w + pb_h));
                    if !edges.auto_left {
                        used.margin.left = edges.margin.left;
                    }
                    if !edges.auto_right {
                        used.margin.right = edges.margin.right;
                    }
                    break;
                }
            }
        }
        flow.y = y;
        flow.margins = Margins::default();
        let lb = self.layout_independent(b, x, y, content_w, &used, cb, forced_h);
        flow.y = lb.border_box().bottom();
        flow.margins.add(edges.margin.bottom);
        (lb, y)
    }

    /// Lays out a box that establishes an independent formatting context
    /// with its border box at `(bx, by)` and the given content width.
    /// `forced_h` overrides the content height (replaced boxes, stretched
    /// boxes). The relative offset is not applied.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_independent(
        &mut self,
        b: &'t BoxNode,
        bx: f32,
        by: f32,
        content_w: f32,
        edges: &BoxEdges,
        cb: Cb,
        forced_h: Option<f32>,
    ) -> LayoutBox {
        let style = &b.style;
        let content_x = bx + edges.border.left + edges.padding.left;
        let content_y = by + edges.border.top + edges.padding.top;
        let content_w = sane_size(content_w);
        let pb_v = edges.pb_v();
        let (min_h, max_h) = height_limits(style, cb.height, pb_v);
        let spec_h = match forced_h {
            Some(h) => Some(sane_size(h)),
            None => specified_height(style, cb.height, pb_v).map(|h| clamp_size(h, min_h, max_h)),
        };
        let frame = style.position.is_positioned().then(|| self.push_frame());
        let content = self.independent_contents(b, content_x, content_y, content_w, spec_h);
        let h = spec_h.unwrap_or_else(|| clamp_size(content.height, min_h, max_h));
        let mut lb = self.new_box(
            b,
            Rect::new(content_x, content_y, content_w, h),
            edges,
            content.children,
            content.lines,
        );
        if let Kind::Replaced(Replaced::Control(c)) = &b.kind {
            if c.kind != crate::ControlKind::TextArea {
                // Single-line controls center their text vertically.
                let lines_h = content.height;
                let dy = (h - lines_h) / 2.0;
                if dy.abs() > EPSILON && !lb.lines.is_empty() {
                    let lines = std::mem::take(&mut lb.lines);
                    let mut tmp = LayoutBox {
                        lines,
                        children: Vec::new(),
                        marker: None,
                        ..lb.clone_shallow()
                    };
                    crate::tree::translate_box(&mut tmp, 0.0, dy);
                    tmp.content = lb.content;
                    lb.lines = tmp.lines;
                }
            }
        }
        self.finish_box(b, &mut lb, frame);
        lb
    }

    /// Lays out the contents of an independent formatting context root.
    fn independent_contents(
        &mut self,
        b: &'t BoxNode,
        x: f32,
        y: f32,
        w: f32,
        spec_h: Option<f32>,
    ) -> ContentLayout {
        let cb = Cb {
            x,
            width: w,
            height: spec_h,
        };
        match &b.kind {
            Kind::Flex => flex::layout_flex_contents(self, b, cb, y),
            Kind::Table => table::layout_table_contents(self, b, cb, y),
            Kind::Inline | Kind::Text(_) | Kind::LineBreak | Kind::WordBreak => {
                ContentLayout::default()
            }
            Kind::Replaced(r) => {
                let show_text = match r {
                    Replaced::Image => b
                        .node
                        .and_then(|n| self.replaced.node_size(n))
                        .is_none(),
                    Replaced::Control(_) => true,
                    _ => false,
                };
                if show_text && b.inline_content && !b.children.is_empty() {
                    let mut floats = FloatContext::new();
                    let out = layout_ifc(self, b, cb, y, &mut floats);
                    ContentLayout {
                        children: out.floats,
                        lines: out.lines,
                        height: out.height,
                    }
                } else {
                    ContentLayout::default()
                }
            }
            Kind::Block | Kind::TableCell | Kind::TableRowGroup | Kind::TableRow => {
                let mut floats = FloatContext::new();
                if b.inline_content {
                    let out = layout_ifc(self, b, cb, y, &mut floats);
                    let mut height = out.height.max(self.empty_marker_height(b));
                    if let Some(fb) = floats.bottom() {
                        height = height.max(fb - y);
                    }
                    ContentLayout {
                        children: out.floats,
                        lines: out.lines,
                        height: sane_size(height),
                    }
                } else {
                    let mut flow = Flow {
                        floats: &mut floats,
                        y,
                        margins: Margins::default(),
                    };
                    let mut children = Vec::new();
                    self.layout_block_children(b, cb, &mut flow, &mut children);
                    let bottom = flow.hypothetical();
                    let mut height = bottom - y;
                    if let Some(fb) = floats.bottom() {
                        height = height.max(fb - y);
                    }
                    ContentLayout {
                        children,
                        lines: Vec::new(),
                        height: sane_size(height),
                    }
                }
            }
        }
    }

    /// Lays out `b` as an independent formatting context root with its margin
    /// box's top-left corner at `(x, y)` — the entry point used by flex and
    /// table layout to lay out an item or a cell.
    ///
    /// `width` is the used content-box width; `None` applies the block width
    /// rules against `cb.width` (auto fills, specified widths and auto
    /// margins are honoured; replaced elements use their used size).
    /// `height` forces the content height (stretching). The relative offset
    /// is not applied (see [`LayoutCx::apply_relative`]).
    pub fn layout_box_at(
        &mut self,
        b: &'t BoxNode,
        x: f32,
        y: f32,
        width: Option<f32>,
        height: Option<f32>,
        cb: Cb,
    ) -> LayoutBox {
        let mut edges = box_edges(&b.style, cb.width);
        let mut forced_h = height;
        let content_w = match width {
            Some(w) => w,
            None => {
                if b.is_replaced() {
                    let (w, h) = self.replaced_size(b, &edges, cb);
                    if forced_h.is_none() {
                        forced_h = h;
                    }
                    w
                } else if matches!(b.kind, Kind::Table) {
                    self.shrink_width(b, &edges, cb).0
                } else {
                    let spec = specified_width(&b.style, Some(cb.width), edges.pb_h());
                    let avail = sane_size(cb.x + cb.width - x);
                    let (w, ml, mr) = block_width(&b.style, &edges, cb.width, avail, spec);
                    edges.margin.left = ml;
                    edges.margin.right = mr;
                    w
                }
            }
        };
        self.layout_independent(
            b,
            x + edges.margin.left,
            y + edges.margin.top,
            content_w,
            &edges,
            cb,
            forced_h,
        )
    }

    /// Used content width (and height for replaced boxes) of a float,
    /// inline-block or other shrink-to-fit box (CSS 2.1 §10.3.5–§10.3.9).
    pub fn shrink_width(&mut self, b: &'t BoxNode, edges: &BoxEdges, cb: Cb) -> (f32, Option<f32>) {
        let style = &b.style;
        let pb_h = edges.pb_h();
        let (min_w, max_w) = width_limits(style, Some(cb.width), pb_h);
        let avail = cb.width - edges.margin.left - edges.margin.right - pb_h;
        match &b.kind {
            Kind::Replaced(_) => self.replaced_size(b, edges, cb),
            Kind::Table => {
                let i = self.content_intrinsic(b);
                let w = match specified_width(style, Some(cb.width), pb_h) {
                    Some(w) => w.max(i.min),
                    None => shrink_to_fit(i.min, i.max, avail),
                };
                (clamp_size(w, min_w, max_w), None)
            }
            _ => match specified_width(style, Some(cb.width), pb_h) {
                Some(w) => (clamp_size(w, min_w, max_w), None),
                None => {
                    let i = self.content_intrinsic(b);
                    (clamp_size(shrink_to_fit(i.min, i.max, avail), min_w, max_w), None)
                }
            },
        }
    }

    /// Lays out an atomic inline-level box with its margin box at the origin.
    pub fn layout_atomic(&mut self, b: &'t BoxNode, cb: Cb) -> AtomicBox {
        let edges = box_edges(&b.style, cb.width);
        let (content_w, forced_h) = self.shrink_width(b, &edges, cb);
        let start = self.pending_mark();
        let lb = self.layout_independent(
            b,
            edges.margin.left,
            edges.margin.top,
            content_w,
            &edges,
            cb,
            forced_h,
        );
        let end = self.pending_mark();
        let border = lb.border_box();
        let width = sane_size(border.width + edges.margin.left + edges.margin.right);
        let height = sane_size(border.height + edges.margin.top + edges.margin.bottom);
        let clips = b.style.overflow_x.clips() || b.style.overflow_y.clips();
        let baseline = match &b.kind {
            Kind::Replaced(Replaced::Control(_)) => {
                crate::tree::first_baseline(&lb).unwrap_or(height)
            }
            Kind::Replaced(_) => match lb.lines.first() {
                // Alt text: baseline of its text.
                Some(l) => l.baseline,
                None => height,
            },
            Kind::Flex | Kind::Table => crate::tree::first_baseline(&lb).unwrap_or(height),
            _ if clips => height,
            _ => crate::tree::last_baseline(&lb).unwrap_or(height),
        };
        AtomicBox {
            lb,
            width,
            height,
            baseline: baseline.clamp(-crate::sizing::MAX_LEN, crate::sizing::MAX_LEN),
            pending: (start, end),
        }
    }

    /// Lays out a float at a tentative position (left edge of `cb`, y = 0).
    pub fn prepare_float(&mut self, b: &'t BoxNode, cb: Cb) -> LaidFloat {
        let edges = box_edges(&b.style, cb.width);
        let (content_w, forced_h) = self.shrink_width(b, &edges, cb);
        let start = self.pending_mark();
        let origin = Point::new(cb.x, 0.0);
        let lb = self.layout_independent(
            b,
            origin.x + edges.margin.left,
            origin.y + edges.margin.top,
            content_w,
            &edges,
            cb,
            forced_h,
        );
        let end = self.pending_mark();
        let border = lb.border_box();
        LaidFloat {
            width: sane_size(border.width + edges.margin.left + edges.margin.right),
            height: sane_size(border.height + edges.margin.top + edges.margin.bottom),
            lb,
            pending: (start, end),
            side: if b.style.float == Float::Right {
                Side::Right
            } else {
                Side::Left
            },
            clear: b.style.clear,
            origin,
        }
    }

    /// Places a prepared float (not above `min_y`) and moves it there.
    pub fn place_float(
        &mut self,
        f: LaidFloat,
        floats: &mut FloatContext,
        min_y: f32,
        cb: Cb,
    ) -> LayoutBox {
        let mut min_y = min_y;
        if f.clear != Clear::None {
            if let Some(cy) = floats.clear_y(f.clear) {
                min_y = min_y.max(cy);
            }
        }
        let p = floats.place(f.side, f.width, f.height, min_y, cb.x, cb.x + cb.width);
        let (dx, dy) = (p.x - f.origin.x, p.y - f.origin.y);
        let mut lb = f.lb;
        crate::tree::translate_box(&mut lb, dx, dy);
        self.translate_pending_range(f.pending.0, f.pending.1, dx, dy);
        self.apply_relative(&mut lb, cb);
        lb
    }

    /// Height of the single line an empty list item with an outside marker gets.
    fn empty_marker_height(&mut self, b: &BoxNode) -> f32 {
        let s = &b.style;
        if s.display != Display::ListItem
            || s.list_style_position != xbro_style::ListStylePosition::Outside
            || (s.list_style_type == xbro_style::ListStyleType::None && s.list_style_image.is_none())
            || !b.children.iter().all(|c| c.is_out_of_flow() || is_ws_text(c))
        {
            return 0.0;
        }
        self.font(s).line_height
    }

    /// Creates a [`LayoutBox`] for `b`.
    pub fn new_box(
        &self,
        b: &BoxNode,
        content: Rect,
        edges: &BoxEdges,
        children: Vec<LayoutBox>,
        lines: Vec<LineBox>,
    ) -> LayoutBox {
        LayoutBox {
            node: b.node.filter(|_| !matches!(b.kind, Kind::Replaced(Replaced::UrlImage(_)))),
            pseudo: b.pseudo,
            style: b.style.clone(),
            kind: layout_kind(self, b),
            content,
            padding: edges.padding,
            border: edges.border,
            margin: edges.margin,
            children,
            lines,
            marker: None,
        }
    }

    /// Completes a box: lays out the absolutely positioned boxes waiting for
    /// it (when positioned) and its list marker.
    pub fn finish_box(&mut self, b: &BoxNode, lb: &mut LayoutBox, frame: Option<u32>) {
        if let Some(f) = frame {
            let pending = self.pop_frame(f);
            let pad = lb.padding_box();
            for p in pending {
                let abs = self.layout_abspos(p, pad);
                lb.children.push(abs);
            }
        }
        if b.style.display == Display::ListItem {
            lb.marker = self.list_marker(b, lb);
        }
    }

    /// Moves the static positions of pending boxes `start..end`.
    pub fn translate_pending_range(&mut self, start: usize, end: usize, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        for p in self
            .pending_abs
            .iter_mut()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            p.static_pos.x += dx;
            p.static_pos.y += dy;
        }
    }
}

/// Collapsible-whitespace-only text box.
fn is_ws_text(b: &BoxNode) -> bool {
    match &b.kind {
        Kind::Text(t) => t.chars().all(crate::text::is_collapsible_space),
        _ => false,
    }
}

impl LayoutBox {
    /// Copy of the box without children, lines or marker.
    pub(crate) fn clone_shallow(&self) -> LayoutBox {
        LayoutBox {
            node: self.node,
            pseudo: self.pseudo,
            style: self.style.clone(),
            kind: self.kind.clone(),
            content: self.content,
            padding: self.padding,
            border: self.border,
            margin: self.margin,
            children: Vec::new(),
            lines: Vec::new(),
            marker: None,
        }
    }
}

#[allow(dead_code)]
fn _edges_used(_: Edges) {}
