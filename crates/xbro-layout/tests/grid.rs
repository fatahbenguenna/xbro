//! Grid approximation.

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
    // Equal columns (track sizes are approximated).
    assert_eq!((k[0].width, k[1].x), (200.0, 200.0));
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
fn unknown_column_count_heuristic() {
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
        if cols == 3 {
            close(k[1].x - k[0].right(), 16.0, 0.05);
            // Same row: same (stretched) height.
            assert_eq!(k[0].height, k[2].height);
        }
    }
}
