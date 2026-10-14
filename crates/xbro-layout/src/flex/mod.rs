//! Flex containers (CSS Flexbox 1 §9, simplified) and the entry point of
//! grid layout ([`crate::grid`]).
//!
//! # Contract
//!
//! [`layout_flex_contents`] is called by block layout
//! ([`LayoutCx::layout_independent`]) once the container's used content
//! width is known:
//!
//! * **Inputs**: `b` is the container (`Kind::Flex`); its `children` are
//!   the flex items (in-flow, all blockified: `BoxNode::flex_item` is set,
//!   anonymous blocks wrap text runs, whitespace-only runs are dropped)
//!   plus absolutely positioned boxes. `cb` describes the container's
//!   *content box*: `cb.x`, `cb.width` and, when definite, `cb.height`
//!   (the used height when `height` is specified or stretched). `y` is the
//!   top of the content box. `limits` are the container's used
//!   `min-height`/`max-height` as content-box sizes (percentages resolved
//!   against its own containing block), which bound an auto height.
//! * **Output**: a [`ContentLayout`] whose `children` are the laid-out
//!   items in absolute document coordinates, in `order`-modified document
//!   order, and whose `height` is the auto content height measured from `y`
//!   (block layout applies `height`/`min-height`/`max-height`).
//! * Absolutely positioned children are registered with the container's
//!   content box origin as static position.
//! * [`flex_intrinsic`] gives the container's content-box min/max-content
//!   widths (used when it is a float, inline-flex, abspos, table cell…).
//!
//! # Algorithm
//!
//! See [`algo`] for the steps. Supported: `flex-direction` (all four),
//! `flex-wrap` (all three), `flex-grow`/`flex-shrink`/`flex-basis`
//! (lengths, percentages against a definite inner main size, `auto`,
//! `content` via auto), `min-*`/`max-*` with the freeze loop, the automatic
//! minimum size of items that are not scroll containers, `order`,
//! `row-gap`/`column-gap`, `justify-content`, `align-items`/`align-self`
//! (stretch relayouts the item with a definite cross size, baseline aligns
//! first baselines — synthesized from the bottom of the border box for
//! items without line boxes), `align-content`, auto margins in both axes.
//!
//! Approximations: baseline alignment only in row containers without
//! `wrap-reverse` (others fall back to `flex-start`); `visibility:
//! collapse` items are laid out normally.
//!
//! # Cost
//!
//! Items are measured through [`item`], which caches auto-height
//! measurements per (item, width) for the whole pass and reuses measured
//! boxes, so nested flex containers lay out in polynomial (not
//! exponential) time.

mod algo;
pub(crate) mod item;

use xbro_style::{BoxSizing, ComputedStyle, Display, FlexWrap, LengthPercentageAuto};

use crate::block::{Cb, ContentLayout};
use crate::box_tree::{BoxNode, Kind};
use crate::cx::LayoutCx;
use crate::intrinsic::Intrinsic;
use crate::sizing::{box_edges, sane, sane_size};

/// True for grid containers (laid out by [`crate::grid`]).
pub(crate) fn is_grid(style: &ComputedStyle) -> bool {
    matches!(style.display, Display::Grid | Display::InlineGrid)
}

/// Lays out the items of flex (or grid) container `b` (see the module docs).
pub(crate) fn layout_flex_contents<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    cb: Cb,
    y: f32,
    limits: (f32, f32),
) -> ContentLayout {
    if is_grid(&b.style) {
        crate::grid::layout_grid_contents(cx, b, cb, y, limits)
    } else {
        algo::layout(cx, b, cb, y, limits)
    }
}

/// In-flow items of a flex or grid container.
pub(crate) fn in_flow_items(b: &BoxNode) -> impl Iterator<Item = &BoxNode> {
    b.children
        .iter()
        .filter(|c| !c.is_absolute() && !c.is_text_like() && !matches!(c.kind, Kind::Inline))
}

/// Min/max-content widths of the content box of flex (or grid) container
/// `b`: row containers sum their items' contributions plus gaps
/// (min-content: sum of the items' min-content contributions when not
/// wrapping, largest one when wrapping), column containers take the
/// largest.
pub(crate) fn flex_intrinsic<'t>(cx: &mut LayoutCx<'t>, b: &'t BoxNode) -> Intrinsic {
    if is_grid(&b.style) {
        return crate::grid::grid_intrinsic(cx, b);
    }
    let style = &b.style;
    let row = style.flex_direction.is_row();
    let wraps = style.flex_wrap != FlexWrap::NoWrap;
    // Percentage gaps resolve against an unknown size: their length part only.
    let gap = sane_size(style.column_gap.px);
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    let mut count = 0usize;
    for item in in_flow_items(b) {
        let o = item_contribution(cx, item);
        count += 1;
        if row {
            min = if wraps { min.max(o.min) } else { min + o.min };
            max += o.max;
        } else {
            min = min.max(o.min);
            max = max.max(o.max);
        }
    }
    if row && count > 1 {
        let gaps = gap * (count - 1) as f32;
        max += gaps;
        if !wraps {
            min += gaps;
        }
    }
    Intrinsic::new(min, max)
}

/// Min/max-content contribution (margin box) of a flex item along a row:
/// a definite length `flex-basis` replaces the content size when the item
/// cannot grow (max-content) or shrink (min-content) away from it.
fn item_contribution<'t>(cx: &mut LayoutCx<'t>, item: &'t BoxNode) -> Intrinsic {
    let o = cx.outer_intrinsic(item);
    let style = &item.style;
    let LengthPercentageAuto::Length(l) = style.flex_basis else {
        return o;
    };
    if l.has_percent() {
        return o;
    }
    let e = box_edges(style, 0.0);
    let pb = e.pb_h();
    let outer_extra = pb + e.margin.left + e.margin.right;
    let basis = match style.box_sizing {
        BoxSizing::ContentBox => sane_size(l.px),
        BoxSizing::BorderBox => sane_size(sane(l.px) - pb),
    } + outer_extra;
    let max = if style.flex_grow > 0.0 {
        o.max.max(basis)
    } else {
        basis
    };
    let min = if style.flex_shrink > 0.0 {
        o.min.min(max)
    } else {
        basis.max(o.min)
    };
    Intrinsic::new(min, max)
}
