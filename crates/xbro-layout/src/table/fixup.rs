//! Box-tree fix-ups for tables (CSS 2.1 §17.2.1).
//!
//! After these fix-ups a [`Kind::Table`] box only contains captions and
//! row groups (rows directly inside a table, and cells or other content,
//! are wrapped in anonymous row groups / rows / cells, like browsers do), a
//! row group only contains rows and a row only contains cells. Misparented
//! table parts inside block containers are wrapped in anonymous tables.
//! Whitespace-only text between table parts is dropped.

use std::sync::Arc;

use xbro_style::{ComputedStyle, Display};

use crate::box_tree::{anonymous_style, finish_block_children, BoxNode, Kind};

/// True for internal table boxes that need a table ancestor.
fn is_table_part(b: &BoxNode) -> bool {
    matches!(
        b.kind,
        Kind::TableRowGroup | Kind::TableRow | Kind::TableCell
    )
}

/// True for a table caption box.
pub(crate) fn is_caption(b: &BoxNode) -> bool {
    matches!(b.kind, Kind::Block) && b.style.display == Display::TableCaption && b.node.is_some()
}

/// Whitespace-only text (dropped between table parts).
fn is_ws(b: &BoxNode) -> bool {
    match &b.kind {
        Kind::Text(t) => t.chars().all(crate::text::is_collapsible_space),
        _ => false,
    }
}

fn anon_style(parent: &ComputedStyle, display: Display) -> Arc<ComputedStyle> {
    let mut s = (*anonymous_style(parent)).clone();
    s.display = display;
    Arc::new(s)
}

fn anonymous(parent: &ComputedStyle, display: Display, kind: Kind) -> BoxNode {
    BoxNode::new(None, None, anon_style(parent, display), kind)
}

/// Anonymous cell around arbitrary content.
fn anonymous_cell(parent: &ComputedStyle, content: Vec<BoxNode>) -> BoxNode {
    let mut cell = anonymous(parent, Display::TableCell, Kind::TableCell);
    let style = cell.style.clone();
    let kids = finish_block_children(content, &style, &mut cell);
    cell.children = kids;
    cell
}

/// Anonymous row around cells.
fn anonymous_row(parent: &ComputedStyle, cells: Vec<BoxNode>) -> BoxNode {
    let mut row = anonymous(parent, Display::TableRow, Kind::TableRow);
    row.children = cells;
    row
}

/// Anonymous row group around rows.
fn anonymous_row_group(parent: &ComputedStyle, rows: Vec<BoxNode>) -> BoxNode {
    let mut group = anonymous(parent, Display::TableRowGroup, Kind::TableRowGroup);
    group.children = rows;
    group
}

/// In a block container: wraps runs of misparented table parts (rows,
/// row groups, cells) into anonymous tables.
pub(crate) fn wrap_misparented(flat: Vec<BoxNode>, parent: &Arc<ComputedStyle>) -> Vec<BoxNode> {
    if !flat.iter().any(is_table_part) {
        return flat;
    }
    let mut out = Vec::with_capacity(flat.len());
    let mut run: Vec<BoxNode> = Vec::new();
    let flush = |run: &mut Vec<BoxNode>, out: &mut Vec<BoxNode>| {
        if run.is_empty() {
            return;
        }
        // Trailing whitespace belongs outside the table.
        let mut trailing = Vec::new();
        while run.last().is_some_and(is_ws) {
            if let Some(t) = run.pop() {
                trailing.push(t);
            }
        }
        let mut table = anonymous(parent, Display::Table, Kind::Table);
        let style = table.style.clone();
        table.children = fixup_table_children(std::mem::take(run), &style);
        out.push(table);
        out.extend(trailing.into_iter().rev());
    };
    for c in flat {
        if is_table_part(&c) || (!run.is_empty() && is_ws(&c)) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Pending content while fixing up the children of a table or row group.
#[derive(Default)]
struct Pending {
    /// Rows waiting for an anonymous row group (tables only).
    rows: Vec<BoxNode>,
    /// Cells waiting for an anonymous row.
    cells: Vec<BoxNode>,
    /// Other content waiting for an anonymous cell.
    other: Vec<BoxNode>,
}

impl Pending {
    fn flush_other(&mut self, parent: &ComputedStyle) {
        if self.other.iter().all(is_ws) {
            self.other.clear();
            return;
        }
        let content = std::mem::take(&mut self.other);
        self.cells.push(anonymous_cell(parent, content));
    }

    fn flush_cells(&mut self, parent: &ComputedStyle) {
        self.flush_other(parent);
        if !self.cells.is_empty() {
            let cells = std::mem::take(&mut self.cells);
            self.rows.push(anonymous_row(parent, cells));
        }
    }
}

/// Children of a table box: captions and row groups are kept; rows (and
/// cells, wrapped in anonymous rows; and other content, wrapped in
/// anonymous cells) are grouped into anonymous row groups.
pub(crate) fn fixup_table_children(
    children: Vec<BoxNode>,
    table: &Arc<ComputedStyle>,
) -> Vec<BoxNode> {
    let mut out = Vec::new();
    let mut p = Pending::default();
    let flush_rows = |p: &mut Pending, out: &mut Vec<BoxNode>| {
        p.flush_cells(table);
        if !p.rows.is_empty() {
            out.push(anonymous_row_group(table, std::mem::take(&mut p.rows)));
        }
    };
    for c in children {
        match c.kind {
            Kind::TableRowGroup => {
                flush_rows(&mut p, &mut out);
                out.push(c);
            }
            _ if is_caption(&c) => {
                flush_rows(&mut p, &mut out);
                out.push(c);
            }
            Kind::TableRow => {
                p.flush_cells(table);
                p.rows.push(c);
            }
            Kind::TableCell => {
                p.flush_other(table);
                p.cells.push(c);
            }
            _ => p.other.push(c),
        }
    }
    flush_rows(&mut p, &mut out);
    out
}

/// Children of a row group: rows are kept; cells and other content are
/// wrapped in anonymous rows (and cells).
pub(crate) fn fixup_row_group_children(
    children: Vec<BoxNode>,
    group: &Arc<ComputedStyle>,
) -> Vec<BoxNode> {
    let mut p = Pending::default();
    for c in children {
        match c.kind {
            Kind::TableRow => {
                p.flush_cells(group);
                p.rows.push(c);
            }
            Kind::TableCell => {
                p.flush_other(group);
                p.cells.push(c);
            }
            _ => p.other.push(c),
        }
    }
    p.flush_cells(group);
    p.rows
}

/// Children of a row: cells are kept; other content is wrapped in
/// anonymous cells.
pub(crate) fn fixup_row_children(children: Vec<BoxNode>, row: &Arc<ComputedStyle>) -> Vec<BoxNode> {
    let mut p = Pending::default();
    for c in children {
        if matches!(c.kind, Kind::TableCell) {
            p.flush_other(row);
            p.cells.push(c);
        } else {
            p.other.push(c);
        }
    }
    p.flush_other(row);
    p.cells
}
