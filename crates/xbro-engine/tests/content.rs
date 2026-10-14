//! Error pages, non-HTML documents, metadata, hover, find and other
//! page-level behaviour.

mod common;

use common::*;
use xbro_engine::{ClickAction, Cursor, NavigationRequest, Page, Point, Size, Url};
use xbro_style::Color;

fn temp_file(name: &str, bytes: &[u8]) -> Url {
    let dir = std::env::temp_dir().join(format!("xbro-engine-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    Url::from_file_path(path).unwrap()
}

#[test]
fn page_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Page>();
}

#[test]
fn missing_file_gives_an_error_page() {
    let url = fixture_url("does-not-exist.html");
    let page = load_url(&url, VIEWPORT);
    assert_eq!(page.url(), &url);
    assert_eq!(page.title(), "Problem loading page");
    assert!(laid_out_text(&page).contains("could not be found"));
    assert!(laid_out_text(&page).contains("does-not-exist.html"));
    assert_eq!(page.console().len(), 1, "{:?}", page.console());
}

#[test]
fn unsupported_scheme_gives_an_error_page() {
    let url = Url::parse("ftp://x").unwrap();
    let page = load_url(&url, VIEWPORT);
    assert_eq!(page.url(), &url);
    assert_eq!(page.title(), "Problem loading page");
    assert!(laid_out_text(&page).contains("ftp:"));
    assert!(page.console()[0].contains("unsupported URL scheme"));
    dump(&page, "error-ftp", 1024, 600);
}

#[test]
fn plain_text_documents_are_escaped_in_a_pre() {
    let url = temp_file("notes.txt", b"line <one> & two\n  indented\tline");
    let page = load_url(&url, VIEWPORT);
    assert_eq!(page.title(), "notes.txt");
    let doc = page.document();
    let pre = doc.get_elements_by_tag_name("pre");
    assert_eq!(pre.len(), 1);
    assert_eq!(
        doc.text_content(pre[0]),
        "line <one> & two\n  indented\tline"
    );
    assert!(laid_out_text(&page).contains("line <one> & two"));
    // JSON through a data: URL.
    let page = load_url(
        &Url::parse("data:application/json,%7B%22a%22%3A%20%3Cb%3E%7D").unwrap(),
        VIEWPORT,
    );
    assert_eq!(
        page.document()
            .text_content(page.document().get_elements_by_tag_name("pre")[0]),
        "{\"a\": <b>}"
    );
}

#[test]
fn image_documents_show_the_decoded_image() {
    let png = std::fs::read(fixtures_dir().join("img/logo.png")).unwrap();
    let url = temp_file("logo copy.png", &png);
    let page = load_url(&url, VIEWPORT);
    assert_eq!(page.title(), "logo copy.png (64 × 64)");
    let img = page.document().get_elements_by_tag_name("img")[0];
    let image = page.images().node(img).expect("decoded image");
    assert_eq!(image.natural_size(), Size::new(64.0, 64.0));
    let r = page.layout_tree().node_rect(img).unwrap();
    assert_eq!((r.width, r.height), (64.0, 64.0));
    // Centered horizontally.
    assert!((r.x + r.width / 2.0 - 512.0).abs() < 2.0, "{r:?}");
    assert!(page.console().is_empty(), "{:?}", page.console());
    // An undecodable image.
    let page = load_url(&temp_file("broken.png", b"not a png"), VIEWPORT);
    assert!(page.title().contains("Cannot display"));
    assert!(!page.console().is_empty());
}

#[test]
fn unsupported_content_page() {
    let url = temp_file("doc.pdf", b"%PDF-1.4 fake");
    let page = load_url(&url, VIEWPORT);
    let text = laid_out_text(&page);
    assert!(text.contains("application/pdf"), "{text}");
    assert!(text.contains("13 bytes"), "{text}");
}

#[test]
fn meta_refresh_and_base_url() {
    let page = load_html(
        "<head><base href='https://other.test/base/'>\
         <meta http-equiv='Refresh' content='5; URL=\"next.html\"'></head><a id=a href='x.html'>x</a>",
    );
    let (delay, url) = page.meta_refresh().unwrap();
    assert_eq!(delay, 5.0);
    assert_eq!(url.as_str(), "https://other.test/base/next.html");
    assert_eq!(page.base_url().as_str(), "https://other.test/base/");
    assert_eq!(
        page.link_at(center_of(&page, "a"), Point::ZERO)
            .unwrap()
            .as_str(),
        "https://other.test/base/x.html"
    );
    let page = load_html("<meta http-equiv=refresh content=0>");
    assert_eq!(
        page.meta_refresh().unwrap().1.as_str(),
        "https://example.test/dir/page.html"
    );
    assert!(load_html("<p>no refresh").meta_refresh().is_none());
    assert_eq!(
        load_html("<p>x").title(),
        "https://example.test/dir/page.html"
    );
    assert_eq!(
        load_html("<svg><title>icon</title></svg><title> Real \n title </title>").title(),
        "Real title"
    );
}

#[test]
fn links_fragments_and_javascript() {
    let mut page = load_html(
        "<style>body{margin:0} .spacer{height:2000px}</style>\
         <p><a id=top href='#'>top</a> <a id=down href='#target%20x'>down</a>\
         <a id=named href='#legacy'>named</a> <a id=js href='javascript:void(0)'>js</a>\
         <a id=ext href='other.html?q=1#frag' target=_blank>other</a></p>\
         <div class=spacer></div><h2 id='target x'>Target</h2>\
         <div class=spacer></div><a name=legacy></a><p>after anchor</p>",
    );
    assert_eq!(click_id(&mut page, "top"), ClickAction::ScrollTo(0.0));
    let ClickAction::ScrollTo(y) = click_id(&mut page, "down") else {
        panic!("scroll expected")
    };
    assert!(y > 1900.0, "{y}");
    let ClickAction::ScrollTo(y2) = click_id(&mut page, "named") else {
        panic!("scroll expected")
    };
    assert!(y2 > y + 1900.0, "{y2}");
    assert_eq!(click_id(&mut page, "js"), ClickAction::None);
    assert_eq!(
        click_id(&mut page, "ext"),
        ClickAction::Navigate(NavigationRequest::Get(
            "https://example.test/dir/other.html?q=1#frag"
                .parse()
                .unwrap()
        ))
    );
    assert_eq!(page.fragment_offset(""), Some(0.0));
    assert_eq!(page.fragment_offset("TOP"), Some(0.0));
    assert_eq!(page.fragment_offset("nope"), None);
    assert_eq!(page.fragment_offset("target x"), Some(y));
}

fn click_id(page: &mut Page, id: &str) -> ClickAction {
    let p = center_of(page, id);
    page.click(p, Point::ZERO)
}

#[test]
fn hover_restyles_links() {
    let mut page = load_html(
        "<style>a { color: rgb(0, 0, 200) } a:hover { color: rgb(255, 0, 0) }</style>\
         <p id=p>Some text and <a id=a href=x>a link</a></p>",
    );
    let a = by_id(&page, "a");
    let color = |page: &Page| page.styles().get(a).unwrap().color;
    assert_eq!(color(&page), Color::rgb(0, 0, 200));
    let link = center_of(&page, "a");
    assert!(page.set_hover(Some((link, Point::ZERO))));
    assert_eq!(page.hovered(), Some(a));
    assert_eq!(color(&page), Color::rgb(255, 0, 0));
    // Same element: nothing to do.
    assert!(!page.set_hover(Some((link, Point::ZERO))));
    // Moving over plain text needs no restyle, but leaving the link does.
    let r = page.layout_tree().node_rect(by_id(&page, "p")).unwrap();
    assert!(page.set_hover(Some((Point::new(r.x + 2.0, r.y + 2.0), Point::ZERO))));
    assert_eq!(color(&page), Color::rgb(0, 0, 200));
    assert!(!page.set_hover(None));
    // Without :hover rules, hovering never restyles.
    let mut plain = load_html("<a id=a href=x>link</a>");
    let p = center_of(&plain, "a");
    assert!(!plain.set_hover(Some((p, Point::ZERO))));
    assert_eq!(plain.hovered(), Some(by_id(&plain, "a")));
}

#[test]
fn cursors() {
    let page = load_html(
        "<p id=t>plain text</p><a id=a href=x>link</a> <button id=b>btn</button>\
         <input id=i> <div id=c style='cursor: wait; height: 20px'>busy</div>\
         <div id=e style='height: 30px'></div>",
    );
    let at = |id: &str| page.cursor_at(center_of(&page, id), Point::ZERO);
    let t = page.layout_tree().node_rect(by_id(&page, "t")).unwrap();
    let over_text = Point::new(t.x + 5.0, t.y + t.height / 2.0);
    assert_eq!(page.cursor_at(over_text, Point::ZERO), Cursor::Text);
    assert_eq!(at("t"), Cursor::Default, "right of the text");
    assert_eq!(at("a"), Cursor::Pointer);
    assert_eq!(at("b"), Cursor::Pointer);
    assert_eq!(at("i"), Cursor::Text);
    assert_eq!(at("c"), Cursor::Wait);
    assert_eq!(at("e"), Cursor::Default);
    assert_eq!(
        page.cursor_at(Point::new(-10.0, -10.0), Point::ZERO),
        Cursor::Default
    );
}

#[test]
fn find_in_page() {
    let page = load_html(
        "<style>p { width: 120px; font: 16px sans-serif }</style>\
         <p id=p>Hello WORLD, hello world, héllo. A long sentence that wraps around the box.</p>\
         <p>CAFÉ café</p>",
    );
    let hits = page.find("hello");
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits[0].y <= hits[1].y);
    assert!(hits.iter().all(|r| r.width > 10.0 && r.height > 5.0));
    assert_eq!(page.find("café").len(), 2);
    assert_eq!(page.find("HÉLLO").len(), 1);
    assert!(page.find("").is_empty());
    assert!(page.find("   ").is_empty());
    assert!(page.find("zzz").is_empty());
    // A match spanning a line break yields one rect per line.
    let across = page.find("sentence that wraps around");
    assert!(across.len() >= 2, "{across:?}");
    let ys: std::collections::HashSet<i32> = across.iter().map(|r| r.y as i32).collect();
    assert!(ys.len() >= 2);
}

#[test]
fn viewport_changes_and_degenerate_inputs() {
    let mut page = load_html("<p style='width: 50vw'>half</p>");
    page.set_viewport(Size::new(f32::NAN, -1.0));
    page.set_viewport(Size::new(f32::INFINITY, 1e30));
    page.set_viewport(Size::new(800.0, 600.0));
    assert_eq!(page.viewport(), Size::new(800.0, 600.0));
    let _ = page.click(
        Point::new(f32::NAN, f32::INFINITY),
        Point::new(f32::NAN, 0.0),
    );
    let _ = page.set_hover(Some((Point::new(f32::NAN, 1.0), Point::ZERO)));
    assert_eq!(page.focus_next(false), None);
    assert_eq!(page.activate_focused(), ClickAction::None);
    assert_eq!(page.submit_focused(), None);
    page.insert_text("x");
    page.backspace();
    let mut pixmap = xbro_paint::tiny_skia::Pixmap::new(10, 10).unwrap();
    page.render(
        &mut pixmap,
        &xbro_engine::RasterOptions {
            scale: f32::NAN,
            ..Default::default()
        },
    );
    // Empty and hostile documents.
    for html in [
        "",
        "<html>",
        "<svg><svg><svg>",
        "<img src='data:,'><img srcset=',,,'><picture><source srcset></picture>",
        "<style>@import url(file:///nonexistent.css); @import 'data:text/css,p{color:red}';</style><p>x",
        "<link rel=stylesheet href='data:text/css,@import \"data:text/css,p{}\";'>",
        "<base href='::bad'><a href='http://[::1'>bad</a>",
    ] {
        let page = load_html(html);
        let _ = page.find("x");
        let _ = page.title();
    }
}

#[test]
fn nested_imports_are_bounded() {
    // a.css imports itself: the depth limit stops the recursion.
    let css_url = temp_file("self.css", b"");
    let css = format!("@import url('{css_url}'); p {{ color: rgb(1, 2, 3) }}");
    std::fs::write(css_url.to_file_path().unwrap(), css).unwrap();
    let html = format!("<link rel=stylesheet href='{css_url}'><p id=p>x</p>");
    // A local document (web pages may not load `file:` stylesheets).
    let page = engine().load_html(&html, fixture_url("imports.html"), VIEWPORT);
    assert_eq!(
        page.styles().get(by_id(&page, "p")).unwrap().color,
        Color::rgb(1, 2, 3)
    );
    assert!(page
        .console()
        .iter()
        .any(|m| m.contains("too deeply nested")));
}

#[test]
fn inline_style_background_urls_are_resolved() {
    let tile = fixture_url("img/tile.png");
    let html = "<div id=d style=\"height: 50px; background: url('img/tile.png')\"></div>\
                <ul style='list-style-image: url(img/y18.gif)'><li>item</li></ul>";
    let page = xbro_engine::Engine::load_html(engine(), html, fixture_url("x.html"), VIEWPORT);
    // Keyed by the raw string used in the style and by the absolute URL.
    assert!(page.images().url("img/tile.png").is_some());
    assert!(page.images().url(tile.as_str()).is_some());
    assert!(page.images().url("img/y18.gif").is_some());
    let painted = page.display_list().items.iter().any(|i| {
        matches!(i, xbro_paint::DisplayItem::BackgroundImage { url, .. } if url == "img/tile.png")
    });
    assert!(painted);
}

#[test]
fn srcset_and_lazy_images() {
    let html = "<img id=a srcset='img/photo.png 1x, img/logo.png 2x'>\
                <img id=b src='data:image/gif;base64,R0lGODlhAQABAAAAACw=' data-src='img/logo.png'>\
                <img id=c srcset='img/logo.png 2x'>\
                <picture><source srcset='img/x.avif' type='image/avif'>\
                <source srcset='img/landscape.jpg' type='image/jpeg'><img id=d src='img/logo.png'></picture>\
                <svg id=s width=10 height=10 style='color: rgb(0, 128, 0)'><rect width=10 height=10 fill=currentColor /></svg>";
    let page = xbro_engine::Engine::load_html(engine(), html, fixture_url("x.html"), VIEWPORT);
    let size = |id: &str| {
        use xbro_layout::ReplacedSizes;
        page.images().node_size(by_id(&page, id))
    };
    // photo.png is 1x.
    let photo = page
        .images()
        .node(by_id(&page, "a"))
        .unwrap()
        .natural_size();
    assert_eq!(size("a"), Some(photo));
    assert_eq!(size("b"), Some(Size::new(64.0, 64.0)));
    // A 2x candidate shows at half its pixel size.
    assert_eq!(size("c"), Some(Size::new(32.0, 32.0)));
    let landscape = page
        .images()
        .node(by_id(&page, "d"))
        .unwrap()
        .natural_size();
    assert_ne!(landscape, Size::new(64.0, 64.0));
    assert_eq!(size("s"), Some(Size::new(10.0, 10.0)));
    // currentColor: the inline svg paints green.
    let pixmap = render(&page, 1024, 400);
    let r = page.layout_tree().node_rect(by_id(&page, "s")).unwrap();
    let px = pixmap
        .pixel((r.x + 5.0) as u32, (r.y + 5.0) as u32)
        .unwrap();
    assert_eq!((px.red(), px.green(), px.blue()), (0, 128, 0));
}

#[test]
fn image_map_areas_are_links() {
    let html = "<style>body { margin: 0 }</style>\
                <img id=img src='img/logo.png' usemap='#m' style='display: block'>\
                <map name=m><area shape=rect coords='0,0,32,64' href='left.html'>\
                <area shape=circle coords='48,32,10' href='#right'></map>\
                <div style='height: 3000px'></div><p id=right>right</p>";
    let mut page =
        xbro_engine::Engine::load_html(engine(), html, fixture_url("map.html"), VIEWPORT);
    let r = page.layout_tree().node_rect(by_id(&page, "img")).unwrap();
    let left = Point::new(r.x + 10.0, r.y + 10.0);
    assert_eq!(
        page.link_at(left, Point::ZERO).unwrap(),
        fixture_url("left.html")
    );
    assert_eq!(page.cursor_at(left, Point::ZERO), Cursor::Pointer);
    assert!(matches!(
        page.click(left, Point::ZERO),
        ClickAction::Navigate(_)
    ));
    let right = Point::new(r.x + 48.0, r.y + 32.0);
    assert!(matches!(page.click(right, Point::ZERO), ClickAction::ScrollTo(y) if y > 2900.0));
    // Outside every area: the image itself is not a link.
    assert_eq!(
        page.link_at(Point::new(r.x + 60.0, r.y + 2.0), Point::ZERO),
        None
    );
}

#[test]
fn image_count_is_limited_per_page() {
    // 450 distinct (tiny, valid) SVG data: URLs.
    let mut html = String::new();
    for i in 0..450 {
        html.push_str(&format!(
            "<img id=i{i} src=\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='{w}' height='2'/%3E\">",
            w = i + 1
        ));
    }
    let page = load_html(&html);
    assert_eq!(page.images().node_count(), 400);
    assert!(page.images().node(by_id(&page, "i0")).is_some());
    assert!(page.images().node(by_id(&page, "i449")).is_none());
    assert!(
        page.console()
            .iter()
            .any(|m| m.contains("50 more image(s)")),
        "{:?}",
        page.console()
    );
}
