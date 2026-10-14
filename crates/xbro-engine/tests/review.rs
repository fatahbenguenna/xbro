//! Regression tests for bugs found by reviewing the engine: hostile-input
//! blowups, content sniffing, metadata parsing, find order, image maps,
//! repaint signalling, the caret, the `file:` policy and inline SVG budgets.

mod common;

use std::time::{Duration, Instant};

use common::*;
use xbro_engine::{ClickAction, NavigationRequest, Page, Point, Size, Url};
use xbro_paint::DisplayItem;
use xbro_style::Color;

fn click_id(page: &mut Page, id: &str) -> ClickAction {
    let p = center_of(page, id);
    page.click(p, Point::ZERO)
}

/// Generous wall-clock bound for work that used to take many seconds
/// (debug builds of the tests are optimised at level 1).
const FAST: Duration = Duration::from_secs(4);

/// The last `Rect` item of the display list (the caret, when a text field
/// has the focus).
fn caret(page: &Page) -> Option<xbro_engine::Rect> {
    match page.display_list().items.last() {
        Some(DisplayItem::Rect { rect, .. }) if rect.width == 1.0 => Some(*rect),
        _ => None,
    }
}

#[test]
fn many_distinct_hover_selectors_load_quickly() {
    // Scanning the sheets for `:hover` used to deduplicate compounds with a
    // linear search: 40 000 distinct `:hover` rules took ~16 s.
    let mut html = String::from("<style>");
    for i in 0..40_000 {
        html.push_str(&format!(".c{i}:hover{{color:rgb(1,2,3)}}"));
    }
    html.push_str("</style><p><a id=a href=x>link</a></p>");
    let start = Instant::now();
    let mut page = load_html(&html);
    assert!(start.elapsed() < FAST, "load took {:?}", start.elapsed());
    // Beyond the compound limit every hover change restyles; none of these
    // rules applies, so nothing needs repainting.
    let link = center_of(&page, "a");
    assert!(!page.set_hover(Some((link, Point::ZERO))));
    assert_eq!(page.hovered(), Some(by_id(&page, "a")));
}

#[test]
fn hover_reports_repaints_only_for_visible_changes() {
    let mut page = load_html(
        "<style>.box:hover { cursor: pointer } a:hover { color: rgb(200, 0, 0) }</style>\
         <div class=box id=box style='height: 40px'>box</div><p><a id=a href=x>link</a></p>",
    );
    // `.box:hover` matches but only changes the cursor: no repaint.
    let r = page.layout_tree().node_rect(by_id(&page, "box")).unwrap();
    assert!(!page.set_hover(Some((Point::new(r.x + 200.0, r.y + 20.0), Point::ZERO))));
    let boxed = by_id(&page, "box");
    assert_eq!(page.hovered(), Some(boxed));
    assert_eq!(
        page.styles().get(boxed).unwrap().cursor,
        xbro_engine::Cursor::Pointer
    );
    let a = by_id(&page, "a");
    // The link changes color: repaint.
    assert!(page.set_hover(Some((center_of(&page, "a"), Point::ZERO))));
    assert_eq!(page.styles().get(a).unwrap().color, Color::rgb(200, 0, 0));
    assert!(page.set_hover(None));
}

#[test]
fn hover_still_works_beyond_the_compound_limit() {
    let mut html = String::from("<style>");
    for i in 0..5_000 {
        html.push_str(&format!(".c{i}:hover{{color:rgb(1,2,3)}}"));
    }
    html.push_str("a:hover{color:rgb(200,0,0)}</style><p><a id=a href=x>link</a></p>");
    let mut page = load_html(&html);
    let link = center_of(&page, "a");
    assert!(page.set_hover(Some((link, Point::ZERO))));
    let a = by_id(&page, "a");
    assert_eq!(page.styles().get(a).unwrap().color, Color::rgb(200, 0, 0));
}

#[test]
fn huge_disabled_fieldsets_stay_linear() {
    // `is_disabled` scanned the fieldset's children for its first legend
    // for every control: quadratic (5.6 s per submission / Tab here).
    let mut html = String::from("<form action=/s><fieldset disabled>");
    for i in 0..30_000 {
        html.push_str(&format!("<input name=n{i} value=v>"));
    }
    html.push_str("<legend>late legend <input id=inlegend name=l value=1></legend>");
    html.push_str("</fieldset><input id=q name=q value=x><input type=submit id=go></form>");
    let mut page = load_html(&html);
    let start = Instant::now();
    let ClickAction::Navigate(request) = click_id(&mut page, "go") else {
        panic!("submission expected");
    };
    let _ = page.focus_next(false);
    assert!(start.elapsed() < FAST, "took {:?}", start.elapsed());
    // Only the enabled fields are submitted: the field after the fieldset,
    // and the one in the first legend (exempt from the fieldset's disabling).
    let query = request.url().query().unwrap_or("");
    assert_eq!(query, "l=1&q=x");
}

#[test]
fn svg_image_documents_with_foreign_xhtml_are_images() {
    // An SVG document mentioning the XHTML namespace (foreignObject) was
    // sniffed as XHTML and parsed as an HTML page.
    let svg =
        "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='10'%3E\
               %3CforeignObject%3E%3Cdiv xmlns='http://www.w3.org/1999/xhtml'%3Ehi%3C/div%3E\
               %3C/foreignObject%3E%3Crect width='20' height='10'/%3E%3C/svg%3E";
    let page = load_url(&Url::parse(svg).unwrap(), VIEWPORT);
    let imgs = page.document().get_elements_by_tag_name("img");
    assert_eq!(imgs.len(), 1, "image document expected");
    let image = page.images().node(imgs[0]).expect("decoded once");
    assert_eq!(image.natural_size(), Size::new(20.0, 10.0));
    // Its title stays short although the URL is long.
    assert!(page.title().ends_with("(20 × 10)"), "{}", page.title());
    assert!(page.title().chars().count() < 130, "{}", page.title());
}

#[test]
fn xml_documents_are_xhtml_only_when_their_root_is_html() {
    // An Atom feed with XHTML content is XML (shown as text), not XHTML.
    let atom = "data:application/atom+xml,%3C?xml version='1.0'?%3E%3Cfeed%3E%3Centry%3E\
                %3Ccontent type='xhtml'%3E%3Cdiv xmlns='http://www.w3.org/1999/xhtml'%3E\
                %3Cp%3EHello%3C/p%3E%3C/div%3E%3C/content%3E%3C/entry%3E%3C/feed%3E";
    let page = load_url(&Url::parse(atom).unwrap(), VIEWPORT);
    let pre = page.document().get_elements_by_tag_name("pre");
    assert_eq!(pre.len(), 1, "text view expected");
    assert!(page.document().text_content(pre[0]).contains("<feed>"));
    // A real XHTML document served as XML (after a prolog and a comment).
    let xhtml = "data:application/xml,%3C?xml version='1.0'?%3E%3C!-- c --%3E\
                 %3Chtml xmlns='http://www.w3.org/1999/xhtml'%3E%3Cbody%3E%3Cp id='x'%3EHi%3C/p%3E\
                 %3C/body%3E%3C/html%3E";
    let page = load_url(&Url::parse(xhtml).unwrap(), VIEWPORT);
    assert!(page.document().get_element_by_id("x").is_some());
    assert!(page.document().get_elements_by_tag_name("pre").is_empty());
}

#[test]
fn meta_refresh_accepts_the_spec_separators() {
    let refresh = |content: &str| {
        load_html(&format!("<meta http-equiv=refresh content=\"{content}\">")).meta_refresh()
    };
    // Whitespace alone separates the time from the URL.
    let (delay, url) = refresh("5 url=next.html").expect("whitespace separator");
    assert_eq!(delay, 5.0);
    assert_eq!(url.as_str(), "https://example.test/dir/next.html");
    // A time starting with '.' is valid (0 s per the spec, kept fractional).
    let (delay, url) = refresh(".5; url=x.html").expect("leading dot");
    assert!((0.0..1.0).contains(&delay), "{delay}");
    assert_eq!(url.as_str(), "https://example.test/dir/x.html");
    let (_, url) = refresh("2 'quoted.html'").unwrap();
    assert_eq!(url.as_str(), "https://example.test/dir/quoted.html");
    assert!(refresh("5x; url=a").is_none());
    assert!(refresh("; url=a").is_none());
}

#[test]
fn titles_collapse_ascii_whitespace_only() {
    let page = load_html("<title>\n 100\u{a0}km \t à\u{3000}pied </title>");
    assert_eq!(page.title(), "100\u{a0}km à\u{3000}pied");
}

#[test]
fn find_reports_generated_content_in_document_order() {
    // `::after` text is attributed to its element, whose node index precedes
    // its children: its match came before the element's own text.
    let page = load_html(
        "<style>body{margin:0;font:16px sans-serif} #p::before{content:'foo '}\
         #p::after{content:' foo'}</style><p id=p>foo <b>bar</b></p><p>foo</p>",
    );
    let hits = page.find("foo");
    assert_eq!(hits.len(), 4, "{hits:?}");
    // First line: ::before, text, ::after — left to right.
    assert!(hits[0].x < hits[1].x && hits[1].x < hits[2].x, "{hits:?}");
    assert!((hits[0].y - hits[2].y).abs() < 1.0);
    assert!(hits[3].y > hits[2].y);
    // `::before` and `::after` texts are not searched as one string.
    assert!(page.find("foo foo").is_empty() || page.find("foo foo").len() == 1);
}

#[test]
fn find_ignores_soft_hyphens() {
    let page = load_html("<p>hyphen\u{ad}ation and co\u{ad}operation</p>");
    assert_eq!(page.find("hyphenation").len(), 1);
    assert_eq!(page.find("cooperation").len(), 1);
    // Text copied from a page keeps its (invisible) soft hyphens.
    assert_eq!(page.find("hyphen\u{ad}ation").len(), 1);
    assert!(page.find("\u{ad}").is_empty());
}

#[test]
fn console_messages_are_bounded() {
    let html = "<img src='http://[::1'>".repeat(5000);
    let page = load_html(&html);
    assert!(page.console().len() <= 1000, "{}", page.console().len());
    assert!(
        page.console().last().unwrap().contains("more message(s)"),
        "{:?}",
        page.console().last()
    );
}

#[test]
fn image_map_coordinates_are_relative_to_the_image_content_box() {
    let html = "<style>body{margin:0}</style>\
                <img id=img src='img/logo.png' usemap='#m'\
                 style='display:block;border:10px solid red;padding:5px'>\
                <map name=m><area shape=rect coords='0,0,4,4' href='corner.html'></map>";
    let page = engine().load_html(html, fixture_url("map.html"), VIEWPORT);
    let r = page.layout_tree().node_rect(by_id(&page, "img")).unwrap();
    // 15 px of border + padding: (16, 16) is (1, 1) in the image.
    assert_eq!(
        page.link_at(Point::new(r.x + 16.0, r.y + 16.0), Point::ZERO),
        Some(fixture_url("corner.html"))
    );
    // The border is not part of the image.
    assert_eq!(
        page.link_at(Point::new(r.x + 1.0, r.y + 1.0), Point::ZERO),
        None
    );
}

#[test]
fn image_button_coordinates_are_relative_to_the_image() {
    let html = "<style>body{margin:0}</style><form action=/go>\
                <input type=image id=b name=pos src='img/logo.png'\
                 style='display:block;border:6px solid;padding:4px'></form>";
    let mut page = engine().load_html(html, fixture_url("img.html"), VIEWPORT);
    let r = page.layout_tree().node_rect(by_id(&page, "b")).unwrap();
    let action = page.click(Point::new(r.x + 13.0, r.y + 20.0), Point::ZERO);
    let ClickAction::Navigate(NavigationRequest::Get(url)) = action else {
        panic!("submission expected, got {action:?}");
    };
    assert_eq!(url.query(), Some("pos.x=3&pos.y=10"));
}

#[test]
fn clicks_that_only_move_the_focus_request_a_repaint() {
    let mut page = load_html(
        "<input id=t><button id=b type=button>b</button>\
         <button id=formless>no form</button><input id=r type=range>",
    );
    assert_eq!(click_id(&mut page, "t"), ClickAction::Repaint);
    assert!(caret(&page).is_some());
    // The caret and ring of the text field go away: the page must repaint.
    assert_eq!(click_id(&mut page, "b"), ClickAction::Repaint);
    assert!(caret(&page).is_none());
    assert_eq!(page.focused(), Some(by_id(&page, "b")));
    click_id(&mut page, "t");
    assert_eq!(click_id(&mut page, "formless"), ClickAction::Repaint);
    click_id(&mut page, "t");
    assert_eq!(click_id(&mut page, "r"), ClickAction::Repaint);
    // Nothing visible changes: nothing to repaint.
    assert_eq!(click_id(&mut page, "b"), ClickAction::None);
    assert_eq!(click_id(&mut page, "b"), ClickAction::None);
}

#[test]
fn the_caret_stays_inside_an_overflowing_textarea() {
    let text = (1..=20)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut page = load_html(&format!("<textarea id=t rows=3>{text}</textarea>"));
    click_id(&mut page, "t");
    let field = page.layout_tree().node_rect(by_id(&page, "t")).unwrap();
    let c = caret(&page).expect("caret");
    assert!(
        c.y >= field.y && c.bottom() <= field.bottom(),
        "caret {c:?} outside {field:?}"
    );
}

#[test]
fn focus_next_into_a_fixed_header_does_not_ask_to_scroll() {
    let mut page = load_html(
        "<div style='position:fixed;top:0;left:0'><input id=search></div>\
         <div style='height:3000px'></div><a id=link href=x>link</a>",
    );
    // The fixed field is focused, but its viewport rect is no scroll target.
    assert_eq!(page.focus_next(false), None);
    assert_eq!(page.focused(), Some(by_id(&page, "search")));
    let rect = page.focus_next(false).expect("in-flow link rect");
    assert_eq!(page.focused(), Some(by_id(&page, "link")));
    assert!(rect.y > 2900.0, "{rect:?}");
}

#[test]
fn web_pages_cannot_load_local_files() {
    let css = fixtures_dir().join("wiki.css");
    let css = Url::from_file_path(css).unwrap();
    let logo = fixture_url("img/logo.png");
    let html = format!(
        "<link rel=stylesheet href='{css}'><style>@import url('{css}');</style>\
         <img id=logo src='{logo}'><div id=bg style=\"background:url('{logo}');height:5px\"></div>"
    );
    let page = load_html(&html);
    let logo_node = by_id(&page, "logo");
    assert!(page.images().node(logo_node).is_none());
    assert!(page.images().url(logo.as_str()).is_none());
    assert!(page.stylesheets().iter().all(|s| s.rules.is_empty()));
    let blocked = page
        .console()
        .iter()
        .filter(|m| m.contains("blocked"))
        .count();
    assert_eq!(blocked, 3, "{:?}", page.console());
    // The same document loaded from a local file may use them.
    let page = engine().load_html(&html, fixture_url("local.html"), VIEWPORT);
    assert!(page.images().node(by_id(&page, "logo")).is_some());
    assert!(page.stylesheets().iter().any(|s| !s.rules.is_empty()));
}

#[test]
fn identical_inline_svgs_share_one_document() {
    // 200 `<svg><use href=#s>` of a ~600 KB symbol: each was serialized
    // with its own copy of the symbol and parsed again (8 s, 160 MB of SVG).
    let mut html = String::from("<svg style='display:none'><symbol id=s viewBox='0 0 10 10'>");
    for i in 0..20_000 {
        html.push_str(&format!("<rect x='{}' width='1' height='1'/>", i % 10));
    }
    html.push_str("</symbol></svg>");
    for i in 0..200 {
        html.push_str(&format!(
            "<svg id=u{i} width=10 height=10><use href='#s'/></svg>"
        ));
    }
    let start = Instant::now();
    let page = engine().load_html(&html, fixture_url("sprites.html"), VIEWPORT);
    assert!(start.elapsed() < FAST, "took {:?}", start.elapsed());
    for i in [0, 100, 199] {
        let node = by_id(&page, &format!("u{i}"));
        let image = page.images().node(node).expect("rendered");
        assert_eq!(image.natural_size(), Size::new(10.0, 10.0));
    }
}

#[test]
fn distinct_inline_svgs_have_a_total_budget() {
    // Distinct `<svg>`s (different sizes) of a large sprite cannot be
    // shared: the total SVG serialized per page is bounded instead.
    let mut html = String::from("<svg style='display:none'><symbol id=s viewBox='0 0 10 10'>");
    for _ in 0..30_000 {
        html.push_str("<rect width='1' height='1'/>");
    }
    html.push_str("</symbol></svg><svg id=icon width=3 height=3></svg>");
    for i in 0..150 {
        html.push_str(&format!(
            "<svg width={} height=10><use href='#s'/></svg>",
            i + 1
        ));
    }
    let start = Instant::now();
    let page = engine().load_html(&html, fixture_url("sprites.html"), VIEWPORT);
    assert!(start.elapsed() < FAST, "took {:?}", start.elapsed());
    assert!(page.images().node(by_id(&page, "icon")).is_some());
    assert!(
        page.console()
            .iter()
            .any(|m| m.contains("Inline SVG budget exceeded")),
        "{:?}",
        page.console()
    );
}

#[test]
fn skipped_images_are_counted_per_url() {
    let mut html = String::new();
    for i in 0..400 {
        html.push_str(&format!(
            "<img src=\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='{}' height='1'/%3E\">",
            i + 1
        ));
    }
    // One more URL, used by 50 elements.
    html.push_str(&"<img src='data:image/svg+xml,%3Csvg/%3E'>".repeat(50));
    let page = load_html(&html);
    assert!(
        page.console()
            .iter()
            .any(|m| m.contains("1 more image(s) not loaded")),
        "{:?}",
        page.console()
    );
}

/// A local HTTP server answering every request after `delay`: `/page`
/// links six sheets, `/a{i}.css` imports `/b{i}.css`, which sets a color.
fn slow_server(delay: Duration) -> Url {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut stream = stream;
                let Ok(clone) = stream.try_clone() else {
                    return;
                };
                let mut reader = BufReader::new(clone);
                let mut request = String::new();
                let _ = reader.read_line(&mut request);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                std::thread::sleep(delay);
                let (ty, body) = if path == "/page" {
                    let links: String = (0..6)
                        .map(|i| format!("<link rel=stylesheet href=/a{i}.css>"))
                        .collect();
                    ("text/html", format!("{links}<p id=p0>x</p><p id=p5>y</p>"))
                } else if let Some(i) = path.strip_prefix("/a").and_then(|p| p.strip_suffix(".css"))
                {
                    ("text/css", format!("@import url(/b{i}.css);"))
                } else if let Some(i) = path.strip_prefix("/b").and_then(|p| p.strip_suffix(".css"))
                {
                    ("text/css", format!("#p{i} {{ color: rgb(9, 8, 7) }}"))
                } else {
                    ("text/plain", String::new())
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {ty}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    Url::parse(&format!("http://127.0.0.1:{port}/page")).unwrap()
}

#[test]
fn imports_of_different_sheets_are_fetched_in_parallel() {
    // Each importing sheet resolved its imports on its own: six sheets
    // with one import each cost six sequential round trips.
    let delay = Duration::from_millis(300);
    let url = slow_server(delay);
    let start = Instant::now();
    let page = load_url(&url, VIEWPORT);
    let elapsed = start.elapsed();
    assert!(page.console().is_empty(), "{:?}", page.console());
    for id in ["p0", "p5"] {
        assert_eq!(
            page.styles().get(by_id(&page, id)).unwrap().color,
            Color::rgb(9, 8, 7)
        );
    }
    // Page, linked sheets, imported sheets: three round trips (not eight).
    assert!(elapsed < delay * 6, "took {elapsed:?}");
}

#[test]
fn tab_skips_invisible_elements() {
    let mut page = load_html(
        "<a id=first href=a>first</a>\
         <ul style='visibility:hidden'><li><a id=menu href=m>menu item</a></li></ul>\
         <a id=last href=b>last</a>",
    );
    page.focus_next(false);
    assert_eq!(page.focused(), Some(by_id(&page, "first")));
    page.focus_next(false);
    assert_eq!(page.focused(), Some(by_id(&page, "last")));
}

#[test]
fn many_controls_with_a_form_attribute_submit_quickly() {
    // Each `form=` owner lookup scanned the document for the id: 20 000
    // such controls made one submission take ~17 s.
    let mut html = String::from("<div style='display:none'>");
    for i in 0..20_000 {
        html.push_str(&format!("<input form=f name=n{i} value=v>"));
    }
    html.push_str(
        "</div><input type=radio name=r form=f id=r1 checked><input type=radio name=r form=f id=r2>\
         <form id=f action=/s><input type=submit id=go></form>",
    );
    let mut page = load_html(&html);
    let start = Instant::now();
    let ClickAction::Navigate(request) = click_id(&mut page, "go") else {
        panic!("submission expected");
    };
    assert_eq!(request.url().query_pairs().count(), 20_001);
    // Radio groups and reset use the same lookups.
    click_id(&mut page, "r2");
    assert!(page
        .document()
        .attr(by_id(&page, "r1"), "checked")
        .is_none());
    assert!(start.elapsed() < FAST, "took {:?}", start.elapsed());
}

/// Pieces exercising the code paths changed by the review.
const PIECES: &[&str] = &[
    "<fieldset disabled>",
    "<legend>",
    "</legend>",
    "</fieldset>",
    "<form id=f action=?q#x>",
    "</form>",
    "<input form=f name=a value=1>",
    "<input form=nope name=b>",
    "<input type=radio name=r form=f>",
    "<input type=image name=i src='data:,' style='border:3px solid;padding:2px'>",
    "<button type=button>b</button>",
    "<input type=reset form=f>",
    "<style>p::before{content:'x '} p::after{content:' x'} a:hover{color:red}</style>",
    "<style>.c:hover{cursor:pointer} :focus{outline:1px solid}</style>",
    "<p class=c>text x",
    "</p>",
    "<svg style='display:none'><symbol id=s viewBox='0 0 2 2'><rect width=2 height=2/></symbol></svg>",
    "<svg width=8 height=8 id=u aria-hidden=true><use href='#s'/></svg>",
    "<svg width=9 height=9 class=k><use href='#missing'/><style>.k{}</style></svg>",
    "<img usemap=#m src='data:image/gif;base64,R0lGODlhAQABAAAAACw=' style='border:4px solid'>",
    "<map name=m><area shape=rect coords='0,0,9,9' href=#top></map>",
    "<div style='position:fixed;top:0'><input name=fixed></div>",
    "<div style='visibility:hidden'><a href=h>hidden</a></div>",
    "<meta http-equiv=refresh content='1 url=next'>",
    "<meta http-equiv=refresh content='.5;file:///etc/passwd'>",
    "<textarea rows=1>a\nb\nc\nd</textarea>",
    "<a href=#x>x</a>",
    "<h2 id=x>x</h2>",
    "hy\u{ad}phen ",
    "<title>t\u{a0}t</title>",
];

#[test]
fn random_documents_through_the_reviewed_paths() {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move |n: usize| {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) % n.max(1) as u64) as usize
    };
    for _ in 0..30 {
        let mut html = String::new();
        for _ in 0..next(80) + 10 {
            html.push_str(PIECES[next(PIECES.len())]);
        }
        let mut page = engine().load_html(&html, fixture_url("review/page.html"), VIEWPORT);
        let size = page.document_size();
        for _ in 0..25 {
            let p = Point::new(
                next(size.width.max(1.0) as usize) as f32,
                next(size.height.clamp(1.0, 2000.0) as usize) as f32,
            );
            let s = Point::new(0.0, next(300) as f32);
            match next(9) {
                0 => {
                    let _ = page.click(p, s);
                }
                1 => {
                    let _ = page.set_hover(Some((p, s)));
                }
                2 => {
                    let _ = page.focus_next(next(2) == 0);
                }
                3 => {
                    let _ = page.activate_focused();
                }
                4 => {
                    let _ = page.find(["x", "x x", "hyphen", "\u{ad}"][next(4)]);
                }
                5 => page.insert_text("ab\n"),
                6 => {
                    let _ = (page.link_at(p, s), page.cursor_at(p, s));
                }
                7 => {
                    let _ = page.blur();
                }
                _ => page.set_viewport(Size::new(
                    200.0 + next(1000) as f32,
                    200.0 + next(800) as f32,
                )),
            }
        }
        let _ = (page.title(), page.meta_refresh());
        if let Some((_, url)) = page.meta_refresh() {
            assert_ne!(url.scheme(), "javascript");
        }
    }
}
