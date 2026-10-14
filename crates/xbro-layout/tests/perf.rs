//! Layout performance on a large, wiki-like document.
//!
//! The budget (< 200 ms for ~15,000 nodes) is asserted in release builds
//! (`cargo test --release -p xbro-layout --test perf`); debug builds only
//! check a generous bound that catches algorithmic blowups.

mod common;

use common::*;
use xbro_layout::{layout, LayoutContext, NoImages, Size};

fn big_wiki_document(min_nodes: usize) -> String {
    let wiki = std::fs::read_to_string(fixtures_dir().join("wiki.html")).expect("fixture");
    let open = wiki.find("<body").expect("body");
    let body_start = wiki[open..].find('>').map_or(open, |i| open + i + 1);
    let body_end = wiki.rfind("</body>").unwrap_or(wiki.len());
    let body = &wiki[body_start..body_end];
    let mut html = wiki[..body_start].to_string();
    loop {
        html.push_str(body);
        if xbro_html::parse_document(&html).len() >= min_nodes {
            break;
        }
        // Grow faster than one copy at a time.
        let copy = html[body_start..].to_string();
        html.push_str(&copy);
    }
    html.push_str("</body></html>");
    html
}

#[test]
fn fifteen_thousand_nodes_lay_out_quickly() {
    let Some(fonts) = fonts() else {
        return;
    };
    let html = big_wiki_document(15_000);
    let l = lay_full(&html, Size::new(1024.0, 768.0), fonts, None);
    assert!(l.doc.len() >= 15_000);
    let viewport = Size::new(1024.0, 768.0);
    let mut best = std::time::Duration::MAX;
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let tree = layout(&LayoutContext {
            document: &l.doc,
            styles: &l.styles,
            fonts,
            replaced: &NoImages,
            viewport,
        });
        best = best.min(t.elapsed());
        assert!(tree.document_size.height > 10_000.0);
    }
    let budget_ms = if cfg!(debug_assertions) { 5_000 } else { 200 };
    assert!(
        best.as_millis() < budget_ms,
        "layout took {best:?} for {} nodes",
        l.doc.len()
    );
}

/// Best of three layouts of an already parsed and styled document.
fn best_layout_time(
    l: &Laid,
    fonts: &xbro_text::FontSystem,
    viewport: Size,
) -> std::time::Duration {
    let mut best = std::time::Duration::MAX;
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let tree = layout(&LayoutContext {
            document: &l.doc,
            styles: &l.styles,
            fonts,
            replaced: &NoImages,
            viewport,
        });
        best = best.min(t.elapsed());
        assert!(tree.root.is_some());
    }
    best
}

/// Budget in ms: `release` in optimised builds, a generous bound that
/// catches algorithmic blowups in debug builds.
fn budget(release: u128) -> u128 {
    if cfg!(debug_assertions) {
        release * 25
    } else {
        release
    }
}

#[test]
fn two_thousand_row_table_lays_out_quickly() {
    let Some(fonts) = fonts() else {
        return;
    };
    let mut html = String::from("<table border=1 cellpadding=2>");
    for r in 0..2000 {
        html.push_str("<tr>");
        for c in 0..10 {
            html.push_str(&format!("<td>row {r} col {c}</td>"));
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    let viewport = Size::new(1024.0, 768.0);
    let l = lay_full(&html, viewport, fonts, None);
    assert!(l.tree.document_size.height > 2000.0 * 15.0);
    let best = best_layout_time(&l, fonts, viewport);
    assert!(best.as_millis() < budget(300), "2000-row table: {best:?}");
}

#[test]
fn hacker_news_like_nested_tables_lay_out_quickly() {
    let Some(fonts) = fonts() else {
        return;
    };
    let hn = std::fs::read_to_string(fixtures_dir().join("hn.html")).expect("fixture");
    // Thirty pages of stories: the story rows repeated inside the main table.
    let start = hn.find("<tr class=\"athing").expect("stories");
    let end = hn.find("<tr class=\"morespace\"").expect("end of stories");
    let stories = &hn[start..end];
    let mut html = hn[..start].to_string();
    for _ in 0..100 {
        html.push_str(stories);
    }
    html.push_str(&hn[end..]);
    let viewport = Size::new(1024.0, 768.0);
    let l = lay_full(&html, viewport, fonts, None);
    let best = best_layout_time(&l, fonts, viewport);
    assert!(best.as_millis() < budget(300), "nested tables: {best:?}");
}

#[test]
fn nested_flex_and_grid_containers_lay_out_quickly() {
    let Some(fonts) = fonts() else {
        return;
    };
    // 40 levels alternating row/column flex and grids, each level with a
    // sibling of a different height (stretch relayouts at every level).
    let mut html = String::new();
    for i in 0..40 {
        match i % 3 {
            0 => html.push_str("<div style='display:flex'><div style='flex:1'>"),
            1 => html.push_str(
                "<div style='display:flex;flex-direction:column;min-height:200px'><div style='flex:1'>",
            ),
            _ => html.push_str("<div style='display:grid;grid-template-columns:1fr 1fr'><div>"),
        }
    }
    html.push_str("leaf");
    for i in 0..40 {
        html.push_str(&format!(
            "</div><div>sibling {i}<br>two<br>three</div></div>"
        ));
    }
    let viewport = Size::new(1024.0, 768.0);
    let l = lay_full(&html, viewport, fonts, None);
    let best = best_layout_time(&l, fonts, viewport);
    assert!(best.as_millis() < budget(50), "nested flex: {best:?}");
}

#[test]
fn nested_positioned_cells_lay_out_once() {
    let Some(fonts) = fonts() else {
        return;
    };
    // Positioned cells with free space below their content, 40 levels
    // deep: each cell must be laid out once (its absolutely positioned
    // boxes wait for its final height) or the cost doubles per level.
    let mut html = String::from("<style>td{position:relative}</style>");
    for i in 0..40 {
        html.push_str(&format!(
            "<table><tr><td><div style='height:{}px'></div></td><td>",
            2000 - i * 10
        ));
    }
    html.push_str("leaf<span style='position:absolute;bottom:0'>abs</span>");
    for _ in 0..40 {
        html.push_str("</td></tr></table>");
    }
    let viewport = Size::new(1024.0, 768.0);
    let l = lay_full(&html, viewport, fonts, None);
    let best = best_layout_time(&l, fonts, viewport);
    assert!(
        best.as_millis() < budget(50),
        "nested positioned cells: {best:?}"
    );
}

#[test]
fn wide_row_spans_stay_linear() {
    let Some(fonts) = fonts() else {
        return;
    };
    // 30 rows of 1000 cells spanning two rows, and a cell spanning 999
    // columns down 10,000 rows (each row has one more cell in the last
    // column): the grid bookkeeping must not be quadratic in rows ×
    // columns.
    let mut html = String::from("<table>");
    for _ in 0..30 {
        html.push_str("<tr>");
        for _ in 0..1000 {
            html.push_str("<td rowspan=2>x");
        }
        html.push_str("<tr><td>y");
    }
    html.push_str("</table><table><tr><td colspan=999 rowspan=0>x");
    for _ in 0..10_000 {
        html.push_str("<tr><td>y");
    }
    html.push_str("</table>");
    let viewport = Size::new(1024.0, 768.0);
    let l = lay_full(&html, viewport, fonts, None);
    assert!(l.tree.document_size.height > 10_000.0 * 15.0);
    let best = best_layout_time(&l, fonts, viewport);
    assert!(best.as_millis() < budget(300), "wide spans: {best:?}");
}

#[test]
fn positioned_boxes_stay_linear_with_many_pending_abspos() {
    let Some(fonts) = fonts() else {
        return;
    };
    // 40,000 absolutely positioned boxes wait for the initial containing
    // block while 40,000 relatively positioned boxes are laid out: ending
    // each positioned box must not scan every waiting box.
    let n = 40_000;
    let html = format!(
        "{}{}",
        "<div style='position:absolute'>a</div>".repeat(n),
        "<div style='position:relative'>b</div>".repeat(n)
    );
    let viewport = Size::new(1024.0, 768.0);
    let l = lay_full(&html, viewport, fonts, None);
    assert!(l.tree.document_size.height > n as f32 * 15.0);
    let best = best_layout_time(&l, fonts, viewport);
    assert!(best.as_millis() < budget(500), "positioned boxes: {best:?}");
}

#[test]
fn many_floats_beside_a_tall_float_stay_linear() {
    let Some(fonts) = fonts() else {
        return;
    };
    // One very tall float, then thousands of small ones in rows and many
    // lines: band queries must not visit every earlier float.
    let n = 40_000;
    let html = format!(
        "<div style='float:right;width:10px;height:1000000px'></div>{}<p>{}</p>",
        "<div style='float:left;width:30px;height:10px'></div>".repeat(n),
        "word ".repeat(n)
    );
    let viewport = Size::new(1024.0, 768.0);
    let l = lay_full(&html, viewport, fonts, None);
    assert!(l.tree.document_size.height >= 1_000_000.0);
    let best = best_layout_time(&l, fonts, viewport);
    assert!(best.as_millis() < budget(300), "floats: {best:?}");
}
