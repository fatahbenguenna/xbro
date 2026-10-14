//! Collapsed borders (`border-collapse: collapse`, CSS 2.1 §17.6.2).
//!
//! Every border between two cells is drawn **once**, by one of them, with
//! the width, style and color of the winning side:
//!
//! * a cell owns (paints) its right and bottom edges when it is on the
//!   last column / row or some slot beyond that edge has no cell (rows of
//!   different lengths: nothing else would paint the part of the edge
//!   facing the empty slots), and its left and top edges unless all the
//!   cells across own that edge (so a shared border is painted twice only
//!   along spans meeting both kinds of neighbours);
//! * the winner among the sides meeting on an edge (the cell's own side,
//!   the facing sides of the cells across the edge, the table's border on
//!   the outer edges): `hidden` suppresses the border, `none` loses, then
//!   the wider side, then the style (`double`, `solid`, `dashed`,
//!   `dotted`, `ridge`, `outset`, `groove`, `inset`), then cells before
//!   the table and the cell further left / up;
//! * the owning cell's used border on that side is the winner's width
//!   (the others are 0), and its style is cloned with the winning sides
//!   when they differ from its own, so paint draws them.
//!
//! Cells abut (no spacing) and the outer cells cover the table's border
//! and padding: the table box does not paint its own border, the outer
//! cells paint the outer edges — unless some slot along the outer edges
//! has no cell ([`Collapsed::covers_edges`]): the table then paints its
//! border too, so that it is not missing next to the empty slots. Row and
//! row group borders are ignored.
//! Lines stay crisp at fractional scales because a 1px border is a single
//! 1px band on a whole CSS px boundary, never two overlapping ones.

use std::sync::Arc;

use xbro_style::{BorderSide, BorderStyle, ComputedStyle};

use super::structure::Grid;
use crate::geom::Edges;
use crate::sizing::sane_size;

/// Largest grid (rows × columns) for which neighbours are looked up;
/// beyond, cells only see their own sides and the table's.
const MAX_SLOTS: usize = 1 << 22;

/// Resolved collapsed borders of the cells of a table.
#[derive(Debug, Default)]
pub(crate) struct Collapsed {
    /// Used border widths of each cell (indexed like `Grid::cells`).
    pub edges: Vec<Edges>,
    /// Style of each cell with its winning border sides, when they differ
    /// from its own.
    pub styles: Vec<Option<Arc<ComputedStyle>>>,
    /// Every slot along the outer edges of the grid is covered by a cell,
    /// so the outer cells paint the whole table border. Otherwise (rows
    /// shorter than others, empty rows) the table box keeps painting its
    /// own border, which would else be missing next to the empty slots.
    pub covers_edges: bool,
}

/// Rank of a border style when widths are equal (higher wins).
fn style_rank(s: BorderStyle) -> u8 {
    match s {
        BorderStyle::Double => 8,
        BorderStyle::Solid => 7,
        BorderStyle::Dashed => 6,
        BorderStyle::Dotted => 5,
        BorderStyle::Ridge => 4,
        BorderStyle::Outset => 3,
        BorderStyle::Groove => 2,
        BorderStyle::Inset => 1,
        BorderStyle::None | BorderStyle::Hidden => 0,
    }
}

/// A side competing for an edge: the side and whether it belongs to a
/// cell (cells win ties against the table).
#[derive(Copy, Clone, Debug)]
struct Candidate {
    side: BorderSide,
    cell: bool,
}

/// The winning side among `cands` (earlier candidates win ties), with a
/// sanitised width; `None` when the edge has no border.
fn winner(cands: &[Candidate]) -> Option<BorderSide> {
    if cands.iter().any(|c| c.side.style == BorderStyle::Hidden) {
        return None;
    }
    let key = |c: &Candidate| (sane_size(c.side.width), style_rank(c.side.style), c.cell);
    let mut best: Option<&Candidate> = None;
    for c in cands {
        if !c.side.style.is_visible() || sane_size(c.side.width) <= 0.0 {
            continue;
        }
        let better = match best {
            None => true,
            Some(b) => {
                let (w, s, cell) = key(c);
                let (bw, bs, bcell) = key(b);
                w > bw || (w == bw && (s > bs || (s == bs && cell && !bcell)))
            }
        };
        if better {
            best = Some(c);
        }
    }
    best.map(|c| BorderSide {
        width: sane_size(c.side.width),
        ..c.side
    })
}

/// Resolves the collapsed borders of the cells of `grid` in a table whose
/// style is `table`.
pub(crate) fn resolve(grid: &Grid<'_>, table: &ComputedStyle) -> Collapsed {
    let nrows = grid.rows.len();
    let ncols = grid.ncols;
    // Slot → index of the cell covering it.
    let slots: Option<Vec<u32>> = nrows
        .checked_mul(ncols)
        .filter(|&n| n <= MAX_SLOTS)
        .map(|n| {
            let mut v = vec![u32::MAX; n];
            let mut budget = MAX_SLOTS;
            for (i, c) in grid.cells.iter().enumerate() {
                for r in c.row..(c.row + c.rowspan).min(nrows) {
                    for col in c.col..(c.col + c.colspan).min(ncols) {
                        if budget == 0 {
                            return v;
                        }
                        budget -= 1;
                        if let Some(s) = v.get_mut(r * ncols + col) {
                            *s = i as u32;
                        }
                    }
                }
            }
            v
        });
    let at = |r: usize, c: usize| -> Option<usize> {
        if r >= nrows || c >= ncols {
            return None;
        }
        let s = slots.as_ref()?.get(r * ncols + c).copied()?;
        (s != u32::MAX).then_some(s as usize)
    };
    let cell_side = |i: usize, f: fn(&ComputedStyle) -> BorderSide| {
        grid.cells.get(i).map(|c| Candidate {
            side: f(&c.b.style),
            cell: true,
        })
    };
    let table_side = |side: BorderSide| Candidate { side, cell: false };
    let covers_edges = slots.is_none()
        || ((0..nrows).all(|r| at(r, 0).is_some() && at(r, ncols.saturating_sub(1)).is_some())
            && (0..ncols).all(|c| at(0, c).is_some() && at(nrows.saturating_sub(1), c).is_some()));
    let mut out = Collapsed {
        edges: Vec::with_capacity(grid.cells.len()),
        styles: Vec::with_capacity(grid.cells.len()),
        covers_edges,
    };
    // Whether each cell owns its right / bottom edge: on the last column
    // / row, or when some slot beyond that edge has no cell (no other
    // cell would paint the part of the edge facing it).
    let owns: Vec<(bool, bool)> = grid
        .cells
        .iter()
        .map(|c| {
            let last_row = (c.row + c.rowspan).min(nrows);
            let last_col = (c.col + c.colspan).min(ncols);
            let right = last_col >= ncols
                || (slots.is_some() && (c.row..last_row).any(|r| at(r, last_col).is_none()));
            let bottom = last_row >= nrows
                || (slots.is_some() && (c.col..last_col).any(|col| at(last_row, col).is_none()));
            (right, bottom)
        })
        .collect();
    let mut cands: Vec<Candidate> = Vec::new();
    for (i, c) in grid.cells.iter().enumerate() {
        let own = &c.b.style.border;
        let mine = |side: BorderSide| Candidate { side, cell: true };
        let last_row = (c.row + c.rowspan).min(nrows);
        let last_col = (c.col + c.colspan).min(ncols);
        let (owns_right, owns_bottom) = owns.get(i).copied().unwrap_or((true, true));
        // The neighbours across an edge (`slots` of the facing column or
        // row; a neighbour spanning several of them is listed once, its
        // slots being consecutive), their facing sides pushed to `cands`.
        // Returns whether all of them own the shared edge (and paint it).
        let neighbours = |cands: &mut Vec<Candidate>,
                          slots: &mut dyn Iterator<Item = (usize, usize)>,
                          side: fn(&ComputedStyle) -> BorderSide,
                          owner: fn((bool, bool)) -> bool| {
            let mut all_own = true;
            let mut last = None;
            for (r, col) in slots {
                match at(r, col).filter(|&n| n != i) {
                    Some(n) => {
                        all_own &= owns.get(n).copied().is_some_and(owner);
                        if last != Some(n) {
                            last = Some(n);
                            cands.extend(cell_side(n, side));
                        }
                    }
                    None => all_own = false,
                }
            }
            all_own
        };
        // Left edge: painted by the cells on the left when they all own
        // their right edge; else by this cell (cells further left win ties).
        let left = if c.col == 0 {
            winner(&[mine(own.left), table_side(table.border.left)])
        } else {
            cands.clear();
            let all_own = neighbours(
                &mut cands,
                &mut (c.row..last_row).map(|r| (r, c.col - 1)),
                |s| s.border.right,
                |o| o.0,
            );
            cands.push(mine(own.left));
            if all_own && slots.is_some() {
                None
            } else {
                winner(&cands)
            }
        };
        // Top edge.
        let top = if c.row == 0 {
            winner(&[mine(own.top), table_side(table.border.top)])
        } else {
            cands.clear();
            let all_own = neighbours(
                &mut cands,
                &mut (c.col..last_col).map(|col| (c.row - 1, col)),
                |s| s.border.bottom,
                |o| o.1,
            );
            cands.push(mine(own.top));
            if all_own && slots.is_some() {
                None
            } else {
                winner(&cands)
            }
        };
        // Right and bottom edges, when owned: this cell's side first (it
        // is further left / up), the cells across, the table last.
        let right = if owns_right {
            cands.clear();
            cands.push(mine(own.right));
            if last_col < ncols {
                neighbours(
                    &mut cands,
                    &mut (c.row..last_row).map(|r| (r, last_col)),
                    |s| s.border.left,
                    |o| o.0,
                );
            } else {
                cands.push(table_side(table.border.right));
            }
            winner(&cands)
        } else {
            None
        };
        let bottom = if owns_bottom {
            cands.clear();
            cands.push(mine(own.bottom));
            if last_row < nrows {
                neighbours(
                    &mut cands,
                    &mut (c.col..last_col).map(|col| (last_row, col)),
                    |s| s.border.top,
                    |o| o.1,
                );
            } else {
                cands.push(table_side(table.border.bottom));
            }
            winner(&cands)
        } else {
            None
        };
        let w = |s: &Option<BorderSide>| s.map_or(0.0, |s| s.width);
        out.edges
            .push(Edges::new(w(&top), w(&right), w(&bottom), w(&left)));
        let differs = |won: &Option<BorderSide>, mine: &BorderSide| {
            won.is_some_and(|s| s.style != mine.style || s.color != mine.color)
        };
        let restyle = differs(&top, &own.top)
            || differs(&right, &own.right)
            || differs(&bottom, &own.bottom)
            || differs(&left, &own.left);
        out.styles.push(restyle.then(|| {
            let mut s = (*c.b.style).clone();
            let set = |dst: &mut BorderSide, won: Option<BorderSide>| {
                if let Some(won) = won {
                    *dst = won;
                }
            };
            set(&mut s.border.top, top);
            set(&mut s.border.right, right);
            set(&mut s.border.bottom, bottom);
            set(&mut s.border.left, left);
            Arc::new(s)
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_style::Color;

    fn side(width: f32, style: BorderStyle, color: Color) -> BorderSide {
        BorderSide {
            width,
            style,
            color,
        }
    }

    fn cell(s: BorderSide) -> Candidate {
        Candidate {
            side: s,
            cell: true,
        }
    }

    #[test]
    fn conflict_resolution() {
        let red = Color::rgb(255, 0, 0);
        let blue = Color::rgb(0, 0, 255);
        let thin = side(1.0, BorderStyle::Solid, red);
        let wide = side(3.0, BorderStyle::Dotted, blue);
        // Wider wins.
        assert_eq!(winner(&[cell(thin), cell(wide)]), Some(wide));
        // `hidden` suppresses, `none` loses.
        let hidden = side(5.0, BorderStyle::Hidden, red);
        assert_eq!(winner(&[cell(wide), cell(hidden)]), None);
        let none = side(5.0, BorderStyle::None, red);
        assert_eq!(winner(&[cell(none), cell(thin)]), Some(thin));
        assert_eq!(winner(&[cell(none)]), None);
        // Same width: the style decides, then the earlier candidate.
        let dashed = side(1.0, BorderStyle::Dashed, blue);
        assert_eq!(winner(&[cell(dashed), cell(thin)]), Some(thin));
        let thin_blue = side(1.0, BorderStyle::Solid, blue);
        assert_eq!(winner(&[cell(thin_blue), cell(thin)]), Some(thin_blue));
        // Cells win ties against the table.
        let t = Candidate {
            side: thin_blue,
            cell: false,
        };
        assert_eq!(winner(&[t, cell(thin)]), Some(thin));
        // Hostile widths are sanitised.
        let nan = side(f32::NAN, BorderStyle::Solid, red);
        assert_eq!(winner(&[cell(nan)]), None);
        let huge = side(f32::INFINITY, BorderStyle::Solid, red);
        assert_eq!(winner(&[cell(huge)]), None);
    }
}
