//! End-to-end tests: HTML → style → layout → display list → pixels, with
//! the system fonts. Assertions are deliberately loose (colors at a few
//! positions, amounts of ink) so they hold whatever fonts are installed.
//! Set `XBRO_DUMP=<dir>` to also write the fixture renderings as PNGs.

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::{assert_px, close, dark_pixels, px, rgb, BLUE, RED, WHITE};
use xbro_css::{parse_stylesheet, MediaContext, Stylesheet};
use xbro_dom::Document;
use xbro_layout::{layout, LayoutContext, LayoutTree, NoImages, Size};
use xbro_paint::{build_display_list_full, rasterize, DisplayList, NoImageProvider, RasterOptions};
use xbro_style::{compute_styles, StyleContext};
use xbro_text::FontSystem;

fn system_fonts() -> &'static FontSystem {
    static FONTS: OnceLock<FontSystem> = OnceLock::new();
    FONTS.get_or_init(FontSystem::new)
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Author sheets: `<style>` contents plus local `<link rel=stylesheet>` files
/// next to the fixtures (with their `@import`s inlined one level deep).
fn author_sheets(doc: &Document) -> Vec<Stylesheet> {
    let mut sheets = Vec::new();
    for id in doc.descendants(doc.root()) {
        if doc.is_element_named(id, "style") {
            sheets.push(parse_stylesheet(&doc.text_content(id)));
        } else if doc.is_element_named(id, "link")
            && doc
                .attr(id, "rel")
                .is_some_and(|r| r.eq_ignore_ascii_case("stylesheet"))
        {
            if let Some(href) = doc.attr(id, "href") {
                if let Ok(css) = std::fs::read_to_string(fixtures_dir().join(href)) {
                    for line in css.lines() {
                        let line = line.trim();
                        if let Some(rest) = line.strip_prefix("@import") {
                            let name: String = rest
                                .chars()
                                .filter(|c| !"\"'();url ".contains(*c))
                                .collect();
                            if let Ok(imported) = std::fs::read_to_string(fixtures_dir().join(name))
                            {
                                sheets.push(parse_stylesheet(&imported));
                            }
                        }
                    }
                    sheets.push(parse_stylesheet(&css));
                }
            }
        }
    }
    sheets
}

struct Rendered {
    tree: LayoutTree,
    list: DisplayList,
    pixmap: tiny_skia::Pixmap,
}

fn render_html(html: &str, width: u32, height: u32) -> Rendered {
    let fonts = system_fonts();
    let doc = xbro_html::parse_document(html);
    let sheets = author_sheets(&doc);
    let viewport = Size::new(width as f32, height as f32);
    let ctx = StyleContext {
        media: MediaContext {
            width: viewport.width,
            height: viewport.height,
            ..MediaContext::default()
        },
        ..StyleContext::default()
    };
    let styles = compute_styles(&doc, &sheets, &ctx);
    let tree = layout(&LayoutContext {
        document: &doc,
        styles: &styles,
        fonts,
        replaced: &NoImages,
        viewport,
    });
    let list = build_display_list_full(&tree, fonts, &NoImageProvider);
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("pixmap");
    rasterize(
        &list,
        fonts,
        &NoImageProvider,
        &mut pixmap,
        &RasterOptions::default(),
    );
    Rendered { tree, list, pixmap }
}

#[test]
fn colored_blocks() {
    let r = render_html(
        r#"<body style="margin:0"><div style="background:red;width:100px;height:50px"></div>
           <div style="background:blue;width:50px;height:50px;border:5px solid lime"></div></body>"#,
        200,
        200,
    );
    assert_px(&r.pixmap, 10, 10, RED);
    assert_px(&r.pixmap, 150, 10, WHITE);
    assert_px(&r.pixmap, 2, 52, xbro_style::Color::rgb(0, 255, 0));
    assert_px(&r.pixmap, 20, 70, BLUE);
}

#[test]
fn body_background_fills_canvas() {
    let r = render_html(
        r#"<body style="background:#ff0000;margin:0;height:10px"></body>"#,
        100,
        100,
    );
    assert_eq!(r.list.background, RED);
    assert_px(&r.pixmap, 50, 90, RED);
}

#[test]
fn text_and_links() {
    let r = render_html(
        r#"<body style="margin:0;font:20px sans-serif"><p style="margin:0">Hello <a href="x">link</a></p></body>"#,
        300,
        60,
    );
    assert!(dark_pixels(&r.pixmap, 0, 0, 150, 30, 128) > 30);
    // The link is blue and underlined.
    let blue = (0..300)
        .flat_map(|x| (0..30).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let (r_, g, b, _) = px(&r.pixmap, x, y);
            b > 150 && r_ < 100 && g < 100
        })
        .count();
    assert!(blue > 20, "{blue}");
}

#[test]
fn z_index_and_overflow() {
    let r = render_html(
        r#"<body style="margin:0">
            <div style="position:absolute;left:0;top:0;width:60px;height:60px;background:red;z-index:2"></div>
            <div style="position:absolute;left:30px;top:30px;width:60px;height:60px;background:blue;z-index:1"></div>
            <div style="position:absolute;left:100px;top:0;width:50px;height:50px;overflow:hidden">
              <div style="width:200px;height:200px;background:blue"></div></div>
           </body>"#,
        200,
        200,
    );
    assert_px(&r.pixmap, 45, 45, RED);
    assert_px(&r.pixmap, 80, 80, BLUE);
    assert_px(&r.pixmap, 120, 20, BLUE);
    assert_px(&r.pixmap, 170, 20, WHITE);
    assert_px(&r.pixmap, 120, 80, WHITE);
}

#[test]
fn form_controls_render() {
    let r = render_html(
        r#"<body style="margin:0"><input type=checkbox checked style="margin:0"> <input value="text"></body>"#,
        300,
        40,
    );
    let accent = (0, 0x75, 0xff, 255);
    let found = (0..20)
        .flat_map(|x| (0..20).map(move |y| (x, y)))
        .any(|(x, y)| close(px(&r.pixmap, x, y), accent, 10));
    assert!(found, "checked checkbox is blue");
}

#[test]
fn fixtures_render() {
    let dump = std::env::var("XBRO_DUMP").ok();
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
        let html = std::fs::read_to_string(fixtures_dir().join(name)).expect("fixture");
        let r = render_html(&html, 1024, 768);
        assert!(!r.list.items.is_empty(), "{name}: empty display list");
        let ink = dark_pixels(&r.pixmap, 0, 0, 1024, 768, 160);
        assert!(
            ink > 100,
            "{name}: almost nothing painted ({ink} dark pixels)"
        );
        if let Some(dir) = &dump {
            let path = PathBuf::from(dir).join(name.replace(".html", ".png"));
            r.pixmap.save_png(&path).expect("save png");
            // Full document at 2x for a closer look at the top.
            let fonts = system_fonts();
            let mut big = tiny_skia::Pixmap::new(2048, 1536).expect("pixmap");
            let options = RasterOptions {
                scale: 2.0,
                ..RasterOptions::default()
            };
            rasterize(&r.list, fonts, &NoImageProvider, &mut big, &options);
            big.save_png(PathBuf::from(dir).join(name.replace(".html", "@2x.png")))
                .expect("save png");
        }
        let _ = (&r.tree, rgb(WHITE));
    }
}

#[test]
fn fixtures_at_any_scroll_scale_and_offset() {
    let fonts = system_fonts();
    for name in ["wiki.html", "layout.html", "modern.html"] {
        let html = std::fs::read_to_string(fixtures_dir().join(name)).expect("fixture");
        let r = render_html(&html, 800, 600);
        let doc_h = r.tree.document_size.height;
        assert!(doc_h.is_finite() && doc_h >= 600.0, "{name}: {doc_h}");
        for (scroll_y, scale, offset) in [
            (0.0, 1.5, (0, 0)),
            (doc_h / 2.0, 1.0, (0, 40)),
            (doc_h - 600.0, 2.0, (-10, 0)),
            (doc_h + 1000.0, 1.0, (0, 0)),
            (-500.0, 0.5, (5, 5)),
        ] {
            let mut target = tiny_skia::Pixmap::new(400, 300).expect("pixmap");
            let options = RasterOptions {
                scroll: xbro_layout::Point::new(0.0, scroll_y),
                scale,
                offset,
                height: Some(250),
            };
            rasterize(&r.list, fonts, &NoImageProvider, &mut target, &options);
            // Rows below the drawing band are untouched (transparent).
            let below = 40u32.max(offset.1.max(0) as u32 + 250);
            if below < 300 {
                assert_eq!(px(&target, 10, below).3, 0, "{name}: drew outside the band");
            }
        }
    }
}

#[test]
fn display_list_build_is_fast() {
    let html = std::fs::read_to_string(fixtures_dir().join("wiki.html")).expect("fixture");
    let r = render_html(&html, 1280, 800);
    let start = std::time::Instant::now();
    let list = build_display_list_full(&r.tree, system_fonts(), &NoImageProvider);
    let elapsed = start.elapsed();
    assert_eq!(list, r.list, "deterministic");
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "display list built in {elapsed:?}"
    );
}

#[test]
fn hostile_nesting_stays_bounded() {
    // 300 nested stacking contexts with translucency and rounded clips:
    // layers are capped, clip chains stay cheap, nothing overflows.
    let mut html = String::from("<body style='margin:0'>");
    for i in 0..300 {
        html.push_str(&format!(
            "<div style='position:relative;z-index:{};opacity:.99;overflow:hidden;\
             border-radius:{}px;padding:1px;background:rgb({},{},200)'>",
            i % 3,
            i % 7,
            i % 256,
            (i * 7) % 256
        ));
    }
    html.push_str("deep text");
    html.push_str(&"</div>".repeat(300));
    let start = std::time::Instant::now();
    let r = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let r = render_html(&html, 400, 300);
            (
                r.list.items.len(),
                dark_pixels(&r.pixmap, 0, 0, 400, 300, 250),
            )
        })
        .expect("thread")
        .join()
        .expect("no panic or overflow");
    assert!(r.0 > 0);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
}
