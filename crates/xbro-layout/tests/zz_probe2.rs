mod common;
use common::*;
use xbro_layout::Fragment;

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}</style>";
fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

#[test]
fn q1_clearfix() {
    let Some(l) = lay(&page("<style>.cf::after{content:'';display:table;clear:both}.cf2::after{content:'';display:block;clear:both}</style><div id=a class=cf><div style='float:left;width:50px;height:100px'></div></div><div id=b class=cf2><div style='float:right;width:50px;height:70px'></div></div><p id=p style='margin:0'>x</p>"), 400.0) else { return };
    eprintln!("Q1 a={:?} b={:?} p={:?}", l.rect("a"), l.rect("b"), l.rect("p"));
}

#[test]
fn q2_text_indent_anon() {
    let Some(l) = lay(&page("<div id=d style='text-indent:50px'>first<div>block</div>after</div>"), 400.0) else { return };
    eprintln!("Q2 {}", l.outline());
    let b = l.box_id("d");
    for c in &b.children { for line in &c.lines { for f in &line.fragments { if let Fragment::Text(t) = f { eprintln!("Q2 {} x={}", t.text, t.rect.x); } } } }
}

#[test]
fn q3_inline_block_margins_baseline() {
    let Some(l) = lay(&page("<div id=d>x<span id=s style='display:inline-block;margin:10px 0 5px;padding:3px'>y</span>z</div>"), 400.0) else { return };
    eprintln!("Q3 s={:?} d={:?}\n{}", l.rect("s"), l.rect("d"), l.dump());
}

#[test]
fn q4_vertical_align_bottom_tall() {
    let Some(l) = lay(&page("<div id=d>x<span id=s style='display:inline-block;height:60px;width:10px;vertical-align:bottom'></span><span id=t style='display:inline-block;height:30px;width:10px;vertical-align:top'></span></div>"), 400.0) else { return };
    eprintln!("Q4 s={:?} t={:?} d={:?}", l.rect("s"), l.rect("t"), l.rect("d"));
}

#[test]
fn q5_float_clear_inside_bfc() {
    let Some(l) = lay(&page("<div style='float:left;width:100px;height:200px'></div><div id=o style='overflow:hidden'><div style='float:left;width:30px;height:20px'></div><div id=c style='clear:left'>x</div></div>"), 400.0) else { return };
    eprintln!("Q5 o={:?} c={:?}", l.rect("o"), l.rect("c"));
}

#[test]
fn q6_shrink_float_with_block_children() {
    let Some(l) = lay(&page("<div id=f style='float:left'><p style='margin:0 10px'>hello world</p><div style='width:50%'>a</div></div>"), 400.0) else { return };
    eprintln!("Q6 f={:?}\n{}", l.rect("f"), l.outline());
}

#[test]
fn q7_min_height_collapse() {
    let Some(l) = lay(&page("<div id=a style='margin-bottom:20px'></div><div id=b style='min-height:10px;margin:10px 0'></div><div id=c style='margin-top:5px'>x</div>"), 400.0) else { return };
    eprintln!("Q7 a={:?} b={:?} c={:?}", l.rect("a"), l.rect("b"), l.rect("c"));
}

#[test]
fn q8_abspos_bottom_auto_height_parent() {
    let Some(l) = lay(&page("<div id=r style='position:relative;padding:5px'>a<br>b<br>c<div id=a style='position:absolute;bottom:0;left:0;width:10px;height:10px'></div></div>"), 400.0) else { return };
    eprintln!("Q8 r={:?} a={:?}", l.rect("r"), l.rect("a"));
}

#[test]
fn q9_flex_column_wrap() {
    let Some(l) = lay(&page("<div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:100px;width:300px'><div id=i1 style='height:60px'>a</div><div id=i2 style='height:60px'>b</div><div id=i3 style='height:30px'>c</div></div>"), 400.0) else { return };
    eprintln!("Q9 c={:?} i1={:?} i2={:?} i3={:?}", l.rect("c"), l.rect("i1"), l.rect("i2"), l.rect("i3"));
}

#[test]
fn q10_flex_item_margin_auto_both() {
    let Some(l) = lay(&page("<div id=c style='display:flex;height:200px'><div id=i style='margin:auto;width:50px;height:50px'></div></div>"), 400.0) else { return };
    eprintln!("Q10 i={:?}", l.rect("i"));
}

#[test]
fn q11_inline_flex_in_text() {
    let Some(l) = lay(&page("<div id=d>a <span id=f style='display:inline-flex;gap:5px'><span>b</span><span>c</span></span> d</div>"), 400.0) else { return };
    eprintln!("Q11 f={:?}\n{}", l.rect("f"), l.outline());
}

#[test]
fn q12_table_cell_percent_width() {
    let Some(l) = lay(&page("<table id=t width=100% cellspacing=0 cellpadding=0><tr><td id=a width=1% nowrap>abc def</td><td id=b>x</td></tr></table>"), 400.0) else { return };
    eprintln!("Q12 t={:?} a={:?} b={:?}", l.rect("t"), l.rect("a"), l.rect("b"));
}

#[test]
fn q13_empty_table_and_cells() {
    let Some(l) = lay(&page("<table id=t><tr><td></td><td></td></tr></table><table id=u></table><div id=v style='display:table'></div>"), 400.0) else { return };
    eprintln!("Q13 t={:?} u={:?} v={:?}", l.rect("t"), l.rect("u"), l.rect("v"));
}

#[test]
fn q14_list_inside_marker_alignment() {
    let Some(l) = lay(&page("<ol style='list-style-position:inside;margin:0;padding:0'><li id=a>alpha</li></ol>"), 400.0) else { return };
    eprintln!("Q14 {:?}", l.line_texts("a"));
}

#[test]
fn q15_img_percent_in_auto_table() {
    let imgs = FakeImages::new().id("i", 400.0, 200.0);
    let Some(l) = lay_images(&page("<table id=t><tr><td><img id=i style='max-width:100%'></td></tr></table>"), 300.0, imgs) else { return };
    eprintln!("Q15 t={:?} i={:?}", l.rect("t"), l.rect("i"));
}

#[test]
fn q16_justify_last_line_inline() {
    let Some(l) = lay(&page("<p id=p style='text-align:justify;width:100px;margin:0'>aa bb cc dd ee ff gg hh</p>"), 400.0) else { return };
    let b = l.box_id("p");
    for line in &b.lines { let mut w = 0.0; for f in &line.fragments { if let Fragment::Text(t) = f { w += t.rect.width; eprintln!("Q16 {:?} x={} w={}", t.text, t.rect.x, t.rect.width); } } eprintln!("Q16 line total {}", w); }
}

#[test]
fn q17_rel_percent_top() {
    let Some(l) = lay(&page("<div style='height:200px'><div id=a style='position:relative;top:10%;left:10%'>x</div></div>"), 400.0) else { return };
    eprintln!("Q17 a={:?}", l.rect("a"));
}

#[test]
fn q18_sup_line_height() {
    let Some(l) = lay(&page("<p id=p style='margin:0'>a<sup>2</sup> b</p><p id=q style='margin:0'>a b</p>"), 400.0) else { return };
    eprintln!("Q18 p={:?} q={:?}", l.rect("p"), l.rect("q"));
}
