//! Flexbox layout.

mod common;

use common::*;
use xbro_layout::{BoxKind, Point, Rect};

/// Lays out `body` (inside a zero-margin body) at `width`.
fn flex(body: &str, width: f32) -> Option<Laid> {
    lay(
        &format!("<style>body{{margin:0}} .i{{height:20px}}</style>{body}"),
        width,
    )
}

/// Border boxes of the children of the element with `id`, in box order.
fn kids(l: &Laid, id: &str) -> Vec<Rect> {
    l.box_id(id)
        .children
        .iter()
        .map(|c| c.border_box())
        .collect()
}

#[track_caller]
fn xs_ws(r: &[Rect], expect: &[(f32, f32)]) {
    assert_eq!(r.len(), expect.len(), "{r:?}");
    for (r, &(x, w)) in r.iter().zip(expect) {
        close(r.x, x, 0.02);
        close(r.width, w, 0.02);
    }
}

#[test]
fn grow_distributes_free_space_by_factor() {
    let Some(l) = flex(
        "<div id=f style='display:flex;width:600px'>
           <div class=i style='flex:1 1 100px'></div>
           <div class=i style='flex:2 1 100px'></div>
           <div class=i style='flex:0 1 100px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    xs_ws(
        &kids(&l, "f"),
        &[(0.0, 200.0), (200.0, 300.0), (500.0, 100.0)],
    );
    assert_eq!(l.box_id("f").kind, BoxKind::Flex);
}

#[test]
fn shrink_is_weighted_by_base_size() {
    let Some(l) = flex(
        "<div id=f style='display:flex;width:300px'>
           <div class=i style='flex:0 1 200px'></div>
           <div class=i style='flex:0 1 400px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    // 300px of overflow, shrunk 1:2.
    xs_ws(&kids(&l, "f"), &[(0.0, 100.0), (100.0, 200.0)]);
}

#[test]
fn min_and_max_constraints_freeze_items() {
    let Some(l) = flex(
        "<div id=a style='display:flex;width:600px'>
           <div class=i style='flex:1 1 0;max-width:100px'></div>
           <div class=i style='flex:1 1 0'></div>
           <div class=i style='flex:1 1 0'></div>
         </div>
         <div id=b style='display:flex;width:300px'>
           <div class=i style='flex:1 1 0;min-width:200px'></div>
           <div class=i style='flex:1 1 0'></div>
           <div class=i style='flex:1 1 0'></div>
         </div>
         <div id=c style='display:flex;width:100px'>
           <div class=i style='flex:0 1 200px;min-width:80px'></div>
           <div class=i style='flex:0 1 200px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    xs_ws(
        &kids(&l, "a"),
        &[(0.0, 100.0), (100.0, 250.0), (350.0, 250.0)],
    );
    xs_ws(
        &kids(&l, "b"),
        &[(0.0, 200.0), (200.0, 50.0), (250.0, 50.0)],
    );
    // Shrinking: the first item stops at its min-width, the second takes the rest.
    xs_ws(&kids(&l, "c"), &[(0.0, 80.0), (80.0, 20.0)]);
}

#[test]
fn hypothetical_size_is_clamped_and_basis_box_sizing() {
    let Some(l) = flex(
        "<div id=f style='display:flex;width:600px'>
           <div class=i style='flex:none;width:300px;max-width:120px'></div>
           <div class=i style='flex:none;flex-basis:100px;padding:0 10px;box-sizing:border-box'></div>
           <div class=i style='flex:none;flex-basis:50%'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    xs_ws(
        &kids(&l, "f"),
        &[(0.0, 120.0), (120.0, 100.0), (220.0, 300.0)],
    );
}

#[test]
fn wrapping_lines_and_gaps() {
    let Some(l) = flex(
        "<div id=f style='display:flex;flex-wrap:wrap;width:500px;column-gap:20px;row-gap:10px'>
           <div style='width:240px'></div>
           <div style='width:240px;height:50px'></div>
           <div style='width:240px;height:40px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let k = kids(&l, "f");
    assert_eq!(k.len(), 3);
    assert_eq!((k[0].x, k[0].y), (0.0, 0.0));
    assert_eq!((k[1].x, k[1].y), (260.0, 0.0));
    // Second line below the tallest item of the first one, plus the row gap.
    assert_eq!((k[2].x, k[2].y), (0.0, 60.0));
    // Auto-height items are stretched to their line's cross size.
    assert_eq!(k[0].height, 50.0);
    assert_eq!(k[2].height, 40.0);
    assert_eq!(l.rect("f").height, 100.0);
}

#[test]
fn justify_content_values() {
    let cases: [(&str, [f32; 3]); 7] = [
        ("flex-start", [0.0, 100.0, 200.0]),
        ("flex-end", [300.0, 400.0, 500.0]),
        ("center", [150.0, 250.0, 350.0]),
        ("space-between", [0.0, 250.0, 500.0]),
        ("space-around", [50.0, 250.0, 450.0]),
        ("space-evenly", [75.0, 250.0, 425.0]),
        ("stretch", [0.0, 100.0, 200.0]),
    ];
    for (mode, xs) in cases {
        let Some(l) = flex(
            &format!(
                "<div id=f style='display:flex;width:600px;justify-content:{mode}'>
                   <div class=i style='width:100px'></div><div class=i style='width:100px'></div><div class=i style='width:100px'></div>
                 </div>"
            ),
            800.0,
        ) else {
            return;
        };
        let k = kids(&l, "f");
        for (r, x) in k.iter().zip(xs) {
            close(r.x, x, 0.02);
        }
    }
}

#[test]
fn align_items_values() {
    for (mode, y, h) in [
        ("flex-start", 0.0, 50.0),
        ("flex-end", 150.0, 50.0),
        ("center", 75.0, 50.0),
        ("stretch", 0.0, 200.0),
    ] {
        let Some(l) = flex(
            &format!(
                "<div id=f style='display:flex;height:200px;align-items:{mode}'>
                   <div style='width:50px'><div style='height:50px'></div></div>
                 </div>"
            ),
            800.0,
        ) else {
            return;
        };
        let k = kids(&l, "f");
        close(k[0].y, y, 0.02);
        close(k[0].height, h, 0.02);
    }
    // align-self overrides align-items.
    let Some(l) = flex(
        "<div id=f style='display:flex;height:100px;align-items:flex-end'>
           <div class=i style='width:10px;align-self:flex-start'></div>
           <div class=i style='width:10px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let k = kids(&l, "f");
    assert_eq!((k[0].y, k[1].y), (0.0, 80.0));
}

#[test]
fn baseline_alignment_lines_up_first_baselines() {
    let Some(l) = flex(
        "<div id=f style='display:flex;align-items:baseline'>
           <div id=a style='font-size:12px'>small</div>
           <div id=b style='font-size:40px;padding-top:7px'>Big</div>
           <div id=c style='font-size:20px'>mid</div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let bl = |id: &str| l.box_id(id).lines[0].baseline;
    close(bl("a"), bl("b"), 0.05);
    close(bl("c"), bl("b"), 0.05);
    // The tallest item (with its padding) sits at the top.
    assert_eq!(l.rect("b").y, 0.0);
    assert!(l.rect("a").y > 10.0);
}

#[test]
fn stretched_items_share_the_tallest_height_and_are_relaid_out() {
    let Some(l) = flex(
        "<div id=f style='display:flex'>
           <div id=a style='width:100px'><div id=inner style='height:100%;background:red'></div></div>
           <div id=b style='width:100px;height:80px'></div>
           <div id=c style='width:100px'>text</div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").height, 80.0);
    assert_eq!(l.rect("a").height, 80.0);
    assert_eq!(l.rect("c").height, 80.0);
    // The stretched size is definite for the item's children.
    assert_eq!(l.rect("inner").height, 80.0);
}

#[test]
fn auto_margins_absorb_free_space() {
    let Some(l) = flex(
        "<nav id=f style='display:flex;width:600px;height:60px'>
           <div id=logo class=i style='width:100px'></div>
           <div id=menu class=i style='width:150px;margin-left:auto'></div>
           <div id=btn class=i style='width:50px'></div>
         </nav>
         <div id=g style='display:flex;width:300px;height:100px'>
           <div id=mid style='width:40px;height:20px;margin:auto'></div>
         </div>
         <div id=h style='display:flex;width:300px;height:100px'>
           <div id=bottom style='width:40px;height:20px;margin-top:auto'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("logo").x, 0.0);
    assert_eq!(l.rect("menu").x, 400.0);
    assert_eq!(l.rect("btn").x, 550.0);
    // Both axes: centered.
    let mid = l.rect("mid");
    let g = l.rect("g");
    assert_eq!((mid.x, mid.y - g.y), (130.0, 40.0));
    let bottom = l.rect("bottom");
    let h = l.rect("h");
    assert_eq!(bottom.bottom(), h.bottom());
    // Auto margins win over justify-content.
    let m = l.box_id("menu");
    assert_eq!(m.margin.left, 300.0);
}

#[test]
fn column_direction_heights_and_growth() {
    let Some(l) = flex(
        "<div id=a style='display:flex;flex-direction:column;row-gap:5px;width:200px'>
           <div style='height:50px'></div><div style='height:70px'></div>
         </div>
         <div id=b style='display:flex;flex-direction:column;height:300px'>
           <div style='height:50px'></div>
           <div id=grow style='flex:1'></div>
           <div style='height:50px'></div>
         </div>
         <div id=c style='display:flex;flex-direction:column;height:200px;justify-content:center;align-items:center;width:400px'>
           <div id=centered style='width:100px;height:50px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").height, 125.0);
    let a = kids(&l, "a");
    assert_eq!(a[1].y - a[0].y, 55.0);
    // Stretched across the container width.
    assert_eq!(a[0].width, 200.0);
    assert_eq!(l.rect("grow").height, 200.0);
    let c = l.rect("c");
    let centered = l.rect("centered");
    assert_eq!((centered.x - c.x, centered.y - c.y), (150.0, 75.0));
}

#[test]
fn column_items_with_text_take_their_content_height() {
    let Some(l) = flex(
        "<div id=f style='display:flex;flex-direction:column;width:120px'>
           <p id=p style='margin:0'>several words that wrap onto lines</p>
           <div id=n style='height:10px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let p = l.box_id("p");
    assert!(p.lines.len() >= 2);
    let last = p.lines.last().unwrap().rect.bottom();
    close(p.border_box().bottom(), last, 0.05);
    close(l.rect("n").y, p.border_box().bottom(), 0.01);
}

#[test]
fn sticky_footer_with_min_height_column() {
    let Some(l) = flex(
        "<div id=f style='display:flex;flex-direction:column;min-height:400px'>
           <div style='height:30px'></div>
           <div id=main style='flex:1'></div>
           <div id=foot style='height:30px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("f").height, 400.0);
    assert_eq!(l.rect("main").height, 340.0);
    assert_eq!(l.rect("foot").y, 370.0);
}

#[test]
fn order_property_reorders_items() {
    let Some(l) = flex(
        "<div id=f style='display:flex'>
           <div id=a class=i style='width:10px;order:2'></div>
           <div id=b class=i style='width:20px'></div>
           <div id=c class=i style='width:30px;order:-1'></div>
           <div id=d class=i style='width:40px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("c").x, 0.0);
    assert_eq!(l.rect("b").x, 30.0);
    assert_eq!(l.rect("d").x, 50.0);
    assert_eq!(l.rect("a").x, 90.0);
    // Boxes are in order-modified document order.
    let order: Vec<_> = l.box_id("f").children.iter().map(|c| c.node).collect();
    assert_eq!(
        order,
        vec![
            Some(l.id("c")),
            Some(l.id("b")),
            Some(l.id("d")),
            Some(l.id("a"))
        ]
    );
}

#[test]
fn reverse_directions() {
    let Some(l) = flex(
        "<div id=r style='display:flex;flex-direction:row-reverse;width:300px'>
           <div id=r1 class=i style='width:50px'></div><div id=r2 class=i style='width:70px;margin-right:10px'></div>
         </div>
         <div id=c style='display:flex;flex-direction:column-reverse;height:200px'>
           <div id=c1 style='height:50px'></div><div id=c2 style='height:30px'></div>
         </div>
         <div id=w style='display:flex;flex-wrap:wrap-reverse;width:100px'>
           <div id=w1 style='width:60px;height:10px'></div><div id=w2 style='width:60px;height:20px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("r1").x, 250.0);
    // Main-start margin of r2 in row-reverse is its right margin.
    assert_eq!(l.rect("r2").right(), 240.0);
    let c = l.rect("c");
    assert_eq!(l.rect("c1").bottom(), c.bottom());
    assert_eq!(l.rect("c2").bottom(), c.bottom() - 50.0);
    // wrap-reverse: the first line is at the bottom.
    let w = l.rect("w");
    assert_eq!(w.height, 30.0);
    assert_eq!(l.rect("w1").bottom(), w.bottom());
    assert_eq!(l.rect("w2").y, w.y);
}

#[test]
fn align_content_distributes_lines() {
    for (mode, second_line_y) in [
        ("flex-start", 20.0),
        ("flex-end", 80.0),
        ("center", 50.0),
        ("space-between", 80.0),
        ("normal", 50.0),
    ] {
        let Some(l) = flex(
            &format!(
                "<div id=f style='display:flex;flex-wrap:wrap;width:100px;height:100px;align-content:{mode};align-items:flex-start'>
                   <div style='width:60px;height:20px'></div><div id=two style='width:60px;height:20px'></div>
                 </div>"
            ),
            800.0,
        ) else {
            return;
        };
        let f = l.rect("f");
        close(l.rect("two").y - f.y, second_line_y, 0.02);
    }
}

#[test]
fn nested_flex_containers() {
    let Some(l) = flex(
        "<div id=page style='display:flex;width:800px'>
           <aside id=side style='flex:0 0 200px;display:flex;flex-direction:column'>
             <div class=i></div><div class=i></div>
           </aside>
           <main id=main style='flex:1;display:flex;flex-wrap:wrap;gap:10px'>
             <div class=card style='flex:1 1 250px;height:40px'></div>
             <div class=card style='flex:1 1 250px;height:40px'></div>
             <div class=card style='flex:1 1 250px;height:40px'></div>
           </main>
         </div>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("side").width, 200.0);
    let main = l.rect("main");
    assert_eq!((main.x, main.width), (200.0, 600.0));
    let cards = kids(&l, "main");
    // Two cards per line (250 + 10 + 250 <= 600 < 770), growing to fill.
    assert_eq!(cards[0].y, cards[1].y);
    assert!(cards[2].y > cards[0].y);
    close(cards[0].width, 295.0, 0.02);
    close(cards[2].width, 600.0, 0.02);
    // The aside is stretched to the main's height.
    assert_eq!(l.rect("side").height, main.height);
}

#[test]
fn percentage_widths_resolve_against_the_container() {
    let Some(l) = flex(
        "<div id=f style='display:flex;width:500px;padding:0 50px'>
           <div class=i style='width:50%;flex-shrink:0'></div>
           <div class=i style='width:30%;flex-shrink:0;padding-left:10%'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let k = kids(&l, "f");
    assert_eq!(k[0].width, 250.0);
    // 30% of 500 + 10% padding.
    assert_eq!(k[1].width, 200.0);
    assert_eq!(k[1].x, 300.0);
}

#[test]
fn text_runs_become_anonymous_items_and_whitespace_is_dropped() {
    let Some(l) = flex(
        "<div id=f style='display:flex'>Hello <b id=b>bold</b> <span id=s>span</span>
         </div>
         <div id=g style='display:flex'>  <span>a</span>  <span>b</span>  </div>",
        800.0,
    ) else {
        return;
    };
    let f = l.box_id("f");
    assert_eq!(f.children.len(), 3);
    assert!(f.children[0].node.is_none());
    assert_eq!(line_text(&f.children[0].lines[0]).trim(), "Hello");
    // Blockified inline elements are items placed side by side.
    let b = l.rect("b");
    let s = l.rect("s");
    assert!(b.x >= f.children[0].border_box().right() - 0.01);
    assert!(s.x >= b.right() - 0.01);
    assert_eq!(b.y, s.y);
    assert_eq!(l.box_id("g").children.len(), 2);
}

#[test]
fn automatic_minimum_size_prevents_shrinking_below_content() {
    let Some(l) = flex(
        "<div id=a style='display:flex;width:50px'><div id=x>unbreakableword</div></div>
         <div id=b style='display:flex;width:50px'><div id=y style='min-width:0'>unbreakableword</div></div>
         <div id=c style='display:flex;width:50px'><div id=z style='overflow:hidden'>unbreakableword</div></div>",
        800.0,
    ) else {
        return;
    };
    assert!(l.rect("x").width > 60.0);
    assert_eq!(l.rect("y").width, 50.0);
    assert_eq!(l.rect("z").width, 50.0);
}

#[test]
fn absolutely_positioned_children_use_the_content_box_origin() {
    let Some(l) = flex(
        "<div id=f style='display:flex;position:relative;padding:10px;justify-content:flex-end'>
           <div class=i style='width:50px'></div>
           <div id=abs style='position:absolute;width:20px;height:20px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let f = l.box_id("f");
    let abs = l.rect("abs");
    assert_eq!((abs.x, abs.y), (f.content.x, f.content.y));
    // Not a flex item: the other item is still pushed to the end alone.
    assert_eq!(f.children.len(), 2);
}

#[test]
fn intrinsic_sizes_of_flex_containers() {
    let Some(l) = flex(
        "<span id=row style='display:inline-flex;gap:5px'><div style='width:50px;height:5px'></div><div style='width:70px;height:5px'></div></span>
         <div id=col style='display:inline-flex;flex-direction:column'><div style='width:50px;height:5px'></div><div style='width:70px;height:5px'></div></div>
         <div id=fl style='float:left;display:flex'><div style='flex:0 0 40px;height:5px'></div><div style='width:30px;height:5px'></div></div>
         <table><tr><td id=cell><div style='display:flex'><div style='width:60px;height:5px'></div><div style='width:60px;height:5px'></div></div></td></tr></table>",
        800.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("row").width, 125.0);
    assert_eq!(l.rect("col").width, 70.0);
    assert_eq!(l.rect("fl").width, 70.0);
    // 120px of content + 1px padding on each side.
    assert_eq!(l.rect("cell").width, 122.0);
}

#[test]
fn hit_testing_finds_flex_items() {
    let Some(l) = flex(
        "<div id=f style='display:flex;justify-content:space-between;width:300px'>
           <div id=a style='width:100px;height:50px'></div>
           <div id=b style='width:100px;height:50px'></div>
         </div>",
        800.0,
    ) else {
        return;
    };
    let hit = |x, y| {
        l.tree
            .hit_test(Point::new(x, y), Point::ZERO)
            .map(|h| h.node)
    };
    assert_eq!(hit(250.0, 10.0), Some(l.id("b")));
    assert_eq!(hit(50.0, 10.0), Some(l.id("a")));
    assert_eq!(hit(150.0, 10.0), Some(l.id("f")));
}

#[test]
fn images_keep_their_ratio_in_flex_items() {
    let Some(l) = lay_images(
        "<style>body{margin:0}</style>
         <div style='display:flex;align-items:flex-start;width:500px'><img id=a style='width:100px'><img id=b></div>
         <div style='display:flex;flex-direction:column;align-items:flex-start'><img id=c></div>",
        800.0,
        FakeImages::new()
            .id("a", 200.0, 100.0)
            .id("b", 40.0, 30.0)
            .id("c", 60.0, 20.0),
    ) else {
        return;
    };
    let a = l.rect("a");
    assert_eq!((a.width, a.height), (100.0, 50.0));
    let b = l.rect("b");
    assert_eq!((b.x, b.width, b.height), (100.0, 40.0, 30.0));
    let c = l.rect("c");
    assert_eq!((c.width, c.height), (60.0, 20.0));
}

#[test]
fn deeply_nested_flex_does_not_explode() {
    // 20 levels alternating directions, each with a sibling of another
    // height so that stretching relayouts happen at every level.
    let mut html = String::new();
    for i in 0..20 {
        let dir = if i % 2 == 0 { "row" } else { "column" };
        html.push_str(&format!(
            "<div style='display:flex;flex-direction:{dir}'><div style='flex:1'>"
        ));
    }
    html.push_str("leaf");
    for i in 0..20 {
        html.push_str(&format!(
            "</div><div>sibling {i}<br>second line</div></div>"
        ));
    }
    let t = std::time::Instant::now();
    let Some(l) = flex(&html, 800.0) else {
        return;
    };
    assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
    assert!(l.texts().iter().any(|t| t.text == "leaf"));
}

#[test]
fn baseline_alignment_synthesizes_baselines_of_empty_items() {
    let Some(l) = flex(
        "<div id=f style='display:flex;align-items:baseline'>
           <div id=icon style='width:20px;height:30px;margin-bottom:5px'></div>
           <div id=t style='font-size:16px'>text</div>
         </div>",
        800.0,
    ) else {
        return;
    };
    // The icon's border-box bottom sits on the text baseline.
    let icon = l.rect("icon");
    close(icon.bottom(), l.box_id("t").lines[0].baseline, 0.05);
}

#[test]
fn legend_and_caption_room_counts_in_flex_items() {
    let Some(l) = flex(
        "<style>legend{font-size:30px;padding:0}fieldset{margin:0;border:2px solid;padding:0 4px}</style>
         <div id=col style='display:flex;flex-direction:column'>
           <fieldset id=f1><legend>Title</legend><div style='height:10px'></div></fieldset>
           <div id=a1 class=i></div>
           <table id=t1 style='border-spacing:0'><caption style='height:15px'>c</caption><tr><td style='height:10px'></td></tr></table>
           <div id=a2 class=i></div>
         </div>
         <div id=row style='display:flex;align-items:flex-start'>
           <fieldset id=f2><legend>Title</legend><div style='height:10px'></div></fieldset>
         </div>
         <div id=def style='display:flex;height:200px'>
           <fieldset id=f3><legend>Title</legend></fieldset>
           <table id=t3 style='border-spacing:0'><caption style='caption-side:bottom;height:15px'>c</caption><tr><td></td></tr></table>
         </div>",
        800.0,
    ) else {
        return;
    };
    // Column: each item follows the margin box of the previous one.
    let f1 = l.box_id("f1");
    assert_eq!(f1.margin_box().y, 0.0);
    assert_eq!(l.rect("a1").y, f1.margin_box().bottom());
    let t1 = l.box_id("t1");
    assert_eq!(t1.border_box().y, l.rect("a1").bottom() + 15.0);
    assert_eq!(l.rect("a2").y, t1.border_box().bottom());
    // Row: the container is as tall as the fieldset's margin box.
    let f2 = l.box_id("f2");
    assert_eq!(l.rect("row").height, f2.margin_box().height);
    assert!(f2.margin_box().height > f2.border_box().height);
    // Stretched in a definite 200px line: margin boxes fill it exactly.
    assert_eq!(l.box_id("f3").margin_box().height, 200.0);
    let t3 = l.box_id("t3");
    assert_eq!(t3.margin_box().height, 200.0);
    assert_eq!(t3.border_box().height, 185.0);
}
