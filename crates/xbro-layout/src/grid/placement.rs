//! Grid item placement (CSS Grid 1 §8, `grid-auto-flow: row`, sparse).
//!
//! * Line numbers count from 1 at the start of the explicit grid;
//!   negative ones from its end (`-1` is its last line, so `1 / -1`
//!   spans every explicit track). Lines that would fall before the
//!   explicit grid are moved to its start (no implicit tracks are created
//!   before it), keeping the item's span.
//! * Items with a definite row and column are placed first, then items
//!   locked to a row (each row's cursor moves past the items placed in it
//!   by that step), then the rest in order-modified document order with
//!   the auto-placement cursor: an item with a definite column goes to the
//!   first row (from the cursor's, or the next one when its column is
//!   before the cursor's) where its area is free; an item without one
//!   goes to the first free area after the cursor, row by row.
//! * The implicit grid has as many columns as the explicit grid, the
//!   definite column lines and the widest auto-placed span need (at least
//!   one, at most [`MAX_TRACKS`]); rows are added as needed.

use xbro_style::{GridLine, GridPlacement};

/// Largest number of columns (and of rows spanned by one item).
pub(crate) const MAX_TRACKS: usize = 1000;

/// Largest row line an item can be placed at.
const MAX_ROW: usize = 20_000;

/// Largest number of occupancy checks of one placement pass: beyond, the
/// remaining auto-placed items go below all the others without searching
/// for holes (hostile pages with huge grids stay linear).
const SEARCH_BUDGET: usize = 4_000_000;

/// Position of an item along one axis.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AxisPos {
    /// Definite start track and span.
    Definite(usize, usize),
    /// Auto-placed with this span.
    Auto(usize),
}

impl AxisPos {
    fn span(self) -> usize {
        match self {
            AxisPos::Definite(_, s) | AxisPos::Auto(s) => s,
        }
    }
}

/// Resolves `p` against an explicit grid of `explicit` tracks; at most
/// `max` tracks (start + span) are used.
pub(crate) fn resolve_axis(p: GridPlacement, explicit: usize, max: usize) -> AxisPos {
    let max = max.max(1) as i64;
    let line = |n: i32| -> i64 {
        if n > 0 {
            i64::from(n) - 1
        } else {
            explicit as i64 + 1 + i64::from(n)
        }
    };
    let span = |n: u32| i64::from(n).clamp(1, max);
    let (s, e) = match (p.start, p.end) {
        (GridLine::Line(a), GridLine::Line(b)) => {
            let (a, b) = (line(a), line(b));
            if a == b {
                (a, a + 1)
            } else {
                (a.min(b), a.max(b))
            }
        }
        (GridLine::Line(a), GridLine::Span(n)) => (line(a), line(a) + span(n)),
        (GridLine::Line(a), GridLine::Auto) => (line(a), line(a) + 1),
        (GridLine::Span(n), GridLine::Line(b)) => (line(b) - span(n), line(b)),
        (GridLine::Auto, GridLine::Line(b)) => (line(b) - 1, line(b)),
        (GridLine::Span(n), _) | (GridLine::Auto, GridLine::Span(n)) => {
            return AxisPos::Auto(span(n) as usize)
        }
        (GridLine::Auto, GridLine::Auto) => return AxisPos::Auto(1),
    };
    let len = (e - s).clamp(1, max);
    let s = s.clamp(0, max - len);
    AxisPos::Definite(s as usize, len as usize)
}

/// The grid area of an item: first column/row and spans.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub col: usize,
    pub col_span: usize,
    pub row: usize,
    pub row_span: usize,
}

/// Result of the placement.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Placement {
    /// Areas of the items, in input order.
    pub areas: Vec<Area>,
    /// Number of columns and rows of the implicit grid.
    pub cols: usize,
    pub rows: usize,
}

/// Occupied cells: per row, the occupied column ranges.
struct Occupancy {
    rows: Vec<Vec<(usize, usize)>>,
    budget: usize,
}

impl Occupancy {
    /// End of a range overlapping the area, if any (where to look next).
    fn conflict(&mut self, row: usize, col: usize, rspan: usize, cspan: usize) -> Option<usize> {
        let end = col + cspan;
        let rows = self.rows.get(row..(row + rspan).min(self.rows.len()))?;
        let mut next = None;
        for r in rows {
            self.budget = self.budget.saturating_sub(r.len() + 1);
            for &(a, b) in r {
                if a < end && col < b {
                    next = Some(next.map_or(b, |n: usize| n.max(b)));
                }
            }
        }
        next
    }

    fn mark(&mut self, a: Area) {
        let end = a.row + a.row_span;
        if self.rows.len() < end {
            self.rows.resize(end, Vec::new());
        }
        if let Some(rows) = self.rows.get_mut(a.row..end) {
            for r in rows {
                r.push((a.col, a.col + a.col_span));
            }
        }
    }

    fn exhausted(&self) -> bool {
        self.budget == 0
    }
}

/// Places items whose `(column, row)` placements are `items` in a grid
/// with `explicit_cols` × `explicit_rows` explicit tracks.
pub(crate) fn place(
    items: &[(GridPlacement, GridPlacement)],
    explicit_cols: usize,
    explicit_rows: usize,
) -> Placement {
    let resolved: Vec<(AxisPos, AxisPos)> = items
        .iter()
        .map(|&(c, r)| {
            (
                resolve_axis(c, explicit_cols, MAX_TRACKS),
                resolve_axis(r, explicit_rows, MAX_ROW),
            )
        })
        .map(|(c, r)| {
            // A row span never exceeds `MAX_TRACKS`.
            let r = match r {
                AxisPos::Definite(s, n) => AxisPos::Definite(s, n.min(MAX_TRACKS)),
                AxisPos::Auto(n) => AxisPos::Auto(n.min(MAX_TRACKS)),
            };
            (c, r)
        })
        .collect();
    // Columns of the implicit grid.
    let mut cols = explicit_cols.max(1);
    for (c, _) in &resolved {
        cols = cols.max(match *c {
            AxisPos::Definite(s, n) => s + n,
            AxisPos::Auto(n) => n,
        });
    }
    let cols = cols.min(MAX_TRACKS);
    let mut occ = Occupancy {
        rows: Vec::new(),
        budget: SEARCH_BUDGET,
    };
    let mut areas: Vec<Option<Area>> = vec![None; resolved.len()];
    // 1. Definite rows and columns.
    for (i, (c, r)) in resolved.iter().enumerate() {
        if let (AxisPos::Definite(col, cs), AxisPos::Definite(row, rs)) = (*c, *r) {
            let a = Area {
                col,
                col_span: cs,
                row,
                row_span: rs,
            };
            occ.mark(a);
            if let Some(slot) = areas.get_mut(i) {
                *slot = Some(a);
            }
        }
    }
    // 2. Items locked to a row.
    let mut row_cursor: std::collections::HashMap<usize, usize> = Default::default();
    for (i, (c, r)) in resolved.iter().enumerate() {
        let (AxisPos::Auto(cs), AxisPos::Definite(row, rs)) = (*c, *r) else {
            continue;
        };
        let cs = cs.min(cols);
        let mut col = row_cursor.get(&row).copied().unwrap_or(0);
        loop {
            if col + cs > cols || occ.exhausted() {
                // No room left in the row: after its last item.
                col = col.min(cols - cs);
                break;
            }
            match occ.conflict(row, col, rs, cs) {
                Some(next) => col = next.max(col + 1),
                None => break,
            }
        }
        let a = Area {
            col,
            col_span: cs,
            row,
            row_span: rs,
        };
        occ.mark(a);
        row_cursor.insert(row, col + cs);
        if let Some(slot) = areas.get_mut(i) {
            *slot = Some(a);
        }
    }
    // 4. The others, with the auto-placement cursor.
    let (mut cur_row, mut cur_col) = (0usize, 0usize);
    for (i, (c, r)) in resolved.iter().enumerate() {
        if areas.get(i).is_some_and(Option::is_some) {
            continue;
        }
        let rs = r.span();
        let a = if occ.exhausted() {
            // Out of budget: below everything placed so far.
            let row = occ.rows.len().max(cur_row);
            let (col, cs) = match *c {
                AxisPos::Definite(s, n) => (s, n),
                AxisPos::Auto(n) => (0, n.min(cols)),
            };
            Area {
                col,
                col_span: cs,
                row,
                row_span: rs,
            }
        } else {
            match *c {
                AxisPos::Definite(col, cs) => {
                    if col < cur_col {
                        cur_row += 1;
                    }
                    let mut row = cur_row;
                    while occ.conflict(row, col, rs, cs).is_some() && !occ.exhausted() {
                        row += 1;
                    }
                    cur_row = row;
                    cur_col = col;
                    Area {
                        col,
                        col_span: cs,
                        row,
                        row_span: rs,
                    }
                }
                AxisPos::Auto(cs) => {
                    let cs = cs.min(cols);
                    let (mut row, mut col) = (cur_row, cur_col);
                    loop {
                        if col + cs > cols {
                            row += 1;
                            col = 0;
                            continue;
                        }
                        if occ.exhausted() {
                            break;
                        }
                        match occ.conflict(row, col, rs, cs) {
                            Some(next) => col = next.max(col + 1),
                            None => break,
                        }
                    }
                    cur_row = row;
                    cur_col = col + cs;
                    Area {
                        col,
                        col_span: cs,
                        row,
                        row_span: rs,
                    }
                }
            }
        };
        occ.mark(a);
        if let Some(slot) = areas.get_mut(i) {
            *slot = Some(a);
        }
    }
    let areas: Vec<Area> = areas
        .into_iter()
        .map(|a| {
            a.unwrap_or(Area {
                col: 0,
                col_span: 1,
                row: 0,
                row_span: 1,
            })
        })
        .collect();
    let rows = areas
        .iter()
        .map(|a| a.row + a.row_span)
        .fold(explicit_rows, usize::max);
    Placement { areas, cols, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gp(start: GridLine, end: GridLine) -> GridPlacement {
        GridPlacement { start, end }
    }

    const AUTO: GridPlacement = GridPlacement::AUTO;
    use GridLine::{Auto, Line, Span};

    fn cells(p: &Placement) -> Vec<(usize, usize, usize, usize)> {
        p.areas
            .iter()
            .map(|a| (a.row, a.col, a.row_span, a.col_span))
            .collect()
    }

    #[test]
    fn line_resolution() {
        assert_eq!(resolve_axis(AUTO, 3, 1000), AxisPos::Auto(1));
        assert_eq!(resolve_axis(gp(Span(2), Auto), 3, 1000), AxisPos::Auto(2));
        assert_eq!(resolve_axis(gp(Auto, Span(3)), 3, 1000), AxisPos::Auto(3));
        // `1 / -1` spans the explicit grid.
        assert_eq!(
            resolve_axis(gp(Line(1), Line(-1)), 3, 1000),
            AxisPos::Definite(0, 3)
        );
        assert_eq!(
            resolve_axis(gp(Line(2), Auto), 3, 1000),
            AxisPos::Definite(1, 1)
        );
        assert_eq!(
            resolve_axis(gp(Line(2), Span(2)), 3, 1000),
            AxisPos::Definite(1, 2)
        );
        assert_eq!(
            resolve_axis(gp(Span(2), Line(-1)), 3, 1000),
            AxisPos::Definite(1, 2)
        );
        assert_eq!(
            resolve_axis(gp(Auto, Line(3)), 3, 1000),
            AxisPos::Definite(1, 1)
        );
        // Swapped and equal lines.
        assert_eq!(
            resolve_axis(gp(Line(3), Line(1)), 3, 1000),
            AxisPos::Definite(0, 2)
        );
        assert_eq!(
            resolve_axis(gp(Line(2), Line(2)), 3, 1000),
            AxisPos::Definite(1, 1)
        );
        // Before the explicit grid: moved to its start.
        assert_eq!(
            resolve_axis(gp(Line(-10), Auto), 3, 1000),
            AxisPos::Definite(0, 1)
        );
        // Beyond: implicit tracks, within the limit.
        assert_eq!(
            resolve_axis(gp(Line(5), Auto), 3, 1000),
            AxisPos::Definite(4, 1)
        );
        assert_eq!(
            resolve_axis(gp(Line(10_000), Auto), 3, 1000),
            AxisPos::Definite(999, 1)
        );
        assert_eq!(
            resolve_axis(gp(Line(1), Span(5000)), 3, 1000),
            AxisPos::Definite(0, 1000)
        );
    }

    #[test]
    fn auto_placement_row_major_with_spans() {
        let items = [
            (gp(Span(2), Auto), AUTO),
            (AUTO, AUTO),
            (AUTO, AUTO),
            (gp(Line(1), Line(-1)), AUTO),
            (AUTO, gp(Span(2), Auto)),
            (AUTO, AUTO),
        ];
        let p = place(&items, 3, 0);
        assert_eq!((p.cols, p.rows), (3, 5));
        assert_eq!(
            cells(&p),
            vec![
                (0, 0, 1, 2),
                (0, 2, 1, 1),
                (1, 0, 1, 1),
                (2, 0, 1, 3),
                (3, 0, 2, 1),
                (3, 1, 1, 1),
            ]
        );
    }

    #[test]
    fn definite_positions_first_then_holes() {
        let items = [
            (AUTO, AUTO),
            (gp(Line(2), Auto), gp(Line(1), Auto)),
            (AUTO, gp(Line(2), Auto)),
            (AUTO, AUTO),
            (gp(Line(1), Auto), AUTO),
        ];
        let p = place(&items, 2, 0);
        assert_eq!(
            cells(&p),
            vec![
                (0, 0, 1, 1),
                (0, 1, 1, 1),
                (1, 0, 1, 1),
                (1, 1, 1, 1),
                (2, 0, 1, 1),
            ]
        );
        // Without columns: one column, a stack.
        let p = place(&[(AUTO, AUTO); 3], 0, 0);
        assert_eq!((p.cols, p.rows), (1, 3));
        // A span wider than the grid widens it.
        let p = place(&[(gp(Span(4), Auto), AUTO)], 2, 0);
        assert_eq!(p.cols, 4);
        // Explicit rows count even when empty.
        assert_eq!(place(&[], 0, 3).rows, 3);
    }

    #[test]
    fn hostile_grids_stay_bounded() {
        let items = vec![(gp(Span(999), Auto), gp(Span(1000), Auto)); 300];
        let p = place(&items, 1000, 0);
        assert_eq!(p.areas.len(), 300);
        assert!(p.cols <= MAX_TRACKS);
        let items = vec![(gp(Line(i32::MAX), Auto), gp(Line(i32::MIN), Line(i32::MAX))); 10];
        let p = place(&items, 0, 0);
        assert!(p.cols <= MAX_TRACKS && p.rows <= MAX_ROW + MAX_TRACKS);
    }
}
