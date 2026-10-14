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
