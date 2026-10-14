//! Block layout: widths, margins, box-sizing, min/max, margin collapsing,
//! percentage heights, anonymous blocks, block-in-inline, `display:
//! contents` and generated content.

mod common;

use common::*;

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}</style>";

fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

#[test]
fn auto_width_fills_containing_block() {
    let Some(l) = lay(
        &page("<div id=a style='margin:0 10px;padding:5px;border:2px solid'>x</div>"),
        400.0,
    ) else {
        return;
    };
    let b = l.box_id("a");
    assert_eq!(b.border_box().x, 10.0);
    assert_eq!(b.border_box().width, 380.0);
    assert_eq!(b.content.width, 366.0);
    assert_eq!(b.content.x, 17.0);
}

#[test]
fn explicit_and_percent_widths() {
    let Some(l) = lay(
        &page(
            "<div id=a style='width:100px;padding:10px'>x</div>\
             <div id=b style='width:50%'>x</div>\
             <div id=c style='width:50%;box-sizing:border-box;padding:0 20px;border:5px solid'>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").width, 120.0);
    assert_eq!(l.box_id("a").content.width, 100.0);
    assert_eq!(l.rect("b").width, 200.0);
    assert_eq!(l.rect("c").width, 200.0);
    assert_eq!(l.box_id("c").content.width, 150.0);
}

#[test]
fn auto_margins_center_and_overconstrained() {
    let Some(l) = lay(
        &page(
            "<div id=a style='width:100px;margin:0 auto'>x</div>\
             <div id=b style='width:100px;margin-left:auto'>x</div>\
             <div id=c style='width:100px;margin-left:30px;margin-right:30px'>x</div>\
             <div id=d style='width:500px;margin:0 auto'>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").x, 150.0);
    assert_eq!(l.rect("b").x, 300.0);
    // Over-constrained: margin-right is ignored.
    assert_eq!(l.rect("c").x, 30.0);
    // Wider than the containing block: never pushed to the left.
    assert_eq!(l.rect("d").x, 0.0);
    assert_eq!(l.rect("d").width, 500.0);
}

#[test]
fn min_and_max_width() {
    let Some(l) = lay(
        &page(
            "<div id=a style='max-width:120px'>x</div>\
             <div id=b style='width:50px;min-width:80px'>x</div>\
             <div id=c style='max-width:100px;min-width:150px'>x</div>\
             <div id=d style='max-width:50%;margin:0 auto'>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").width, 120.0);
    assert_eq!(l.rect("b").width, 80.0);
    // min-width wins over max-width.
    assert_eq!(l.rect("c").width, 150.0);
    assert_eq!(l.rect("d").width, 200.0);
    assert_eq!(l.rect("d").x, 100.0);
}

#[test]
fn heights_explicit_auto_min_max() {
    let Some(l) = lay(
        &page(
            "<div id=a style='height:50px;padding:5px'>x</div>\
             <div id=b>one<br>two</div>\
             <div id=c style='min-height:70px'>x</div>\
             <div id=d style='max-height:10px'>x<br>y</div>\
             <div id=e style='height:40px;box-sizing:border-box;padding:10px'></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").height, 60.0);
    assert_eq!(l.rect("b").height, 40.0);
    assert_eq!(l.rect("c").height, 70.0);
    assert_eq!(l.rect("d").height, 10.0);
    assert_eq!(l.rect("e").height, 40.0);
    assert_eq!(l.box_id("e").content.height, 20.0);
    // Boxes follow each other.
    assert_eq!(l.rect("b").y, l.rect("a").bottom());
}

#[test]
fn percentage_heights_need_definite_containing_block() {
    let Some(l) = lay(
        &page(
            "<div id=p style='height:200px'><div id=a style='height:50%'></div></div>\
             <div id=q><div id=b style='height:50%'>x</div></div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").height, 100.0);
    // Indefinite containing block height: behaves as auto.
    assert_eq!(l.rect("b").height, 20.0);
}

#[test]
fn html_and_body_percentage_height_fill_viewport() {
    let Some(l) = lay(
        "<style>html,body{height:100%;margin:0}</style><body><div id=a style='height:50%'></div></body>",
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").height, 300.0);
}

#[test]
fn sibling_margins_collapse() {
    let Some(l) = lay(
        &page(
            "<div id=a style='margin-bottom:20px'>a</div>\
             <div id=b style='margin-top:30px'>b</div>\
             <div id=c style='margin-top:-10px'>c</div>\
             <div id=d style='margin-bottom:-5px;margin-top:0'>d</div>\
             <div id=e style='margin-top:15px'>e</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("b").y - l.rect("a").bottom(), 30.0);
    assert_eq!(l.rect("c").y - l.rect("b").bottom(), -10.0);
    assert_eq!(l.rect("e").y - l.rect("d").bottom(), 10.0);
}

#[test]
fn parent_and_first_last_child_margins_collapse() {
    let Some(l) = lay(
        &page(
            "<div id=p style='margin-top:10px;margin-bottom:5px'>\
               <div id=c style='margin-top:25px;margin-bottom:40px'>c</div>\
             </div>\
             <div id=n>next</div>",
        ),
        400.0,
    ) else {
        return;
    };
    // The child's margin escapes the parent (no border/padding).
    assert_eq!(l.rect("p").y, 25.0);
    assert_eq!(l.rect("c").y, 25.0);
    assert_eq!(l.rect("p").height, 20.0);
    assert_eq!(l.rect("n").y, 25.0 + 20.0 + 40.0);
}

#[test]
fn margins_do_not_collapse_through_border_padding_or_bfc() {
    let Some(l) = lay(
        &page(
            "<div id=p style='padding-top:1px'><div id=c style='margin-top:20px'>c</div></div>\
             <div id=q style='overflow:hidden'><div id=d style='margin-top:20px;margin-bottom:20px'>d</div></div>\
             <div id=r>r</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("p").y, 0.0);
    assert_eq!(l.rect("c").y, 21.0);
    let q = l.rect("q");
    assert_eq!(l.rect("d").y, q.y + 20.0);
    // A BFC root contains its children's margins.
    assert_eq!(q.height, 60.0);
    assert_eq!(l.rect("r").y, q.bottom());
}

#[test]
fn empty_blocks_collapse_through() {
    let Some(l) = lay(
        &page(
            "<div id=a>a</div>\
             <div id=e style='margin:30px 0 10px'></div>\
             <div id=b style='margin-top:20px'>b</div>\
             <div id=f style='margin:10px 0;height:0;padding:0'></div>\
             <div id=g style='margin:10px 0;min-height:5px'></div>\
             <div id=c>c</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("b").y - l.rect("a").bottom(), 30.0);
    assert_eq!(l.rect("e").height, 0.0);
    // height:0 boxes collapse too, min-height prevents it.
    let g = l.rect("g");
    assert_eq!(g.height, 5.0);
    assert_eq!(g.y - l.rect("b").bottom(), 10.0);
    assert_eq!(l.rect("c").y - g.bottom(), 10.0);
}

#[test]
fn h1_in_body_like_a_browser() {
    let Some(l) = lay(
        "<!doctype html><body><h1 id=h>Title</h1><p id=p>text</p></body>",
        800.0,
    ) else {
        return;
    };
    let h = l.rect("h");
    close(h.y, 21.44, 0.01);
    assert_eq!(h.x, 8.0);
    let body = l.box_of(l.doc.body().unwrap()).unwrap();
    assert_eq!(body.content.x, 8.0);
    assert_eq!(body.content.width, 784.0);
    close(l.rect("p").y, h.bottom() + 21.44, 0.01);
}

#[test]
fn anonymous_blocks_wrap_inline_runs() {
    let Some(l) = lay(
        &page("<div id=d>text <p id=p>para</p> tail <!-- --> </div>"),
        400.0,
    ) else {
        return;
    };
    let d = l.box_id("d");
    assert!(d.lines.is_empty());
    assert_eq!(d.children.len(), 3, "{}", l.dump());
    assert!(d.children[0].node.is_none());
    assert_eq!(line_text(&d.children[0].lines[0]), "text");
    assert_eq!(d.children[1].node, Some(l.id("p")));
    assert_eq!(line_text(&d.children[2].lines[0]), "tail");
}

#[test]
fn whitespace_only_runs_between_blocks_are_dropped() {
    let Some(l) = lay(
        &page("<div id=d>\n  <p>a</p>\n  <p>b</p>\n</div><div id=e>   \n </div>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.box_id("d").children.len(), 2);
    assert_eq!(l.rect("e").height, 0.0);
}

#[test]
fn block_inside_inline_splits_the_inline() {
    let Some(l) = lay(
        &page("<p id=p><a id=a href=x>before<div id=d>inside</div>after</a></p>"),
        400.0,
    ) else {
        return;
    };
    // `<p>` is closed by `<div>` in HTML; use a div container instead.
    let _ = l;
    let Some(l) = lay(
        &page("<div id=c><a id=a href=x>before<div id=d>inside</div>after</a></div>"),
        400.0,
    ) else {
        return;
    };
    let c = l.box_id("c");
    assert_eq!(c.children.len(), 3, "{}", l.dump());
    assert_eq!(line_text(&c.children[0].lines[0]), "before");
    assert_eq!(c.children[1].node, Some(l.id("d")));
    assert_eq!(line_text(&c.children[2].lines[0]), "after");
    assert_eq!(l.rect("d").y, 20.0);
    // The link's inline fragments are kept around its text.
    assert!(l.tree.node_rect(l.id("a")).is_some());
    // The text inside the block is hit-testable and belongs to the link.
    let hit = l
        .tree
        .hit_test(xbro_layout::Point::new(5.0, 25.0), xbro_layout::Point::ZERO)
        .expect("hit");
    assert!(l.doc.ancestors(hit.node).any(|a| a == l.id("a")));
}

#[test]
fn display_contents_promotes_children() {
    let Some(l) = lay(
        &page("<div id=c><section id=s style='display:contents'><p id=p style='margin:0'>a</p></section></div>"),
        400.0,
    ) else {
        return;
    };
    let c = l.box_id("c");
    assert_eq!(c.children.len(), 1);
    assert_eq!(c.children[0].node, Some(l.id("p")));
    assert!(l.box_of(l.id("s")).is_none());
}

#[test]
fn display_none_generates_nothing() {
    let Some(l) = lay(
        &page("<div id=a style='display:none'>x</div><div id=b>y</div>"),
        400.0,
    ) else {
        return;
    };
    assert!(l.box_of(l.id("a")).is_none());
    assert_eq!(l.rect("b").y, 0.0);
}

#[test]
fn before_and_after_generate_boxes() {
    let Some(l) = lay(
        &page(
            "<style>#a::before{content:'[' attr(id) ']'} #a::after{content:'!';display:block;height:30px}\
             q::before{content:open-quote} q::after{content:close-quote}</style>\
             <div id=a>x</div><p id=p><q>quote</q></p>",
        ),
        400.0,
    ) else {
        return;
    };
    let a = l.box_id("a");
    assert_eq!(a.children.len(), 2, "{}", l.dump());
    assert_eq!(line_text(&a.children[0].lines[0]), "[a]x");
    let after = &a.children[1];
    assert_eq!(after.pseudo, Some(xbro_layout::Pseudo::After));
    assert_eq!(after.border_box().height, 30.0);
    assert_eq!(line_text(&after.lines[0]), "!");
    assert_eq!(l.line_texts("p"), vec!["\u{201C}quote\u{201D}"]);
}

#[test]
fn before_with_image_content() {
    let images = FakeImages::new().url("icon.png", 16.0, 12.0);
    let Some(l) = lay_images(
        &page("<style>#a::before{content:url(icon.png)}</style><div id=a>x</div>"),
        400.0,
        images,
    ) else {
        return;
    };
    let a = l.box_id("a");
    let atomic = a.lines[0]
        .fragments
        .iter()
        .find_map(|f| match f {
            xbro_layout::Fragment::Atomic(b) => Some(b),
            _ => None,
        })
        .expect("image fragment");
    assert_eq!(atomic.kind, xbro_layout::BoxKind::Image);
    assert_eq!(atomic.content.width, 16.0);
    assert_eq!(atomic.content.height, 12.0);
}

#[test]
fn overflow_hidden_and_inline_block_are_bfc_roots_containing_floats() {
    let Some(l) = lay(
        &page(
            "<div id=a style='overflow:hidden'><div style='float:left;height:80px;width:10px'></div>x</div>\
             <div id=b><div style='float:left;height:80px;width:10px'></div>x</div>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").height, 80.0);
    assert_eq!(l.rect("b").height, 20.0);
}

#[test]
fn hr_and_auto_margins_from_ua() {
    let Some(l) = lay(
        "<body style='margin:0'><hr id=h style='width:100px'></body>",
        400.0,
    ) else {
        return;
    };
    let h = l.rect("h");
    // 100px content + 1px inset borders, centered.
    assert_eq!(h.x, 149.0);
    assert_eq!(h.width, 102.0);
    assert_eq!(h.height, 2.0);
}
