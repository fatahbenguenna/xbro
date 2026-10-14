//! Grid approximation for `display: grid` / `inline-grid` containers.
//!
//! Only equal-width columns are supported; explicit placement properties
//! (`grid-column`, `grid-row`, `grid-area`, spans) and track sizing
//! functions are ignored:
//!
//! * in-flow children (in `order`-modified document order) are placed
//!   row-major into `N` columns of width
//!   `(inner − (N − 1) · column-gap) / N`, rows separated by `row-gap`;
//! * `N` is `grid_columns` when the style knows it (`repeat(3, 1fr)`,
//!   `200px 1fr auto`…). When it is 0 (no template, `auto-fill`/`auto-fit`
//!   or unparsed tracks) the column count is guessed — see [`auto_columns`];
//! * a row is as tall as its tallest item (margin box); items with
//!   `align-self: stretch` (the default) and an auto height are stretched to
//!   it (relaid out with that definite height), `start`/`end`/`center`
//!   offset the others (`baseline` behaves as `start`);
//! * items with an auto width fill their column (minus margins); specified
//!   widths and replaced elements keep their size at the start of the cell;
//! * the container's auto height is the sum of the rows and gaps.

use xbro_style::{AlignItems, ComputedStyle};

use crate::block::{Cb, ContentLayout};
use crate::box_tree::BoxNode;
use crate::cx::LayoutCx;
use crate::flex::{in_flow_items, item};
use crate::geom::Point;
use crate::intrinsic::Intrinsic;
use crate::sizing::{
    box_edges, clamp_size, height_limits, lp, sane, sane_size, specified_height, specified_width,
    width_limits,
};

/// Largest number of columns of the approximation.
const MAX_COLUMNS: usize = 1000;

/// Items whose max-content widths differ by more than this factor are
/// considered heterogeneous content (a vertical stack), see [`auto_columns`].
const HETEROGENEOUS_RATIO: f32 = 4.0;

/// Column gap (percentages against the container width).
fn column_gap(style: &ComputedStyle, width: f32) -> f32 {
    sane_size(lp(&style.column_gap, width))
}

/// Row gap (a percentage of an indefinite height counts as zero).
fn row_gap(style: &ComputedStyle, height: Option<f32>) -> f32 {
    sane_size(match height {
        Some(h) => lp(&style.row_gap, h),
        None => sane(style.row_gap.px),
    })
}

/// Number of columns used when the template is unknown (`grid_columns ==
/// 0`), for `count` items with outer min/max-content widths `widths`:
///
/// 1. with `m` the smallest max-content width (at least 1px), take as many
///    columns as fit: `N = floor((inner + gap) / (m + gap))`, clamped to
///    `1..=count`;
/// 2. reduce `N` while the widest min-content width does not fit in a
///    column (content must never be squeezed below its min-content);
/// 3. a single column when the items are heterogeneous (max-content widths
///    differing by more than [`HETEROGENEOUS_RATIO`]): such grids are
///    usually vertical stacks of different content (`display: grid; gap`
///    without template) rather than card grids.
fn auto_columns(widths: &[Intrinsic], inner: f32, gap: f32) -> usize {
    let count = widths.len();
    if count <= 1 {
        return 1;
    }
    let smallest = widths.iter().map(|w| w.max).fold(f32::INFINITY, f32::min);
    let largest = widths.iter().map(|w| w.max).fold(0.0f32, f32::max);
    if largest > smallest.max(1.0) * HETEROGENEOUS_RATIO {
        return 1;
    }
    let m = smallest.max(1.0);
    let fit = ((inner + gap) / (m + gap)).floor();
    let mut n = if fit.is_finite() && fit >= 1.0 {
        (fit as usize).min(count).min(MAX_COLUMNS)
    } else {
        1
    };
    let widest_min = widths.iter().map(|w| w.min).fold(0.0f32, f32::max);
    while n > 1 && (inner - (n - 1) as f32 * gap) / n as f32 + 0.01 < widest_min {
        n -= 1;
    }
    n.max(1)
}

/// Number of columns of grid container `b` with inner width `inner`.
fn column_count<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    items: &[&'t BoxNode],
    gap: f32,
    inner: f32,
) -> usize {
    match b.style.grid_columns {
        0 => {
            let widths: Vec<Intrinsic> = items.iter().map(|&i| cx.outer_intrinsic(i)).collect();
            auto_columns(&widths, inner, gap)
        }
        n => (n as usize).clamp(1, MAX_COLUMNS),
    }
}

/// Lays out the items of grid container `b` (same contract as
/// [`crate::flex::layout_flex_contents`]).
pub(crate) fn layout_grid_contents<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    cb: Cb,
    y: f32,
) -> ContentLayout {
    let style = &b.style;
    for c in b.children.iter().filter(|c| c.is_absolute()) {
        cx.register_abs(c, Point::new(cb.x, y));
    }
    let mut items: Vec<&'t BoxNode> = in_flow_items(b).collect();
    items.sort_by_key(|c| c.style.order);
    let mut out = ContentLayout::default();
    if items.is_empty() {
        return out;
    }
    let cgap = column_gap(style, cb.width);
    let rgap = row_gap(style, cb.height);
    let n = column_count(cx, b, &items, cgap, cb.width);
    let col_w = sane_size((cb.width - (n - 1) as f32 * cgap) / n as f32);

    let mut row_y = y;
    for (ri, row) in items.chunks(n).enumerate() {
        if ri > 0 {
            row_y += rgap;
        }
        // Measure every item of the row at its width.
        struct Cell<'t> {
            b: &'t BoxNode,
            edges: crate::sizing::BoxEdges,
            cb: Cb,
            w: f32,
            h: f32,
            stretch: bool,
            align: AlignItems,
            laid: Option<item::Laid>,
        }
        let mut cells: Vec<Cell<'t>> = Vec::with_capacity(row.len());
        let mut row_h = 0.0f32;
        for (ci, &it) in row.iter().enumerate() {
            let x = cb.x + ci as f32 * (col_w + cgap);
            let icb = Cb {
                x,
                width: col_w,
                height: None,
            };
            let s = &it.style;
            let edges = box_edges(s, col_w);
            let pb_h = edges.pb_h();
            let avail = col_w - edges.margin.left - edges.margin.right - pb_h;
            let (min_w, max_w) = width_limits(s, Some(col_w), pb_h);
            let w = if it.is_replaced() {
                cx.replaced_size(it, &edges, icb).0
            } else {
                match specified_width(s, Some(col_w), pb_h) {
                    Some(w) => clamp_size(w, min_w, max_w),
                    None => clamp_size(avail, min_w, max_w),
                }
            };
            let align = match s.align_self {
                AlignItems::Auto => style.align_items,
                a => a,
            };
            let stretch = align == AlignItems::Stretch
                && !it.is_replaced()
                && specified_height(s, None, edges.pb_v()).is_none()
                && !edges.auto_top
                && !edges.auto_bottom;
            let (m, laid) = item::measure(
                cx,
                it,
                x + edges.margin.left,
                row_y + edges.margin.top,
                w,
                &edges,
                icb,
            );
            let outer = m.height + edges.pb_v() + edges.margin.top + edges.margin.bottom;
            row_h = row_h.max(outer);
            cells.push(Cell {
                b: it,
                edges,
                cb: icb,
                w,
                h: m.height,
                stretch,
                align,
                laid,
            });
        }
        let row_h = sane_size(row_h);
        for c in cells {
            let e = &c.edges;
            let outer = c.h + e.pb_v() + e.margin.top + e.margin.bottom;
            let (forced, dy) = if c.stretch {
                let (min_h, max_h) = height_limits(&c.b.style, None, e.pb_v());
                let h = clamp_size(
                    row_h - e.margin.top - e.margin.bottom - e.pb_v(),
                    min_h,
                    max_h,
                );
                (Some(h), 0.0)
            } else {
                let dy = match c.align {
                    AlignItems::FlexEnd => row_h - outer,
                    AlignItems::Center => (row_h - outer) / 2.0,
                    _ => 0.0,
                };
                (None, sane(dy))
            };
            let bx = c.cb.x + e.margin.left;
            let by = row_y + e.margin.top + dy;
            let laid = item::finish_item(cx, c.b, c.laid, bx, by, c.w, forced, e, c.cb);
            let item::Laid {
                mut lb, start, end, ..
            } = laid;
            cx.apply_relative_range(&mut lb, c.cb, (start, end));
            out.children.push(lb);
        }
        row_y += row_h;
    }
    out.height = sane_size(row_y - y);
    out
}

/// Min/max-content widths of the content box of grid container `b`.
pub(crate) fn grid_intrinsic<'t>(cx: &mut LayoutCx<'t>, b: &'t BoxNode) -> Intrinsic {
    let style = &b.style;
    let gap = sane_size(style.column_gap.px);
    let widths: Vec<Intrinsic> = in_flow_items(b).map(|i| cx.outer_intrinsic(i)).collect();
    if widths.is_empty() {
        return Intrinsic::default();
    }
    let widest_min = widths.iter().map(|w| w.min).fold(0.0f32, f32::max);
    let widest_max = widths.iter().map(|w| w.max).fold(0.0f32, f32::max);
    match style.grid_columns {
        0 => {
            let sum: f32 = widths.iter().map(|w| w.max).sum();
            let n = widths.len() as f32;
            Intrinsic::new(widest_min, sum + (n - 1.0) * gap)
        }
        n => {
            // Min-content: each column takes the min-content of its own
            // items (fr tracks under a min-content constraint), so nested
            // grids grow linearly. Max-content: `1fr` columns are all as
            // wide as the widest item.
            let n = (n as usize).clamp(1, MAX_COLUMNS);
            let mut col_min = vec![0.0f32; n];
            for (i, w) in widths.iter().enumerate() {
                if let Some(c) = col_min.get_mut(i % n) {
                    *c = c.max(w.min);
                }
            }
            let gaps = (n - 1) as f32 * gap;
            let min = col_min.iter().sum::<f32>().max(widest_min) + gaps;
            Intrinsic::new(min, n as f32 * widest_max + gaps)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(min: f32, max: f32) -> Intrinsic {
        Intrinsic::new(min, max)
    }

    #[test]
    fn auto_column_heuristic() {
        // Four 100px items in 450px with 10px gaps: four columns fit.
        let items = [w(50.0, 100.0); 4];
        assert_eq!(auto_columns(&items, 450.0, 10.0), 4);
        // Only three fit in 330px.
        assert_eq!(auto_columns(&items, 330.0, 10.0), 3);
        // Never more columns than items.
        assert_eq!(auto_columns(&items[..2], 1000.0, 10.0), 2);
        // Min-content must fit in a column.
        let wide_min = [w(300.0, 320.0), w(50.0, 300.0), w(50.0, 300.0)];
        assert_eq!(auto_columns(&wide_min, 700.0, 0.0), 2);
        // Heterogeneous content: a vertical stack.
        let mixed = [w(20.0, 40.0), w(100.0, 900.0)];
        assert_eq!(auto_columns(&mixed, 1000.0, 0.0), 1);
        assert_eq!(auto_columns(&[], 100.0, 0.0), 1);
        // Empty items do not explode the column count.
        assert_eq!(auto_columns(&[w(0.0, 0.0); 5000], 1000.0, 0.0), 1000);
    }
}
