//! `<fieldset>` and its rendered `<legend>` (HTML "The fieldset and
//! legend elements" rendering, simplified).
//!
//! * A fieldset establishes a block formatting context
//!   ([`BoxNode::fieldset`]).
//! * Its *rendered legend* is its first child `<legend>` element that is
//!   neither floated nor absolutely positioned, when it is block-level
//!   (floated and absolutely positioned legends are ordinary children). It is laid out shrink-to-fit at the
//!   inline start of the content box (after the fieldset's border and
//!   `padding-left`; `margin-left/right: auto` center it or push it to
//!   the end) and vertically centred on the top border.
//! * When the legend's margin box (height `H`) is taller than the top
//!   border (`B`), the border moves down by `(H − B) / 2` (the fieldset's
//!   margin box does not move: [`ContentLayout::offset_top`] grows its top
//!   margin) and the content starts below the legend: the used top padding
//!   grows by the same amount, so the content box starts
//!   `max(B, H) + padding-top` below the top of the legend.
//! * The top border is not painted behind the legend:
//!   [`LayoutBox::border_top_gap`](crate::LayoutBox::border_top_gap) is the
//!   x-range of the legend's border box.

use crate::block::{Cb, ContentLayout, Flow, Margins};
use crate::box_tree::BoxNode;
use crate::cx::LayoutCx;
use crate::float::FloatContext;
use crate::sizing::{box_edges, sane, sane_size, BoxEdges};

impl<'t> LayoutCx<'t> {
    /// Index in `b.children` of the rendered legend of fieldset `b`.
    pub(crate) fn rendered_legend(&self, b: &BoxNode) -> Option<usize> {
        if !b.fieldset || b.inline_content {
            return None;
        }
        let doc = self.doc;
        let is_legend = |c: &BoxNode| {
            c.pseudo.is_none()
                && c.node
                    .is_some_and(|n| n.index() < doc.len() && doc.is_element_named(n, "legend"))
        };
        // The first legend child that is neither floated nor absolutely
        // positioned (as in browsers: a floated first legend leaves the
        // role to the next one); it must be block-level.
        let i = b
            .children
            .iter()
            .position(|c| is_legend(c) && !c.is_float() && !c.is_absolute())?;
        b.children
            .get(i)
            .is_some_and(BoxNode::is_block_level)
            .then_some(i)
    }

    /// Lays out the contents of fieldset `b` whose rendered legend is
    /// `b.children[legend]` (see the module docs). `cb` is its content box
    /// as laid out without the legend's extra space, `y` the top of that
    /// content box and `edges` the fieldset's used edges.
    pub(crate) fn fieldset_contents(
        &mut self,
        b: &'t BoxNode,
        legend: usize,
        cb: Cb,
        y: f32,
        edges: &BoxEdges,
    ) -> ContentLayout {
        let border = edges.border.top;
        let border_top = y - edges.padding.top - border;
        let mut children = Vec::with_capacity(b.children.len());
        let mut gap = None;
        let mut legend_h = 0.0f32;
        if let Some(l) = b.children.get(legend) {
            let mark = self.pending_mark();
            let mut le = box_edges(&l.style, cb.width);
            let legend_cb = Cb { height: None, ..cb };
            let (w, forced_h) = self.shrink_width(l, &le, legend_cb);
            let free = sane_size(cb.width - w - le.pb_h() - le.margin.horizontal());
            let dx = match (le.auto_left, le.auto_right) {
                (true, true) => free / 2.0,
                (true, false) => free,
                _ => 0.0,
            };
            le.margin.left += dx;
            let mut lb = self.layout_independent(
                l,
                cb.x + le.margin.left,
                border_top + le.margin.top,
                w,
                &le,
                legend_cb,
                forced_h,
            );
            let h = sane_size(lb.margin_box().height);
            if h < border {
                // A legend thinner than the border is centred on it.
                self.move_box(&mut lb, mark, 0.0, (border - h) / 2.0);
            }
            let bb = lb.border_box();
            gap = Some((sane(bb.x), sane(bb.right())));
            self.apply_relative(&mut lb, legend_cb, mark);
            legend_h = h;
            children.push(lb);
        }
        // The border line moves down to the middle of a taller legend.
        let shift = sane_size((legend_h - border) / 2.0);
        let content_top = border_top + border.max(legend_h) + edges.padding.top;
        let mut floats = FloatContext::new();
        let mut flow = Flow {
            floats: &mut floats,
            y: content_top,
            margins: Margins::default(),
        };
        self.layout_block_children_skipping(b, cb, &mut flow, &mut children, false, Some(legend));
        let bottom = flow.hypothetical();
        let mut height = bottom - content_top;
        if let Some(fb) = floats.bottom() {
            height = height.max(fb - content_top);
        }
        ContentLayout {
            children,
            lines: Vec::new(),
            height: sane_size(height),
            offset_top: shift,
            extra_padding_top: shift,
            border_top_gap: gap,
            ..ContentLayout::default()
        }
    }
}
