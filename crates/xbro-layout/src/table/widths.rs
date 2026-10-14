//! Column widths: automatic table layout (CSS 2.1 §17.5.2.2 as refined by
//! the CSS Tables 3 draft and browsers) and fixed table layout (§17.5.2.1).
//!
//! # Automatic layout
//!
//! 1. Every cell gives min/max-content *border-box* widths. A cell with a
//!    fixed `width` gets `max = max(min, width)` (its column becomes
//!    "fixed"); a percentage `width` makes its column a percentage column.
//! 2. Single-column cells set their column's min/max (largest wins), then
//!    `<col>` hints apply, then cells spanning several columns (narrowest
//!    span first) distribute their excess min/max widths over the spanned
//!    columns in proportion to the columns' max-content widths (equally
//!    when those are all zero), and their excess percentage to the
//!    spanned non-percentage columns.
//! 3. Distribution of a used grid width `W` ([`distribute`]): four guesses
//!    are made — all columns at min; percentage columns at their
//!    percentage of `W`; fixed columns at their width too; every column
//!    at its max — and the used widths interpolate linearly between the
//!    two consecutive guesses around `W`. Beyond the max guess the extra
//!    width goes to auto columns (proportionally to max-content), else to
//!    fixed columns, else to percentage columns. Below the min guess
//!    columns keep their min (the table overflows).
//!
//! # Fixed layout
//!
//! Widths come from `<col>` elements and the cells of the first row
//! (border-box widths; percentages of the grid width); the other columns
//! share the remaining space equally; if no column is left, extra space is
//! spread over all columns proportionally.

use xbro_style::{BoxSizing, ComputedStyle, LengthPercentageAuto};

use super::structure::Grid;
use crate::box_tree::BoxNode;
use crate::cx::LayoutCx;
use crate::sizing::{sane, sane_size, width_limits, EPSILON};

/// Stand-in for an "infinite" max-content width (percentage columns adding
/// up to 100% next to other content make a table as wide as possible).
pub(crate) const HUGE_WIDTH: f32 = 1.0e6;

/// Width constraints of one column (border-box widths of its cells).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct Column {
    pub min: f32,
    pub max: f32,
    /// Some cell (or `<col>`) gives the column a fixed width.
    pub fixed: bool,
    /// Percentage width (0 = none).
    pub percent: f32,
}

/// Width constraints of one cell.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
struct CellWidths {
    min: f32,
    max: f32,
    fixed: bool,
    percent: f32,
}

/// A `width` value as a fixed border-box width or a percentage.
enum WidthHint {
    Auto,
    Fixed(f32),
    Percent(f32),
}

fn width_hint(style: &ComputedStyle, width: &LengthPercentageAuto, pb: f32) -> WidthHint {
    match width {
        LengthPercentageAuto::Length(l) if l.has_percent() => {
            if l.px == 0.0 && l.percent.is_finite() && l.percent > 0.0 {
                WidthHint::Percent(l.percent.min(100.0))
            } else {
                // `calc()` mixes are treated as auto.
                WidthHint::Auto
            }
        }
        LengthPercentageAuto::Length(l) => {
            let v = sane_size(l.px);
            WidthHint::Fixed(match style.box_sizing {
                BoxSizing::ContentBox => v + pb,
                BoxSizing::BorderBox => v.max(pb),
            })
        }
        LengthPercentageAuto::Auto => WidthHint::Auto,
    }
}

/// Min/max-content border-box widths of a cell whose horizontal padding
/// and used borders add up to `pb`.
fn cell_widths<'t>(cx: &mut LayoutCx<'t>, cell: &'t BoxNode, pb: f32) -> CellWidths {
    let style = &cell.style;
    let content = cx.content_intrinsic(cell);
    let (min_w, max_w) = width_limits(style, None, pb);
    let mut min = content.min.max(min_w) + pb;
    let mut max = content.max.max(min_w) + pb;
    if max_w.is_finite() {
        min = min.min(max_w + pb).max(min_w + pb);
        max = max.min(max_w + pb);
    }
    let mut w = CellWidths {
        min,
        max,
        fixed: false,
        percent: 0.0,
    };
    match width_hint(style, &style.width, pb) {
        WidthHint::Fixed(f) => {
            w.fixed = true;
            w.max = f;
        }
        WidthHint::Percent(p) => w.percent = p,
        WidthHint::Auto => {}
    }
    w.min = sane_size(w.min);
    w.max = sane_size(w.max).max(w.min);
    w
}

/// Adds `excess` to the values selected by `field` of `cols`, in
/// proportion to the columns' max-content widths (equally if all zero).
fn spread(cols: &mut [Column], excess: f32, field: fn(&mut Column) -> &mut f32) {
    if cols.is_empty() || excess <= 0.0 || !excess.is_finite() {
        return;
    }
    let total: f32 = cols.iter().map(|c| c.max).sum();
    let n = cols.len() as f32;
    for c in cols.iter_mut() {
        let share = if total > 0.0 {
            excess * c.max / total
        } else {
            excess / n
        };
        *field(c) += share;
    }
}

/// Column constraints of the grid (automatic layout). `inner` is the
/// horizontal spacing between two columns; `pb` the horizontal padding
/// plus used borders of each cell (indexed like `grid.cells`).
pub(crate) fn columns<'t>(
    cx: &mut LayoutCx<'t>,
    grid: &Grid<'t>,
    table: &'t BoxNode,
    inner: f32,
    pb: &[f32],
) -> Vec<Column> {
    let n = grid.ncols;
    let mut cols = vec![Column::default(); n];
    let mut spanning = Vec::new();
    for (i, cell) in grid.cells.iter().enumerate() {
        let w = cell_widths(cx, cell.b, pb.get(i).copied().unwrap_or(0.0));
        if cell.colspan > 1 {
            spanning.push((cell.col, cell.colspan, w));
        } else if let Some(c) = cols.get_mut(cell.col) {
            c.min = c.min.max(w.min);
            c.max = c.max.max(w.max);
            c.fixed |= w.fixed;
            c.percent = c.percent.max(w.percent);
        }
    }
    // `<col>` / `<colgroup>` hints.
    let mut idx = 0usize;
    'cols: for spec in &table.cols {
        for _ in 0..spec.span {
            let Some(c) = cols.get_mut(idx) else {
                break 'cols;
            };
            match width_hint(&table.style, &spec.width, 0.0) {
                WidthHint::Fixed(f) => {
                    c.fixed = true;
                    c.max = c.max.max(f);
                }
                WidthHint::Percent(p) => c.percent = c.percent.max(p),
                WidthHint::Auto => {}
            }
            idx += 1;
        }
    }
    for c in &mut cols {
        c.max = c.max.max(c.min);
    }
    // Spanning cells, narrowest spans first.
    spanning.sort_by_key(|&(_, span, _)| span);
    for (col, span, w) in spanning {
        let end = (col + span).min(n);
        let Some(range) = cols.get_mut(col..end) else {
            continue;
        };
        if range.is_empty() {
            continue;
        }
        let inside = inner * (range.len() - 1) as f32;
        let cur_min: f32 = range.iter().map(|c| c.min).sum::<f32>() + inside;
        spread(range, w.min - cur_min, |c| &mut c.min);
        for c in range.iter_mut() {
            c.max = c.max.max(c.min);
        }
        let cur_max: f32 = range.iter().map(|c| c.max).sum::<f32>() + inside;
        spread(range, w.max - cur_max, |c| &mut c.max);
        if w.percent > 0.0 {
            let cur: f32 = range.iter().map(|c| c.percent).sum();
            let extra = w.percent - cur;
            if extra > 0.0 {
                let free: Vec<usize> = (0..range.len())
                    .filter(|&i| range.get(i).is_some_and(|c| c.percent == 0.0))
                    .collect();
                let total: f32 = free
                    .iter()
                    .filter_map(|&i| range.get(i))
                    .map(|c| c.max)
                    .sum();
                let k = free.len() as f32;
                for &i in &free {
                    if let Some(c) = range.get_mut(i) {
                        c.percent = if total > 0.0 {
                            extra * c.max / total
                        } else {
                            extra / k
                        };
                    }
                }
            }
        }
    }
    // Percentages never add up to more than 100%.
    let mut used = 0.0f32;
    for c in &mut cols {
        let p = c.percent.min(100.0 - used).max(0.0);
        c.percent = p;
        used += p;
        c.min = sane_size(c.min);
        c.max = sane_size(c.max).max(c.min);
    }
    cols
}

/// Min-content and max-content widths of the columns together (spacing
/// excluded). Percentage columns widen the max-content width so that they
/// can get their percentage.
pub(crate) fn grid_min_max(cols: &[Column]) -> (f32, f32) {
    let min: f32 = cols.iter().map(|c| c.min).sum();
    let mut max: f32 = cols.iter().map(|c| c.max).sum();
    let pct: f32 = cols.iter().map(|c| c.percent).sum();
    if pct > 0.0 {
        for c in cols.iter().filter(|c| c.percent > 0.0) {
            max = max.max(c.max * 100.0 / c.percent);
        }
        let others: f32 = cols
            .iter()
            .filter(|c| c.percent == 0.0)
            .map(|c| c.max)
            .sum();
        if pct < 100.0 - EPSILON {
            max = max.max(others * 100.0 / (100.0 - pct));
        } else if others > 0.0 {
            max = max.max(HUGE_WIDTH);
        }
    }
    let min = sane_size(min);
    (min, sane_size(max.min(HUGE_WIDTH.max(min))).max(min))
}

/// Linear interpolation between two guesses `a` and `b` (totals `ta`, `tb`)
/// for the target total `t`.
fn interpolate(a: &[f32], b: &[f32], ta: f32, tb: f32, t: f32) -> Vec<f32> {
    let f = if tb - ta > EPSILON {
        ((t - ta) / (tb - ta)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    a.iter().zip(b).map(|(x, y)| x + (y - x) * f).collect()
}

/// Used widths of the columns for a grid width `target` (spacing excluded).
pub(crate) fn distribute(cols: &[Column], target: f32) -> Vec<f32> {
    if cols.is_empty() {
        return Vec::new();
    }
    let target = sane_size(target);
    let g_min: Vec<f32> = cols.iter().map(|c| c.min).collect();
    let g_pct: Vec<f32> = cols
        .iter()
        .map(|c| {
            if c.percent > 0.0 {
                c.min.max(c.percent * target / 100.0)
            } else {
                c.min
            }
        })
        .collect();
    let g_spec: Vec<f32> = cols
        .iter()
        .zip(&g_pct)
        .map(|(c, p)| {
            if c.percent > 0.0 {
                *p
            } else if c.fixed {
                c.max
            } else {
                c.min
            }
        })
        .collect();
    let g_max: Vec<f32> = cols
        .iter()
        .zip(&g_pct)
        .map(|(c, p)| if c.percent > 0.0 { *p } else { c.max })
        .collect();
    let total = |g: &[f32]| g.iter().sum::<f32>();
    let (t1, t2, t3, t4) = (total(&g_min), total(&g_pct), total(&g_spec), total(&g_max));
    let out = if target <= t1 {
        g_min
    } else if target <= t2 {
        interpolate(&g_min, &g_pct, t1, t2, target)
    } else if target <= t3 {
        interpolate(&g_pct, &g_spec, t2, t3, target)
    } else if target <= t4 {
        interpolate(&g_spec, &g_max, t3, t4, target)
    } else {
        let extra = target - t4;
        let mut w = g_max;
        let auto: Vec<usize> = (0..cols.len())
            .filter(|&i| cols.get(i).is_some_and(|c| !c.fixed && c.percent == 0.0))
            .collect();
        let fixed: Vec<usize> = (0..cols.len())
            .filter(|&i| cols.get(i).is_some_and(|c| c.fixed && c.percent == 0.0))
            .collect();
        let pct: Vec<usize> = (0..cols.len())
            .filter(|&i| cols.get(i).is_some_and(|c| c.percent > 0.0))
            .collect();
        let all: Vec<usize> = (0..cols.len()).collect();
        let (set, weight): (&[usize], fn(&Column) -> f32) = if !auto.is_empty() {
            (&auto, |c| c.max)
        } else if !fixed.is_empty() {
            (&fixed, |c| c.max)
        } else if !pct.is_empty() {
            (&pct, |c| c.percent)
        } else {
            (&all, |c| c.max)
        };
        let sum: f32 = set.iter().filter_map(|&i| cols.get(i)).map(weight).sum();
        let k = set.len().max(1) as f32;
        for &i in set {
            let share = match cols.get(i) {
                Some(c) if sum > 0.0 => extra * weight(c) / sum,
                _ => extra / k,
            };
            if let Some(v) = w.get_mut(i) {
                *v += share;
            }
        }
        w
    };
    out.into_iter().map(sane_size).collect()
}

/// Column widths with `table-layout: fixed` for a grid width `target`
/// (spacing excluded; `inner` is the spacing between columns, `pb` the
/// horizontal padding plus used borders of each cell).
pub(crate) fn fixed_widths(
    grid: &Grid<'_>,
    table: &BoxNode,
    target: f32,
    inner: f32,
    pb: &[f32],
) -> Vec<f32> {
    let n = grid.ncols;
    let target = sane_size(target);
    let mut widths: Vec<Option<f32>> = vec![None; n];
    let resolve = |hint: WidthHint| match hint {
        WidthHint::Fixed(f) => Some(f),
        WidthHint::Percent(p) => Some(sane_size(p * target / 100.0)),
        WidthHint::Auto => None,
    };
    let mut idx = 0usize;
    'cols: for spec in &table.cols {
        for _ in 0..spec.span {
            let Some(slot) = widths.get_mut(idx) else {
                break 'cols;
            };
            *slot = resolve(width_hint(&table.style, &spec.width, 0.0));
            idx += 1;
        }
    }
    if let Some(first) = grid.rows.first() {
        for (i, cell) in grid
            .cells
            .iter()
            .enumerate()
            .skip(first.cells.start)
            .take(first.cells.len())
        {
            let end = (cell.col + cell.colspan).min(n);
            let Some(slots) = widths.get_mut(cell.col..end) else {
                continue;
            };
            if slots.is_empty() || slots.iter().any(Option::is_some) {
                continue;
            }
            let style = &cell.b.style;
            let pb = pb.get(i).copied().unwrap_or(0.0);
            if let Some(w) = resolve(width_hint(style, &style.width, pb)) {
                let k = slots.len() as f32;
                let each = sane_size((w - inner * (k - 1.0)) / k);
                for s in slots {
                    *s = Some(each);
                }
            }
        }
    }
    let assigned: f32 = widths.iter().flatten().sum();
    let free = widths.iter().filter(|w| w.is_none()).count();
    let remaining = target - assigned;
    let mut out: Vec<f32> = if free > 0 {
        let each = sane_size(remaining / free as f32);
        widths.iter().map(|w| w.unwrap_or(each)).collect()
    } else {
        widths.iter().map(|w| w.unwrap_or(0.0)).collect()
    };
    if free == 0 && remaining > EPSILON && n > 0 {
        let sum: f32 = out.iter().sum();
        for w in &mut out {
            *w += if sum > 0.0 {
                remaining * *w / sum
            } else {
                remaining / n as f32
            };
        }
    }
    out.into_iter().map(|w| sane_size(sane(w))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(min: f32, max: f32) -> Column {
        Column {
            min,
            max,
            ..Column::default()
        }
    }

    #[test]
    fn distribution_between_min_and_max() {
        let cols = [col(10.0, 100.0), col(20.0, 50.0)];
        // Enough room: max widths plus extra in proportion to max.
        let w = distribute(&cols, 300.0);
        assert!(
            (w[0] - 200.0).abs() < 0.01 && (w[1] - 100.0).abs() < 0.01,
            "{w:?}"
        );
        // Exactly max.
        assert_eq!(distribute(&cols, 150.0), vec![100.0, 50.0]);
        // Halfway between min (30) and max (150): interpolated.
        let w = distribute(&cols, 90.0);
        assert!(
            (w[0] - 55.0).abs() < 0.01 && (w[1] - 35.0).abs() < 0.01,
            "{w:?}"
        );
        // Too narrow: min widths.
        assert_eq!(distribute(&cols, 10.0), vec![10.0, 20.0]);
        assert!(distribute(&[], 100.0).is_empty());
    }

    #[test]
    fn fixed_and_percentage_columns() {
        let fixed = Column {
            min: 10.0,
            max: 80.0,
            fixed: true,
            percent: 0.0,
        };
        let auto = col(10.0, 400.0);
        // Room for the fixed width but not the auto max: fixed first.
        let w = distribute(&[fixed, auto], 200.0);
        assert!((w[0] - 80.0).abs() < 0.01, "{w:?}");
        // Extra width goes to auto columns only.
        let w = distribute(&[fixed, auto], 1000.0);
        assert!((w[0] - 80.0).abs() < 0.01 && (w[1] - 920.0).abs() < 0.01);
        let pct = Column {
            min: 10.0,
            max: 20.0,
            fixed: false,
            percent: 50.0,
        };
        let w = distribute(&[pct, col(10.0, 20.0)], 400.0);
        assert!((w[0] - 200.0).abs() < 0.01, "{w:?}");
        // Max-content width honours percentages.
        assert_eq!(grid_min_max(&[pct, col(10.0, 100.0)]), (20.0, 200.0));
        let full = Column {
            percent: 100.0,
            ..pct
        };
        assert_eq!(grid_min_max(&[full, col(10.0, 100.0)]).1, HUGE_WIDTH);
    }

    #[test]
    fn spreading_excess() {
        let mut cols = [col(0.0, 30.0), col(0.0, 10.0)];
        spread(&mut cols, 40.0, |c| &mut c.min);
        assert_eq!((cols[0].min, cols[1].min), (30.0, 10.0));
        let mut zero = [col(0.0, 0.0), col(0.0, 0.0)];
        spread(&mut zero, 10.0, |c| &mut c.max);
        assert_eq!((zero[0].max, zero[1].max), (5.0, 5.0));
    }
}
