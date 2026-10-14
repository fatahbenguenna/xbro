//! Floats (CSS 2.1 §9.5): placement, text wrapping, clear, BFC narrowing,
//! shrink-to-fit widths and containing BFC heights.

mod common;

use common::*;

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}p{margin:0}</style>";

fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

#[test]
fn left_and_right_floats_are_placed_at_the_edges() {
    let Some(l) = lay(
        &page(
            "<div id=c style='width:300px'>\
               <div id=l style='float:left;width:100px;height:50px'></div>\
               <div id=r style='float:right;width:50px;height:70px'></div>\
               <div id=l2 style='float:left;width:100px;height:20px'></div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("l"), xbro_layout::Rect::new(0.0, 0.0, 100.0, 50.0));
    assert_eq!(l.rect("r"), xbro_layout::Rect::new(250.0, 0.0, 50.0, 70.0));
    assert_eq!(l.rect("l2").x, 100.0);
    assert_eq!(l.rect("l2").y, 0.0);
    // Floats do not contribute to a non-BFC parent's height.
    assert_eq!(l.rect("c").height, 0.0);
}

#[test]
fn float_moves_down_when_it_does_not_fit() {
    let Some(l) = lay(
        &page(
            "<div style='width:300px'>\
               <div style='float:left;width:200px;height:50px'></div>\
               <div id=b style='float:left;width:200px;height:20px'></div>\
               <div id=c style='float:right;width:10px;height:10px'></div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("b").y, 50.0);
    assert_eq!(l.rect("b").x, 0.0);
    // Not above an earlier float.
    assert_eq!(l.rect("c").y, 50.0);
    assert_eq!(l.rect("c").x, 290.0);
}

#[test]
fn text_wraps_around_floats() {
    let Some(l) = lay(
        &page(
            "<div style='width:300px'>\
               <div id=f style='float:left;width:100px;height:45px'></div>\
               <p id=p>aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm nnn ooo ppp qqq rrr sss ttt</p>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    let p = l.box_id("p");
    // The paragraph box itself ignores the float.
    assert_eq!(p.border_box().x, 0.0);
    assert_eq!(p.border_box().width, 300.0);
    // Lines next to the float are shortened; lines below are not.
    assert_eq!(p.lines[0].rect.x, 100.0);
    assert_eq!(p.lines[0].rect.width, 200.0);
    assert_eq!(p.lines[2].rect.x, 100.0);
    assert_eq!(p.lines[3].rect.x, 0.0);
    assert_eq!(p.lines[3].rect.width, 300.0);
    let first_text = match &p.lines[0].fragments[0] {
        xbro_layout::Fragment::Text(t) => t.rect.x,
        _ => panic!(),
    };
    assert_eq!(first_text, 100.0);
}

#[test]
fn float_in_the_middle_of_a_line() {
    let Some(l) = lay(
        &page("<p id=p style='width:300px'>aaa <span id=f style='float:right;width:50px;height:30px'></span>bbb</p>"),
        400.0,
    ) else {
        return;
    };
    let f = l.rect("f");
    assert_eq!(f.x, 250.0);
    assert_eq!(f.y, 0.0);
    assert_eq!(l.line_texts("p"), vec!["aaa bbb"]);
    assert_eq!(l.box_id("p").lines[0].rect.width, 250.0);
}

#[test]
fn clear_moves_below_floats() {
    let Some(l) = lay(
        &page(
            "<div style='width:300px'>\
               <div style='float:left;width:100px;height:50px'></div>\
               <div style='float:right;width:100px;height:80px'></div>\
               <div id=a style='clear:left'>a</div>\
               <div id=b style='clear:both'>b</div>\
               <p id=p>x<br style='clear:both'>y</p>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").y, 50.0);
    assert_eq!(l.rect("b").y, 80.0);
    let Some(l2) = lay(
        &page(
            "<div style='width:300px'><div style='float:left;width:100px;height:50px'></div>\
             <p id=p>x<br style='clear:left'>y</p></div>",
        ),
        400.0,
    ) else {
        return;
    };
    let p = l2.box_id("p");
    assert_eq!(p.lines[0].rect.x, 100.0);
    assert_eq!(p.lines[1].rect.y, 50.0);
    assert_eq!(p.lines[1].rect.x, 0.0);
    let _ = l;
}

#[test]
fn bfc_roots_and_replaced_boxes_are_narrowed_next_to_floats() {
    let images = FakeImages::new().id("img", 100.0, 20.0);
    let Some(l) = lay_images(
        &page(
            "<div style='width:300px'>\
               <div style='float:left;width:80px;height:100px'></div>\
               <div id=o style='overflow:hidden'>x</div>\
               <div id=w style='overflow:hidden;width:100px'>x</div>\
               <img id=img style='display:block'>\
               <div id=wide style='overflow:hidden;width:250px'>x</div>\
             </div>",
        ),
        400.0,
        images,
    ) else {
        return;
    };
    let o = l.rect("o");
    assert_eq!((o.x, o.width, o.y), (80.0, 220.0, 0.0));
    let w = l.rect("w");
    assert_eq!((w.x, w.width), (80.0, 100.0));
    assert_eq!(l.rect("img").x, 80.0);
    // Too wide to fit next to the float: moved below it.
    let wide = l.rect("wide");
    assert_eq!((wide.x, wide.y), (0.0, 100.0));
}

#[test]
fn shrink_to_fit_float_width() {
    let Some(l) = lay(
        &page(
            "<div style='width:300px'>\
               <div id=s style='float:left;padding:5px'>short</div>\
               <div id=long style='float:left;clear:left'>aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm nnn</div>\
               <div id=b style='float:left;clear:left'><div style='width:120px'>x</div></div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    let s = l.box_id("s");
    let text_w = match &s.lines[0].fragments[0] {
        xbro_layout::Fragment::Text(t) => t.rect.width,
        _ => panic!(),
    };
    close(s.content.width, text_w, 0.01);
    assert_eq!(l.rect("long").width, 300.0);
    assert_eq!(l.rect("b").width, 120.0);
}

#[test]
fn bfc_root_auto_height_includes_floats() {
    let Some(l) = lay(
        &page(
            "<div id=a style='display:inline-block'><div style='float:left;width:10px;height:90px'></div></div>\
             <div id=b style='float:left'><div style='float:left;width:10px;height:70px'></div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").height, 90.0);
    assert_eq!(l.rect("b").height, 70.0);
    // The root element always contains floats.
    let root = l.tree.root.as_ref().unwrap();
    assert!(root.border_box().height >= 90.0);
}

#[test]
fn float_margins_and_relative_offset() {
    let Some(l) = lay(
        &page("<div style='width:300px'><div id=f style='float:right;width:50px;height:50px;margin:10px;position:relative;left:-5px'></div></div>"),
        400.0,
    ) else {
        return;
    };
    let f = l.rect("f");
    assert_eq!(f.x, 300.0 - 10.0 - 50.0 - 5.0);
    assert_eq!(f.y, 10.0);
}

#[test]
fn many_floats_and_long_word_do_not_lose_content() {
    let mut body = String::from("<div style='width:300px'>");
    for _ in 0..200 {
        body.push_str("<div style='float:left;width:290px;height:2px'></div>");
    }
    body.push_str("<p id=p>wwwwwwwwwwwwwwwwwwwwwwwwwwww end</p></div>");
    let Some(l) = lay(&page(&body), 400.0) else {
        return;
    };
    let texts = l.line_texts("p").concat();
    assert!(texts.contains("end"), "{texts:?}");
}

#[test]
fn float_before_collapsing_margins_is_placed_after_them() {
    let Some(l) = lay(
        "<!doctype html><body style='font:16px sans-serif'><div id=f style='float:left;width:50px;height:20px'></div><h1 id=h>Title</h1></body>",
        400.0,
    ) else {
        return;
    };
    // body margin (8) and h1 margin (21.44) collapse; the float follows them.
    close(l.rect("f").y, 21.44, 0.01);
    close(l.rect("h").y, 21.44, 0.01);
    // With a border, the parent's content top is fixed.
    let Some(l) = lay(
        "<!doctype html><body style='margin:0'><div style='border-top:1px solid'><div id=f style='float:left;width:5px;height:5px'></div><p id=p style='margin-top:30px'>x</p></div></body>",
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").y, 1.0);
    assert_eq!(l.rect("p").y, 31.0);
}

#[test]
fn float_after_content_stays_at_the_current_position() {
    let Some(l) = lay(
        "<!doctype html><body style='margin:0;font:16px/20px sans-serif'><div>a</div><div id=f style='float:left;width:5px;height:5px'></div><p id=p style='margin:30px 0 0'>x</p></body>",
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").y, 20.0);
    assert_eq!(l.rect("p").y, 50.0);
}
