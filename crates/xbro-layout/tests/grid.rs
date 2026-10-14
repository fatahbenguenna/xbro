//! Grid layout: track sizing, placement, alignment, intrinsic sizes.

mod common;

use common::*;
use xbro_layout::{BoxKind, Rect};

fn grid(body: &str, width: f32) -> Option<Laid> {
    lay(&format!("<style>body{{margin:0}}</style>{body}"), width)
}

fn kids(l: &Laid, id: &str) -> Vec<Rect> {
    l.box_id(id)
        .children
        .iter()
        .map(|c| c.border_box())
        .collect()
}

#[test]
fn explicit_columns_row_major_with_gaps() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:repeat(3,1fr);gap:10px 20px;width:340px'>
           <div style='height:30px'></div><div style='height:50px'></div><div></div>
           <div style='height:20px'></div><div style='height:20px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.box_id("g").kind, BoxKind::Flex);
    let k = kids(&l, "g");
    assert_eq!(k.len(), 5);
    // (340 - 2 × 20) / 3 = 100px columns.
    for (i, r) in k.iter().enumerate() {
        assert_eq!(r.width, 100.0, "item {i}");
        assert_eq!(r.x, (i % 3) as f32 * 120.0, "item {i}");
    }
    // First row: as tall as its tallest item; auto heights stretch.
    assert_eq!((k[0].y, k[1].y, k[2].y), (0.0, 0.0, 0.0));
    assert_eq!(k[0].height, 30.0);
    assert_eq!(k[2].height, 50.0);
    // Second row after the row gap.
    assert_eq!(k[3].y, 60.0);
    assert_eq!(l.rect("g").height, 80.0);
}

#[test]
fn track_list_counts_columns_and_alignment() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:100px 1fr;width:400px;align-items:center'>
           <div style='height:40px'></div><div id=small style='height:10px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let k = kids(&l, "g");
    // A fixed column and a flexible one taking the rest.
    assert_eq!((k[0].width, k[1].x, k[1].width), (100.0, 100.0, 300.0));
    assert_eq!(l.rect("small").y, 15.0);
}

#[test]
fn specified_widths_and_margins_inside_cells() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:1fr 1fr;width:400px'>
           <div id=a style='width:50px;height:10px'></div>
           <div id=b style='margin:0 10px;height:10px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").width, 50.0);
    let b = l.rect("b");
    assert_eq!((b.x, b.width), (210.0, 180.0));
}

#[test]
fn auto_fill_columns_and_stacks_without_template() {
    // auto-fill: four 100px items fit in 450px with 10px gaps.
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:repeat(auto-fill,minmax(100px,1fr));gap:10px;width:450px'>
           <div style='width:100px;height:10px'></div><div style='width:100px;height:10px'></div>
           <div style='width:100px;height:10px'></div><div style='width:100px;height:10px'></div>
           <div style='width:100px;height:10px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let k = kids(&l, "g");
    assert_eq!(k[3].y, 0.0);
    assert_eq!(k[4].y, 20.0);
    assert_eq!(k[1].x, 115.0);
    // No template and heterogeneous content: a single column (a stack).
    let Some(l) = grid(
        "<div id=s style='display:grid;gap:8px;width:500px'>
           <h2 style='margin:0'>Title</h2>
           <p style='margin:0'>A much longer paragraph of text that is clearly wider than the title above it.</p>
         </div>",
        800.0,
    ) else {
        return;
    };
    let k = kids(&l, "s");
    assert_eq!(k[0].x, k[1].x);
    assert!(k[1].y > k[0].bottom());
    assert_eq!(k[0].width, 500.0);
}

#[test]
fn intrinsic_width_of_a_grid_container() {
    let Some(l) = grid(
        "<div id=g style='float:left;display:grid;grid-template-columns:repeat(2,1fr);column-gap:10px'>
           <div style='width:30px;height:5px'></div><div style='width:50px;height:5px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    // Two columns as wide as the widest item, plus the gap.
    assert_eq!(l.rect("g").width, 110.0);
}

#[test]
fn modern_fixture_stats_grid() {
    let html = std::fs::read_to_string(fixtures_dir().join("modern.html")).expect("fixture");
    for (width, cols) in [(1024.0, 3usize), (360.0, 1)] {
        let Some(fonts) = fonts() else {
            return;
        };
        let l = lay_full(&html, xbro_layout::Size::new(width, 768.0), fonts, None);
        let k = kids(&l, "stats");
        assert_eq!(k.len(), 6);
        let first_row = k.iter().filter(|r| (r.y - k[0].y).abs() < 0.5).count();
        assert_eq!(first_row, cols, "at {width}px");
        // Equal widths, no overlap.
        for r in &k {
            close(r.width, k[0].width, 0.05);
        }
        // `repeat(3, 1fr)` (one `1fr` column below 700px) with 16px gaps.
        let g = l.box_id("stats").content;
        close(
            k[0].width,
            (g.width - 16.0 * (cols - 1) as f32) / cols as f32,
            0.05,
        );
        close(k[0].x, g.x, 0.01);
        if cols == 3 {
            close(k[1].x - k[0].right(), 16.0, 0.05);
            close(k[2].right(), g.right(), 0.05);
            // Same row: same (stretched) height.
            assert_eq!(k[0].height, k[2].height);
            assert_eq!(k[3].y, k[0].bottom() + 16.0);
        } else {
            for w in k.windows(2) {
                close(w[1].y, w[0].bottom() + 16.0, 0.05);
            }
        }
    }
}

#[test]
fn auto_fill_cards_and_template_less_grids() {
    // Cards sharing a class with very different text lengths: a card
    // grid (auto-fill), whatever the spread of their widths.
    let Some(l) = grid(
        "<style>.card{padding:8px}</style>
         <div id=g style='display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:16px;width:976px'>
           <div class=card>Some text in the first card.</div>
           <div class='card wide'>Some more text, a bit longer than the first one, long enough to be much wider than the others.</div>
           <div class=card>Short.</div><div class=card>Another card.</div><div class=card>Fifth.</div>
         </div>",
        1024.0,
    ) else {
        return;
    };
    let k = kids(&l, "g");
    let first_row = k.iter().filter(|r| (r.y - k[0].y).abs() < 0.5).count();
    assert!(first_row >= 3, "{k:?}");
    for r in &k {
        close(r.width, k[0].width, 0.05);
    }
    // Different elements (a stack of content), or text runs: one column.
    for body in [
        "<div id=g style='display:grid;gap:8px;width:600px'><h3 style='margin:0'>Hi</h3><p style='margin:0'>a</p><div>b</div></div>",
        "<div id=g style='display:grid;gap:8px;width:600px'><span class=x>a</span>text<span class=x>b</span></div>",
        "<div id=g style='display:grid;gap:8px;width:600px'><div class=a>a</div><div class=b>b</div></div>",
    ] {
        let Some(l) = grid(body, 800.0) else {
            return;
        };
        let k = kids(&l, "g");
        assert!(k.len() >= 2);
        for w in k.windows(2) {
            assert_eq!(w[0].x, w[1].x, "{body}");
            assert!(w[1].y >= w[0].bottom(), "{body}");
        }
        assert_eq!(k[0].width, 600.0);
    }
    // Without a template, even similar items are stacked (one column).
    let Some(l) = grid(
        "<ul id=g style='display:grid;gap:10px;width:300px;list-style:none;margin:0;padding:0'><li>a</li><li>b</li><li>c</li></ul>",
        800.0,
    ) else {
        return;
    };
    let k = kids(&l, "g");
    assert_eq!(k[0].x, k[2].x);
    assert!(k[2].y > k[1].y && k[1].y > k[0].y);
    assert_eq!(k[0].width, 300.0);
}

// ------------------------------------------------------------------
// Track sizing
// ------------------------------------------------------------------

fn widths(k: &[Rect]) -> Vec<f32> {
    k.iter()
        .map(|r| (r.width * 100.0).round() / 100.0)
        .collect()
}

#[test]
fn fixed_percentage_and_fr_columns() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:100px 25% 1fr 2fr;column-gap:10px;width:600px'>
           <div></div><div></div><div></div><div></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let k = kids(&l, "g");
    // 600 − 3 × 10 − 100 − 150 = 320 for 3fr.
    assert_eq!(widths(&k), vec![100.0, 150.0, 106.67, 213.33]);
    assert_eq!(k[1].x, 110.0);
    close(k[3].right(), 600.0, 0.01);
}

#[test]
fn content_sized_columns() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:auto 1fr;width:500px'>
           <div><span style='display:inline-block;width:120px'></span></div><div>x</div>
           <div><span style='display:inline-block;width:80px'></span></div><div>y</div>
         </div>
         <div id=m style='display:grid;grid-template-columns:min-content max-content 1fr;width:600px'>
           <div><span style='display:inline-block;width:40px'></span> <span style='display:inline-block;width:60px'></span></div>
           <div><span style='display:inline-block;width:40px'></span> <span style='display:inline-block;width:60px'></span></div>
           <div></div>
         </div>
         <div id=f style='display:grid;grid-template-columns:minmax(150px,1fr) fit-content(100px) minmax(50px,200px);width:700px'>
           <div></div>
           <div><span style='display:inline-block;width:60px'></span> <span style='display:inline-block;width:70px'></span></div>
           <div></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    // auto: the widest item (max-content); 1fr: the rest.
    let k = kids(&l, "g");
    assert_eq!(widths(&k[..2]), vec![120.0, 380.0]);
    assert_eq!(k[2].width, 120.0);
    // min-content: the widest word; max-content: the whole line.
    let k = kids(&l, "m");
    assert_eq!(k[0].width, 60.0);
    assert!(k[1].width > 100.0 && k[1].width < 110.0, "{k:?}");
    // fit-content(100px) caps the max-content width (134px) at 100px;
    // minmax(50px, 200px) grows to 200px; the 1fr track gets the rest.
    let k = kids(&l, "f");
    assert_eq!(k[1].width, 100.0);
    assert_eq!(k[2].width, 200.0);
    assert_eq!(k[0].width, 400.0);
}

#[test]
fn auto_fill_and_auto_fit() {
    let html = |mode: &str| {
        format!(
            "<div id=g style='display:grid;grid-template-columns:repeat({mode},minmax(200px,1fr));gap:16px;width:1000px'>
               <div style='height:10px'></div><div style='height:10px'></div>
             </div>"
        )
    };
    let (Some(fill), Some(fit)) = (
        grid(&html("auto-fill"), 1024.0),
        grid(&html("auto-fit"), 1024.0),
    ) else {
        return;
    };
    // floor((1000 + 16) / (200 + 16)) = 4 columns of (1000 − 48) / 4.
    let k = kids(&fill, "g");
    assert_eq!(widths(&k), vec![238.0, 238.0]);
    assert_eq!(k[1].x, 254.0);
    // auto-fit: the two empty columns collapse, the others share the space.
    let k = kids(&fit, "g");
    assert_eq!(widths(&k), vec![492.0, 492.0]);
    // At 360px (320px wide container): one column.
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:16px;width:320px'>
           <div style='height:10px'></div><div style='height:10px'></div>
         </div>",
        360.0,
    ) else {
        return;
    };
    let k = kids(&l, "g");
    assert_eq!(widths(&k), vec![320.0, 320.0]);
    assert_eq!(k[1].y, 26.0);
}

// ------------------------------------------------------------------
// Placement and rows
// ------------------------------------------------------------------

#[test]
fn spans_and_explicit_lines() {
    let Some(l) = grid(
        "<style>#g>div{height:20px}</style>
         <div id=g style='display:grid;grid-template-columns:repeat(3,100px);gap:10px'>
           <div id=a style='grid-column:span 2'></div>
           <div id=b></div>
           <div id=full style='grid-column:1 / -1'></div>
           <div id=c style='grid-column:3;grid-row:4'></div>
           <div id=d style='grid-column:-2 / -1'></div>
           <div id=e></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let r = |id: &str| l.rect(id);
    assert_eq!((r("a").x, r("a").y, r("a").width), (0.0, 0.0, 210.0));
    assert_eq!((r("b").x, r("b").y), (220.0, 0.0));
    assert_eq!(
        (r("full").x, r("full").y, r("full").width),
        (0.0, 30.0, 320.0)
    );
    // Definite row and column.
    assert_eq!((r("c").x, r("c").y), (220.0, 90.0));
    // `-2 / -1`: the last column, on the next free row.
    assert_eq!((r("d").x, r("d").y), (220.0, 60.0));
    // Auto-placed after it: wraps to row 4, first free cell.
    assert_eq!((r("e").x, r("e").y), (0.0, 90.0));
    assert_eq!(l.rect("g").height, 110.0);
}

#[test]
fn template_rows_implicit_rows_and_alignment() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:1fr 1fr;grid-template-rows:50px 30px;row-gap:5px;width:200px'>
           <div id=s1></div><div id=s2 style='align-self:end;height:10px'></div>
           <div id=c1 style='align-self:center;height:10px'></div><div id=c2 style='align-self:start;height:10px'></div>
           <div id=i1 style='height:40px'></div><div id=i2></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    // Stretched to the 50px row, end-aligned in it.
    assert_eq!(l.rect("s1").height, 50.0);
    assert_eq!((l.rect("s2").y, l.rect("s2").height), (40.0, 10.0));
    // Second row (30px) after the gap: centred / start.
    assert_eq!(l.rect("c1").y, 65.0);
    assert_eq!(l.rect("c2").y, 55.0);
    // An implicit row sized to its content; the auto item stretches.
    assert_eq!(l.rect("i1").y, 90.0);
    assert_eq!(l.rect("i2").height, 40.0);
    assert_eq!(l.rect("g").height, 130.0);
}

#[test]
fn fr_rows_share_a_definite_height() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-rows:1fr 3fr;height:200px;width:100px'>
           <div id=a></div><div id=b></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").height, 50.0);
    assert_eq!((l.rect("b").y, l.rect("b").height), (50.0, 150.0));
}

#[test]
fn auto_margins_and_justify_content() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-columns:300px;width:500px;justify-content:center'>
           <div id=m style='margin:0 auto'><span style='display:inline-block;width:50px;height:10px'></span></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    // The 300px column is centred in 500px; the item shrinks to 50px and
    // is centred in its column.
    let m = l.rect("m");
    assert_eq!((m.x, m.width), (225.0, 50.0));
}

#[test]
fn grid_container_intrinsic_sizes() {
    let Some(l) = grid(
        "<div id=g style='display:inline-grid;grid-template-columns:auto auto;column-gap:10px'>
           <div style='width:30px;height:5px'></div><div style='width:50px;height:5px'></div>
         </div>
         <div id=f style='float:left;display:grid;grid-template-columns:100px 1fr'>
           <div></div><div style='width:20px'></div>
         </div>
         <div style='width:50px'><div id=n style='display:inline-grid;grid-template-columns:1fr 1fr'>
           <div>aaaa bbbb</div><div>cc</div>
         </div></div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("g").width, 90.0);
    assert_eq!(l.rect("f").width, 120.0);
    // In a narrow container: the min-content of the columns.
    let n = l.rect("n");
    let k = kids(&l, "n");
    assert!(n.width < 100.0, "{n:?}");
    assert!(k[0].right() <= k[1].x + 0.01);
}

#[test]
fn fr_rows_fill_the_min_height() {
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-rows:auto 1fr auto;min-height:300px;row-gap:10px;width:200px'>
           <div id=h style='height:40px'></div><div id=m></div><div id=f style='height:30px'></div>
         </div>
         <div id=a style='display:grid;min-height:100px;width:200px'><div id=s style='height:20px'></div></div>",
        800.0,
    ) else {
        return;
    };
    // 300 − 40 − 30 − 2 × 10 = 210 for the 1fr row.
    assert_eq!(l.rect("m").y, 50.0);
    assert_eq!(l.rect("m").height, 210.0);
    assert_eq!(l.rect("f").y, 270.0);
    assert_eq!(l.rect("g").height, 300.0);
    // An auto row stretches to the min-height (align-content: normal),
    // its stretched item with it.
    assert_eq!(l.rect("a").height, 100.0);
    assert_eq!(l.rect("s").height, 20.0);
}

// ------------------------------------------------------------------
// Minimum contributions, spanning items, legend and caption room
// ------------------------------------------------------------------

#[test]
fn fr_columns_do_not_blow_out_without_a_minimum_contribution() {
    // A 600px-wide child in the first `1fr` column of a 400px grid. Its
    // automatic minimum size widens the column (control), but not when
    // the item is a scroll container, has `min-width: 0`, or spans
    // several tracks with a flexible one (CSS Grid 1 §6.6).
    let Some(l) = grid(
        "<style>.g{display:grid;grid-template-columns:1fr 1fr;width:400px}.w{width:600px;height:5px}</style>
         <div class=g id=ctl><div><div class=w></div></div><div></div></div>
         <div class=g id=ovf><div style='overflow:hidden'><div class=w></div></div><div></div></div>
         <div class=g id=min0><div style='min-width:0'><div class=w></div></div><div></div></div>
         <div class=g id=span><div style='grid-column:1 / -1'><div class=w></div></div><div></div><div></div></div>
         <div class=g id=mw><div style='min-width:300px;overflow:hidden'><div class=w></div></div><div></div></div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(widths(&kids(&l, "ctl")), vec![600.0, 0.0]);
    assert_eq!(widths(&kids(&l, "ovf")), vec![200.0, 200.0]);
    assert_eq!(widths(&kids(&l, "min0")), vec![200.0, 200.0]);
    let k = kids(&l, "span");
    assert_eq!(widths(&k), vec![400.0, 200.0, 200.0]);
    assert_eq!(k[2].x, 200.0);
    // An explicit `min-width` is the minimum contribution.
    assert_eq!(widths(&kids(&l, "mw")), vec![300.0, 100.0]);
    // The grid's own min-content width still counts the content.
    let Some(l) = grid(
        "<div style='width:10px'><div id=g style='display:inline-grid;grid-template-columns:1fr'>
           <div style='min-width:0'><div style='width:150px;height:5px'></div></div></div></div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("g").width, 150.0);
}

#[test]
fn fr_rows_of_an_app_shell_keep_their_size() {
    // header / scrolling main / footer in a 300px-tall grid: the main
    // area (a scroll container) does not push the footer out.
    let Some(l) = grid(
        "<div id=g style='display:grid;grid-template-rows:auto 1fr auto;height:300px;width:200px'>
           <div style='height:20px'></div>
           <main id=m style='overflow:auto'><div style='height:500px'></div></main>
           <div id=f style='height:20px'></div>
         </div>
         <div id=v style='display:grid;grid-template-rows:auto 1fr auto;height:300px;width:200px'>
           <div style='height:20px'></div>
           <main id=vm><div style='height:500px'></div></main>
           <div id=vf style='height:20px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("m").height, 260.0);
    assert_eq!(l.rect("f").y, 280.0);
    // Without `overflow`, the content-based minimum keeps the row tall.
    let top = l.rect("v").y;
    assert_eq!(l.rect("vm").height, 500.0);
    assert_eq!(l.rect("vf").y - top, 520.0);
}

#[test]
fn spanning_item_widens_the_track_that_has_room() {
    // Column 1 holds a 100px item (at its growth limit); column 2 has no
    // item of its own: the 300px spanning item's extra 200px go to it
    // (CSS Grid 1 §11.5.1), not 100px to each.
    let html = |style: &str| {
        format!(
            "<div id=g style='{style};grid-template-columns:auto auto'>
               <div id=a><span style='display:inline-block;width:100px;height:5px'></span></div>
               <div id=s style='grid-column:span 2;grid-row:2;width:300px;height:5px'></div>
             </div>"
        )
    };
    let Some(l) = grid(&html("display:inline-grid"), 800.0) else {
        return;
    };
    assert_eq!(l.rect("g").width, 300.0);
    assert_eq!(l.rect("a").width, 100.0);
    // In a 400px grid the 100px left over stretch both auto columns.
    let Some(l) = grid(&html("display:grid;width:400px"), 800.0) else {
        return;
    };
    assert_eq!(l.rect("a").width, 150.0);
}

#[test]
fn legend_and_caption_room_counts_in_grid_rows() {
    let Some(l) = grid(
        "<style>legend{font-size:30px;padding:0}fieldset{margin:0;border:2px solid;padding:0 4px}</style>
         <div id=g style='display:grid;grid-template-columns:200px'>
           <fieldset id=f><legend id=lg>Title</legend><div style='height:10px'></div></fieldset>
           <div id=after style='height:20px'></div>
           <table id=t style='border-spacing:0'><caption style='height:15px'>c</caption>
             <caption style='caption-side:bottom;height:5px'>c</caption><tr><td style='height:10px'></td></tr></table>
           <div id=last style='height:20px'></div>
         </div>
         <div id=s style='display:grid;height:300px;grid-template-rows:1fr;width:200px'>
           <fieldset id=sf><legend>Title</legend></fieldset>
         </div>",
        800.0,
    ) else {
        return;
    };
    let f = l.box_id("f");
    // The legend's margin box starts the row; the next item follows the
    // fieldset's margin box (legend room included).
    assert_eq!(l.rect("lg").y, 0.0);
    let bottom = f.border_box().bottom();
    assert_eq!(l.rect("after").y, bottom);
    assert!(bottom > l.rect("lg").bottom() + 10.0);
    // Captions above and below the table box are in its row too.
    let t = l.box_id("t");
    assert_eq!(t.border_box().y, l.rect("after").bottom() + 15.0);
    assert_eq!(l.rect("last").y, t.border_box().bottom() + 5.0);
    // A stretched fieldset fills its row: margin box = 300px.
    let sf = l.box_id("sf");
    assert_eq!(sf.margin_box().height, 300.0);
    assert_eq!(sf.margin_box().y, l.rect("s").y);
}
