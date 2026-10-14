mod common;
use common::*;
use xbro_layout::Point;

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}</style>";
fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

#[test]
fn r1_relative_cell_abspos() {
    let Some(l) = lay(&page("<table cellspacing=0 cellpadding=0><tr><td id=c style='position:relative;vertical-align:middle;width:100px'>x<span id=b style='position:absolute;bottom:0;right:0;width:10px;height:10px'></span><span id=t style='position:absolute;top:0;left:0;width:10px;height:10px'></span></td><td style='height:100px'>y</td></tr></table>"), 400.0) else { return };
    eprintln!("R1 c={:?} b={:?} t={:?}", l.rect("c"), l.rect("b"), l.rect("t"));
}

#[test]
fn r2_hit_float_over_block() {
    let Some(l) = lay(&page("<div id=f style='float:left;width:100px;height:100px;background:red'></div><div id=b style='height:50px;background:blue'></div>"), 400.0) else { return };
    let h = l.tree.hit_test(Point::new(10.0, 10.0), Point::ZERO);
    eprintln!("R2 hit={:?} f={:?} b={:?}", h, l.id("f"), l.id("b"));
}

#[test]
fn r3_rowspan_across_groups() {
    let Some(l) = lay(&page("<table id=t><tbody><tr><td rowspan=5 id=a>a</td><td>b</td></tr></tbody><tbody><tr><td id=c>c</td><td>d</td></tr></tbody></table>"), 400.0) else { return };
    eprintln!("R3 a={:?} c={:?}", l.rect("a"), l.rect("c"));
}

#[test]
fn r4_col_width() {
    let Some(l) = lay(&page("<table id=t cellspacing=0 cellpadding=0><colgroup><col width=100><col style='width:50px'></colgroup><tr><td id=a>a</td><td id=b>b</td><td id=c>c</td></tr></table>"), 400.0) else { return };
    eprintln!("R4 a={:?} b={:?} c={:?}", l.rect("a"), l.rect("b"), l.rect("c"));
}

#[test]
fn r5_fixed_doc_size_and_hit() {
    let Some(l) = lay(&page("<div id=f style='position:fixed;top:0;left:0;width:100px;height:3000px'></div><div style='height:10px'></div>"), 400.0) else { return };
    eprintln!("R5 ds={:?} hit={:?}", l.tree.document_size, l.tree.hit_test(Point::new(10.0, 1000.0), Point::new(0.0, 900.0)));
}

#[test]
fn r6_abspos_in_inline_block_rel() {
    let Some(l) = lay(&page("<p style='margin:0'>aaaa <span id=r style='position:relative;display:inline-block;width:50px;height:20px'><span id=a style='position:absolute;top:0;left:0;width:5px;height:5px'></span></span></p>"), 400.0) else { return };
    eprintln!("R6 r={:?} a={:?}", l.rect("r"), l.rect("a"));
}

#[test]
fn r7_overflow_hidden_doc_size() {
    let Some(l) = lay(&page("<div style='overflow:hidden;height:100px'><div style='height:5000px'></div></div>"), 400.0) else { return };
    eprintln!("R7 ds={:?}", l.tree.document_size);
}

#[test]
fn r8_float_right_margin_negative() {
    let Some(l) = lay(&page("<div style='width:300px'><div id=f style='float:right;width:100px;height:50px;margin-right:-20px'></div><p id=p style='margin:0'>text</p></div>"), 400.0) else { return };
    eprintln!("R8 f={:?}", l.rect("f"));
}

#[test]
fn r9_inline_block_rel_offset_and_line() {
    let Some(l) = lay(&page("<div>a<span id=s style='display:inline-block;position:relative;top:10px'>b</span></div>"), 400.0) else { return };
    eprintln!("R9 s={:?}", l.rect("s"));
}

#[test]
fn r10_percent_height_in_table_cell() {
    let Some(l) = lay(&page("<table style='height:200px'><tr><td><div id=d style='height:50%'>x</div></td></tr></table>"), 400.0) else { return };
    eprintln!("R10 d={:?}", l.rect("d"));
}

#[test]
fn r11_flex_row_reverse_and_order() {
    let Some(l) = lay(&page("<div style='display:flex;flex-direction:row-reverse;width:300px'><div id=a style='width:50px'>a</div><div id=b style='width:50px;order:-1'>b</div></div>"), 400.0) else { return };
    eprintln!("R11 a={:?} b={:?}", l.rect("a"), l.rect("b"));
}

#[test]
fn r12_flex_img_auto_min() {
    let imgs = FakeImages::new().id("i", 400.0, 200.0);
    let Some(l) = lay_images(&page("<div style='display:flex;width:300px'><img id=i><div id=t style='flex:1'>text</div></div>"), 400.0, imgs) else { return };
    eprintln!("R12 i={:?} t={:?}", l.rect("i"), l.rect("t"));
}

#[test]
fn r13_inline_svg_viewbox_in_shrink() {
    let Some(l) = lay(&page("<div id=f style='float:left'><svg id=s viewBox='0 0 24 24'></svg></div>"), 400.0) else { return };
    eprintln!("R13 f={:?} s={:?}", l.rect("f"), l.rect("s"));
}

#[test]
fn r14_textarea_baseline() {
    let Some(l) = lay(&page("<div id=d>label <textarea id=t rows=3></textarea></div>"), 400.0) else { return };
    eprintln!("R14 t={:?} d={:?}\n{}", l.rect("t"), l.rect("d"), l.dump());
}
