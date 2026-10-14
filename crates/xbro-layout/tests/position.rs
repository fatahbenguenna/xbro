//! Relative, absolute, fixed and sticky positioning.

mod common;

use common::*;
use xbro_layout::Rect;

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}p{margin:0}</style>";

fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

#[test]
fn relative_offsets_move_the_subtree_only() {
    let Some(l) = lay(
        &page(
            "<div id=r style='position:relative;left:10px;top:5px'><p id=c>x</p></div>\
             <div id=n>next</div>\
             <div id=rb style='position:relative;right:4px;bottom:3px'>y</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("r").x, 10.0);
    assert_eq!(l.rect("r").y, 5.0);
    assert_eq!(l.rect("c").x, 10.0);
    assert_eq!(l.rect("c").y, 5.0);
    // Siblings are laid out as if the box had not moved.
    assert_eq!(l.rect("n").y, 20.0);
    assert_eq!(l.rect("rb").x, -4.0);
    assert_eq!(l.rect("rb").y, 37.0);
}

#[test]
fn sticky_is_positioned_without_offset() {
    let Some(l) = lay(
        &page("<div id=s style='position:sticky;top:30px'><div id=a style='position:absolute;top:0;left:0'>a</div>x</div>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("s").y, 0.0);
    // It still establishes a containing block.
    assert_eq!(l.rect("a").y, 0.0);
}

#[test]
fn absolute_insets_against_positioned_ancestor_padding_box() {
    let Some(l) = lay(
        &page(
            "<div style='height:30px'></div>\
             <div id=cb style='position:relative;margin:0 20px;padding:10px;border:5px solid;height:100px'>\
               <div id=tl style='position:absolute;top:0;left:0;width:10px;height:10px'></div>\
               <div id=br style='position:absolute;right:0;bottom:0;width:10px;height:10px'></div>\
               <div id=pc style='position:absolute;left:50%;top:50%;width:10px;height:10px'></div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    let cb = l.box_id("cb").padding_box();
    assert_eq!(cb, Rect::new(25.0, 35.0, 350.0, 120.0));
    assert_eq!(l.rect("tl"), Rect::new(25.0, 35.0, 10.0, 10.0));
    assert_eq!(l.rect("br"), Rect::new(365.0, 145.0, 10.0, 10.0));
    assert_eq!(l.rect("pc").x, 25.0 + 175.0);
    assert_eq!(l.rect("pc").y, 35.0 + 60.0);
}

#[test]
fn inset_zero_with_auto_margins_centers() {
    let Some(l) = lay(
        &page(
            "<div style='position:relative;width:300px;height:200px'>\
               <div id=c style='position:absolute;inset:0;margin:auto;width:100px;height:50px'></div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("c"), Rect::new(100.0, 75.0, 100.0, 50.0));
}

#[test]
fn absolute_auto_width_stretches_or_shrinks() {
    let Some(l) = lay(
        &page(
            "<div style='position:relative;width:300px;height:100px'>\
               <div id=s style='position:absolute;left:10px;right:20px;top:0;bottom:0;padding:5px'>x</div>\
               <div id=k style='position:absolute;left:0;top:0'>shrink</div>\
               <div id=r style='position:absolute;right:0;top:50px'>right</div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("s"), Rect::new(10.0, 0.0, 270.0, 100.0));
    let k = l.box_id("k");
    assert!(k.content.width < 60.0, "{}", l.dump());
    let r = l.rect("r");
    close(r.right(), 300.0, 0.01);
    assert_eq!(r.y, 50.0);
}

#[test]
fn absolute_static_position_when_insets_are_auto() {
    let Some(l) = lay(
        &page(
            "<div style='position:relative;padding:7px'>\
               <p>line</p>\
               <div id=a style='position:absolute'>abs</div>\
               <p id=after>after</p>\
               <p>x<span id=i style='position:absolute'>in</span></p>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").x, 7.0);
    assert_eq!(l.rect("a").y, 27.0);
    // Out of flow: the next paragraph is not pushed down.
    assert_eq!(l.rect("after").y, 27.0);
    let i = l.rect("i");
    assert!(i.x > 7.0 && i.y == 47.0, "{i:?}");
}

#[test]
fn absolute_without_positioned_ancestor_uses_initial_containing_block() {
    let Some(l) = lay(
        &page("<div style='margin:50px'><div id=a style='position:absolute;bottom:0;right:0;width:10px;height:10px'></div></div>"),
        400.0,
    ) else {
        return;
    };
    // Viewport 400×600 at the document origin.
    assert_eq!(l.rect("a"), Rect::new(390.0, 590.0, 10.0, 10.0));
}

#[test]
fn absolute_boxes_do_not_affect_parent_height() {
    let Some(l) = lay(
        &page("<div id=p style='position:relative'><div style='position:absolute;height:500px;width:10px'></div></div><div id=n>n</div>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("p").height, 0.0);
    assert_eq!(l.rect("n").y, 0.0);
    // But they extend the scrollable document.
    assert!(l.tree.document_size.height >= 500.0);
}

#[test]
fn absolute_replaced_and_percent_heights() {
    let images = FakeImages::new().id("img", 40.0, 20.0);
    let Some(l) = lay_images(
        &page(
            "<div style='position:relative;width:200px;height:100px'>\
               <img id=img style='position:absolute;right:0;bottom:0'>\
               <div id=h style='position:absolute;top:0;height:50%;width:10px'></div>\
             </div>",
        ),
        400.0,
        images,
    ) else {
        return;
    };
    assert_eq!(l.rect("img"), Rect::new(160.0, 80.0, 40.0, 20.0));
    assert_eq!(l.rect("h").height, 50.0);
}

#[test]
fn fixed_boxes_use_viewport_coordinates() {
    let Some(l) = lay(
        &page(
            "<div style='height:2000px'></div>\
             <div id=f style='position:fixed;bottom:10px;right:10px;width:50px;height:20px'>\
               <div id=inner style='position:absolute;top:0;left:0'>i</div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    let f = l.box_id("f");
    assert!(f.is_fixed());
    assert_eq!(f.border_box(), Rect::new(340.0, 570.0, 50.0, 20.0));
    // Descendants are positioned relative to it (viewport coordinates too).
    assert_eq!(l.rect("inner").x, 340.0);
    assert_eq!(l.rect("inner").y, 570.0);
    // Fixed boxes do not extend the document.
    assert_eq!(l.tree.document_size.height, 2000.0);
}

#[test]
fn z_index_does_not_change_geometry_and_nested_relative() {
    let Some(l) = lay(
        &page(
            "<div id=o style='position:relative;top:10px'>\
               <div id=i style='position:relative;left:5px'>\
                 <div id=a style='position:absolute;left:0;top:0'>a</div>\
               </div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    // Offsets accumulate through the subtree moves.
    assert_eq!(l.rect("a").x, 5.0);
    assert_eq!(l.rect("a").y, 10.0);
}

#[test]
fn absolute_inside_inline_block_and_float_follow_their_ancestors() {
    let Some(l) = lay(
        &page(
            "<div style='position:relative'>\
               <span style='display:inline-block;margin-left:100px'>x<span id=a style='position:absolute'>a</span></span>\
               <div style='float:right;width:50px'><div id=b style='position:absolute'>b</div></div>\
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert!(l.rect("a").x > 100.0, "{}", l.dump());
    assert_eq!(l.rect("b").x, 350.0);
}

#[test]
fn relative_inline_elements_move_their_fragments() {
    let Some(l) = lay(
        &page("<p id=p>a<span id=s style='position:relative;top:-4px;left:3px;background:red'>b<i id=i>c</i></span>d</p>"),
        400.0,
    ) else {
        return;
    };
    let line = &l.box_id("p").lines[0];
    let mut text = Vec::new();
    let mut span = None;
    for f in &line.fragments {
        match f {
            xbro_layout::Fragment::Text(t) => text.push((t.text.clone(), t.rect, t.baseline)),
            xbro_layout::Fragment::InlineBox(ib) if ib.node == l.id("s") => span = Some(ib.rect),
            _ => {}
        }
    }
    let base = line.baseline;
    let get = |s: &str| text.iter().find(|t| t.0 == s).unwrap().clone();
    assert_eq!(get("a").2, base);
    assert_eq!(get("d").2, base);
    // Moved text, nested inline included; neighbours are not affected.
    assert_eq!(get("b").2, base - 4.0);
    assert_eq!(get("c").2, base - 4.0);
    assert_eq!(get("b").1.x, get("a").1.right() + 3.0);
    assert_eq!(get("d").1.x, get("c").1.right() - 3.0);
    let span = span.unwrap();
    assert_eq!(span.x, get("b").1.x);
    assert!((span.right() - get("c").1.right()).abs() < 0.01);
}
