//! Full-page renders of the fixtures for visual inspection (ignored by default).
//!
//! `cargo test -p xbro-engine --test visual_dump -- --ignored` writes PNGs to
//! `$XBRO_REVIEW_DIR` (default `/tmp/xbro-verify/engine-review`).

mod common;

use common::*;
use xbro_engine::{Page, RasterOptions, Size, Url};
use xbro_paint::tiny_skia::Pixmap;

fn out_dir() -> std::path::PathBuf {
    std::env::var_os("XBRO_REVIEW_DIR")
        .map(Into::into)
        .unwrap_or_else(|| "/tmp/xbro-verify/engine-review".into())
}

fn save(page: &Page, name: &str, max_height: f32) {
    let dir = out_dir();
    let _ = std::fs::create_dir_all(&dir);
    let w = page.viewport().width.max(1.0) as u32;
    let h = page.document_size().height.clamp(1.0, max_height) as u32;
    let Some(mut pixmap) = Pixmap::new(w, h) else {
        return;
    };
    page.render(&mut pixmap, &RasterOptions::default());
    let _ = pixmap.save_png(dir.join(format!("{name}.png")));
}

#[test]
#[ignore]
fn dump_fixtures() {
    for name in [
        "basic.html",
        "hn.html",
        "wiki.html",
        "modern.html",
        "forms.html",
        "layout.html",
        "tables.html",
        "text.html",
    ] {
        let mut page = load_fixture(name);
        let stem = name.trim_end_matches(".html");
        save(&page, stem, 4000.0);
        page.set_viewport(Size::new(420.0, 800.0));
        save(&page, &format!("{stem}-420"), 3000.0);
        println!("{name}: console {:?}", page.console());
    }
}

#[test]
#[ignore]
fn dump_generated_pages() {
    let pages = [
        ("error-missing", fixture_url("nope.html")),
        ("error-ftp", Url::parse("ftp://example.org/file").unwrap()),
        ("text-css", fixture_url("wiki.css")),
        ("image-photo", fixture_url("img/photo.png")),
        ("image-svg", fixture_url("img/icon.svg")),
    ];
    for (name, url) in pages {
        let page = load_url(&url, Size::new(800.0, 600.0));
        save(&page, name, 1200.0);
        println!("{name}: {:?} {:?}", page.title(), page.console());
    }
}

#[test]
#[ignore]
fn dump_focus_states() {
    use xbro_engine::Point;
    let mut page = load_fixture("forms.html");
    let q = page.document().get_element_by_id("q").unwrap();
    let r = page.layout_tree().node_rect(q).unwrap();
    page.click(Point::new(r.x + 5.0, r.y + 5.0), Point::ZERO);
    page.insert_text("rust browser");
    save(&page, "forms-typing", 500.0);
    let msg = page.document().get_element_by_id("msg").unwrap();
    let r = page.layout_tree().node_rect(msg).unwrap();
    page.click(Point::new(r.x + 5.0, r.y + 5.0), Point::ZERO);
    page.insert_text("\nmore\nlines\nhere\nand\nmore");
    save(&page, "forms-textarea", 500.0);
}
