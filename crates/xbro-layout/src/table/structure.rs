//! The table grid (CSS 2.1 §17.5, HTML table processing model, simplified):
//! captions, row groups in visual order, rows, and cells with the grid
//! slots they cover.
//!
//! * Row groups are ordered visually: the first header group, then the
//!   body groups in document order (other header/footer groups included),
//!   then the first footer group.
//! * Cells are placed left to right in the first column not covered by a
//!   cell spanning down from an earlier row. `colspan` (default 1, 0 → 1)
//!   is clamped to `1..=1000`; `rowspan` 0 spans to the end of the row
//!   group and every row span is clamped to the rows left in the group.
//!   Only `<td>`/`<th>` read these attributes. Overlapping cells are
//!   allowed (later ones are simply placed over earlier spans).
//! * At most [`MAX_COLUMNS`] columns: cells starting beyond are dropped.

use std::ops::Range;

use xbro_dom::Document;
use xbro_style::Display;

use super::fixup::is_caption;
use crate::box_tree::{BoxNode, Kind};

/// Largest number of grid columns.
pub(crate) const MAX_COLUMNS: usize = 1000;

/// Largest row span (HTML clamps `rowspan` to 65534).
const MAX_ROWSPAN: i64 = 65_534;

/// A row group and the rows it contains (indices into [`Grid::rows`]).
#[derive(Debug)]
pub(crate) struct Group<'t> {
    pub b: &'t BoxNode,
    pub rows: Range<usize>,
}

/// A row and the cells that start in it (indices into [`Grid::cells`]).
#[derive(Debug)]
pub(crate) struct Row<'t> {
    pub b: &'t BoxNode,
    pub cells: Range<usize>,
}

/// A cell and the slots it covers.
#[derive(Debug)]
pub(crate) struct Cell<'t> {
    pub b: &'t BoxNode,
    pub row: usize,
    pub col: usize,
    pub colspan: usize,
    pub rowspan: usize,
}

/// The grid of a table box.
#[derive(Debug, Default)]
pub(crate) struct Grid<'t> {
    pub captions: Vec<&'t BoxNode>,
    /// Row groups in visual order.
    pub groups: Vec<Group<'t>>,
    /// Rows in visual order.
    pub rows: Vec<Row<'t>>,
    /// Cells in row order, then column order.
    pub cells: Vec<Cell<'t>>,
    pub ncols: usize,
}

/// Parses a non-negative integer attribute the HTML way (leading
/// whitespace and `+`, then digits; trailing garbage ignored).
fn parse_span(s: &str) -> Option<i64> {
    let t = s.trim_start();
    let t = t.strip_prefix('+').unwrap_or(t);
    let digits: String = t
        .chars()
        .take_while(char::is_ascii_digit)
        .take(10)
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// `colspan`/`rowspan` attribute of a cell (only `<td>` and `<th>`).
fn span_attr(doc: &Document, cell: &BoxNode, name: &str) -> Option<i64> {
    let node = cell.node.filter(|_| cell.pseudo.is_none())?;
    if node.index() >= doc.len() {
        return None;
    }
    let tag = doc.tag_name(node)?;
    if !(tag.eq_ignore_ascii_case("td") || tag.eq_ignore_ascii_case("th")) {
        return None;
    }
    doc.attr(node, name).and_then(parse_span)
}

/// Builds the grid of table box `table`.
pub(crate) fn build<'t>(doc: &Document, table: &'t BoxNode) -> Grid<'t> {
    let mut grid = Grid::default();
    let mut header = None;
    let mut footer = None;
    let mut bodies = Vec::new();
    for c in &table.children {
        if is_caption(c) {
            grid.captions.push(c);
        } else if matches!(c.kind, Kind::TableRowGroup) {
            match c.style.display {
                Display::TableHeaderGroup if header.is_none() => header = Some(c),
                Display::TableFooterGroup if footer.is_none() => footer = Some(c),
                _ => bodies.push(c),
            }
        }
    }
    let ordered = header.into_iter().chain(bodies).chain(footer);
    // Per column, the index of the first row (in `grid.rows`) no longer
    // covered by a cell spanning down from an earlier row of the group.
    let mut covered_until: Vec<usize> = Vec::new();
    for g in ordered {
        covered_until.clear();
        let rows: Vec<&'t BoxNode> = g
            .children
            .iter()
            .filter(|r| matches!(r.kind, Kind::TableRow))
            .collect();
        let first = grid.rows.len();
        let count = rows.len();
        for (ri, row) in rows.into_iter().enumerate() {
            let r = grid.rows.len();
            let left = (count - ri) as i64;
            let cell_start = grid.cells.len();
            let mut col = 0usize;
            for cell in row
                .children
                .iter()
                .filter(|c| matches!(c.kind, Kind::TableCell))
            {
                while covered_until.get(col).is_some_and(|&until| until > r) {
                    col += 1;
                }
                if col >= MAX_COLUMNS {
                    break;
                }
                let colspan = span_attr(doc, cell, "colspan").unwrap_or(1).clamp(1, 1000) as usize;
                let colspan = colspan.min(MAX_COLUMNS - col);
                let rowspan = match span_attr(doc, cell, "rowspan") {
                    Some(0) => left,
                    Some(n) => n.min(MAX_ROWSPAN),
                    None => 1,
                }
                .clamp(1, left.max(1)) as usize;
                grid.cells.push(Cell {
                    b: cell,
                    row: r,
                    col,
                    colspan,
                    rowspan,
                });
                if rowspan > 1 {
                    if covered_until.len() < col + colspan {
                        covered_until.resize(col + colspan, 0);
                    }
                    if let Some(slots) = covered_until.get_mut(col..col + colspan) {
                        for s in slots {
                            *s = (*s).max(r + rowspan);
                        }
                    }
                }
                col += colspan;
                grid.ncols = grid.ncols.max(col);
            }
            grid.rows.push(Row {
                b: row,
                cells: cell_start..grid.cells.len(),
            });
        }
        grid.groups.push(Group {
            b: g,
            rows: first..grid.rows.len(),
        });
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_parsing() {
        assert_eq!(parse_span("3"), Some(3));
        assert_eq!(parse_span(" +2px"), Some(2));
        assert_eq!(parse_span("0"), Some(0));
        assert_eq!(parse_span("-1"), None);
        assert_eq!(parse_span("x"), None);
        assert_eq!(parse_span("99999999999999999999"), Some(9_999_999_999));
    }
}
