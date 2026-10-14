//! Table layout: captions, column positions, row heights, cell placement
//! and vertical alignment.
//!
//! * Captions are laid out as blocks as wide as the table's border box,
//!   above it (`caption-side: top`, [`ContentLayout::offset_top`]) or
//!   below it (`bottom`, [`ContentLayout::offset_bottom`]).
//! * Rows are processed top to bottom; each cell is laid out once, as a
//!   block container at the width of the columns it spans, at the top of
//!   its first row. A row is as tall as its tallest single-row cell (after
//!   baseline alignment), its specified height and the specified heights
//!   of its cells; what a cell spanning rows lacks is spread over the rows
//!   it spans in proportion to their heights.
//! * A specified (or minimum) table height larger than the rows
//!   distributes the extra height to the rows in proportion to their
//!   heights.
//! * Each cell box is then stretched to the full height of its rows (so
//!   backgrounds fill them) and its content is moved per `vertical-align`:
//!   `top`, `middle`, `bottom`, or `baseline` (other values) aligning the
//!   first baselines of the row's baseline cells.
//! * A positioned cell is the containing block of its absolutely
//!   positioned descendants: their layout is deferred until the cell has
//!   its final height and position (cells are never laid out twice, so
//!   nested tables stay linear).
//! * Separated borders: `border-spacing` around and between cells.
//!   Collapsed borders ([`collapse`]): cells abut, each shared border is
//!   the used border of one cell only (so a 1px grid stays one device
//!   pixel at any scale), the outer cells cover the table's border and
//!   padding and paint the outer edges instead of the table box.

use xbro_style::{BorderCollapse, CaptionSide, ComputedStyle, TableLayout, VerticalAlign};

use super::collapse::{self, Collapsed};
use super::structure::{self, Grid};
use super::widths;
use crate::block::{Cb, ContentLayout};
use crate::box_tree::BoxNode;
use crate::cx::{translate_deferred, LayoutCx, PendingAbs};
use crate::geom::{Edges, Rect};
use crate::intrinsic::Intrinsic;
use crate::sizing::{box_edges, height_limits, sane, sane_size, specified_height, BoxEdges};
use crate::LayoutBox;

/// Spacing around and between cells (negative values make cells overlap).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct Spacing {
    /// Between two columns / two rows.
    pub inner_h: f32,
    pub inner_v: f32,
    /// Between the content edge of the table and the outer cells.
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Spacing {
    /// Spacing of table `style` with grid `grid` and used edges `edges`.
    pub fn new(style: &ComputedStyle, grid: &Grid<'_>, edges: &BoxEdges) -> Self {
        if style.border_collapse == BorderCollapse::Separate {
            let h = sane_size(style.border_spacing.0);
            let v = sane_size(style.border_spacing.1);
            return Spacing {
                inner_h: h,
                inner_v: v,
                left: h,
                right: h,
                top: v,
                bottom: v,
            };
        }
        // Collapsed borders: cells abut and cover the table's border and
        // padding (see [`collapse`]). An empty table keeps its own border.
        if grid.cells.is_empty() {
            return Spacing::default();
        }
        let p = &edges.padding;
        let b = &edges.border;
        Spacing {
            inner_h: 0.0,
            inner_v: 0.0,
            left: -(p.left + b.left),
            right: -(p.right + b.right),
            top: -(p.top + b.top),
            bottom: -(p.bottom + b.bottom),
        }
    }

    /// Total horizontal spacing for `n` columns.
    pub fn total_h(&self, n: usize) -> f32 {
        if n == 0 {
            0.0
        } else {
            self.left + self.right + self.inner_h * (n - 1) as f32
        }
    }

    /// Total vertical spacing for `n` rows.
    pub fn total_v(&self, n: usize) -> f32 {
        if n == 0 {
            0.0
        } else {
            self.top + self.bottom + self.inner_v * (n - 1) as f32
        }
    }
}

/// True when `table-layout: fixed` applies: the table has a specified
/// width (CSS 2.1 §17.5.2).
pub(crate) fn is_fixed_layout(b: &BoxNode) -> bool {
    b.style.table_layout == TableLayout::Fixed && !b.style.width.is_auto()
}

/// Collapsed borders of the cells, when the table collapses them.
fn collapsed_borders(grid: &Grid<'_>, style: &ComputedStyle) -> Option<Collapsed> {
    (style.border_collapse == BorderCollapse::Collapse && !grid.cells.is_empty())
        .then(|| collapse::resolve(grid, style))
}

/// Used edges of every cell (indexed like `grid.cells`): no margins,
/// padding percentages against `width`, collapsed borders when resolved.
fn cell_edges(grid: &Grid<'_>, collapsed: Option<&Collapsed>, width: f32) -> Vec<BoxEdges> {
    grid.cells
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut e = box_edges(&c.b.style, width);
            e.margin = Edges::ZERO;
            if let Some(b) = collapsed.and_then(|col| col.edges.get(i)) {
                e.border = *b;
            }
            e
        })
        .collect()
}

/// Column widths of the grid for a grid width `target` (`pb`: horizontal
/// padding and borders of each cell).
fn column_widths<'t>(
    cx: &mut LayoutCx<'t>,
    grid: &Grid<'t>,
    b: &'t BoxNode,
    sp: &Spacing,
    target: f32,
    pb: &[f32],
) -> Vec<f32> {
    if is_fixed_layout(b) {
        widths::fixed_widths(grid, b, target, sp.inner_h, pb)
    } else {
        let cols = widths::columns(cx, grid, b, sp.inner_h, pb);
        widths::distribute(&cols, target)
    }
}

/// Min/max-content widths of the content box of table `b`.
pub(crate) fn table_intrinsic<'t>(cx: &mut LayoutCx<'t>, b: &'t BoxNode) -> Intrinsic {
    let grid = structure::build(cx.doc, b);
    let edges = box_edges(&b.style, 0.0);
    let sp = Spacing::new(&b.style, &grid, &edges);
    let spacing = sp.total_h(grid.ncols);
    let collapsed = collapsed_borders(&grid, &b.style);
    let pb: Vec<f32> = cell_edges(&grid, collapsed.as_ref(), 0.0)
        .iter()
        .map(BoxEdges::pb_h)
        .collect();
    let (min, max) = if is_fixed_layout(b) {
        let w: f32 = widths::fixed_widths(&grid, b, 0.0, sp.inner_h, &pb)
            .iter()
            .sum();
        (w, w)
    } else {
        let cols = widths::columns(cx, &grid, b, sp.inner_h, &pb);
        widths::grid_min_max(&cols)
    };
    let mut min = min + spacing;
    let max = max + spacing;
    // Captions are as wide as the table: their min-content widens it.
    for &cap in &grid.captions {
        let o = cx.outer_intrinsic(cap);
        min = min.max(o.min - edges.pb_h());
    }
    Intrinsic::new(min, max.max(min))
}

/// Min/max-content widths of a row group or row measured on its own
/// (only reachable for boxes outside a table, which box construction
/// never produces): its cells side by side.
pub(crate) fn table_part_intrinsic<'t>(cx: &mut LayoutCx<'t>, b: &'t BoxNode) -> Intrinsic {
    let rows: Vec<&'t BoxNode> = match b.kind {
        crate::box_tree::Kind::TableRow => vec![b],
        _ => b.children.iter().collect(),
    };
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    for r in rows {
        let mut rmin = 0.0;
        let mut rmax = 0.0;
        for cell in &r.children {
            let o = cx.outer_intrinsic(cell);
            rmin += o.min;
            rmax += o.max;
        }
        min = min.max(rmin);
        max = max.max(rmax);
    }
    Intrinsic::new(min, max)
}

/// How a cell's content is aligned vertically.
#[derive(Copy, Clone, Debug, PartialEq)]
enum CellAlign {
    Top,
    Middle,
    Bottom,
    Baseline,
}

fn cell_align(style: &ComputedStyle) -> CellAlign {
    match style.vertical_align {
        VerticalAlign::Top => CellAlign::Top,
        VerticalAlign::Middle => CellAlign::Middle,
        VerticalAlign::Bottom => CellAlign::Bottom,
        _ => CellAlign::Baseline,
    }
}

/// A cell laid out at the top of its first row.
struct LaidCell<'t> {
    lb: LayoutBox,
    /// Pending-box marks of its layout.
    start: u64,
    end: u64,
    /// Absolutely positioned boxes whose containing block is the cell
    /// (positioned cells), laid out once the cell has its final height.
    deferred: Vec<PendingAbs<'t>>,
    /// Border-box height as laid out (content height).
    h: f32,
    /// Border-box height the cell needs: its content, or its specified
    /// height if larger.
    need: f32,
    align: CellAlign,
    /// First baseline from the border-box top (baseline cells).
    baseline: f32,
    /// Content shift for baseline alignment.
    shift: f32,
}

/// Largest number of (row, spanning cell) pairs examined by
/// [`covered_edges`].
const COVER_BUDGET: usize = 1 << 20;

/// For each row, how many leading and trailing columns are covered by
/// cells spanning down from earlier rows. Row boxes leave those columns
/// out so that their backgrounds do not paint over such cells (browsers
/// paint a spanning cell over the background of its first row only).
fn covered_edges(grid: &Grid<'_>) -> Vec<(usize, usize)> {
    let nrows = grid.rows.len();
    let mut out = vec![(0, 0); nrows];
    let mut per_row: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nrows];
    let mut budget = COVER_BUDGET;
    for c in grid.cells.iter().filter(|c| c.rowspan > 1) {
        let end = (c.row + c.rowspan).min(nrows);
        for r in c.row + 1..end {
            if budget == 0 {
                return out;
            }
            budget -= 1;
            if let Some(v) = per_row.get_mut(r) {
                v.push((c.col, c.col + c.colspan));
            }
        }
    }
    let n = grid.ncols;
    for (r, spans) in per_row.iter_mut().enumerate() {
        if spans.is_empty() {
            continue;
        }
        spans.sort_unstable();
        let mut lead = 0;
        for &(a, b) in spans.iter() {
            if a <= lead {
                lead = lead.max(b);
            }
        }
        // Extend the trailing run leftwards: with the spans by decreasing
        // end, each one either touches the run (and extends it) or ends
        // before it, as do all the following ones.
        spans.sort_unstable_by_key(|y| std::cmp::Reverse(y.1));
        let mut trail_start = n;
        for &(a, b) in spans.iter() {
            if b < trail_start {
                break;
            }
            trail_start = trail_start.min(a);
        }
        let lead = lead.min(n);
        let trail = n.saturating_sub(trail_start);
        if lead + trail < n {
            if let Some(slot) = out.get_mut(r) {
                *slot = (lead, trail);
            }
        }
    }
    out
}

/// Moves the content (lines and children, not the box) of a laid-out cell,
/// with the static positions of the boxes registered inside it.
fn shift_content(cx: &mut LayoutCx<'_>, c: &mut LaidCell<'_>, dy: f32) {
    if dy == 0.0 || !dy.is_finite() {
        return;
    }
    crate::tree::translate_lines(&mut c.lb.lines, 0.0, dy);
    for child in &mut c.lb.children {
        crate::tree::translate_box(child, 0.0, dy);
    }
    cx.translate_pending_range(c.start, c.end, 0.0, dy);
    translate_deferred(&mut c.deferred, 0.0, dy);
}

/// Moves a laid-out cell (box, content and pending boxes).
fn move_cell(cx: &mut LayoutCx<'_>, c: &mut LaidCell<'_>, dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    crate::tree::translate_box(&mut c.lb, dx, dy);
    cx.translate_pending_range(c.start, c.end, dx, dy);
    translate_deferred(&mut c.deferred, dx, dy);
}

/// Lays out the contents of table `b` (see the module docs of
/// [`crate::table`]). `edges` are the table's used edges.
pub(crate) fn layout_table_contents<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    cb: Cb,
    y: f32,
    edges: &BoxEdges,
    min_height: f32,
) -> ContentLayout {
    let grid = structure::build(cx.doc, b);
    let sp = Spacing::new(&b.style, &grid, edges);
    let collapsed = collapsed_borders(&grid, &b.style);
    let cell_edges = cell_edges(&grid, collapsed.as_ref(), cb.width);
    let mut out = ContentLayout {
        hide_border: collapsed.as_ref().is_some_and(|c| c.covers_edges),
        ..ContentLayout::default()
    };

    // ---- top captions, above the table box ----
    let bx = cb.x - edges.padding.left - edges.border.left;
    let border_top = y - edges.padding.top - edges.border.top;
    let caption_cb = Cb {
        x: bx,
        width: sane_size(cb.width + edges.pb_h()),
        height: None,
    };
    let (bottom_captions, top_captions): (Vec<&'t BoxNode>, Vec<&'t BoxNode>) = grid
        .captions
        .iter()
        .partition(|c| c.style.caption_side == CaptionSide::Bottom);
    let mut cy = border_top;
    for cap in top_captions {
        let mark = cx.pending_mark();
        let mut lb = cx.layout_box_at(cap, bx, cy, None, None, caption_cb);
        cy = cy.max(lb.margin_box().bottom());
        cx.apply_relative(&mut lb, caption_cb, mark);
        out.children.push(lb);
    }
    let caption_h = sane_size(cy - border_top);
    out.offset_top = caption_h;
    let top = y + caption_h;

    // ---- columns ----
    let n = grid.ncols;
    let grid_w = sane_size(cb.width - sp.total_h(n));
    let pb: Vec<f32> = cell_edges.iter().map(BoxEdges::pb_h).collect();
    let widths = column_widths(cx, &grid, b, &sp, grid_w, &pb);
    let mut col_x = Vec::with_capacity(n);
    let mut x = cb.x + sp.left;
    for w in &widths {
        col_x.push(x);
        x += w + sp.inner_h;
    }
    let grid_left = cb.x + sp.left;
    let grid_width = if n == 0 {
        0.0
    } else {
        sane_size(widths.iter().sum::<f32>() + sp.inner_h * (n - 1) as f32)
    };
    // Prefix sums of the column widths: spanned widths in O(1).
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0f64);
    for w in &widths {
        let last = prefix.last().copied().unwrap_or(0.0);
        prefix.push(last + f64::from(*w));
    }
    let span_width = |col: usize, span: usize| -> f32 {
        let end = (col + span).min(n);
        if end <= col {
            return 0.0;
        }
        let sum = match (prefix.get(end), prefix.get(col)) {
            (Some(e), Some(c)) => (e - c) as f32,
            _ => 0.0,
        };
        sane_size(sum + sp.inner_h * (end - col - 1) as f32)
    };

    // ---- rows ----
    let nrows = grid.rows.len();
    let mut ends_at: Vec<Vec<usize>> = vec![Vec::new(); nrows];
    for (i, c) in grid.cells.iter().enumerate() {
        if c.rowspan > 1 {
            if let Some(v) = ends_at.get_mut(c.row + c.rowspan - 1) {
                v.push(i);
            }
        }
    }
    let cell_cb = Cb {
        x: cb.x,
        width: cb.width,
        height: None,
    };
    let mut laid: Vec<LaidCell<'t>> = Vec::with_capacity(grid.cells.len());
    let mut heights = vec![0.0f32; nrows];
    let mut tops = vec![0.0f32; nrows];
    let mut ry = top + if nrows > 0 { sp.top } else { 0.0 };
    for (r, row) in grid.rows.iter().enumerate() {
        if let Some(t) = tops.get_mut(r) {
            *t = ry;
        }
        let mut h = specified_height(&row.b.style, None, 0.0).unwrap_or(0.0);
        let first = laid.len();
        for (ci, c) in grid
            .cells
            .iter()
            .enumerate()
            .skip(row.cells.start)
            .take(row.cells.len())
        {
            let x = col_x.get(c.col).copied().unwrap_or(grid_left);
            // Cells have no margins.
            let e = cell_edges
                .get(ci)
                .copied()
                .unwrap_or_else(|| box_edges(&c.b.style, cb.width));
            let w = sane_size(span_width(c.col, c.colspan) - e.pb_h());
            let start = cx.pending_mark();
            let (mut lb, deferred) =
                cx.layout_independent_deferring(c.b, x, ry, w, &e, cell_cb, None, true);
            let end = cx.pending_mark();
            // Collapsed borders: paint the winning sides.
            if let Some(st) = collapsed
                .as_ref()
                .and_then(|col| col.styles.get(ci))
                .and_then(Option::as_ref)
            {
                lb.style = st.clone();
            }
            let align = cell_align(&c.b.style);
            let bb = lb.border_box();
            let baseline = crate::tree::first_baseline(&lb)
                .map(|bl| bl - bb.y)
                .unwrap_or(lb.content.bottom() - bb.y);
            // `height` and `min-height` of a cell are minimums of its row.
            let (min_h, _) = height_limits(&c.b.style, None, e.pb_v());
            let spec = specified_height(&c.b.style, None, e.pb_v())
                .unwrap_or(0.0)
                .max(min_h)
                + e.pb_v();
            laid.push(LaidCell {
                h: bb.height,
                need: sane_size(bb.height.max(spec)),
                lb,
                start,
                end,
                deferred,
                align,
                baseline: sane(baseline),
                shift: 0.0,
            });
        }
        // Baseline alignment of the cells starting in this row.
        let cells_here = laid.get_mut(first..).unwrap_or(&mut []);
        let row_baseline = cells_here
            .iter()
            .filter(|c| c.align == CellAlign::Baseline)
            .map(|c| c.baseline)
            .fold(0.0f32, f32::max);
        for (k, c) in cells_here.iter_mut().enumerate() {
            if c.align == CellAlign::Baseline {
                c.shift = sane_size(row_baseline - c.baseline);
            }
            let single = grid
                .cells
                .get(row.cells.start + k)
                .is_none_or(|g| g.rowspan == 1);
            if single {
                h = h.max((c.shift + c.h).max(c.need));
            }
        }
        // Cells spanning down to this row: what they lack is spread over
        // the rows they span in proportion to the rows' heights (equally
        // when those are all zero).
        for &i in ends_at.get(r).map_or(&[][..], |v| v.as_slice()) {
            let (Some(g), Some(c)) = (grid.cells.get(i), laid.get(i)) else {
                continue;
            };
            let above: f32 = heights.get(g.row..r).map_or(0.0, |hs| hs.iter().sum());
            let spanned = above + h + sp.inner_v * (r - g.row) as f32;
            let need = (c.shift + c.h).max(c.need);
            let extra = need - spanned;
            if extra > 0.0 {
                let total = above + h;
                let count = (r - g.row + 1) as f32;
                let share = |row_h: f32| {
                    if total > 0.0 {
                        extra * row_h / total
                    } else {
                        extra / count
                    }
                };
                if let Some(hs) = heights.get_mut(g.row..r) {
                    for rh in hs {
                        *rh = sane_size(*rh + share(*rh));
                    }
                }
                h += share(h);
            }
        }
        let h = sane_size(h);
        if let Some(slot) = heights.get_mut(r) {
            *slot = h;
        }
        ry += h + sp.inner_v;
    }

    // ---- specified (or minimum) table height ----
    let rows_total = heights.iter().sum::<f32>() + sp.total_v(nrows);
    let target = match cb.height {
        Some(h) => Some(h.max(min_height)),
        None => (min_height > 0.0).then_some(min_height),
    };
    if let Some(th) = target {
        let extra = th - rows_total;
        if extra > 0.0 && nrows > 0 {
            let sum: f32 = heights.iter().sum();
            for h in &mut heights {
                *h += if sum > 0.0 {
                    extra * *h / sum
                } else {
                    extra / nrows as f32
                };
            }
        }
    }
    let old_tops = tops.clone();
    let mut ry = top + if nrows > 0 { sp.top } else { 0.0 };
    for (r, t) in tops.iter_mut().enumerate() {
        *t = ry;
        ry += heights.get(r).copied().unwrap_or(0.0) + sp.inner_v;
    }

    // ---- stretch cells, vertical alignment ----
    let mut row_children: Vec<Vec<LayoutBox>> = (0..nrows).map(|_| Vec::new()).collect();
    for (g, mut c) in grid.cells.iter().zip(laid) {
        let r0 = g.row;
        let r1 = (g.row + g.rowspan - 1).min(nrows.saturating_sub(1));
        let t0 = tops.get(r0).copied().unwrap_or(top);
        let dy = t0 - old_tops.get(r0).copied().unwrap_or(t0);
        move_cell(cx, &mut c, 0.0, dy);
        let bottom = tops.get(r1).copied().unwrap_or(t0) + heights.get(r1).copied().unwrap_or(0.0);
        let full = sane_size(bottom - t0);
        let free = sane_size(full - c.h);
        let content_dy = match c.align {
            CellAlign::Top => 0.0,
            CellAlign::Middle => free / 2.0,
            CellAlign::Bottom => free,
            CellAlign::Baseline => c.shift.min(free),
        };
        shift_content(cx, &mut c, content_dy);
        c.lb.content.height += free;
        // Relative offset first: the deferred absolutely positioned boxes
        // are then laid out against the final padding box of the cell.
        let (rdx, rdy) = crate::position::relative_offset(&c.lb.style, cell_cb);
        move_cell(cx, &mut c, rdx, rdy);
        let mut lb = c.lb;
        cx.finish_deferred_abs(&mut lb, c.deferred);
        // A cell spanning rows is a child of the last one so that it paints
        // (and is hit) above the backgrounds of all the rows it covers.
        if let Some(v) = row_children.get_mut(r1) {
            v.push(lb);
        }
    }

    // ---- row and row group boxes ----
    let covered = covered_edges(&grid);
    let mut rows_iter = row_children.into_iter();
    let mut cur_y = top + if nrows > 0 { sp.top } else { 0.0 };
    for group in &grid.groups {
        let mut rows = Vec::with_capacity(group.rows.len());
        let gy = tops.get(group.rows.start).copied().unwrap_or(cur_y);
        let mut gbottom = gy;
        for r in group.rows.clone() {
            let children = rows_iter.next().unwrap_or_default();
            let (Some(row), Some(&t), Some(&h)) = (grid.rows.get(r), tops.get(r), heights.get(r))
            else {
                continue;
            };
            let rect = match covered.get(r) {
                Some(&(lead, trail)) if lead + trail > 0 && lead + trail < n => {
                    let left = col_x.get(lead).copied().unwrap_or(grid_left);
                    let last = n - 1 - trail;
                    let right = col_x.get(last).copied().unwrap_or(grid_left)
                        + widths.get(last).copied().unwrap_or(0.0);
                    Rect::new(left, t, sane_size(right - left), h)
                }
                _ => Rect::new(grid_left, t, grid_width, h),
            };
            rows.push(cx.new_box(row.b, rect, &BoxEdges::default(), children, Vec::new()));
            gbottom = t + h;
            cur_y = t + h + sp.inner_v;
        }
        let rect = Rect::new(grid_left, gy, grid_width, sane_size(gbottom - gy));
        out.children
            .push(cx.new_box(group.b, rect, &BoxEdges::default(), rows, Vec::new()));
    }
    out.height = sane_size(heights.iter().sum::<f32>() + sp.total_v(nrows));

    // ---- bottom captions, below the table box ----
    let border_bottom = top + out.height + edges.padding.bottom + edges.border.bottom;
    let mut cy = border_bottom;
    for cap in bottom_captions {
        let mark = cx.pending_mark();
        let mut lb = cx.layout_box_at(cap, bx, cy, None, None, caption_cb);
        cy = cy.max(lb.margin_box().bottom());
        cx.apply_relative(&mut lb, caption_cb, mark);
        out.children.push(lb);
    }
    out.offset_bottom = sane_size(cy - border_bottom);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_totals() {
        let sp = Spacing {
            inner_h: 2.0,
            inner_v: 3.0,
            left: 2.0,
            right: 2.0,
            top: 3.0,
            bottom: 3.0,
        };
        assert_eq!(sp.total_h(0), 0.0);
        assert_eq!(sp.total_h(3), 8.0);
        assert_eq!(sp.total_v(2), 9.0);
    }
}
