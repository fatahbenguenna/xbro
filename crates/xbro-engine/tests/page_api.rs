//! Page APIs used by the browser UI: find matches in fixed content,
//! fragment offsets of anchors that generate no box, and the caret rect of
//! focused text fields.

mod common;

use common::*;
use xbro_engine::{FindMatch, Page, Point, Rect};

fn top_of(page: &Page, id: &str) -> f32 {
    page.layout_tree()
        .node_rect(by_id(page, id))
        .unwrap_or_else(|| panic!("#{id} has no box"))
        .y
}

/// Whether the `<a name>` anchor `name` generates a box of its own.
fn anchor_has_box(page: &Page, name: &str) -> bool {
    let doc = page.document();
    let a = doc
        .descendants(doc.root())
        .find(|&n| doc.attr(n, "name") == Some(name))
        .expect("anchor");
    page.layout_tree().node_rect(a).is_some()
}

#[test]
fn find_flags_matches_in_fixed_content() {
    let page = load_html(
        "<body style='margin:0'>\
         <p id=first>needle in the flow</p>\
         <div style='position:fixed;top:10px;left:0;right:0;height:30px'>\
           fixed needle <span style='position:absolute;top:40px'>absolute needle</span></div>\
         <div style='height:3000px'></div>\
         <p id=last>needle at the end</p></body>",
    );
    let matches = page.find_matches("needle");
    assert_eq!(matches.len(), 4, "{matches:?}");
    // Document order, fixed ones flagged (the absolutely positioned span
    // is inside the fixed subtree).
    let fixed: Vec<bool> = matches.iter().map(|m| m.fixed).collect();
    assert_eq!(fixed, [false, true, true, false]);
    assert!(matches[0].rect.y < 30.0);
    assert!((10.0..40.0).contains(&matches[1].rect.y), "{matches:?}");
    assert!(matches[2].rect.y >= 50.0 && matches[2].rect.y < 100.0);
    assert!(matches[3].rect.y > 3000.0);

    // Conversions for a scrolled page.
    let scroll = Point::new(0.0, 2500.0);
    let m = matches[1];
    assert_eq!(m.document_rect(scroll).y, m.rect.y + 2500.0);
    assert_eq!(m.viewport_rect(scroll), m.rect);
    let flow = matches[3];
    assert_eq!(flow.document_rect(scroll), flow.rect);
    assert_eq!(flow.viewport_rect(scroll).y, flow.rect.y - 2500.0);

    // `find` keeps giving document rects (fixed ones for the unscrolled page).
    let rects: Vec<Rect> = page.find("needle");
    let expected: Vec<Rect> = matches
        .iter()
        .map(|m| m.document_rect(Point::ZERO))
        .collect();
    assert_eq!(rects, expected);
}

#[test]
fn fixed_backdrops_are_the_fixed_boxes_with_a_background() {
    let page = load_html(
        "<body style='margin:0'>\
         <div style='position:fixed;top:0;left:0;width:300px;height:40px;background:white'>header</div>\
         <div style='position:fixed;bottom:0;right:0;width:50px;height:20px'>no background</div>\
         <div style='position:fixed;top:100px;left:0;width:10px;height:10px;background:red;\
                     visibility:hidden'></div>\
         <div style='position:absolute;top:0;left:0;width:10px;height:10px;background:red'></div>\
         <div style='height:3000px'></div></body>",
    );
    assert_eq!(
        page.fixed_backdrops(),
        vec![Rect::new(0.0, 0.0, 300.0, 40.0)]
    );
    assert!(load_html("<p>plain</p>").fixed_backdrops().is_empty());
}

#[test]
fn find_match_conversions() {
    let m = FindMatch {
        rect: Rect::new(5.0, 10.0, 20.0, 16.0),
        fixed: true,
    };
    let scroll = Point::new(3.0, 100.0);
    assert_eq!(m.document_rect(scroll), Rect::new(8.0, 110.0, 20.0, 16.0));
    assert_eq!(m.viewport_rect(scroll), m.rect);
    let flow = FindMatch { fixed: false, ..m };
    assert_eq!(flow.document_rect(scroll), m.rect);
    assert_eq!(
        flow.viewport_rect(scroll),
        Rect::new(2.0, -90.0, 20.0, 16.0)
    );
}

#[test]
fn anchors_without_boxes_use_their_neighbours() {
    let page = load_html(
        "<body style='margin:0'>\
         <div style='height:500px'>intro</div>\
         <a name=gap></a>\
         <div id=next style='height:100px'>next</div>\
         <section id=contents style='display:contents'><p id=inner style='margin:0'>inside</p></section>\
         <div style='height:700px'><span id=tall>tall</span></div>\
         <a name=end></a> <span id=empty></span>\
         </body>",
    );
    // An empty anchor between blocks: the block that follows.
    assert!(!anchor_has_box(&page, "gap") && !anchor_has_box(&page, "end"));
    assert_eq!(page.fragment_offset("gap"), Some(500.0));
    assert_eq!(top_of(&page, "next"), 500.0);
    // `display: contents`: its own content.
    assert_eq!(
        page.fragment_offset("contents"),
        Some(top_of(&page, "inner"))
    );
    // Nothing rendered after it: just after the previous block (not the
    // top of the text inside it).
    let end = page.fragment_offset("end").expect("placed");
    let tall_block_bottom = top_of(&page, "inner") + 18.0 + 700.0;
    assert!(
        (end - tall_block_bottom).abs() < 10.0,
        "{end} vs {tall_block_bottom}"
    );
    assert!(end > top_of(&page, "tall") + 600.0);
}

#[test]
fn anchors_without_boxes_in_inline_and_nested_content() {
    let page = load_html(
        "<body style='margin:0'>\
         <div style='height:300px'></div>\
         <div id=wrap style='padding-top:20px'><span id=only style='display:contents'></span></div>\
         <p id=para style='margin:0'>words before <span id=inline style='display:contents'></span></p>\
         </body>",
    );
    let has_box = |id: &str| page.layout_tree().node_rect(by_id(&page, id)).is_some();
    assert!(!has_box("only") && !has_box("inline"));
    // The last target of the document, after inline text: on the text's line.
    let inline = page.fragment_offset("inline").expect("placed");
    assert_eq!(inline, top_of(&page, "para"));
    // A target alone in a box, followed by more content: that content.
    let only = page.fragment_offset("only").expect("placed");
    assert_eq!(only, top_of(&page, "para"));
    assert!(only > top_of(&page, "wrap"));
}

#[test]
fn fixed_or_hidden_targets_do_not_scroll() {
    let page = load_html(
        "<body style='margin:0'><div style='height:2000px'></div>\
         <div id=bar style='position:fixed;bottom:0'>bar <a name=infixed></a></div>\
         <div id=gone style='display:none'><a name=hidden></a></div>\
         <a name=last></a></body>",
    );
    assert_eq!(page.fragment_offset("bar"), None, "always in view");
    assert_eq!(page.fragment_offset("infixed"), None);
    assert_eq!(page.fragment_offset("gone"), None);
    assert_eq!(page.fragment_offset("hidden"), None);
    // The fixed bar after the spacer is skipped: the end of the spacer.
    assert_eq!(page.fragment_offset("last"), Some(2000.0));
}

#[test]
fn caret_rect_follows_the_focused_field() {
    let mut page = load_html(
        "<body style='margin:0'><div style='height:1500px'></div>\
         <input id=q size=10>\
         <div style='position:fixed;top:0'><input id=f></div>\
         <p id=out>text</p></body>",
    );
    assert_eq!(page.caret_rect(), None, "no focus");
    let q = center_of(&page, "q");
    page.click(q, Point::ZERO);
    assert!(page.has_text_focus());
    let caret = page.caret_rect().expect("a caret");
    let field = page.layout_tree().node_rect(by_id(&page, "q")).unwrap();
    assert!(
        caret.y >= field.y && caret.bottom() <= field.bottom(),
        "{caret:?} {field:?}"
    );
    assert!(caret.x >= field.x && caret.x <= field.right());
    // Typing moves it along.
    page.insert_text("hello");
    let typed = page.caret_rect().unwrap();
    assert!(typed.x > caret.x, "{typed:?} {caret:?}");
    // A field in fixed content is always in view.
    let f = center_of(&page, "f");
    page.click(f, Point::ZERO);
    assert_eq!(page.focused(), Some(by_id(&page, "f")));
    assert_eq!(page.caret_rect(), None);
    page.blur();
    assert_eq!(page.caret_rect(), None);
}
