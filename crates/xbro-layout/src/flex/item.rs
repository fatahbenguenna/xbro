//! Laying out and measuring flex and grid items.
//!
//! Flex and grid layout need the height an item gets at a given width
//! before deciding its final size. Measuring means laying the item's whole
//! subtree out, so two mechanisms keep nested containers from relaying out
//! exponentially:
//!
//! * a per-pass **measurement cache** ([`LayoutCx::measure_cache`]) keyed by
//!   the item's identity, its content width and — only when the item's own
//!   style depends on it — its containing block height: an item is measured
//!   once per width, however many times its ancestors are laid out;
//! * **reuse**: the box laid out for a measurement is kept ([`Laid`]) and,
//!   when the final size turns out to be the measured one, moved into place
//!   instead of being laid out again.
//!
//! Boxes thrown away must [`discard`](Laid::discard) the absolutely
//! positioned descendants they registered.

use crate::block::Cb;
use crate::box_tree::BoxNode;
use crate::cx::{LayoutCx, Measured, NO_HEIGHT};
use crate::sizing::{sane_size, BoxEdges};
use crate::LayoutBox;

/// Sizes closer than this are considered equal (avoids relayouts caused by
/// rounding noise).
const SAME_SIZE: f32 = 0.01;

/// An item laid out at some position, with the range of pending
/// absolutely positioned boxes registered during its layout.
#[derive(Debug)]
pub(crate) struct Laid {
    pub lb: LayoutBox,
    /// Pending-box marks (`start` inclusive, `end` exclusive).
    pub start: u64,
    pub end: u64,
    /// Content width used.
    pub content_w: f32,
    /// Forced content height used (`None` = auto).
    pub forced_h: Option<f32>,
    /// Border-box origin the box was laid out at (a table's border box
    /// ends up lower when it has captions, which sit above it).
    pub x: f32,
    pub y: f32,
}

impl Laid {
    /// Moves the box (and its pending descendants) as if it had been laid
    /// out with its border box at `(x, y)`.
    pub fn move_to(&mut self, cx: &mut LayoutCx<'_>, x: f32, y: f32) {
        let (dx, dy) = (x - self.x, y - self.y);
        if dx != 0.0 || dy != 0.0 {
            crate::tree::translate_box(&mut self.lb, dx, dy);
            cx.translate_pending_range(self.start, self.end, dx, dy);
            self.x = x;
            self.y = y;
        }
    }

    /// Records the used margins `m` on the box (the top margin also covers
    /// the captions of a table).
    pub fn set_margins(&mut self, m: crate::geom::Edges) {
        let caption = sane_size(self.lb.border_box().y - self.y);
        self.lb.margin = m;
        self.lb.margin.top += caption;
    }

    /// Throws the box away, forgetting its pending descendants.
    pub fn discard(self, cx: &mut LayoutCx<'_>) {
        cx.discard_pending_range(self.start, self.end);
    }

    /// Whether this box is what laying `b` out at content width `w` with
    /// forced height `forced` would produce.
    pub fn fits(&self, b: &BoxNode, w: f32, forced: Option<f32>) -> bool {
        if (self.content_w - w).abs() > SAME_SIZE {
            return false;
        }
        match (self.forced_h, forced) {
            (None, None) => true,
            (Some(a), Some(c)) => (a - c).abs() <= SAME_SIZE,
            // An auto height equal to the forced one gives the same layout,
            // unless children resolve percentage heights against it.
            (None, Some(c)) => {
                (self.lb.content.height - c).abs() <= SAME_SIZE && !children_use_percent_heights(b)
            }
            (Some(_), None) => false,
        }
    }
}

/// True when some child of `b` has a percentage `height`, `min-height` or
/// `max-height` (whose resolution depends on `b`'s height being definite).
fn children_use_percent_heights(b: &BoxNode) -> bool {
    b.children.iter().any(depends_on_cb_height)
}

/// True when the used height of `b` depends on its containing block's
/// height (percentage heights).
pub(crate) fn depends_on_cb_height(b: &BoxNode) -> bool {
    let s = &b.style;
    let lpa = |v: &xbro_style::LengthPercentageAuto| match v {
        xbro_style::LengthPercentageAuto::Length(l) => l.has_percent(),
        xbro_style::LengthPercentageAuto::Auto => false,
    };
    lpa(&s.height)
        || lpa(&s.min_height)
        || matches!(&s.max_height, xbro_style::MaxSize::Length(l) if l.has_percent())
}

/// Lays out item `b` with its border box at `(x, y)`, content width `w` and
/// forced content height `forced` (`None`: auto; replaced boxes then get
/// the height matching `w`). The relative offset is not applied.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_item<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    x: f32,
    y: f32,
    w: f32,
    forced: Option<f32>,
    edges: &BoxEdges,
    cb: Cb,
) -> Laid {
    let w = sane_size(w);
    let mut fh = forced.map(sane_size);
    if fh.is_none() && b.is_replaced() {
        fh = cx.replaced_height_for_width(b, edges, cb, w);
    }
    let start = cx.pending_mark();
    let lb = cx.layout_independent(b, x, y, w, edges, cb, fh);
    let end = cx.pending_mark();
    Laid {
        lb,
        start,
        end,
        content_w: w,
        forced_h: forced.map(sane_size),
        x,
        y,
    }
}

/// Content-box height (and first baseline relative to the border-box top
/// it is laid out at, see [`Laid::y`]) of item `b` at content width `w`
/// with an auto height. Cached per pass; on a
/// cache miss the item is laid out with its border box at `(x, y)` and the
/// resulting box is returned for possible reuse.
#[allow(clippy::too_many_arguments)]
pub(crate) fn measure<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    x: f32,
    y: f32,
    w: f32,
    edges: &BoxEdges,
    cb: Cb,
) -> (Measured, Option<Laid>) {
    let w = sane_size(w);
    let cbh = if depends_on_cb_height(b) {
        cb.height.map_or(NO_HEIGHT, f32::to_bits)
    } else {
        NO_HEIGHT
    };
    let key = (b as *const BoxNode as usize, w.to_bits(), cbh);
    if let Some(m) = cx.measure_cache.get(&key) {
        return (*m, None);
    }
    let laid = layout_item(cx, b, x, y, w, None, edges, cb);
    let top = laid.y;
    let m = Measured {
        height: sane_size(laid.lb.content.height),
        baseline: crate::tree::first_baseline(&laid.lb).map(|bl| bl - top),
    };
    cx.measure_cache.insert(key, m);
    (m, Some(laid))
}

/// Lays out `b` for its final size, reusing `kept` when it already has that
/// size (moving it as if laid out at `(x, y)`), otherwise
/// discarding it and laying the item out again.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_item<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    kept: Option<Laid>,
    x: f32,
    y: f32,
    w: f32,
    forced: Option<f32>,
    edges: &BoxEdges,
    cb: Cb,
) -> Laid {
    if let Some(mut laid) = kept {
        if laid.fits(b, w, forced) {
            laid.move_to(cx, x, y);
            return laid;
        }
        laid.discard(cx);
    }
    layout_item(cx, b, x, y, w, forced, edges, cb)
}
