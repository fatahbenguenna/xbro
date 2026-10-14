//! Tables: box-tree fix-ups (CSS 2.1 §17.2.1) and table layout (automatic
//! and fixed table layout, `colspan`/`rowspan`, separated and collapsed
//! borders, captions, vertical alignment in cells).
//!
//! # Contract
//!
//! * **Box tree** (built by [`fixup`]): a [`Kind::Table`](crate::box_tree::Kind::Table) box's `children`
//!   are captions (`Kind::Block` boxes whose `style.display` is
//!   `TableCaption`) and row groups (`Kind::TableRowGroup`,
//!   `style.display` tells header/body/footer; rows found directly in a
//!   table are wrapped in anonymous row groups); row groups contain rows
//!   only and rows contain cells (`Kind::TableCell`) only. Columns
//!   (`<col>`, `<colgroup>`) generate no boxes: their spans and widths are
//!   recorded in the table's `BoxNode::cols`. `colspan` / `rowspan` are DOM
//!   attributes of the cells' `node`.
//! * **Inputs** of [`layout_table_contents`]: the table box `b`, `cb`
//!   describing the table's *content box* (the table's used width: block
//!   layout already applied `width` — the table uses `box-sizing:
//!   border-box` from the UA sheet, and is never narrower than its
//!   min-content width — and shrink-to-fit with [`table_intrinsic`] when
//!   `width` is auto; `cb.height` when definite), `y`, the top of the
//!   content box, and the table's used edges.
//! * **Output**: a [`ContentLayout`](crate::block::ContentLayout) whose `children` are the laid-out
//!   captions (`caption-side: top` ones above the table: `offset_top` is
//!   their height, which block layout adds above the table's border box
//!   and to its top margin; `bottom` ones below it: `offset_bottom`, added
//!   to its bottom margin) and
//!   row groups in visual order (header first, footer last) with their rows
//!   and cells as children (a cell spanning several rows is a child of the
//!   last row it spans, so that it paints above all their backgrounds), all
//!   in absolute coordinates; `height` is the height of the rows and
//!   spacing. Row and row group boxes span the columns horizontally;
//!   anonymous ones have no `node`.
//!
//! See [`structure`] for the grid model, [`widths`] for column widths and
//! [`layout`] for heights, alignment and borders.
//!
//! # Approximations
//!
//! * `table-layout: fixed` applies when the table has a specified width.
//! * `empty-cells` and `visibility: collapse` are ignored.
//! * Collapsed borders (see [`collapse`]) are resolved per cell edge
//!   between cells and the table; row, row group and column borders are
//!   ignored, and a cell owns one width per side.
//! * Row and row group backgrounds cover the whole row area (including
//!   the spacing between cells).

mod collapse;
mod fixup;
mod layout;
mod structure;
mod widths;

pub(crate) use fixup::{
    fixup_row_children, fixup_row_group_children, fixup_table_children, wrap_misparented,
};
pub(crate) use layout::{layout_table_contents, table_intrinsic, table_part_intrinsic};
