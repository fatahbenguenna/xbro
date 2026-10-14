//! Positioning schemes (CSS 2.1 §9.3, §9.4.3, §10.3.7, §10.3.8, §10.6.4,
//! §10.6.5).
//!
//! * `relative`: the box is laid out in flow, then the whole subtree is
//!   moved by its `left`/`right`/`top`/`bottom` offsets.
//!   `sticky` is treated as `relative` without offsets (it establishes a
//!   containing block but never moves).
//! * `absolute`: registered at its static position while its parent is laid
//!   out (see [`LayoutCx::register_abs`]) and laid out once the padding box
//!   of its containing block (nearest positioned ancestor, else the initial
//!   containing block: the viewport rectangle at the document origin) is
//!   known. It becomes a child of its containing block's box (or of the
//!   root box). Positioned inline elements are handled by inline layout
//!   (see `inline::Ifc::layout_inline_abspos`).
//! * `fixed`: like `absolute` with the viewport as containing block; its
//!   subtree is in viewport coordinates.

use xbro_style::Position;

use crate::block::Cb;
use crate::box_tree::BoxNode;
use crate::cx::{LayoutCx, PendingAbs};
use crate::geom::Rect;
use crate::sizing::{
    box_edges, clamp_size, height_limits, lpa, lpa_definite, sane, sane_size, shrink_to_fit,
    specified_height, specified_width, width_limits, EPSILON,
};
use crate::LayoutBox;

/// Relative offset `(dx, dy)` of a box against its containing block.
pub(crate) fn relative_offset(style: &xbro_style::ComputedStyle, cb: Cb) -> (f32, f32) {
    if style.position != Position::Relative {
        return (0.0, 0.0);
    }
    let dx = match lpa(&style.left, cb.width) {
        Some(l) => l,
        None => lpa(&style.right, cb.width).map_or(0.0, |r| -r),
    };
    let dy = match lpa_definite(&style.top, cb.height) {
        Some(t) => t,
        None => lpa_definite(&style.bottom, cb.height).map_or(0.0, |b| -b),
    };
    (sane(dx), sane(dy))
}

impl<'t> LayoutCx<'t> {
    /// Applies the relative offset of a laid-out box (no-op unless
    /// `position: relative`). `mark` is the [`pending_mark`](Self::pending_mark)
    /// taken before the box was laid out: static positions of boxes
    /// registered inside it move with it.
    pub fn apply_relative(&mut self, lb: &mut LayoutBox, cb: Cb, mark: u64) {
        let (dx, dy) = relative_offset(&lb.style, cb);
        if dx != 0.0 || dy != 0.0 {
            self.move_box(lb, mark, dx, dy);
        }
    }

    /// Like [`apply_relative`](Self::apply_relative) for a box whose
    /// pending absolutely positioned descendants were registered between
    /// the marks `pending.0` and `pending.1`.
    pub fn apply_relative_range(&mut self, lb: &mut LayoutBox, cb: Cb, pending: (u64, u64)) {
        let (dx, dy) = relative_offset(&lb.style, cb);
        if dx != 0.0 || dy != 0.0 {
            crate::tree::translate_box(lb, dx, dy);
            self.translate_pending_range(pending.0, pending.1, dx, dy);
        }
    }

    /// Lays out an absolutely positioned box against the padding box `cbr`
    /// of its containing block.
    pub fn layout_abspos(&mut self, p: PendingAbs<'t>, cbr: Rect) -> LayoutBox {
        let b: &'t BoxNode = p.b;
        let style = &b.style;
        let cbr = Rect::new(
            sane(cbr.x),
            sane(cbr.y),
            sane_size(cbr.width),
            sane_size(cbr.height),
        );
        let cb = Cb {
            x: cbr.x,
            width: cbr.width,
            height: Some(cbr.height),
        };
        let mut edges = box_edges(style, cbr.width);
        let pb_h = edges.pb_h();
        let pb_v = edges.pb_v();
        let left = lpa(&style.left, cbr.width);
        let right = lpa(&style.right, cbr.width);
        let top = lpa(&style.top, cbr.height);
        let bottom = lpa(&style.bottom, cbr.height);
        let static_left = p.static_pos.x - cbr.x;
        let static_top = p.static_pos.y - cbr.y;

        // ---- horizontal (§10.3.7 / §10.3.8) ----
        let (min_w, max_w) = width_limits(style, Some(cbr.width), pb_h);
        let mut forced_h = None;
        let replaced_w = if b.is_replaced() {
            let (w, h) = self.replaced_size(b, &edges, cb);
            forced_h = h;
            Some(w)
        } else {
            None
        };
        let spec_w = replaced_w.or_else(|| {
            specified_width(style, Some(cbr.width), pb_h)
                .or_else(|| self.ratio_width_of(b, &edges, cb))
                .map(|w| clamp_size(w, min_w, max_w))
        });
        let ml = edges.margin.left;
        let mr = edges.margin.right;
        let shrink = |cx: &mut Self, avail: f32| -> f32 {
            let i = cx.content_intrinsic(b);
            clamp_size(shrink_to_fit(i.min, i.max, avail), min_w, max_w)
        };
        let (x_left, width) = match (left, spec_w, right) {
            (None, None, None) => {
                let avail = cbr.width - static_left - ml - mr - pb_h;
                (static_left, shrink(self, avail))
            }
            (None, None, Some(r)) => {
                let w = shrink(self, cbr.width - r - ml - mr - pb_h);
                (cbr.width - r - mr - pb_h - w - ml, w)
            }
            (Some(l), None, None) => (l, shrink(self, cbr.width - l - ml - mr - pb_h)),
            (Some(l), None, Some(r)) => {
                let stretched = cbr.width - l - r - ml - mr - pb_h;
                let w = clamp_size(stretched, min_w, max_w);
                if (w - sane_size(stretched)).abs() > EPSILON {
                    // `min-width`/`max-width` changed the width: solve
                    // again with it as the specified width (§10.4), auto
                    // margins taking up the difference.
                    let rest = cbr.width - l - r - w - pb_h;
                    resolve_auto_margins(
                        &mut edges.margin.left,
                        &mut edges.margin.right,
                        (edges.auto_left, edges.auto_right),
                        rest,
                    );
                }
                (l, w)
            }
            (None, Some(w), None) => (static_left, w),
            (None, Some(w), Some(r)) => (cbr.width - r - mr - pb_h - w - ml, w),
            (Some(l), Some(w), None) => (l, w),
            (Some(l), Some(w), Some(r)) => {
                // Over-constrained or auto margins.
                let rest = cbr.width - l - r - w - pb_h;
                resolve_auto_margins(
                    &mut edges.margin.left,
                    &mut edges.margin.right,
                    (edges.auto_left, edges.auto_right),
                    rest,
                );
                (l, w)
            }
        };
        // `x_left` is the margin-box left edge relative to the containing block.
        let border_x = cbr.x + x_left + edges.margin.left;

        // ---- vertical (§10.6.4 / §10.6.5) ----
        let (min_h, max_h) = height_limits(style, Some(cbr.height), pb_v);
        let mt = edges.margin.top;
        let mb = edges.margin.bottom;
        let spec_h = forced_h.or_else(|| {
            specified_height(style, Some(cbr.height), pb_v).map(|h| clamp_size(h, min_h, max_h))
        });
        // Stretched height when top and bottom are set and height is auto.
        // When `min-height`/`max-height` change it, it acts as a specified
        // height for the auto margins (§10.6.4, §10.7).
        let (stretched, constrained) = match (top, spec_h, bottom) {
            (Some(t), None, Some(bt)) => {
                let raw = cbr.height - t - bt - mt - mb - pb_v;
                let h = clamp_size(raw, min_h, max_h);
                (Some(h), (h - sane_size(raw)).abs() > EPSILON)
            }
            _ => (None, false),
        };
        let height_known = spec_h.or(stretched);
        let margin_h = spec_h.or(stretched.filter(|_| constrained));
        if let (Some(t), Some(h), Some(bt)) = (top, margin_h, bottom) {
            let rest = cbr.height - t - bt - h - pb_v;
            match (edges.auto_top, edges.auto_bottom) {
                (true, true) => {
                    edges.margin.top = rest / 2.0;
                    edges.margin.bottom = rest / 2.0;
                }
                (true, false) => edges.margin.top = rest - mb,
                (false, true) => edges.margin.bottom = rest - mt,
                (false, false) => {}
            }
        }
        let tentative_top = match top {
            Some(t) => t,
            None if bottom.is_none() => static_top,
            None => 0.0,
        };
        let border_y = cbr.y + tentative_top + edges.margin.top;
        let mark = self.pending_mark();
        let mut lb =
            self.layout_independent(b, border_x, border_y, width, &edges, cb, height_known);
        if top.is_none() {
            if let Some(bt) = bottom {
                // Anchored at the bottom: move up once the height is known.
                let h = lb.border_box().height;
                let want = cbr.bottom() - bt - edges.margin.bottom - h;
                let dy = want - lb.border_box().y;
                self.move_box(&mut lb, mark, 0.0, dy);
            }
        }
        lb
    }
}

/// Resolves the horizontal auto margins of an absolutely positioned box
/// whose `left`, `width` and `right` are known (§10.3.7): `rest` is the
/// space left by them (margins excluded). Equal auto margins never make
/// the box start left of `left` (ltr); with no auto margin, `right` is
/// ignored.
fn resolve_auto_margins(ml: &mut f32, mr: &mut f32, auto: (bool, bool), rest: f32) {
    match auto {
        (true, true) => {
            let m = rest / 2.0;
            if m >= 0.0 {
                *ml = m;
                *mr = m;
            } else {
                *ml = 0.0;
                *mr = rest;
            }
        }
        (true, false) => *ml = rest - *mr,
        (false, true) => *mr = rest - *ml,
        (false, false) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_style::{ComputedStyle, LengthPercentageAuto};

    #[test]
    fn relative_offsets() {
        let cb = Cb {
            x: 0.0,
            width: 200.0,
            height: None,
        };
        let mut s = ComputedStyle {
            position: Position::Relative,
            left: LengthPercentageAuto::percent(10.0),
            right: LengthPercentageAuto::px(5.0),
            bottom: LengthPercentageAuto::px(7.0),
            ..ComputedStyle::default()
        };
        assert_eq!(relative_offset(&s, cb), (20.0, -7.0));
        s.top = LengthPercentageAuto::percent(50.0);
        // Percentage of an indefinite height: auto, so bottom applies.
        assert_eq!(relative_offset(&s, cb), (20.0, -7.0));
        s.position = Position::Sticky;
        assert_eq!(relative_offset(&s, cb), (0.0, 0.0));
    }
}
