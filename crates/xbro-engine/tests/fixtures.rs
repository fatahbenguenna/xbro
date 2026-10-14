//! Loading the HTML fixtures through `file://` URLs.

mod common;

use common::*;
use xbro_engine::{ClickAction, Point, Size};
use xbro_paint::DisplayItem;
use xbro_style::Display;

const FIXTURES: &[&str] = &[
    "basic.html",
    "hn.html",
    "wiki.html",
    "modern.html",
    "forms.html",
    "layout.html",
    "tables.html",
    "text.html",
];

#[test]
fn every_fixture_loads_with_a_sane_size() {
    for name in FIXTURES {
        let page = load_fixture(name);
        let size = page.document_size();
        assert!(
            size.width >= 1024.0 && size.width < 4000.0,
            "{name}: width {}",
            size.width
        );
        assert!(
            size.height >= 768.0 && size.height < 20_000.0,
            "{name}: height {}",
            size.height
        );
        assert!(
            !page.display_list().is_empty(),
            "{name}: empty display list"
        );
        assert_eq!(page.url(), &fixture_url(name));
        assert!(!page.title().is_empty());
        dump(&page, name.trim_end_matches(".html"), 1024, 768);
        // Interaction entry points never panic anywhere on the page.
        let scroll = Point::ZERO;
        let mut page = page;
        for y in (0..size.height as i32).step_by(97) {
            for x in (0..1024).step_by(131) {
                let p = Point::new(x as f32, y as f32);
                let _ = page.node_at(p, scroll);
                let _ = page.link_at(p, scroll);
                let _ = page.cursor_at(p, scroll);
            }
        }
        let _ = page.find("e");
        let _ = page.set_hover(Some((Point::new(100.0, 100.0), scroll)));
        page.set_viewport(Size::new(500.0, 700.0));
        assert!(page.document_size().height >= 700.0);
    }
}

#[test]
fn basic_title_links_noscript_and_scripts() {
    let mut page = load_fixture("basic.html");
    assert_eq!(page.title(), "Basic document — xbro");
    let doc = page.document();
    let link = doc
        .descendants(doc.root())
        .find(|&n| doc.attr(n, "href") == Some("#section-2"))
        .expect("internal link");
    let r = page.layout_tree().node_rect(link).expect("link box");
    let p = Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0);
    let url = page.link_at(p, Point::ZERO).expect("link under point");
    assert_eq!(url.fragment(), Some("section-2"));
    assert_eq!(url.path(), fixture_url("basic.html").path());
    let section = page.fragment_offset("section-2").expect("anchor");
    assert!(section > 100.0, "section offset {section}");
    match page.click(p, Point::ZERO) {
        ClickAction::ScrollTo(y) => assert_eq!(y, section),
        other => panic!("unexpected {other:?}"),
    }
    let text = laid_out_text(&page);
    assert!(
        text.contains("Le JavaScript est désactivé"),
        "noscript content rendered"
    );
    assert!(
        !text.contains("ne doit pas apparaître"),
        "script content not rendered"
    );
    // The external link navigates.
    let doc = page.document();
    let external = doc
        .descendants(doc.root())
        .find(|&n| doc.attr(n, "href") == Some("https://www.rust-lang.org/"))
        .unwrap();
    let r = page.layout_tree().node_rect(external).unwrap();
    let p = Point::new(r.x + 2.0, r.y + r.height / 2.0);
    assert_eq!(
        page.click(p, Point::ZERO),
        ClickAction::Navigate(xbro_engine::NavigationRequest::Get(
            "https://www.rust-lang.org/".parse().unwrap()
        ))
    );
}

#[test]
fn wiki_stylesheets_imports_images_and_media_queries() {
    let mut page = load_fixture("wiki.html");
    assert!(page.console().is_empty(), "console: {:?}", page.console());
    // <link rel=stylesheet>: the body background (#f8f9fa) reaches the canvas.
    let bg = page.display_list().background;
    assert_eq!((bg.r, bg.g, bg.b), (0xf8, 0xf9, 0xfa));
    // @import: `#siteSub::before` exists.
    let site_sub = by_id(&page, "siteSub");
    assert!(
        page.styles().before(site_sub).is_some(),
        "@import rule applied"
    );
    // Images: the logo is 64×64.
    let doc = page.document();
    let logo = doc
        .descendants(doc.root())
        .find(|&n| doc.attr(n, "src") == Some("img/logo.png"))
        .unwrap();
    let image = page.images().node(logo).expect("logo loaded");
    let size = image.natural_size();
    assert_eq!((size.width, size.height), (64.0, 64.0));
    let has_logo_item = page
        .display_list()
        .items
        .iter()
        .any(|i| matches!(i, DisplayItem::NodeImage { node, .. } if *node == logo));
    assert!(has_logo_item);
    // Find and fragments.
    assert!(page.find("Rust").len() > 3);
    assert!(page.fragment_offset("History").unwrap() > 0.0);
    // Media query: the TOC disappears below 720px.
    let toc = by_id(&page, "vector-toc");
    assert_ne!(page.styles().get(toc).map(|s| s.display), None);
    page.set_viewport(Size::new(600.0, 800.0));
    assert!(
        page.styles()
            .get(toc)
            .is_none_or(|s| s.display == Display::None),
        "TOC hidden at 600px"
    );
    assert!(page.layout_tree().node_rect(toc).is_none());
    page.set_viewport(Size::new(1024.0, 768.0));
    assert!(page.layout_tree().node_rect(toc).is_some());
}

#[test]
fn layout_fixture_images_and_rendering() {
    let page = load_fixture("layout.html");
    let doc = page.document();
    let img = |src: &str| {
        doc.descendants(doc.root())
            .find(|&n| doc.attr(n, "src") == Some(src))
            .unwrap_or_else(|| panic!("no img {src}"))
    };
    for src in [
        "img/photo.png",
        "img/landscape.jpg",
        "img/icon.svg",
        "img/logo.png",
    ] {
        assert!(page.images().node(img(src)).is_some(), "{src} loaded");
    }
    let svg = doc
        .descendants(doc.root())
        .find(|&n| doc.is_element_named(n, "svg"))
        .unwrap();
    let inline = page.images().node(svg).expect("inline svg rendered");
    assert_eq!(inline.natural_size(), xbro_engine::Size::new(40.0, 40.0));
    assert!(page.images().node(img("img/missing.png")).is_none());
    assert!(
        page.console().iter().any(|m| m.contains("missing.png")),
        "broken image reported: {:?}",
        page.console()
    );
    // CSS background images are keyed by their absolute URL.
    let tile = fixture_url("img/tile.png");
    assert!(page.images().url(tile.as_str()).is_some());
    let pixmap = render(&page, 800, 600);
    let first = pixmap.pixel(0, 0).unwrap();
    let mut distinct = std::collections::HashSet::new();
    let mut different = 0usize;
    for p in pixmap.pixels() {
        if *p != first {
            different += 1;
        }
        distinct.insert((p.red(), p.green(), p.blue()));
    }
    assert!(different > 10_000, "{different} non-background pixels");
    assert!(distinct.len() > 50, "{} distinct colors", distinct.len());
}
