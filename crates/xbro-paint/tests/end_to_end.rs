//! End-to-end tests: HTML → style → layout → display list → pixels.
//!
//! They depend on `xbro-layout` being implemented and are ignored by
//! default: `cargo test -p xbro-paint --test end_to_end -- --ignored`.
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
#[ignore = "requires xbro-layout"]
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
#[ignore = "requires xbro-layout"]
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
#[ignore = "requires xbro-layout"]
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
#[ignore = "requires xbro-layout"]
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
#[ignore = "requires xbro-layout"]
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
#[ignore = "requires xbro-layout"]
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
