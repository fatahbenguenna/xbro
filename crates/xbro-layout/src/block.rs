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
use crate::geom::{Point, Rect};
use crate::inline::{layout_ifc, AtomicBox};
use crate::sizing::{
    block_width, box_edges, clamp_size, height_limits, ratio_height, ratio_used_height,
    ratio_width, sane_size, shrink_to_fit, specified_height, specified_width, width_limits,
    BoxEdges, EPSILON,
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
    /// Space laid out above the box's border box (table captions): the
    /// content and border box move down by this amount and it is added to
    /// the box's top margin, so its margin box still starts at the position
    /// the caller chose.
    pub offset_top: f32,
}

/// A float laid out but not yet placed.
#[derive(Debug)]
pub(crate) struct LaidFloat {
    pub lb: LayoutBox,
    /// Marks of the pending absolutely positioned boxes registered during its layout.
    pub pending: (u64, u64),
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
        || b.flex_item
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
        let mark = self.pending_mark();
        let mut lb = self.layout_independent(
            root,
            edges.margin.left,
            edges.margin.top,
            content_w,
            &edges,
            cb,
            forced_h,
        );
        self.apply_relative(&mut lb, cb, mark);
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
    /// applied) and, unless the box collapsed through (empty, margins joined
    /// the flow), where the content of its parent starts if it is the
    /// parent's first in-flow box: the y of its border box top before the
    /// relative offset, or — when clearance or float avoidance moved it
    /// down — the position its margins resolved to before that move (the
    /// parent's top does not move with a cleared child, CSS 2.1 §8.3.1).
    pub fn layout_flow_child(
        &mut self,
        b: &'t BoxNode,
        cb: Cb,
        flow: &mut Flow<'_>,
    ) -> (LayoutBox, Option<f32>) {
        let style = &b.style;
        let mut edges = box_edges(style, cb.width);
        let mark = self.pending_mark();
        if establishes_bfc(b) {
            let (mut lb, top) = self.layout_flow_bfc(b, cb, flow, &edges);
            self.apply_relative(&mut lb, cb, mark);
            return (lb, Some(top));
        }
        let pb_h = edges.pb_h();
        let pb_v = edges.pb_v();
        let (min_h, max_h) = height_limits(style, cb.height, pb_v);
        let spec_h = specified_height(style, cb.height, pb_v).map(|h| clamp_size(h, min_h, max_h));
        // An auto width with a definite height and an `aspect-ratio` comes
        // from the ratio; an auto height from the width (`ratio_h`).
        let spec_w = specified_width(style, Some(cb.width), pb_h)
            .or_else(|| spec_h.and_then(|h| ratio_width(style, h, pb_h, pb_v)));
        let (content_w, ml, mr) = block_width(style, &edges, cb.width, cb.width, spec_w);
        edges.margin.left = ml;
        edges.margin.right = mr;
        let ratio_h = match spec_h {
            Some(_) => None,
            None => ratio_height(style, content_w, pb_h, pb_v),
        };

        // Clearance. The margins pending before the box resolve where they
        // are (the parent's content starts there); the box's own top margin
        // is absorbed by the clearance.
        let mut cleared = None;
        if style.clear != Clear::None {
            if let Some(cy) = flow.floats.clear_y(style.clear) {
                let mut m = flow.margins;
                m.add(edges.margin.top);
                if flow.y + m.resolve() < cy {
                    cleared = Some(flow.hypothetical());
                    flow.y = cy;
                    flow.margins = Margins::default();
                }
            }
        }
        let collapse_top = cleared.is_none() && edges.padding.top == 0.0 && edges.border.top == 0.0;
        let border_top = if cleared.is_some() {
            Some(flow.y)
        } else {
            flow.margins.add(edges.margin.top);
            if collapse_top {
                None
            } else {
                Some(flow.resolve())
            }
        };

        let content_x = cb.x + ml + edges.border.left + edges.padding.left;
        if let Some(bt) = border_top {
            flow.y = bt + edges.border.top + edges.padding.top;
        }
        let child_cb = Cb {
            x: content_x,
            width: content_w,
            height: spec_h.or_else(|| ratio_h.map(|h| clamp_size(h, min_h, max_h))),
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
            self.layout_block_children(b, child_cb, flow, &mut children, border_top.is_none())
        };

        let no_bottom_pb = edges.padding.bottom == 0.0 && edges.border.bottom == 0.0;
        let collapse_bottom = spec_h.is_none() && ratio_h.is_none() && no_bottom_pb;
        // Top and bottom margins of an empty box are adjoining when its
        // height is auto or zero (CSS 2.1 §8.3.1).
        let collapse_through =
            no_bottom_pb && min_h <= 0.0 && spec_h.or(ratio_h).is_none_or(|h| h <= 0.0);
        let content_top = match border_top {
            Some(bt) => bt + edges.border.top + edges.padding.top,
            None => match first_top {
                Some(t) => t,
                None if collapse_through => {
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
                    self.apply_relative(&mut lb, cb, mark);
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
        let h = match (spec_h, ratio_h) {
            (Some(h), _) => h,
            (None, Some(rh)) => ratio_used_height(style, rh, auto_h, (min_h, max_h)),
            (None, None) => clamp_size(auto_h, min_h, max_h),
        };
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
        self.apply_relative(&mut lb, cb, mark);
        (lb, Some(cleared.unwrap_or(border_top_y)))
    }

    /// Lays out the children of a block container whose children are
    /// block-level. Returns the border-box top of the first child that did
    /// not collapse through. `top_unresolved` tells that the container's top
    /// margin collapses with its first child's (its position is not known
    /// until in-flow content is found).
    pub fn layout_block_children(
        &mut self,
        b: &'t BoxNode,
        cb: Cb,
        flow: &mut Flow<'_>,
        out: &mut Vec<LayoutBox>,
        top_unresolved: bool,
    ) -> Option<f32> {
        let mut first_top = None;
        for (i, c) in b.children.iter().enumerate() {
            if c.is_float() {
                let laid = self.prepare_float(c, cb);
                let mut m = flow.margins;
                if top_unresolved && first_top.is_none() {
                    // The container's top is where its pending margins and
                    // those of the next in-flow content collapse: the float
                    // starts there, not above.
                    let rest = b.children.get(i + 1..).unwrap_or(&[]);
                    let mut budget = LOOKAHEAD_BUDGET;
                    upcoming_margins(rest, cb.width, &mut m, &mut budget, 0);
                }
                let min_y = flow.y + m.resolve();
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
    /// it, narrowed or moved down to avoid floats. Also returns where the
    /// content of its parent starts if it is the parent's first in-flow box
    /// (see [`LayoutCx::layout_flow_child`]).
    fn layout_flow_bfc(
        &mut self,
        b: &'t BoxNode,
        cb: Cb,
        flow: &mut Flow<'_>,
        edges: &BoxEdges,
    ) -> (LayoutBox, f32) {
        let style = &b.style;
        let before = flow.hypothetical();
        flow.margins.add(edges.margin.top);
        let mut y = flow.hypothetical();
        // Without clearance the box's margins collapse with its parent's:
        // the parent starts where they resolve, even if the box itself is
        // then moved below floats to avoid them.
        let mut parent_top = y;
        if style.clear != Clear::None {
            if let Some(cy) = flow.floats.clear_y(style.clear) {
                if cy > y {
                    y = cy;
                    parent_top = before;
                }
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
                // A table is never narrower than its min-content width,
                // whatever its `width`/`max-width` say.
                let i = self.content_intrinsic(b);
                match specified_width(style, Some(cb.width), pb_h) {
                    Some(w) => WidthMode::Fixed(clamp_size(w, min_w, max_w).max(i.min)),
                    None => WidthMode::Shrink(i.min, i.max),
                }
            }
            _ => match specified_width(style, Some(cb.width), pb_h)
                .or_else(|| self.ratio_width_of(b, edges, cb))
            {
                Some(w) => WidthMode::Fixed(clamp_size(w, min_w, max_w)),
                None => WidthMode::Fill,
            },
        };
        let est_h = forced_h
            .or_else(|| specified_height(style, cb.height, edges.pb_v()))
            .map_or(1.0, |h| h + edges.pb_v());
        let mut used = *edges;
        let content_w;
        let x;
        let mut guard = 0;
        loop {
            guard += 1;
            let narrowed = flow.floats.narrows(y, est_h, cb.x, cb.x + cb.width);
            if !narrowed {
                let spec = match mode {
                    WidthMode::Fill => None,
                    WidthMode::Fixed(w) => Some(w),
                    WidthMode::Shrink(lo, hi) => Some(
                        clamp_size(
                            shrink_to_fit(lo, hi, cb.width - edges.margin.horizontal() - pb_h),
                            min_w,
                            max_w,
                        )
                        .max(lo),
                    ),
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
                    let w = clamp_size(shrink_to_fit(lo, hi, space - pb_h), min_w, max_w).max(lo);
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
        (lb, parent_top)
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
        let mut content_w = sane_size(content_w);
        if matches!(b.kind, Kind::Table) {
            // Tables are never narrower than their min-content width
            // (whatever flex, grid or `max-width` asked for): they overflow.
            content_w = content_w.max(self.content_intrinsic(b).min);
        }
        let pb_v = edges.pb_v();
        let (min_h, max_h) = height_limits(style, cb.height, pb_v);
        let spec_h = match forced_h {
            Some(h) => Some(sane_size(h)),
            None => specified_height(style, cb.height, pb_v).map(|h| clamp_size(h, min_h, max_h)),
        };
        // `aspect-ratio` gives an auto height from the width (not for
        // replaced elements, sized by `replaced`, nor table parts).
        let ratio_h = match (spec_h, &b.kind) {
            (Some(_), _) | (None, Kind::Replaced(_) | Kind::Table | Kind::TableCell) => None,
            (None, _) => ratio_height(style, content_w, edges.pb_h(), pb_v),
        };
        let frame = style.position.is_positioned().then(|| self.push_frame());
        let content = self.independent_contents(
            b,
            content_x,
            content_y,
            content_w,
            spec_h.or_else(|| ratio_h.map(|h| clamp_size(h, min_h, max_h))),
            (min_h, max_h),
            edges,
        );
        let h = match spec_h {
            // The specified height of a table is a minimum; the one of a
            // cell is applied by table layout (it heightens the row while
            // the content is aligned with `vertical-align`).
            Some(s) if forced_h.is_none() && matches!(b.kind, Kind::Table) => {
                sane_size(s.max(content.height))
            }
            Some(_) if forced_h.is_none() && matches!(b.kind, Kind::TableCell) => {
                sane_size(content.height)
            }
            Some(s) => s,
            None => match ratio_h {
                Some(rh) => ratio_used_height(style, rh, content.height, (min_h, max_h)),
                None => clamp_size(content.height, min_h, max_h),
            },
        };
        let offset = sane_size(content.offset_top);
        let mut used = *edges;
        used.margin.top += offset;
        let mut lb = self.new_box(
            b,
            Rect::new(content_x, content_y + offset, content_w, h),
            &used,
            content.children,
            content.lines,
        );
        if let Kind::Replaced(Replaced::Control(c)) = &b.kind {
            if c.kind != crate::ControlKind::TextArea {
                // Single-line controls center their text vertically.
                let dy = (h - content.height) / 2.0;
                if dy.abs() > EPSILON {
                    crate::tree::translate_lines(&mut lb.lines, 0.0, dy);
                }
            }
        }
        self.finish_box(b, &mut lb, frame);
        lb
    }

    /// Lays out the contents of an independent formatting context root
    /// whose used padding, border and margins are `edges`. `limits` are its
    /// used `min-height`/`max-height` (content-box px, resolved against its
    /// containing block), which flex containers need to size their lines.
    #[allow(clippy::too_many_arguments)]
    fn independent_contents(
        &mut self,
        b: &'t BoxNode,
        x: f32,
        y: f32,
        w: f32,
        spec_h: Option<f32>,
        limits: (f32, f32),
        edges: &BoxEdges,
    ) -> ContentLayout {
        let cb = Cb {
            x,
            width: w,
            height: spec_h,
        };
        match &b.kind {
            Kind::Flex => flex::layout_flex_contents(self, b, cb, y, limits),
            Kind::Table => table::layout_table_contents(self, b, cb, y, edges),
            Kind::Inline | Kind::Text(_) | Kind::LineBreak | Kind::WordBreak => {
                ContentLayout::default()
            }
            Kind::Replaced(r) => {
                let show_text = match r {
                    Replaced::Image => self.shows_alt_text(b),
                    Replaced::Control(_) => true,
                    _ => false,
                };
                if show_text && b.inline_content && !b.children.is_empty() {
                    let mut floats = FloatContext::new();
                    let out = layout_ifc(self, b, cb, y, &mut floats);
                    let mut height = out.height;
                    if let Some(fb) = floats.bottom() {
                        height = height.max(fb - y);
                    }
                    ContentLayout {
                        children: out.floats,
                        lines: out.lines,
                        height: sane_size(height),
                        offset_top: 0.0,
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
                        offset_top: 0.0,
                    }
                } else {
                    let mut flow = Flow {
                        floats: &mut floats,
                        y,
                        margins: Margins::default(),
                    };
                    let mut children = Vec::new();
                    self.layout_block_children(b, cb, &mut flow, &mut children, false);
                    let bottom = flow.hypothetical();
                    let mut height = bottom - y;
                    if let Some(fb) = floats.bottom() {
                        height = height.max(fb - y);
                    }
                    ContentLayout {
                        children,
                        lines: Vec::new(),
                        height: sane_size(height),
                        offset_top: 0.0,
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
                    Some(w) => w,
                    None => shrink_to_fit(i.min, i.max, avail),
                };
                (clamp_size(w, min_w, max_w).max(i.min), None)
            }
            _ => match specified_width(style, Some(cb.width), pb_h)
                .or_else(|| self.ratio_width_of(b, edges, cb))
            {
                Some(w) => (clamp_size(w, min_w, max_w), None),
                None => {
                    let i = self.content_intrinsic(b);
                    (
                        clamp_size(shrink_to_fit(i.min, i.max, avail), min_w, max_w),
                        None,
                    )
                }
            },
        }
    }

    /// Content width that the `aspect-ratio` of non-replaced box `b` gives
    /// it from a definite specified height (`None` without one).
    pub fn ratio_width_of(&self, b: &BoxNode, edges: &BoxEdges, cb: Cb) -> Option<f32> {
        if b.is_replaced() || matches!(b.kind, Kind::Table | Kind::TableCell) {
            return None;
        }
        let style = &b.style;
        crate::sizing::aspect_ratio(style)?;
        let pb_v = edges.pb_v();
        let (min_h, max_h) = height_limits(style, cb.height, pb_v);
        let h = specified_height(style, cb.height, pb_v)?;
        ratio_width(style, clamp_size(h, min_h, max_h), edges.pb_h(), pb_v)
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
        // `lb.margin` rather than `edges.margin`: table captions extend the top margin.
        let width = sane_size(border.width + lb.margin.left + lb.margin.right);
        let height = sane_size(border.height + lb.margin.top + lb.margin.bottom);
        let clips = b.style.overflow_x.clips() || b.style.overflow_y.clips();
        let baseline = match &b.kind {
            Kind::Replaced(Replaced::Control(c)) => match crate::tree::first_baseline(&lb) {
                Some(bl) => bl,
                None if matches!(
                    c.kind,
                    crate::ControlKind::TextField
                        | crate::ControlKind::Button
                        | crate::ControlKind::Select
                        | crate::ControlKind::TextArea
                ) =>
                {
                    // Empty text control: baseline of its (empty) first line,
                    // centered like the text would be.
                    let font = self.font(&b.style);
                    let top = match c.kind {
                        crate::ControlKind::TextArea => lb.content.y,
                        _ => lb.content.y + (lb.content.height - font.line_height) / 2.0,
                    };
                    top + font.above
                }
                None => height,
            },
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
            width: sane_size(border.width + lb.margin.left + lb.margin.right),
            height: sane_size(border.height + lb.margin.top + lb.margin.bottom),
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
        self.apply_relative_range(&mut lb, cb, f.pending);
        lb
    }

    /// Height of the single line an empty list item with an outside marker gets.
    fn empty_marker_height(&mut self, b: &BoxNode) -> f32 {
        let s = &b.style;
        if s.display != Display::ListItem
            || s.list_style_position != xbro_style::ListStylePosition::Outside
            || (s.list_style_type == xbro_style::ListStyleType::None
                && s.list_style_image.is_none())
            || !b
                .children
                .iter()
                .all(|c| c.is_out_of_flow() || is_ws_text(c))
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
            node: b
                .node
                .filter(|_| !matches!(b.kind, Kind::Replaced(Replaced::UrlImage(_)))),
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
}

/// Largest number of boxes examined by [`upcoming_margins`].
const LOOKAHEAD_BUDGET: usize = 64;

/// Adds to `m` the top margins that will collapse with the current pending
/// margins: those of the next in-flow block, of its first in-flow child
/// (when they collapse), and of empty boxes collapsing through. Returns
/// true when in-flow content (or a margin-collapsing barrier) was reached.
fn upcoming_margins(
    siblings: &[BoxNode],
    cb_width: f32,
    m: &mut Margins,
    budget: &mut usize,
    depth: usize,
) -> bool {
    for s in siblings {
        if *budget == 0 || depth > 32 {
            return true;
        }
        *budget -= 1;
        if s.is_out_of_flow() {
            continue;
        }
        if s.is_text_like() || matches!(s.kind, Kind::Inline) {
            return true;
        }
        let e = box_edges(&s.style, cb_width);
        m.add(e.margin.top);
        if s.style.clear != Clear::None
            || establishes_bfc(s)
            || e.padding.top > 0.0
            || e.border.top > 0.0
        {
            return true;
        }
        if s.inline_content {
            if !s
                .children
                .iter()
                .all(|c| c.is_out_of_flow() || is_ws_text(c))
            {
                return true;
            }
        } else if upcoming_margins(&s.children, cb_width, m, budget, depth + 1) {
            return true;
        }
        // Empty box: its margins collapse through unless it has a height.
        let (min_h, _) = height_limits(&s.style, None, e.pb_v());
        let h = specified_height(&s.style, None, e.pb_v()).unwrap_or(0.0);
        if h > 0.0 || min_h > 0.0 || e.padding.bottom > 0.0 || e.border.bottom > 0.0 {
            return true;
        }
        m.add(e.margin.bottom);
    }
    false
}

/// Collapsible-whitespace-only text box.
fn is_ws_text(b: &BoxNode) -> bool {
    match &b.kind {
        Kind::Text(t) => t.chars().all(crate::text::is_collapsible_space),
        _ => false,
    }
}
