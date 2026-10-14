//! Regression tests for assorted layout corner cases (block, inline,
//! floats, positioning, aspect ratios, flex, tables, form controls), with
//! the geometry browsers produce.

mod common;

use common::*;
use xbro_layout::{Point, Rect, Size};

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}</style>";

fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

fn at(l: &Laid, id: &str) -> (f32, f32) {
    let r = l.rect(id);
    (r.x, r.y)
}

fn size(r: Rect) -> (f32, f32) {
    (r.width, r.height)
}

#[test]
fn clearfix_pseudo_elements_contain_floats() {
    let Some(l) = lay(
        &page(
            "<style>.cf::after{content:'';display:table;clear:both}
                    .cf2::after{content:'';display:block;clear:both}</style>
             <div id=a class=cf><div style='float:left;width:50px;height:100px'></div></div>
             <div id=b class=cf2><div style='float:right;width:50px;height:70px'></div></div>
             <p id=p style='margin:0'>x</p>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a"), Rect::new(0.0, 0.0, 400.0, 100.0));
    assert_eq!(l.rect("b"), Rect::new(0.0, 100.0, 400.0, 70.0));
    assert_eq!(l.rect("p").y, 170.0);
}

#[test]
fn inline_block_margins_and_vertical_align_extremes() {
    let Some(l) = lay(
        &page(
            "<div id=d>x<span id=s style='display:inline-block;margin:10px 0 5px;padding:3px'>y</span>z</div>
             <div id=e>x<span id=v style='display:inline-block;height:60px;width:10px;vertical-align:bottom'></span><span
               id=t style='display:inline-block;height:30px;width:10px;vertical-align:top'></span></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // The margin box of the inline-block sits on the line.
    assert_eq!(l.rect("s").y, 10.0);
    assert_eq!(l.rect("s").height, 26.0);
    assert_eq!(l.rect("d").height, 41.0);
    let e = l.rect("e");
    assert_eq!(e.height, 60.0);
    assert_eq!(l.rect("v").y, e.y);
    assert_eq!(l.rect("t").y, e.y);
}

#[test]
fn floats_clearance_and_margin_collapsing() {
    let Some(l) = lay(
        &page(
            "<div style='float:left;width:100px;height:200px'></div>
             <div id=o style='overflow:hidden'><div style='float:left;width:30px;height:20px'></div><div id=c style='clear:left'>x</div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // A BFC root next to a float; clearance inside it only clears its own floats.
    assert_eq!(l.rect("o"), Rect::new(100.0, 0.0, 300.0, 40.0));
    assert_eq!(l.rect("c").y, 20.0);

    let Some(l) = lay(
        &page(
            "<div id=a style='margin-bottom:20px'></div><div id=b style='min-height:10px;margin:10px 0'></div>
             <div id=c style='margin-top:5px'>x</div>
             <div id=d style='margin-bottom:20px;height:10px'></div><div style='position:absolute'></div>
             <div id=e style='margin-top:30px;height:10px'></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").height, 0.0);
    assert_eq!(l.rect("b").y, 20.0);
    assert_eq!(l.rect("c").y, 40.0);
    // An absolutely positioned box between two margins does not separate them.
    let d = l.rect("d");
    assert_eq!(l.rect("e").y, d.bottom() + 30.0);
}

#[test]
fn floats_next_to_text_and_bfc_roots() {
    let Some(l) = lay(
        &page(
            "<div style='width:300px'>first line text<div id=f style='float:left;width:100px;height:50px'></div><p id=p style='margin:0'>para</p></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // A float that fits on the current line goes to its top.
    assert_eq!(at(&l, "f"), (0.0, 0.0));
    assert_eq!(l.rect("p").y, 20.0);
    assert!(l.texts().iter().all(|t| t.rect.x >= 100.0));

    let Some(l) = lay(
        &page(
            "<div style='width:300px'><div style='float:left;width:200px;height:50px'></div>
             <table id=t><tr><td>aaaaaaaaaaaaaaaaaaaaa</td></tr></table>
             <div id=o style='overflow:hidden;width:150px;height:10px'></div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // Too wide to fit next to the float: moved below it.
    assert_eq!(l.rect("t").y, 50.0);
    assert!(l.rect("t").width > 100.0);
    assert_eq!(l.rect("o").y, l.rect("t").bottom());

    let Some(l) = lay(
        &page(
            "<div style='width:300px'><div id=f style='float:right;width:100px;height:50px;margin-right:-20px'></div>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").x, 220.0);

    let Some(l) = lay(
        &page(
            "<div style='width:1001px'><div id=a style='float:left;width:33.3333%'>a</div><div
               id=b style='float:left;width:33.3333%'>b</div><div id=c style='float:left;width:33.3333%'>c</div></div>",
        ),
        1100.0,
    ) else {
        return;
    };
    // Rounding never pushes the last float to the next line.
    assert_eq!(l.rect("c").y, 0.0);
    close(l.rect("c").x, 667.33, 0.01);
}

#[test]
fn absolutely_positioned_boxes() {
    let Some(l) = lay(
        &page(
            "<div id=r style='position:relative;padding:5px'>a<br>b<br>c<div id=a style='position:absolute;bottom:0;left:0;width:10px;height:10px'></div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("r").height, 70.0);
    assert_eq!(at(&l, "a"), (0.0, 60.0));

    let Some(l) = lay(
        &page(
            "<div id=m style='position:fixed;left:0;right:0;top:0;bottom:0;margin:auto;max-width:200px;max-height:100px'>modal</div>
             <div id=n style='position:absolute;left:0;right:0;margin:0 auto;max-width:100px'>x</div>
             <div id=o style='position:absolute;left:10px;right:10px;min-width:500px'>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("m"), Rect::new(100.0, 250.0, 200.0, 100.0));
    assert_eq!((l.rect("n").x, l.rect("n").width), (150.0, 100.0));
    assert_eq!((l.rect("o").x, l.rect("o").width), (10.0, 500.0));

    let Some(l) = lay(
        &page(
            "<p style='margin:0'>aaaa <span id=r style='position:relative;display:inline-block;width:50px;height:20px'><span
               id=a style='position:absolute;top:0;left:0;width:5px;height:5px'></span></span></p>
             <div id=q style='position:relative;width:300px;height:100px'>text <a id=i style='position:relative;padding:0 4px'>Cart<span
               id=b style='position:absolute;top:0;right:0;width:6px;height:6px'></span></a></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(at(&l, "a"), at(&l, "r"));
    // A positioned inline is the containing block of its abspos content.
    let frag = l.tree.node_rect(l.id("i")).expect("inline fragment");
    let b = l.rect("b");
    close(b.right(), frag.right(), 0.01);
    close(b.y, frag.y, 0.01);
}

#[test]
fn relative_offsets_and_document_size() {
    let Some(l) = lay(
        &page(
            "<div style='height:200px'><div id=a style='position:relative;top:10%;left:10%'>x</div></div>
             <div>a<span id=s style='display:inline-block;position:relative;top:10px'>b</span></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(at(&l, "a"), (40.0, 20.0));
    assert_eq!(l.rect("s").y, 210.0);

    // Fixed boxes and clipped overflow do not extend the document.
    let Some(l) = lay(
        &page(
            "<div id=f style='position:fixed;top:0;left:0;width:100px;height:3000px'></div>
             <div style='overflow:hidden;height:100px'><div style='height:5000px'></div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.tree.document_size, Size::new(400.0, 600.0));
    // Fixed content is hit in viewport coordinates.
    let hit = l
        .tree
        .hit_test(Point::new(10.0, 1000.0), Point::new(0.0, 900.0));
    assert_eq!(hit.map(|h| h.node), Some(l.id("f")));
}

#[test]
fn hit_testing_floats_and_links() {
    let Some(l) = lay(
        &page(
            "<div id=f style='float:left;width:100px;height:100px'></div><div id=b style='height:50px'></div>
             <p style='margin:0;clear:both'><a id=a href=#>link</a></p>",
        ),
        400.0,
    ) else {
        return;
    };
    let hit = |x, y| {
        l.tree
            .hit_test(Point::new(x, y), Point::ZERO)
            .map(|h| h.node)
    };
    // Floats paint (and are hit) above the in-flow block they overlap.
    assert_eq!(hit(10.0, 10.0), Some(l.id("f")));
    let a = l.id("a");
    let link_hit = hit(5.0, 110.0).expect("hit");
    assert!(link_hit == a || l.doc.parent(link_hit) == Some(a));
}

#[test]
fn aspect_ratio_and_percentage_heights() {
    let Some(l) = lay(
        &page(
            "<div id=a style='aspect-ratio:16/9'></div><div id=b style='height:50px;aspect-ratio:2'></div>
             <div id=c style='width:100px;aspect-ratio:1;box-sizing:border-box;padding:10px'></div>
             <div id=d style='width:100px;aspect-ratio:4'>line one line two line three</div>
             <div id=e style='width:100px;aspect-ratio:4;overflow:hidden'>line one line two line three</div>
             <span id=f style='display:inline-block;height:30px;aspect-ratio:1/2'></span>
             <div id=g style='float:left;height:40px;aspect-ratio:2'></div>
             <div id=h style='position:absolute;top:0;height:10px;aspect-ratio:3'></div>
             <div style='display:flex;width:300px'><div id=i style='flex:1;aspect-ratio:3'></div></div>
             <div id=p style='height:50%'>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(size(l.rect("a")), (400.0, 225.0));
    assert_eq!(size(l.rect("b")), (100.0, 50.0));
    assert_eq!(size(l.rect("c")), (100.0, 100.0));
    // `min-height: auto`: the content still fits, unless it is clipped.
    assert_eq!(size(l.rect("d")), (100.0, 60.0));
    assert_eq!(size(l.rect("e")), (100.0, 25.0));
    assert_eq!(size(l.rect("f")), (15.0, 30.0));
    assert_eq!(size(l.rect("g")), (80.0, 40.0));
    assert_eq!(size(l.rect("h")), (30.0, 10.0));
    assert_eq!(size(l.rect("i")), (300.0, 100.0));
    // A percentage of the auto-height body is auto.
    assert_eq!(l.rect("p").height, 20.0);
}

#[test]
fn shrink_to_fit_columns_and_whitespace() {
    let Some(l) = lay(
        &page(
            "<div id=f style='float:left'><p style='margin:0 10px'>hello world</p><div style='width:50%'>a</div></div>
             <div style='clear:both;width:300px;font-size:0'><div id=a style='display:inline-block;width:50%;font-size:16px'>a</div><div
               id=b style='display:inline-block;width:50%;font-size:16px'>b</div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    let f = l.rect("f");
    assert!(f.width > 80.0 && f.width < 120.0, "{f:?}");
    assert_eq!(f.height, 40.0);
    let (a, b) = (l.rect("a"), l.rect("b"));
    assert_eq!((a.x, b.x, a.y), (0.0, 150.0, b.y));
}

#[test]
fn flex_corner_cases() {
    let Some(l) = lay(
        &page(
            "<div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:100px;width:300px'>
               <div id=i1 style='height:60px'>a</div><div id=i2 style='height:60px'>b</div><div id=i3 style='height:30px'>c</div></div>
             <div style='display:flex;height:200px;width:400px'><div id=m style='margin:auto;width:50px;height:50px'></div></div>
             <div style='display:flex;flex-direction:row-reverse;width:300px'><div id=ra style='width:50px'>a</div><div
               id=rb style='width:50px;order:-1'>b</div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // Column wrap: two lines sharing the cross size (align-content: normal).
    let (i1, i2, i3) = (l.rect("i1"), l.rect("i2"), l.rect("i3"));
    assert_eq!((i1.x, i1.y, i2.y, i3.y), (0.0, 0.0, 0.0, 60.0));
    close(i2.x, i1.right(), 0.01);
    close(i2.x, 150.0, 1.0);
    close(i3.x, i2.x, 0.01);
    // Auto margins center in both axes.
    let m = l.rect("m");
    assert_eq!((m.x, m.y - 100.0), (175.0, 75.0));
    // Order then reversed: b first, at the right.
    assert_eq!((l.rect("rb").x, l.rect("ra").x), (250.0, 200.0));
}

#[test]
fn inline_flex_and_images_in_flex_rows() {
    let Some(l) = lay(
        &page(
            "<div>a <span id=f style='display:inline-flex;gap:5px'><span id=b>b</span><span id=c>c</span></span> d</div>",
        ),
        400.0,
    ) else {
        return;
    };
    let (f, b, c) = (l.rect("f"), l.rect("b"), l.rect("c"));
    assert!(f.x > 0.0);
    close(f.width, b.width + c.width + 5.0, 0.01);
    close(c.x, b.right() + 5.0, 0.01);

    // An image keeps its natural width (automatic minimum size) and
    // overflows a narrow flex row, like in browsers.
    let imgs = FakeImages::new().id("i", 400.0, 200.0);
    let Some(l) = lay_images(
        &page("<div style='display:flex;width:300px'><img id=i><div id=t style='flex:1'>text</div></div>"),
        400.0,
        imgs,
    ) else {
        return;
    };
    assert_eq!(l.rect("i"), Rect::new(0.0, 0.0, 400.0, 200.0));
    let t = l.rect("t");
    assert_eq!((t.x, t.height), (400.0, 200.0));
}

#[test]
fn table_corner_cases() {
    let Some(l) = lay(
        &page(
            "<table id=t width=100% cellspacing=0 cellpadding=0><tr><td id=a width=1% nowrap>abc def</td><td id=b>x</td></tr></table>
             <table id=e><tr><td></td><td></td></tr></table><table id=u></table><div id=v style='display:table'></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // A 1% column still gets its (nowrap) min-content width.
    let (a, b) = (l.rect("a"), l.rect("b"));
    assert!(a.width > 50.0 && a.width < 70.0, "{a:?}");
    close(b.x, a.right(), 0.01);
    close(b.right(), 400.0, 0.01);
    // Empty cells keep their padding and the spacing; empty tables are empty.
    assert_eq!(size(l.rect("e")), (10.0, 6.0));
    assert_eq!(size(l.rect("u")), (0.0, 0.0));
    assert_eq!(size(l.rect("v")), (0.0, 0.0));

    let Some(l) = lay(
        &page(
            "<table><tbody><tr><td rowspan=5 id=ra>a</td><td>b</td></tr></tbody><tbody><tr><td id=rc>c</td><td>d</td></tr></tbody></table>
             <table cellspacing=0 cellpadding=0><colgroup><col width=100><col style='width:50px'></colgroup>
               <tr><td id=ca>a</td><td id=cb>b</td><td id=cc>c</td></tr></table>",
        ),
        400.0,
    ) else {
        return;
    };
    // Row spans stop at the end of their row group.
    assert_eq!(l.rect("rc").y, l.rect("ra").bottom() + 2.0);
    // `<col>` widths.
    assert_eq!(l.rect("ca").width, 100.0);
    assert_eq!(l.rect("cb").width, 50.0);
    assert_eq!(l.rect("cc").x, 150.0);

    // Images with `max-width: 100%` do not widen an auto table.
    let imgs = FakeImages::new().id("i", 400.0, 200.0);
    let Some(l) = lay_images(
        &page("<table id=t><tr><td><img id=i style='max-width:100%'></td></tr></table>"),
        300.0,
        imgs,
    ) else {
        return;
    };
    assert_eq!(l.rect("t").width, 300.0);
    assert_eq!(size(l.rect("i")), (294.0, 147.0));
}

#[test]
fn buttons_center_their_content_vertically() {
    let Some(l) = lay(
        &page(
            "<button id=b style='height:60px;padding:0;border:0'>OK</button><input id=i type=submit value=Go style='height:60px;padding:0;border:0'>",
        ),
        400.0,
    ) else {
        return;
    };
    let (b, i) = (l.box_id("b"), l.box_id("i"));
    assert_eq!(b.border_box().y, 0.0);
    assert_eq!(i.border_box().y, 0.0);
    let (bl, il) = (&b.lines[0], &i.lines[0]);
    close(bl.rect.y, il.rect.y, 0.01);
    close(bl.rect.y + bl.rect.height / 2.0, 30.0, 0.01);
}

#[test]
fn sup_raises_the_line() {
    let Some(l) = lay(
        &page("<p id=p style='margin:0'>a<sup>2</sup> b</p><p id=q style='margin:0'>a b</p>"),
        400.0,
    ) else {
        return;
    };
    assert!(l.rect("p").height > 20.0);
    assert_eq!(l.rect("q").height, 20.0);
}

#[test]
fn text_indent_only_on_the_first_anonymous_block() {
    let Some(l) = lay(
        &page("<div id=d style='text-indent:50px'>first<div id=i>block</div>after</div>"),
        400.0,
    ) else {
        return;
    };
    let x_of = |s: &str| {
        l.texts()
            .iter()
            .find(|t| t.text == s)
            .map(|t| t.rect.x)
            .expect("text")
    };
    assert_eq!(x_of("first"), 50.0);
    assert_eq!(x_of("block"), 50.0);
    assert_eq!(x_of("after"), 0.0);
}

#[test]
fn list_markers_and_justified_lines() {
    let Some(l) = lay(
        &page(
            "<ol style='list-style-position:inside;margin:0;padding:0'><li id=a>alpha</li></ol>
             <ol style='margin:0'><li id=o>x<ol><li id=n>y</li></ol></li></ol>
             <p id=p style='text-align:justify;width:100px;margin:0'>aa bb cc dd ee ff gg hh</p>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.line_texts("a"), vec!["1.\u{a0}alpha".to_string()]);
    // Outside markers sit left of their item's content box.
    for id in ["o", "n"] {
        let b = l.box_id(id);
        let m = b.marker.as_ref().expect("marker");
        assert!(m.rect.right() <= b.content.x + 0.01, "{id}");
        assert!(m.rect.x >= b.content.x - 40.0, "{id}");
    }
    // Justified lines fill the width, except the last one.
    let p = l.box_id("p");
    assert_eq!(p.lines.len(), 2);
    let width = |i: usize| -> f32 {
        p.lines[i]
            .fragments
            .iter()
            .filter_map(|f| match f {
                xbro_layout::Fragment::Text(t) => Some(t.rect.width),
                _ => None,
            })
            .sum()
    };
    close(width(0), 100.0, 0.01);
    assert!(width(1) < 95.0);
}
