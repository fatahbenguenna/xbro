//! Fixture pages rendered with their images (`tests/fixtures/img/*`):
//! HTML → style → layout (with natural image sizes) → display list (with
//! an image provider) → pixels. With `XBRO_DUMP=<dir>` set, the whole
//! document of every fixture is also written as a PNG for inspection.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use common::{close, px};
use xbro_css::{parse_stylesheet, MediaContext, Stylesheet};
use xbro_dom::{Document, NodeId};
use xbro_layout::{layout, LayoutContext, LayoutTree, ReplacedSizes, Size};
use xbro_paint::{build_display_list_full, rasterize, DisplayList, Image, ImageProvider};
use xbro_paint::{DisplayItem, RasterOptions};
use xbro_style::{compute_styles, StyleContext};
use xbro_text::FontSystem;

fn system_fonts() -> &'static FontSystem {
    static FONTS: OnceLock<FontSystem> = OnceLock::new();
    FONTS.get_or_init(FontSystem::new)
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Images of a fixture page: `<img src>` by node, CSS URLs by (relative) URL.
#[derive(Default)]
struct FixtureImages {
    nodes: HashMap<usize, Image>,
    urls: HashMap<String, Option<Image>>,
}

impl FixtureImages {
    fn load(path: &str) -> Option<Image> {
        let bytes = std::fs::read(fixtures_dir().join(path)).ok()?;
        Image::decode(&bytes, None)
    }

    fn for_document(doc: &Document) -> FixtureImages {
        let mut images = FixtureImages::default();
        for id in doc.descendants(doc.root()) {
            if doc.is_element_named(id, "img") {
                if let Some(image) = doc.attr(id, "src").and_then(Self::load) {
                    images.nodes.insert(id.0, image);
                }
            }
        }
        for url in [
            "img/tile.png",
            "img/logo.png",
            "img/photo.png",
            "img/y18.gif",
        ] {
            images.urls.insert(url.to_string(), Self::load(url));
        }
        images
    }
}

impl ImageProvider for FixtureImages {
    fn node_image(&self, node: NodeId) -> Option<Image> {
        self.nodes.get(&node.0).cloned()
    }
    fn url_image(&self, url: &str) -> Option<Image> {
        self.urls.get(url).cloned().flatten()
    }
}

impl ReplacedSizes for FixtureImages {
    fn node_size(&self, node: NodeId) -> Option<Size> {
        self.nodes.get(&node.0).map(Image::natural_size)
    }
    fn url_size(&self, url: &str) -> Option<Size> {
        self.url_image(url).map(|i| i.natural_size())
    }
}

fn author_sheets(doc: &Document) -> Vec<Stylesheet> {
    let mut sheets = Vec::new();
    for id in doc.descendants(doc.root()) {
        if doc.is_element_named(id, "style") {
            sheets.push(parse_stylesheet(&doc.text_content(id)));
        } else if doc.is_element_named(id, "link") {
            if let Some(css) = doc
                .attr(id, "href")
                .and_then(|h| std::fs::read_to_string(fixtures_dir().join(h)).ok())
            {
                sheets.push(parse_stylesheet(&css));
            }
        }
    }
    sheets
}

struct Page {
    tree: LayoutTree,
    list: DisplayList,
    images: FixtureImages,
}

fn load(name: &str, width: f32, height: f32) -> Page {
    let html = std::fs::read_to_string(fixtures_dir().join(name)).expect("fixture");
    load_html(&html, width, height)
}

fn load_html(html: &str, width: f32, height: f32) -> Page {
    let fonts = system_fonts();
    let doc = xbro_html::parse_document(html);
    let images = FixtureImages::for_document(&doc);
    let viewport = Size::new(width, height);
    let ctx = StyleContext {
        media: MediaContext {
            width,
            height,
            ..MediaContext::default()
        },
        ..StyleContext::default()
    };
    let styles = compute_styles(&doc, &author_sheets(&doc), &ctx);
    let tree = layout(&LayoutContext {
        document: &doc,
        styles: &styles,
        fonts,
        replaced: &images,
        viewport,
    });
    let list = build_display_list_full(&tree, fonts, &images);
    Page { tree, list, images }
}

fn render(page: &Page, width: u32, height: u32, options: &RasterOptions) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("pixmap");
    rasterize(
        &page.list,
        system_fonts(),
        &page.images,
        &mut pixmap,
        options,
    );
    pixmap
}

#[test]
fn fixtures_with_images_render() {
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
        let page = load(name, 1024.0, 768.0);
        let shot = render(&page, 1024, 768, &RasterOptions::default());
        assert!(shot.data().iter().any(|&b| b != 255), "{name}: blank");
        if let Some(dir) = &dump {
            let h = page.tree.document_size.height.clamp(1.0, 8000.0) as u32;
            let full = render(&page, 1024, h, &RasterOptions::default());
            let path = PathBuf::from(dir).join(name.replace(".html", "-full.png"));
            full.save_png(path).expect("save png");
        }
    }
}

/// Pixels of the background tile and the centred logo of layout.html
/// come from the images (not just the fallback background color).
#[test]
fn layout_fixture_background_images_are_drawn() {
    let page = load("layout.html", 1024.0, 768.0);
    let tile_items = page
        .list
        .items
        .iter()
        .filter(|i| matches!(i, DisplayItem::BackgroundImage { url, .. } if url == "img/tile.png"))
        .count();
    assert_eq!(
        tile_items, 1,
        "tile background resolved with the image size"
    );
    let Some(DisplayItem::BackgroundImage { clip, .. }) =
        page.list.items.iter().find(
            |i| matches!(i, DisplayItem::BackgroundImage { url, .. } if url == "img/tile.png"),
        )
    else {
        unreachable!()
    };
    let scroll_y = (clip.y - 10.0).max(0.0);
    let shot = render(
        &page,
        1024,
        200,
        &RasterOptions {
            scroll: xbro_layout::Point::new(0.0, scroll_y),
            ..RasterOptions::default()
        },
    );
    let y = (clip.y - scroll_y + clip.height / 2.0) as u32;
    let eee = (0xee, 0xee, 0xee, 255);
    let differing = (clip.x as u32 + 2..(clip.right() as u32).min(1020))
        .filter(|&x| !close(px(&shot, x, y), eee, 3))
        .count();
    assert!(differing > 20, "tile pixels drawn ({differing})");
}

/// `content: url()` images and `list-style-image` markers (outside and
/// inside) are painted with the provider's images.
#[test]
fn generated_and_marker_images_are_painted() {
    let page = load_html(
        r#"<body style="margin:0;font:16px sans-serif">
           <p class="a" style="margin:0">x</p>
           <style>.a::before { content: url(img/logo.png) }</style>
           <ul style="margin:0;padding-left:60px;list-style-image:url(img/tile.png)">
             <li>outside</li>
             <li style="list-style-position:inside">inside</li>
           </ul></body>"#,
        300.0,
        300.0,
    );
    let urls: Vec<&str> = page
        .list
        .items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::UrlImage { url, .. } => Some(url.as_str()),
            _ => None,
        })
        .collect();
    assert!(urls.contains(&"img/logo.png"), "{urls:?}");
    assert_eq!(
        urls.iter().filter(|u| **u == "img/tile.png").count(),
        2,
        "{urls:?}"
    );
    let shot = render(&page, 300, 300, &RasterOptions::default());
    // The logo (an orange disc on transparent) is drawn at the start of the paragraph.
    let Some(DisplayItem::UrlImage { rect, .. }) = page
        .list
        .items
        .iter()
        .find(|i| matches!(i, DisplayItem::UrlImage { url, .. } if url == "img/logo.png"))
    else {
        unreachable!()
    };
    let (cx, cy) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let c = px(&shot, cx as u32, cy as u32);
    assert!(c.0 > 180 && c.2 < 120, "logo pixel {c:?}");
}
