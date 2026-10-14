mod common;
use common::*;

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}</style>";
fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

#[test]
fn s1_abspos_max_width_centering() {
    let Some(l) = lay(&page("<div id=m style='position:fixed;left:0;right:0;top:0;bottom:0;margin:auto;max-width:200px;max-height:100px'>modal</div><div id=n style='position:absolute;left:0;right:0;margin:0 auto;max-width:100px'>x</div><div id=o style='position:absolute;left:10px;right:10px;min-width:500px'>x</div>"), 400.0) else { return };
    eprintln!("S1 m={:?} n={:?} o={:?}", l.rect("m"), l.rect("n"), l.rect("o"));
}

#[test]
fn s2_float_percentage_width_rounding() {
    let Some(l) = lay(&page("<div style='width:1001px'><div id=a style='float:left;width:33.3333%'>a</div><div id=b style='float:left;width:33.3333%'>b</div><div id=c style='float:left;width:33.3333%'>c</div></div>"), 1100.0) else { return };
    eprintln!("S2 a={:?} b={:?} c={:?}", l.rect("a"), l.rect("b"), l.rect("c"));
}

#[test]
fn s3_inline_block_whitespace_columns() {
    let Some(l) = lay(&page("<div style='width:300px;font-size:0'><div id=a style='display:inline-block;width:50%;font-size:16px'>a</div><div id=b style='display:inline-block;width:50%;font-size:16px'>b</div></div>"), 400.0) else { return };
    eprintln!("S3 a={:?} b={:?}", l.rect("a"), l.rect("b"));
}

#[test]
fn s4_nested_list_marker_positions() {
    let Some(l) = lay(&page("<ol style='margin:0'><li id=a>x<ol><li id=b>y</li></ol></li></ol>"), 400.0) else { return };
    eprintln!("S4 {}", l.dump());
}

#[test]
fn s5_rel_inline_hit() {
    let Some(l) = lay(&page("<p style='margin:0'><a id=a href=#>link</a></p>"), 400.0) else { return };
    let h = l.tree.hit_test(xbro_layout::Point::new(5.0, 10.0), xbro_layout::Point::ZERO);
    eprintln!("S5 hit={:?}", h);
}

#[test]
fn s6_margin_collapse_with_abspos_between() {
    let Some(l) = lay(&page("<div id=a style='margin-bottom:20px;height:10px'></div><div style='position:absolute'></div><div id=b style='margin-top:30px;height:10px'></div>"), 400.0) else { return };
    eprintln!("S6 a={:?} b={:?}", l.rect("a"), l.rect("b"));
}

#[test]
fn s7_float_after_text_line() {
    let Some(l) = lay(&page("<div style='width:300px'>first line text<div id=f style='float:left;width:100px;height:50px'></div><p id=p style='margin:0'>para</p></div>"), 400.0) else { return };
    eprintln!("S7 f={:?} p={:?}\n{}", l.rect("f"), l.rect("p"), l.outline());
}

#[test]
fn s8_bfc_root_too_wide_next_to_float() {
    let Some(l) = lay(&page("<div style='width:300px'><div style='float:left;width:200px;height:50px'></div><table id=t><tr><td>aaaaaaaaaaaaaaaaaaaaa</td></tr></table><div id=o style='overflow:hidden;width:150px;height:10px'></div></div>"), 400.0) else { return };
    eprintln!("S8 t={:?} o={:?}", l.rect("t"), l.rect("o"));
}

#[test]
fn s9_aspect_ratio_div() {
    let Some(l) = lay(&page("<div id=d style='width:200px;aspect-ratio:2/1;background:red'></div>"), 400.0) else { return };
    eprintln!("S9 d={:?}", l.rect("d"));
}

#[test]
fn s10_auto_height_root_percent() {
    let Some(l) = lay(&page("<div id=d style='height:50%'>x</div>"), 400.0) else { return };
    eprintln!("S10 d={:?}", l.rect("d"));
}

#[test]
fn s11_aspect_ratio_variants() {
    let Some(l) = lay(&page("<div id=a style='aspect-ratio:16/9'></div><div id=b style='height:50px;aspect-ratio:2'></div><div id=c style='width:100px;aspect-ratio:1;box-sizing:border-box;padding:10px'></div><div id=d style='width:100px;aspect-ratio:4'>line one line two line three</div><div id=e style='width:100px;aspect-ratio:4;overflow:hidden'>line one line two line three</div><span id=f style='display:inline-block;height:30px;aspect-ratio:1/2'></span><div id=g style='float:left;height:40px;aspect-ratio:2'></div><div id=h style='position:absolute;top:0;height:10px;aspect-ratio:3'></div><div style='display:flex;width:300px'><div id=i style='flex:1;aspect-ratio:3'></div></div>"), 400.0) else { return };
    for id in ["a","b","c","d","e","f","g","h","i"] { eprintln!("S11 {id}={:?}", l.rect(id)); }
}

#[test]
fn s12_inline_cb() {
    let Some(l) = lay(&page("<div style='position:relative;width:300px;height:100px'>text <a id=a style='position:relative;padding:0 4px'>Cart<span id=b style='position:absolute;top:0;right:0;width:6px;height:6px'></span></a></div>"), 400.0) else { return };
    eprintln!("S12 b={:?} a_frag={:?}", l.rect("b"), l.tree.node_rect(l.id("a")));
}
