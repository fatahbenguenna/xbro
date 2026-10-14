//! Hit testing, node rectangles, text fragments, document size and dumps.

mod common;

use common::*;
use xbro_layout::{Point, Rect};

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}p{margin:0}</style>";

fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

fn hit(l: &Laid, x: f32, y: f32) -> Option<xbro_dom::NodeId> {
    l.tree
        .hit_test(Point::new(x, y), Point::ZERO)
        .map(|h| h.node)
}

#[test]
fn hit_text_node_link_and_block() {
    let Some(l) = lay(
        &page("<p id=p style='padding-bottom:20px'>Hello <a id=a href=x>link</a></p>"),
        400.0,
    ) else {
        return;
    };
    let p = l.id("p");
    let text = l.doc.children(p)[0];
    assert_eq!(hit(&l, 5.0, 10.0), Some(text));
    // Inside the link: its text node, whose parent is the link.
    let a_rect = l.tree.node_rect(l.id("a")).unwrap();
    let h = hit(&l, a_rect.x + 2.0, 10.0).unwrap();
    assert_eq!(l.doc.parent(h), Some(l.id("a")));
    // In the paragraph's padding: the paragraph.
    assert_eq!(hit(&l, 5.0, 30.0), Some(p));
    // Right of the text on the line: the paragraph (line boxes are transparent).
    assert_eq!(hit(&l, 300.0, 10.0), Some(p));
    // Below the content, on the canvas: the root element; outside: nothing.
    let root = l.doc.document_element().unwrap();
    assert_eq!(hit(&l, 5.0, 500.0), Some(root));
    assert_eq!(hit(&l, -5.0, 10.0), None);
    assert_eq!(hit(&l, 5.0, 5000.0), None);
}

#[test]
fn hit_positioned_above_flow_and_z_index() {
    let Some(l) = lay(
        &page(
            "<div style='position:relative'>\
               <span><b id=deep style='position:absolute;top:0;left:0;width:100px;height:40px;display:block'></b></span>\
             </div>\
             <p id=p>text that is covered</p>\
             <div id=lo style='position:absolute;top:100px;left:0;width:50px;height:50px;z-index:1'></div>\
             <div id=hi style='position:absolute;top:100px;left:0;width:50px;height:50px;z-index:5'></div>\
             <div id=later style='position:absolute;top:100px;left:0;width:50px;height:50px;z-index:1'></div>\
             <div id=neg style='position:absolute;top:200px;left:0;width:50px;height:50px;z-index:-1'></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // The positioned box nested in an earlier sibling is above later flow content.
    assert_eq!(hit(&l, 10.0, 5.0), Some(l.id("deep")));
    assert_eq!(hit(&l, 120.0, 5.0), Some(l.doc.children(l.id("p"))[0]));
    assert_eq!(hit(&l, 10.0, 110.0), Some(l.id("hi")));
    assert_eq!(hit(&l, 10.0, 210.0), Some(l.id("neg")));
}

#[test]
fn hit_fixed_uses_scroll() {
    let Some(l) = lay(
        &page(
            "<div id=tall style='height:3000px'></div>\
             <div id=f style='position:fixed;top:0;left:0;width:100px;height:30px'></div>",
        ),
        400.0,
    ) else {
        return;
    };
    let scroll = Point::new(0.0, 1000.0);
    // Document point (10, 1010) is viewport (10, 10): the fixed box.
    let h = l.tree.hit_test(Point::new(10.0, 1010.0), scroll).unwrap();
    assert_eq!(h.node, l.id("f"));
    // Without scroll, document (10, 1010) is the tall div.
    let h = l
        .tree
        .hit_test(Point::new(10.0, 1010.0), Point::ZERO)
        .unwrap();
    assert_eq!(h.node, l.id("tall"));
}

#[test]
fn hit_skips_hidden_and_respects_overflow_clip() {
    let Some(l) = lay(
        &page(
            "<div id=v style='height:40px'><div id=h style='visibility:hidden;height:40px'><span id=s style='visibility:visible'>shown</span></div></div>\
             <div id=clip style='overflow:hidden;height:20px;width:100px'><div id=big style='height:200px;width:300px'></div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // Hidden box: falls through to its visible parent.
    assert_eq!(hit(&l, 300.0, 10.0), Some(l.id("v")));
    // Its visible descendant is hit.
    let h = hit(&l, 2.0, 10.0).unwrap();
    assert_eq!(l.doc.parent(h), Some(l.id("s")));
    // Clipped: outside the overflow box nothing of its content is hit.
    assert_eq!(hit(&l, 50.0, 45.0), Some(l.id("big")));
    assert_ne!(hit(&l, 200.0, 45.0), Some(l.id("big")));
    assert_ne!(hit(&l, 50.0, 70.0), Some(l.id("big")));
}

#[test]
fn node_rect_finds_boxes_and_inline_fragments() {
    let Some(l) = lay(
        &page("<div style='height:500px'></div><h2 id=t style='margin:0'>Title</h2><p>x <span id=s>span</span> <a id=e></a></p>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(
        l.tree.node_rect(l.id("t")),
        Some(Rect::new(0.0, 500.0, 400.0, l.rect("t").height))
    );
    let s = l.tree.node_rect(l.id("s")).unwrap();
    assert!(s.x > 0.0 && s.y >= 500.0);
    // An empty inline element still has a fragment on its line.
    assert!(l.tree.node_rect(l.id("e")).is_some());
    // Text nodes too.
    let text = l.doc.children(l.id("t"))[0];
    assert!(l.tree.node_rect(text).is_some());
    // Nodes that generate nothing.
    assert_eq!(l.tree.node_rect(l.doc.head().unwrap()), None);
}

#[test]
fn text_fragments_in_order() {
    let Some(l) = lay(
        &page("<p>one <b>two</b></p><div style='float:left'>three</div><p>four</p>"),
        400.0,
    ) else {
        return;
    };
    let texts: Vec<&str> = l.texts().iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"one "));
    assert!(texts.contains(&"two"));
    assert!(texts.contains(&"three"));
    assert!(texts.contains(&"four"));
    let pos = |s: &str| texts.iter().position(|t| *t == s).unwrap();
    assert!(pos("one ") < pos("two"));
    assert!(pos("two") < pos("four"));
}

#[test]
fn document_size_includes_overflow_but_not_fixed_or_clipped() {
    let Some(l) = lay(
        &page(
            "<div style='width:1000px;height:10px'></div>\
             <div style='height:800px'></div>\
             <div style='position:fixed;top:5000px;width:10px;height:10px'></div>\
             <div style='overflow:hidden;height:10px'><div style='height:9000px'></div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.tree.document_size.width, 1000.0);
    assert_eq!(l.tree.document_size.height, 820.0);
    let Some(small) = lay(&page("<p>x</p>"), 400.0) else {
        return;
    };
    assert_eq!(
        small.tree.document_size,
        xbro_layout::Size::new(400.0, 600.0)
    );
    assert_eq!(small.tree.viewport, xbro_layout::Size::new(400.0, 600.0));
}

#[test]
fn dumps_are_readable() {
    let Some(l) = lay(
        &page("<p id=p>Hello <b>world</b></p><ul><li>item</li></ul>"),
        400.0,
    ) else {
        return;
    };
    let dump = l.dump();
    assert!(dump.contains("Block <html>"), "{dump}");
    assert!(dump.contains("Block <p id=\"p\">"), "{dump}");
    assert!(dump.contains("text #text \"Hello \""), "{dump}");
    assert!(dump.contains("inline <b>"), "{dump}");
    assert!(dump.contains("marker disc"), "{dump}");
    let outline = l.outline();
    assert!(outline.contains("p 0,0 400x20"), "{outline}");
    assert!(
        outline.contains("| 0,0 400x20 \"Hello world\""),
        "{outline}"
    );
}
