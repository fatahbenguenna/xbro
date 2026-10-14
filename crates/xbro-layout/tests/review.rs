//! Regression tests for the problems found by the adversarial review of the
//! layout crate: line box alignment of `top`/`bottom` boxes, float
//! placement next to cleared boxes, `<button>` sizing, overflow
//! propagation to the viewport, line breaking around atomic inlines and
//! soft hyphens, and hit testing in the painter's stacking order.

mod common;

use common::*;
use xbro_layout::{Fragment, LayoutBox, LineBox, Point, Rect, Size};

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}p{margin:0}</style>";

fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

fn hit(l: &Laid, x: f32, y: f32) -> Option<xbro_dom::NodeId> {
    l.tree
        .hit_test(Point::new(x, y), Point::ZERO)
        .map(|h| h.node)
}

/// The first text fragment of a line.
fn first_text(line: &LineBox) -> &xbro_layout::TextFragment {
    line.fragments
        .iter()
        .find_map(|f| match f {
            Fragment::Text(t) => Some(t),
            _ => None,
        })
        .expect("a text fragment")
}

/// The atomic inline box of `node` on `line`.
fn atomic<'a>(line: &'a LineBox, l: &Laid, id: &str) -> &'a LayoutBox {
    let node = l.id(id);
    line.fragments
        .iter()
        .find_map(|f| match f {
            Fragment::Atomic(a) if a.node == Some(node) => Some(a),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no atomic #{id} on the line"))
}

// ---------------------------------------------------------------------
// vertical-align: top / bottom
// ---------------------------------------------------------------------

#[test]
fn bottom_aligned_box_taller_than_the_line_pushes_the_text_down() {
    let Some(l) = lay(
        &page(
            "<div id=ref>x</div>
             <div id=d>x<span id=v style='display:inline-block;width:10px;height:60px;vertical-align:bottom'></span></div>",
        ),
        400.0,
    ) else {
        return;
    };
    let reference = &l.box_id("ref").lines[0];
    let below = reference.rect.bottom() - reference.baseline;
    let line = &l.box_id("d").lines[0];
    assert_eq!(line.rect.height, 60.0);
    // Like browsers, the rest of the line sits at the bottom of the line:
    // the baseline is as far from the line's bottom as in a plain line.
    close(line.rect.bottom() - line.baseline, below, 0.01);
    close(first_text(line).baseline, line.baseline, 0.01);
    let v = atomic(line, &l, "v").margin_box();
    assert_eq!((v.y, v.bottom()), (line.rect.y, line.rect.bottom()));
}

#[test]
fn top_aligned_box_taller_than_the_line_keeps_the_text_at_the_top() {
    let Some(l) = lay(
        &page(
            "<div id=ref>x</div>
             <div id=d>x<span id=v style='display:inline-block;width:10px;height:60px;vertical-align:top'></span></div>",
        ),
        400.0,
    ) else {
        return;
    };
    let reference = &l.box_id("ref").lines[0];
    let above = reference.baseline - reference.rect.y;
    let line = &l.box_id("d").lines[0];
    assert_eq!(line.rect.height, 60.0);
    close(line.baseline - line.rect.y, above, 0.01);
    assert_eq!(atomic(line, &l, "v").margin_box().y, line.rect.y);
}

#[test]
fn top_and_bottom_aligned_boxes_on_one_line() {
    let Some(l) = lay(
        &page(
            "<div id=ref>x</div>
             <div id=d>x<span id=t style='display:inline-block;width:10px;height:60px;vertical-align:top'></span><span
               id=b style='display:inline-block;width:10px;height:80px;vertical-align:bottom'></span></div>",
        ),
        400.0,
    ) else {
        return;
    };
    let reference = &l.box_id("ref").lines[0];
    let above = reference.baseline - reference.rect.y;
    let line = &l.box_id("d").lines[0];
    // The tallest box sets the line height; the top box extends the line
    // below the text (60 - above), then the bottom box pushes it down.
    assert_eq!(line.rect.height, 80.0);
    let t = atomic(line, &l, "t").margin_box();
    let b = atomic(line, &l, "b").margin_box();
    assert_eq!(t.y, line.rect.y);
    assert_eq!(b.bottom(), line.rect.bottom());
    close(line.baseline - line.rect.y, 80.0 - (60.0 - above), 0.01);
    assert!(l.rect("d").height == 80.0);
}

// ---------------------------------------------------------------------
// Floats and clearance
// ---------------------------------------------------------------------

#[test]
fn float_before_a_box_that_clears_it_stays_at_the_top() {
    // Clearance stops the cleared box's margin from collapsing with its
    // parent's: the float is not pushed down by that margin.
    let Some(l) = lay(
        &page(
            "<div id=f style='float:left;width:50px;height:100px'></div>
             <div id=c style='clear:left;margin-top:30px'>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").y, 0.0);
    assert_eq!(l.rect("c").y, 100.0);

    // A margin larger than the float: no clearance is needed.
    let Some(l) = lay(
        &page(
            "<div id=f style='float:left;width:50px;height:100px'></div>
             <div id=c style='clear:both;margin-top:150px'>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").y, 0.0);
    assert_eq!(l.rect("c").y, 150.0);
}

#[test]
fn float_before_a_box_clearing_the_other_side_goes_below_the_margin() {
    // `clear: right` does not clear a left float: the next box's margin
    // collapses with the body's and the float starts below it.
    let Some(l) = lay(
        &page(
            "<div id=f style='float:left;width:50px;height:100px'></div>
             <div id=c style='clear:right;margin-top:30px'>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").y, 30.0);
    assert_eq!(l.rect("c").y, 30.0);
    assert!(l.texts()[0].rect.x >= 50.0);
}

// ---------------------------------------------------------------------
// <button>
// ---------------------------------------------------------------------

#[test]
fn block_level_button_is_as_wide_as_its_content() {
    let Some(l) = lay(
        &page(
            "<div style='width:300px'>
               <button id=a style='display:block;padding:0;border:0'>Click</button>
               <button id=b style='display:block;padding:0;border:0;margin:0 auto'>Click</button>
               <button id=c style='display:block;padding:0;border:0;width:100%'>Click</button>
               <div id=d style='display:block'>Click</div>
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    let a = l.rect("a");
    assert!(a.width > 20.0 && a.width < 100.0, "{a:?}");
    assert_eq!(a.x, 0.0);
    let b = l.rect("b");
    close(b.width, a.width, 0.01);
    close(b.x, (300.0 - b.width) / 2.0, 0.01);
    assert_eq!(l.rect("c").width, 300.0);
    assert_eq!(l.rect("d").width, 300.0);
}

#[test]
fn block_level_button_contains_its_floats_and_margins() {
    let Some(l) = lay(
        &page(
            "<button id=a style='display:block;padding:0;border:0;margin:0'><span
               style='float:left;width:40px;height:50px'></span></button>
             <button id=b style='display:block;padding:0;border:0;margin:0'><span
               style='display:block;margin-top:20px;height:10px'></span></button>",
        ),
        400.0,
    ) else {
        return;
    };
    // A formatting context: the float is inside, the child's margin too.
    assert_eq!(l.rect("a"), Rect::new(0.0, 0.0, 40.0, 50.0));
    let b = l.rect("b");
    assert_eq!((b.y, b.height), (50.0, 30.0));
}

#[test]
fn shrink_to_fit_width_with_floats_and_following_blocks() {
    let Some(l) = lay(
        &page(
            "<div id=a style='float:left'><div style='float:left;width:40px;height:50px'></div><div
               id=o style='overflow:hidden;width:30px;height:10px'></div></div>
             <div id=b style='float:left;clear:left'><div style='float:left;width:40px;height:50px'></div><div
               id=p style='width:30px;height:10px'></div></div>
             <div id=c style='float:left;clear:left'><div style='float:left;width:40px;height:10px'></div><div
               style='height:10px'></div><div style='float:left;width:40px;height:10px'></div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    // A formatting context root is placed beside the floats before it.
    assert_eq!(l.rect("a").width, 70.0);
    assert_eq!(l.rect("o").x, 40.0);
    // An ordinary block does not add to them.
    assert_eq!(l.rect("b").width, 40.0);
    // An in-flow block ends the row of floats.
    assert_eq!(l.rect("c").width, 40.0);
}

// ---------------------------------------------------------------------
// overflow propagation to the viewport
// ---------------------------------------------------------------------

#[test]
fn body_overflow_applies_to_the_viewport() {
    // `overflow-x: hidden` on <body> (very common) goes to the viewport:
    // body is not a formatting context (its child's margin collapses
    // through it) and the page does not scroll horizontally.
    let Some(l) = lay(
        "<!doctype html><style>body{margin:0;font:16px/20px sans-serif}</style>
         <body id=body style='overflow-x:hidden'>
           <div id=c style='margin-top:30px;width:2000px;height:1000px'></div></body>",
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("body").y, 30.0);
    assert_eq!(l.rect("c").y, 30.0);
    assert_eq!(l.tree.document_size, Size::new(400.0, 1030.0));
    // Body does not clip its content: the overflowing part is hit.
    let c = l.id("c");
    assert_eq!(hit(&l, 390.0, 500.0), Some(c));

    // `overflow: hidden` on the root: no scrolling at all; <body> keeps
    // its own `overflow` (a formatting context: no margin collapsing).
    let Some(l) = lay(
        "<!doctype html><style>html{overflow:hidden}body{margin:0;font:16px/20px sans-serif}</style>
         <body id=body style='overflow:auto'>
           <div id=c style='margin-top:30px;width:2000px;height:1000px'></div></body>",
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("body").y, 0.0);
    assert_eq!(l.rect("c").y, 30.0);
    assert_eq!(l.tree.document_size, Size::new(400.0, 600.0));

    // Without any overflow, the document grows with its content.
    let Some(l) = lay(
        &page("<div style='width:2000px;height:1000px'></div>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.tree.document_size, Size::new(2000.0, 1000.0));
}

// ---------------------------------------------------------------------
// Line breaking
// ---------------------------------------------------------------------

#[test]
fn no_break_between_an_atomic_inline_and_adjacent_punctuation() {
    let Some(l) = lay(
        &page(
            "<div id=a style='width:100px'>aaaa <span style='display:inline-block;width:60px;height:10px'></span>, bbb</div>
             <div id=b style='width:100px'>aaaa (<span style='display:inline-block;width:60px;height:10px'></span>) bbb</div>
             <div id=c style='width:100px'>aaaa <img style='width:60px;height:10px'><span>.</span></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.line_texts("a"), ["aaaa", ",", "bbb"]);
    assert_eq!(l.line_texts("b"), ["aaaa", "()", "bbb"]);
    assert_eq!(l.line_texts("c"), ["aaaa", "."]);
    // The punctuation stays on the line of the atomic inline.
    for id in ["a", "b", "c"] {
        let line = &l.box_id(id).lines[1];
        assert!(line
            .fragments
            .iter()
            .any(|f| matches!(f, Fragment::Atomic(_))));
    }
}

#[test]
fn soft_hyphen_break_leaves_room_for_the_hyphen() {
    let Some(l) = lay(
        &page("<p id=p style='width:60px'>aa&shy;bb&shy;cc&shy;dd&shy;ee</p>"),
        400.0,
    ) else {
        return;
    };
    let lines = l.line_texts("p");
    assert_eq!(lines, ["aabb-", "ccddee"]);
    for line in &l.box_id("p").lines {
        let right = line
            .fragments
            .iter()
            .filter_map(|f| match f {
                Fragment::Text(t) => Some(t.rect.right()),
                _ => None,
            })
            .fold(0.0f32, f32::max);
        assert!(right <= 60.0 + 0.01, "line overflows: {right}");
    }
    // A single overlong piece still breaks at the soft hyphen.
    let Some(l) = lay(&page("<p id=p style='width:20px'>aaaa&shy;bbbb</p>"), 400.0) else {
        return;
    };
    assert_eq!(l.line_texts("p"), ["aaaa-", "bbbb"]);
}

// ---------------------------------------------------------------------
// Hit testing in paint order
// ---------------------------------------------------------------------

#[test]
fn hit_test_z_index_inside_a_z_auto_positioned_box() {
    // A drop-down menu (z-index: 10) inside a relative box with `z-index:
    // auto` is above later positioned content: it belongs to the root
    // stacking context, not to its parent's layer.
    let Some(l) = lay(
        &page(
            "<div style='position:relative;height:20px'>
               <div id=menu style='position:absolute;top:20px;z-index:10;width:100px;height:100px'></div>
             </div>
             <div id=content style='position:relative;height:200px'></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(hit(&l, 50.0, 50.0), Some(l.id("menu")));
    assert_eq!(hit(&l, 150.0, 50.0), Some(l.id("content")));

    // A real stacking context (z-index: 0) keeps its content inside it:
    // the later positioned sibling is above.
    let Some(l) = lay(
        &page(
            "<div style='position:relative;z-index:0;height:20px'>
               <div id=menu style='position:absolute;top:20px;z-index:10;width:100px;height:100px'></div>
             </div>
             <div id=content style='position:relative;height:200px'></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(hit(&l, 50.0, 50.0), Some(l.id("content")));
}

#[test]
fn hit_test_follows_floats_and_inline_content_paint_order() {
    let Some(l) = lay(
        &page(
            "<div id=f style='float:left;width:100px;height:50px'></div>
             <div id=b style='height:60px'><span id=s
               style='display:inline-block;width:50px;height:20px;margin-left:-100px'></span></div>",
        ),
        400.0,
    ) else {
        return;
    };
    let s = l.rect("s");
    assert_eq!(s.x, 0.0);
    // The inline-block (inline content) is painted above the float...
    assert_eq!(hit(&l, 10.0, s.y + 5.0), Some(l.id("s")));
    // ...and the float above the following block's background.
    assert_eq!(hit(&l, 80.0, 45.0), Some(l.id("f")));
    assert_eq!(hit(&l, 200.0, 45.0), Some(l.id("b")));
}

#[test]
fn hit_test_fixed_box_inside_positioned_parent_uses_viewport_coordinates() {
    let Some(l) = lay(
        &page(
            "<div style='position:relative;overflow:hidden;height:10px'>
               <div id=bar style='position:fixed;bottom:0;left:0;width:100px;height:30px'></div>
             </div>
             <div style='height:3000px'></div>",
        ),
        400.0,
    ) else {
        return;
    };
    let bar = l.id("bar");
    // The bar is at the bottom of the 600px viewport whatever the scroll,
    // and escapes the clip of its parent.
    let scroll = Point::new(0.0, 1000.0);
    let h = l.tree.hit_test(Point::new(50.0, 1000.0 + 590.0), scroll);
    assert_eq!(h.map(|h| h.node), Some(bar));
    let h = l.tree.hit_test(Point::new(50.0, 590.0), Point::ZERO);
    assert_eq!(h.map(|h| h.node), Some(bar));
    let h = l.tree.hit_test(Point::new(50.0, 590.0), scroll);
    assert_ne!(h.map(|h| h.node), Some(bar));
}

#[test]
fn hit_test_after_pseudo_element_follows_document_order() {
    // A positioned ::after is painted after the element's positioned
    // children (document order), so it is hit first where they overlap.
    let Some(l) = lay(
        &page(
            "<style>#o::after{content:'';position:absolute;top:0;left:0;width:50px;height:50px}</style>
             <div id=o style='position:relative;height:100px'>
               <div id=kid style='position:absolute;top:0;left:0;width:100px;height:100px'></div>
             </div>",
        ),
        400.0,
    ) else {
        return;
    };
    let o = l.id("o");
    // The ::after box hits as its originating element.
    assert_eq!(hit(&l, 10.0, 10.0), Some(o));
    assert_eq!(hit(&l, 80.0, 80.0), Some(l.id("kid")));
}

// ---------------------------------------------------------------------
// Geometry sanity of the changes on the fixtures
// ---------------------------------------------------------------------

#[test]
fn fixtures_keep_their_geometry_after_the_review_fixes() {
    let Some(f) = fonts() else {
        return;
    };
    let html = std::fs::read_to_string(fixtures_dir().join("basic.html")).expect("fixture");
    let l = lay_full(&html, Size::new(1024.0, 768.0), f, None);
    let h1 = l.tag("h1", 0);
    let r = l.box_of(h1).map(LayoutBox::border_box).expect("h1 box");
    assert!((r.y - 21.44).abs() < 0.1, "{r:?}");
    assert_eq!(r.x, 8.0);
    assert_eq!(l.tree.document_size.width, 1024.0);
    // Every text fragment of every fixture is hit at its centre unless it
    // is hidden, clipped or covered on purpose.
    for name in ["basic", "hn", "wiki", "tables", "text", "forms"] {
        let html =
            std::fs::read_to_string(fixtures_dir().join(format!("{name}.html"))).expect("fixture");
        let l = lay_full(&html, Size::new(1024.0, 768.0), f, None);
        for t in l.texts() {
            if t.rect.width < 1.0 || t.text.trim().is_empty() {
                continue;
            }
            let p = Point::new(t.rect.x + t.rect.width / 2.0, t.baseline - 2.0);
            let got = hit(&l, p.x, p.y);
            assert_eq!(got, Some(t.node), "{name}: {:?} at {p:?}", t.text);
        }
    }
    let _ = Rect::ZERO;
}
