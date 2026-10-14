//! Table layout.

mod common;

use common::*;
use xbro_layout::{BoxKind, LayoutBox, Point, Rect};

fn table(body: &str, width: f32) -> Option<Laid> {
    lay(
        &format!("<style>body{{margin:0}} td,th{{padding:0}}</style>{body}"),
        width,
    )
}

/// Every box of `kind` under `b`, in tree order.
fn boxes_of<'a>(b: &'a LayoutBox, kind: &BoxKind, out: &mut Vec<&'a LayoutBox>) {
    if &b.kind == kind {
        out.push(b);
    }
    for c in &b.children {
        boxes_of(c, kind, out);
    }
}

fn cells(l: &Laid, id: &str) -> Vec<Rect> {
    let mut v = Vec::new();
    boxes_of(l.box_id(id), &BoxKind::TableCell, &mut v);
    let mut r: Vec<Rect> = v.iter().map(|b| b.border_box()).collect();
    r.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    r
}

fn rows(l: &Laid, id: &str) -> Vec<Rect> {
    let mut v = Vec::new();
    boxes_of(l.box_id(id), &BoxKind::TableRow, &mut v);
    v.iter().map(|b| b.border_box()).collect()
}

#[test]
fn box_structure_and_auto_width_from_content() {
    let Some(l) = table(
        "<table id=t style='border-spacing:0'><tr>
           <td><div style='width:50px;height:10px'></div></td>
           <td><div style='width:120px;height:10px'></div></td>
         </tr></table>",
        800.0,
    ) else {
        return;
    };
    let t = l.box_id("t");
    assert_eq!(t.kind, BoxKind::Table);
    // Implied tbody + tr from the parser.
    assert_eq!(t.children.len(), 1);
    assert_eq!(t.children[0].kind, BoxKind::TableRowGroup);
    assert_eq!(t.children[0].children[0].kind, BoxKind::TableRow);
    assert_eq!(t.border_box().width, 170.0);
    let c = cells(&l, "t");
    assert_eq!((c[0].x, c[0].width), (0.0, 50.0));
    assert_eq!((c[1].x, c[1].width), (50.0, 120.0));
}

#[test]
fn border_spacing_between_and_around_cells() {
    let Some(l) = table(
        "<table id=t style='border-spacing:4px 6px;border:1px solid'>
           <tr><td style='width:20px;height:10px'></td><td style='width:30px;height:10px'></td></tr>
           <tr><td style='height:10px'></td><td style='height:10px'></td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let t = l.rect("t");
    // 1 + 4 + 20 + 4 + 30 + 4 + 1.
    assert_eq!(t.width, 64.0);
    assert_eq!(t.height, 1.0 + 6.0 + 10.0 + 6.0 + 10.0 + 6.0 + 1.0);
    let c = cells(&l, "t");
    assert_eq!((c[0].x, c[0].y), (5.0, 7.0));
    assert_eq!((c[1].x, c[1].y), (29.0, 7.0));
    assert_eq!((c[2].x, c[2].y), (5.0, 23.0));
}

#[test]
fn specified_width_distributes_extra_space() {
    let Some(l) = table(
        "<table id=t style='width:400px;border-spacing:0'><tr>
           <td><div style='width:100px;height:5px'></div></td>
           <td><div style='width:50px;height:5px'></div></td>
         </tr></table>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("t").width, 400.0);
    let c = cells(&l, "t");
    // Extra 250px shared in proportion to max-content widths (2:1).
    close(c[0].width, 266.67, 0.02);
    close(c[1].width, 133.33, 0.02);
}

#[test]
fn narrow_tables_interpolate_between_min_and_max() {
    let Some(l) = table(
        "<div style='width:200px'><table id=t style='border-spacing:0'><tr>
           <td>aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm</td>
           <td>x</td>
         </tr></table></div>",
        800.0,
    ) else {
        return;
    };
    let t = l.rect("t");
    assert_eq!(t.width, 200.0);
    let c = cells(&l, "t");
    assert!(c[0].width > 100.0 && c[1].width < 30.0, "{c:?}");
    close(c[0].width + c[1].width, 200.0, 0.02);
}

#[test]
fn percentage_and_fixed_columns() {
    let Some(l) = table(
        "<table id=t style='width:400px;border-spacing:0'><tr>
           <td style='width:25%;height:5px'></td>
           <td style='width:60px;height:5px'></td>
           <td style='height:5px'>auto</td>
         </tr></table>",
        800.0,
    ) else {
        return;
    };
    let c = cells(&l, "t");
    assert_eq!(c[0].width, 100.0);
    assert_eq!(c[1].width, 60.0);
    assert_eq!(c[2].width, 240.0);
}

#[test]
fn colspan_distributes_its_width() {
    let Some(l) = table(
        "<table id=t style='border-spacing:0'>
           <tr><td><div style='width:20px;height:5px'></div></td><td><div style='width:20px;height:5px'></div></td></tr>
           <tr><td colspan=2><div style='width:100px;height:5px'></div></td></tr>
           <tr><td colspan=0><div style='width:10px;height:5px'></div></td><td></td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let c = cells(&l, "t");
    assert_eq!(c[0].width, 50.0);
    assert_eq!(c[1].width, 50.0);
    assert_eq!((c[2].x, c[2].width), (0.0, 100.0));
    assert_eq!(c.len(), 5);
    assert_eq!(c[4].x, c[1].x);
}

#[test]
fn rowspan_heights_are_distributed_over_the_spanned_rows() {
    let Some(l) = table(
        "<table id=t style='border-spacing:0'>
           <tr><td rowspan=2><div style='width:10px;height:100px'></div></td><td style='height:20px'>a</td></tr>
           <tr><td style='height:20px'>b</td></tr>
           <tr><td>c</td><td style='height:15px'>d</td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let r = rows(&l, "t");
    // The 60px the spanning cell lacks are shared by its rows in
    // proportion to their heights (20px each).
    assert_eq!(r[0].height, 50.0);
    assert_eq!(r[1].height, 50.0);
    let c = cells(&l, "t");
    // The spanning cell covers both rows; the cell after it is in column 2.
    assert_eq!((c[0].y, c[0].height), (0.0, 100.0));
    assert_eq!(c[2].x, c[1].x);
    assert_eq!(c[2].y, 50.0);
    // The third row starts below the span.
    assert_eq!(c[3].y, 100.0);
    // rowspan=0 spans to the end of the group.
    let Some(l) = table(
        "<table id=u style='border-spacing:0'><tr><td rowspan=0 id=z>z</td><td>1</td></tr><tr><td>2</td></tr><tr><td>3</td></tr></table>",
        800.0,
    ) else {
        return;
    };
    let r = rows(&l, "u");
    assert_eq!(l.rect("z").bottom(), r[2].bottom());
}

#[test]
fn collapsed_borders_are_shared() {
    let Some(l) = table(
        "<table id=t style='border-collapse:collapse;border:1px solid'>
           <tr><td style='border:1px solid;width:20px;height:10px'></td><td style='border:1px solid;width:20px;height:10px'></td></tr>
           <tr><td style='border:1px solid;height:10px'></td><td style='border:1px solid;height:10px'></td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let c = cells(&l, "t");
    let t = l.rect("t");
    // Shared borders are drawn once: neighbours abut, each line is the
    // left/top border of the cell after it, and the outer cells cover the
    // table's border.
    assert_eq!(c[1].x, c[0].right());
    assert_eq!(c[2].y, c[0].bottom());
    assert_eq!(c[0].x, t.x);
    assert_eq!(c[0].y, t.y);
    assert_eq!(c[1].right(), t.right());
    assert_eq!(c[3].bottom(), t.bottom());
    assert_eq!(t.width, 22.0 + 22.0 - 1.0);
    let b = |i: usize| l.box_id("t").children[0].children[i / 2].children[i % 2].border;
    assert_eq!(b(0), xbro_layout::Edges::new(1.0, 0.0, 0.0, 1.0));
    assert_eq!(b(1), xbro_layout::Edges::new(1.0, 1.0, 0.0, 1.0));
    assert_eq!(b(3), xbro_layout::Edges::new(1.0, 1.0, 1.0, 1.0));
    assert_eq!(t.height, 11.0 + 12.0);
}

#[test]
fn collapsed_border_conflicts() {
    let Some(l) = table(
        "<table id=t style='border-collapse:collapse;border:4px solid rgb(0,0,255)'>
           <tr><td id=a style='border:1px solid rgb(255,0,0);border-right:3px dotted rgb(0,128,0)'>a</td>
               <td id=b style='border:2px solid rgb(255,0,0)'>b</td>
               <td id=c style='border:1px solid;border-left-style:hidden'>c</td></tr>
           <tr><td id=d style='border:1px solid'>d</td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let find = |id: &str| l.box_id(id);
    // The table's 4px border wins on the outer edges of the outer cells.
    let a = find("a");
    assert_eq!((a.border.top, a.border.left), (4.0, 4.0));
    assert_eq!(a.style.border.left.color, xbro_style::Color::rgb(0, 0, 255));
    // The wider dotted right side of `a` wins over `b`'s 2px left side:
    // `b` paints it, in `a`'s style.
    let b = find("b");
    assert_eq!((a.border.right, b.border.left), (0.0, 3.0));
    assert_eq!(b.style.border.left.style, xbro_style::BorderStyle::Dotted);
    assert_eq!(b.style.border.left.color, xbro_style::Color::rgb(0, 128, 0));
    // `hidden` suppresses the border between `b` and `c`.
    assert_eq!(find("c").border.left, 0.0);
    // No cell right of `d` (a short row): `d` paints its own right border.
    let d = find("d");
    assert_eq!(d.border.right, 1.0);
    // The second row is short: the slots right of `d` have no cell, so
    // the table box keeps painting its own border (it would else be
    // missing there); the cells still cover it.
    let t = l.box_id("t");
    assert_eq!(t.border.left, 4.0);
    assert_eq!(t.style.border.left.style, xbro_style::BorderStyle::Solid);
    assert_eq!(a.border_box().x, t.border_box().x);
}

#[test]
fn collapsed_table_border_painted_by_cells_or_table() {
    // Every slot along the outer edges has a cell (a rowspan and a
    // colspan included): the cells paint the table's border, the table
    // box does not.
    let full = "<table id=t style='border-collapse:collapse;border:3px solid'>
           <tr><td rowspan=2>a</td><td>b</td><td>c</td></tr>
           <tr><td colspan=2>d</td></tr>
         </table>";
    // Rows of different lengths, or an empty row: empty slots along the
    // edges, where only the table box can paint its border.
    let short = "<table id=t style='border-collapse:collapse;border:3px solid'>
           <tr><td>a</td><td>b</td><td>c</td></tr>
           <tr><td>d</td></tr>
           <tr><td>e</td><td>f</td><td>g</td></tr>
         </table>";
    let empty_row = "<table id=t style='border-collapse:collapse;border:3px solid'>
           <tr><td>a</td><td>b</td></tr><tr></tr><tr><td>c</td><td>d</td></tr>
         </table>";
    for (html, table_paints) in [(full, false), (short, true), (empty_row, true)] {
        let Some(l) = table(html, 800.0) else {
            return;
        };
        let t = l.box_id("t");
        assert_eq!(
            t.style.border.right.style.is_visible(),
            table_paints,
            "{html}"
        );
        // Either way the outer cells take the table's border width.
        let c = cells(&l, "t");
        assert_eq!(c[0].x, t.border_box().x, "{html}");
        let mut v = Vec::new();
        boxes_of(t, &BoxKind::TableCell, &mut v);
        assert!(v
            .iter()
            .all(|c| c.border.left == 3.0 || c.border_box().x > t.border_box().x));
    }
}

#[test]
fn bottom_captions_sit_below_the_table_box() {
    let Some(l) = table(
        "<style>#t caption.b{caption-side:bottom;height:25px}</style>
         <table id=t style='border:2px solid;border-spacing:0;margin:10px 0 7px'>
           <caption id=top style='height:30px'>Top</caption>
           <caption id=bot class=b>Bottom</caption>
           <tr><td style='width:200px;height:20px'></td></tr>
         </table><div id=after style='height:5px'></div>",
        800.0,
    ) else {
        return;
    };
    let t = l.box_id("t");
    let bb = t.border_box();
    assert_eq!(l.rect("top").y, 10.0);
    assert_eq!(bb.y, 40.0);
    assert_eq!(bb.height, 24.0);
    let bot = l.rect("bot");
    assert_eq!((bot.y, bot.height, bot.width), (64.0, 25.0, 204.0));
    // Both captions are in the table's margin box; the next block follows.
    assert_eq!(t.margin.bottom, 7.0 + 25.0);
    assert_eq!(l.rect("after").y, 64.0 + 25.0 + 7.0);
    // Floats and inline tables count it too.
    let Some(l) = table(
        "<div><table id=f style='float:left;border-spacing:0'><caption style='caption-side:bottom;height:15px'>c</caption>
           <tr><td style='width:50px;height:10px'></td></tr></table>
           <div id=next style='clear:left;height:5px'></div></div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("next").y, 25.0);
}

#[test]
fn table_layout_fixed_from_a_stylesheet() {
    let Some(l) = table(
        "<style>.fixed{table-layout:fixed;width:300px;border-spacing:0}</style>
         <table id=t class=fixed>
           <tr><td style='width:50px'></td><td></td><td></td></tr>
           <tr><td>a_very_long_unbreakable_word_that_would_widen_an_auto_column</td><td></td><td></td></tr>
         </table>
         <table id=a style='table-layout:fixed;border-spacing:0'><tr><td>x</td><td>yyyyyy</td></tr></table>",
        800.0,
    ) else {
        return;
    };
    let c: Vec<Rect> = l.box_id("t").children[0].children[1]
        .children
        .iter()
        .map(|b| b.border_box())
        .collect();
    // Widths from the first row (not the long word of the second one);
    // the other columns share the rest.
    assert_eq!((c[0].width, c[1].width, c[2].width), (50.0, 125.0, 125.0));
    assert_eq!(l.rect("t").width, 300.0);
    // `fixed` without a width: automatic layout.
    let a = cells(&l, "a");
    assert!(a[1].width > a[0].width);
}

#[test]
fn table_min_height_grows_the_rows() {
    let Some(l) = table(
        "<table id=t style='min-height:100px;border-spacing:0'><tr><td style='height:10px'></td></tr><tr><td style='height:30px'></td></tr></table>",
        800.0,
    ) else {
        return;
    };
    let r = rows(&l, "t");
    assert_eq!((r[0].height, r[1].height), (25.0, 75.0));
    assert_eq!(l.rect("t").height, 100.0);
}

#[test]
fn captions_sit_above_the_table_box() {
    let Some(l) = table(
        "<table id=t style='border:2px solid;border-spacing:0;margin-top:10px'>
           <caption id=cap style='height:30px'>Title</caption>
           <tr><td style='width:200px;height:20px'></td></tr>
         </table><div id=after style='height:5px'></div>",
        800.0,
    ) else {
        return;
    };
    let t = l.box_id("t");
    let cap = l.rect("cap");
    assert_eq!(cap.y, 10.0);
    assert_eq!(cap.width, 204.0);
    let bb = t.border_box();
    assert_eq!(bb.y, 40.0);
    assert_eq!(bb.height, 24.0);
    // The caption is part of the table's margin box.
    assert_eq!(t.margin_box().y, 0.0);
    assert_eq!(l.rect("after").y, 64.0);
    assert_eq!(t.children[0].node, Some(l.id("cap")));
}

#[test]
fn header_and_footer_groups_are_ordered_visually() {
    let Some(l) = table(
        "<table id=t style='border-spacing:0'>
           <tfoot><tr><td id=foot style='height:10px'></td></tr></tfoot>
           <tbody><tr><td id=body style='height:10px'></td></tr></tbody>
           <thead><tr><td id=head style='height:10px'></td></tr></thead>
         </table>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("head").y, 0.0);
    assert_eq!(l.rect("body").y, 10.0);
    assert_eq!(l.rect("foot").y, 20.0);
    let t = l.box_id("t");
    assert_eq!(t.children[0].node, Some(l.tag("thead", 0)));
}

#[test]
fn vertical_align_in_cells() {
    let Some(l) = table(
        "<table id=t style='border-spacing:0'><tr>
           <td style='height:100px;vertical-align:top'><div id=top style='height:20px'></div></td>
           <td style='vertical-align:middle'><div id=mid style='height:20px'></div></td>
           <td style='vertical-align:bottom'><div id=bot style='height:20px'></div></td>
           <td id=plain><div id=dflt style='height:20px'></div></td>
         </tr></table>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("top").y, 0.0);
    assert_eq!(l.rect("mid").y, 40.0);
    assert_eq!(l.rect("bot").y, 80.0);
    // The UA sheet makes cells inherit `middle` from the row group.
    assert_eq!(l.rect("dflt").y, 40.0);
    // Every cell box spans the full row height.
    for c in cells(&l, "t") {
        assert_eq!(c.height, 100.0);
    }
}

#[test]
fn baseline_alignment_across_cells() {
    let Some(l) = table(
        "<table style='border-spacing:0'><tr style='vertical-align:baseline'>
           <td id=a style='font-size:30px'>Big</td>
           <td id=b style='font-size:10px;padding-top:3px'>small</td>
         </tr></table>",
        800.0,
    ) else {
        return;
    };
    let a = l.box_id("a").lines[0].baseline;
    let b = l.box_id("b").lines[0].baseline;
    close(a, b, 0.05);
}

#[test]
fn anonymous_table_around_display_table_cells() {
    let Some(l) = table(
        "<div id=wrap><div id=c1 style='display:table-cell;width:40px;height:10px'></div><div id=c2 style='display:table-cell;width:60px;height:10px'></div></div>
         <div id=row style='display:table-row'><div id=c3 style='display:table-cell'>x</div></div>
         <div id=tbl style='display:table'><div id=c4 style='display:table-cell'>direct</div></div>",
        800.0,
    ) else {
        return;
    };
    let c1 = l.rect("c1");
    let c2 = l.rect("c2");
    assert_eq!(c1.y, c2.y);
    // `border-spacing` is inherited: 0 here.
    assert_eq!(c2.x, c1.right());
    assert_eq!((c1.width, c2.width), (40.0, 60.0));
    // The anonymous table and its anonymous row group / row.
    let wrap = l.box_id("wrap");
    let anon = &wrap.children[0];
    assert_eq!(anon.kind, BoxKind::Table);
    assert!(anon.node.is_none());
    assert_eq!(anon.children[0].kind, BoxKind::TableRowGroup);
    assert!(anon.children[0].node.is_none());
    assert_eq!(anon.children[0].children[0].kind, BoxKind::TableRow);
    assert_eq!(l.box_id("c3").kind, BoxKind::TableCell);
    // Cells directly in a table get an anonymous row group and row.
    let tbl = l.box_id("tbl");
    assert_eq!(tbl.children[0].kind, BoxKind::TableRowGroup);
    assert_eq!(l.box_id("c4").kind, BoxKind::TableCell);
}

#[test]
fn implied_structure_from_the_parser() {
    let Some(l) = table("<table id=t><td>a<td>b<tr><td>c</table>", 800.0) else {
        return;
    };
    assert_eq!(rows(&l, "t").len(), 2);
    let c = cells(&l, "t");
    assert_eq!(c.len(), 3);
    assert_eq!(c[0].y, c[1].y);
    assert!(c[2].y > c[0].y);
    assert_eq!(c[2].x, c[0].x);
}

#[test]
fn fixed_table_layout_uses_the_first_row() {
    let Some(l) = table(
        "<table id=t style='width:300px;table-layout:fixed;border-spacing:0'>
           <tr><td style='width:50px'>a</td><td>b</td><td>c</td></tr>
           <tr><td>averyveryveryverylongwordthatwouldwidenanautotable</td><td>x</td><td>y</td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let c = cells(&l, "t");
    assert_eq!(l.rect("t").width, 300.0);
    assert_eq!(c[0].width, 50.0);
    assert_eq!(c[1].width, 125.0);
    assert_eq!(c[2].width, 125.0);
    assert_eq!(c[3].width, 50.0);
}

#[test]
fn col_elements_give_width_hints() {
    let Some(l) = table(
        "<table id=t style='border-spacing:0;width:500px'>
           <colgroup><col style='width:100px'><col span=2></colgroup>
           <tr><td style='height:5px'></td><td>a</td><td>b</td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let c = cells(&l, "t");
    assert_eq!(c[0].width, 100.0);
    close(c[1].width + c[2].width, 400.0, 0.02);
}

#[test]
fn specified_table_height_grows_rows() {
    let Some(l) = table(
        "<table id=t style='height:100px;border-spacing:0'>
           <tr><td style='height:10px;width:10px'></td></tr><tr><td style='height:30px'></td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let r = rows(&l, "t");
    assert_eq!(l.rect("t").height, 100.0);
    assert_eq!((r[0].height, r[1].height), (25.0, 75.0));
}

#[test]
fn shrink_to_fit_and_inline_tables() {
    let Some(l) = table(
        "<div id=f style='float:left'><table id=t style='border-spacing:0'><tr><td><div style='width:70px;height:5px'></div></td><td><div style='width:30px;height:5px'></div></td></tr></table></div>
         <div style='clear:both'>before <table id=it style='display:inline-table;border-spacing:0'><tr><td>x</td></tr></table> after</div>
         <div style='text-align:center;width:400px'><table id=c align=center style='border-spacing:0'><tr><td style='width:100px;height:5px'></td></tr></table></div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").width, 100.0);
    assert_eq!(l.rect("t").width, 100.0);
    let it = l.rect("it");
    assert!(it.width > 0.0 && it.width < 50.0);
    // align=center centers the table with auto margins.
    assert_eq!(l.rect("c").x, 150.0);
}

#[test]
fn nested_tables_like_hacker_news() {
    let Some(l) = lay(
        "<center><table id=main width=85% cellspacing=0 cellpadding=0><tr><td>
           <table id=inner width=100% cellspacing=0 cellpadding=0><tr>
             <td style='width:18px'><div style='width:18px;height:18px'></div></td>
             <td id=title>Hacker News new | past | comments</td>
             <td id=login style='text-align:right'>login</td>
           </tr></table>
         </td></tr>
         <tr><td><table id=list cellspacing=0 cellpadding=0>
           <tr><td id=rank align=right valign=top>1.</td><td id=story>A story title that is long enough</td></tr>
           <tr><td colspan=1></td><td id=sub>12 points by someone</td></tr>
         </table></td></tr></table></center>",
        1000.0,
    ) else {
        return;
    };
    let body = l.box_of(l.doc.body().unwrap()).unwrap().content;
    let main = l.rect("main");
    close(main.width, body.width * 0.85, 0.05);
    close(main.x - body.x, body.right() - main.right(), 0.05);
    let inner = l.rect("inner");
    close(inner.width, main.width, 0.05);
    let login = l.rect("login");
    close(login.right(), main.right(), 0.05);
    // Rows of the story list are inside the main table.
    for id in ["rank", "story", "sub"] {
        let r = l.rect(id);
        assert!(r.x >= main.x && r.right() <= main.right() + 0.01, "{id}");
    }
    assert!(l.rect("sub").y >= l.rect("story").bottom() - 0.01);
}

#[test]
fn hit_testing_finds_cells_and_rows() {
    let Some(l) = table(
        "<table id=t style='border-spacing:10px'><tr id=r>
           <td id=a style='width:50px;height:20px'></td><td id=b style='width:50px;height:20px'></td>
         </tr></table>",
        800.0,
    ) else {
        return;
    };
    let hit = |x, y| {
        l.tree
            .hit_test(Point::new(x, y), Point::ZERO)
            .map(|h| h.node)
    };
    let b = l.rect("b");
    assert_eq!(hit(b.x + 5.0, b.y + 5.0), Some(l.id("b")));
    let a = l.rect("a");
    assert_eq!(hit(a.x + 5.0, a.y + 5.0), Some(l.id("a")));
    // The spacing between cells belongs to the row.
    assert_eq!(hit(a.right() + 5.0, a.y + 5.0), Some(l.id("r")));
}

#[test]
fn rowspan_cells_paint_above_later_rows() {
    let Some(l) = table(
        "<table id=t style='border-spacing:0'>
           <tr><td id=span rowspan=2 style='width:30px'>s</td><td style='width:30px;height:10px'>a</td></tr>
           <tr id=second style='background:gray'><td style='height:10px'>b</td></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    // The spanning cell is a child of the last row it spans, and that row's
    // box leaves the spanned column out.
    let second = l.box_id("second");
    assert!(second.children.iter().any(|c| c.node == Some(l.id("span"))));
    let span = l.rect("span");
    assert_eq!(second.border_box().x, span.right());
    let hit = l
        .tree
        .hit_test(Point::new(span.x + 2.0, span.bottom() - 1.0), Point::ZERO)
        .map(|h| h.node);
    assert_eq!(hit, Some(l.id("span")));
}

#[test]
fn large_spans_and_columns_are_bounded() {
    let Some(l) = table(
        "<table id=t><tr><td colspan=99999>a</td><td colspan=5000>b</td></tr><tr><td rowspan=99999>c</td></tr></table>",
        800.0,
    ) else {
        return;
    };
    let c = cells(&l, "t");
    assert!(c.len() >= 2);
    for r in &c {
        assert!(r.width.is_finite() && r.width >= 0.0);
    }
}

#[test]
fn floated_table_with_caption_reserves_its_caption() {
    let Some(l) = table(
        "<div style='width:400px'>
           <table id=t align=right style='border-spacing:0'><caption id=cap style='height:20px'>c</caption>
             <tr><td style='width:100px;height:50px'></td></tr></table>
           <p id=p style='margin:0'>text next to the float that wraps around it on several lines of the paragraph</p>
           <div id=after style='clear:both;height:1px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let t = l.box_id("t");
    assert_eq!(t.style.float, xbro_style::Float::Right);
    let tb = t.border_box();
    assert_eq!((tb.right(), tb.width, tb.y), (400.0, 100.0, 20.0));
    assert_eq!(l.rect("cap").y, 0.0);
    // Clearance below the whole float, caption included.
    assert_eq!(l.rect("after").y, 70.0);
    for line in &l.box_id("p").lines {
        if line.rect.y < 70.0 {
            assert!(line.rect.right() <= 300.01);
        }
    }
}

#[test]
fn inline_table_baseline_is_its_first_row() {
    let Some(l) = table(
        "<div id=d>before <table id=t style='display:inline-table;border-spacing:0'>
           <caption style='height:40px'>cap</caption><tr><td id=cell>cell</td></tr></table> after</div>",
        800.0,
    ) else {
        return;
    };
    let d = l.box_id("d");
    let cell = l.box_id("cell");
    close(d.lines[0].baseline, cell.lines[0].baseline, 0.05);
}

#[test]
fn tables_and_flex_nest_in_each_other() {
    let Some(l) = table(
        "<div style='display:flex;width:600px'>
           <div style='flex:1'><table id=t width=100% style='border-spacing:0'><tr><td>a</td><td>b</td></tr></table></div>
           <div style='flex:1'></div>
         </div>
         <table style='width:400px;border-spacing:0'><tr>
           <td id=cell><div id=f style='display:flex;justify-content:space-between'><span>x</span><span id=y>y</span></div></td>
         </tr></table>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("t").width, 300.0);
    let cell = l.rect("cell");
    assert_eq!(cell.width, 400.0);
    close(l.rect("y").right(), cell.right(), 0.01);
}

#[test]
fn row_and_group_backgrounds_cover_their_cells() {
    let Some(l) = table(
        "<table id=t style='border-spacing:0'>
           <tbody id=g style='background:yellow'><tr id=r style='background:red'><td style='width:30px;height:10px'></td><td style='width:20px;height:10px'></td></tr></tbody>
         </table>",
        800.0,
    ) else {
        return;
    };
    let r = l.rect("r");
    let g = l.rect("g");
    assert_eq!((r.width, r.height), (50.0, 10.0));
    assert_eq!(g, r);
}

#[test]
fn positioned_cells_contain_their_absolutely_positioned_boxes() {
    // The cell is stretched to the row (100px) and its content centered:
    // `bottom: 0` boxes sit at the bottom of the stretched cell, `top: 0`
    // ones at its top, static-position ones move with the content.
    let Some(l) = table(
        "<table id=t style='border-spacing:0'><tr>
           <td id=c style='position:relative;top:5px;vertical-align:middle;width:100px'>x<span
             id=b style='position:absolute;bottom:0;right:0;width:10px;height:10px'></span><span
             id=tl style='position:absolute;top:0;left:0;width:10px;height:10px'></span><div
             id=s style='position:absolute;width:10px;height:10px'></div></td>
           <td style='height:100px'>y</td>
         </tr></table>",
        400.0,
    ) else {
        return;
    };
    let c = l.rect("c");
    assert_eq!((c.x, c.y, c.width, c.height), (0.0, 5.0, 100.0, 100.0));
    let b = l.rect("b");
    assert_eq!((b.x, b.y), (90.0, 95.0));
    let tl = l.rect("tl");
    assert_eq!((tl.x, tl.y), (0.0, 5.0));
    // Static position: on the (centered) line of text, after the `x`.
    let s = l.rect("s");
    assert!(s.y > c.y + 30.0 && s.y < c.y + 60.0, "{s:?}");
    assert!(s.x > 0.0 && s.x < 20.0, "{s:?}");
}

#[test]
fn collapsed_spans_beside_empty_slots_keep_their_whole_edge() {
    // `wide` spans two columns above a row with a single cell: the part of
    // its bottom edge over the empty slot has no other cell to paint it,
    // so `wide` paints its whole bottom edge (and `d` not its top). Same
    // for the right edge of `tall` beside an empty row.
    let Some(l) = table(
        "<style>td{border:2px solid}</style>
         <table id=t style='border-collapse:collapse'>
           <tr><td>x</td><td>yyyy</td><td>z</td></tr>
           <tr><td id=wide colspan=2>wide</td><td id=c>c</td></tr>
           <tr><td id=d>d</td></tr>
         </table>
         <table id=u style='border-collapse:collapse'>
           <tr><td id=tall rowspan=3>tall</td><td id=a>a</td></tr>
           <tr><td id=b>b</td></tr>
           <tr style='height:30px'></tr>
         </table>",
        800.0,
    ) else {
        return;
    };
    let b = |id: &str| l.box_id(id).border;
    assert_eq!(b("wide").bottom, 2.0);
    assert_eq!(b("d").top, 0.0);
    // `c` is above an empty slot too; the regular shared edges stay
    // painted once (by the cell after / below).
    assert_eq!(b("c").bottom, 2.0);
    assert_eq!((b("wide").right, b("c").left), (0.0, 2.0));
    assert_eq!(b("tall").right, 2.0);
    assert_eq!((b("a").left, b("b").left), (0.0, 0.0));
    assert_eq!((b("a").bottom, b("b").top), (0.0, 2.0));
    // The bottom edge of `b` faces the empty row: `b` paints it.
    assert_eq!(b("b").bottom, 2.0);
    // Geometry: `d` starts right below `wide`'s border.
    assert_eq!(l.rect("d").y, l.rect("wide").bottom());
}
