//! Helpers shared by the engine integration tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use xbro_engine::{Engine, NavigationRequest, Page, Point, RasterOptions, Size, Url};
use xbro_net::{Fetcher, FetcherOptions};
use xbro_paint::tiny_skia::Pixmap;
use xbro_text::FontSystem;

/// One engine (system fonts are loaded once) shared by all tests of a binary.
pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let fetcher = Fetcher::with_options(FetcherOptions {
            use_env_proxy: false,
            ..FetcherOptions::default()
        });
        Engine::with_parts(Arc::new(FontSystem::new()), Arc::new(fetcher))
    })
}

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .canonicalize()
        .expect("fixtures directory")
}

pub fn fixture_url(name: &str) -> Url {
    Url::from_file_path(fixtures_dir().join(name)).expect("file URL")
}

pub const VIEWPORT: Size = Size::new(1024.0, 768.0);

pub fn load_url(url: &Url, viewport: Size) -> Page {
    engine().load(&NavigationRequest::Get(url.clone()), viewport)
}

pub fn load_fixture(name: &str) -> Page {
    load_url(&fixture_url(name), VIEWPORT)
}

pub fn load_html(html: &str) -> Page {
    engine().load_html(
        html,
        Url::parse("https://example.test/dir/page.html").unwrap(),
        VIEWPORT,
    )
}

pub fn by_id(page: &Page, id: &str) -> xbro_dom::NodeId {
    page.document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("no element #{id}"))
}

/// Center of the border box of an element (document coordinates).
pub fn center_of(page: &Page, id: &str) -> Point {
    let node = by_id(page, id);
    let r = page
        .layout_tree()
        .node_rect(node)
        .unwrap_or_else(|| panic!("#{id} has no box"));
    Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
}

/// Renders the top of the page into a new pixmap.
pub fn render(page: &Page, width: u32, height: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).expect("pixmap");
    page.render(&mut pixmap, &RasterOptions::default());
    pixmap
}

/// With `XBRO_DUMP=<dir>`, writes a PNG of the page for visual inspection.
pub fn dump(page: &Page, name: &str, width: u32, height: u32) {
    if let Some(dir) = std::env::var_os("XBRO_DUMP") {
        let pixmap = render(page, width, height);
        let path = PathBuf::from(dir).join(format!("{name}.png"));
        let _ = pixmap.save_png(path);
    }
}

/// The text of an element (whitespace collapsed).
pub fn text_of(page: &Page, node: xbro_dom::NodeId) -> String {
    page.document()
        .text_content(node)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Concatenated text of all laid-out text fragments.
pub fn laid_out_text(page: &Page) -> String {
    page.layout_tree()
        .text_fragments()
        .iter()
        .map(|f| f.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
