mod common;
use common::*;

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}</style>";
fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

#[test]
fn p1_br_clear_at_end() {
    let Some(l) = lay(&page("<div id=d><div style='float:left;width:50px;height:100px'></div>text<br clear=all></div><p id=after style='margin:0'>after</p>"), 400.0) else { return };
    eprintln!("P1 d={:?} after={:?}", l.rect("d"), l.rect("after"));
}

#[test]
fn p2_sticky_footer() {
    let Some(l) = lay(&page("<style>html,body{height:100%}</style><div id=w style='display:flex;flex-direction:column;min-height:100%'><main id=m style='flex:1'>x</main><footer id=f>foot</footer></div>"), 400.0) else { return };
    eprintln!("P2 w={:?} m={:?} f={:?}", l.rect("w"), l.rect("m"), l.rect("f"));
}

#[test]
fn p3_nested_flex_percent_heights() {
    let mut s = String::new();
    for _ in 0..22 { s.push_str("<div style='display:flex;height:100%'>"); }
    s.push_str("x");
    for _ in 0..22 { s.push_str("</div>"); }
    let t = std::time::Instant::now();
    let Some(_l) = lay(&page(&s), 400.0) else { return };
    eprintln!("P3 nested flex % heights: {:?}", t.elapsed());
    let mut s = String::new();
    for _ in 0..22 { s.push_str("<div style='display:flex;flex-direction:column'><div style='height:100%;display:flex'>"); }
    s.push_str("x");
    for _ in 0..22 { s.push_str("</div></div>"); }
    let t = std::time::Instant::now();
    let Some(_l) = lay(&page(&s), 400.0) else { return };
    eprintln!("P3b nested column flex % heights: {:?}", t.elapsed());
    let mut s = String::new();
    for _ in 0..22 { s.push_str("<div style='display:grid;height:100%'>"); }
    s.push_str("x");
    for _ in 0..22 { s.push_str("</div>"); }
    let t = std::time::Instant::now();
    let Some(_l) = lay(&page(&s), 400.0) else { return };
    eprintln!("P3c nested grid % heights: {:?}", t.elapsed());
}

#[test]
fn p5_inline_block_only_float() {
    let Some(l) = lay(&page("<div id=p>a<span id=ib style='display:inline-block'><span style='float:left;width:30px;height:40px'></span></span>b</div>"), 400.0) else { return };
    eprintln!("P5 ib={:?} p={:?}\n{}", l.rect("ib"), l.rect("p"), l.outline());
}

#[test]
fn p6_negative_margins() {
    let Some(l) = lay(&page("<div id=a style='height:10px;margin-bottom:20px'></div><div id=b style='height:10px;margin-top:-30px'></div><div id=c style='margin-top:-5px'><div id=c1 style='margin-top:15px;height:5px'></div></div>"), 400.0) else { return };
    eprintln!("P6 a={:?} b={:?} c={:?} c1={:?}", l.rect("a"), l.rect("b"), l.rect("c"), l.rect("c1"));
}

#[test]
fn p10_pre_line() {
    let Some(l) = lay(&page("<div id=d style='white-space:pre-line'>  a   b  \n   c  </div>"), 400.0) else { return };
    eprintln!("P10 {:?}", l.line_texts("d"));
}

#[test]
fn p11_negative_indent() {
    let Some(l) = lay(&page("<div id=d style='text-indent:-20px;padding-left:20px;width:100px'>aaa bbb ccc ddd eee fff</div>"), 400.0) else { return };
    let b = l.box_id("d");
    for line in &b.lines { eprintln!("P11 line {:?} {:?}", line.rect, line.fragments.iter().filter_map(|f| if let xbro_layout::Fragment::Text(t)=f {Some((t.text.clone(), t.rect.x))} else {None}).collect::<Vec<_>>()); }
}

#[test]
fn p13_inline_margin() {
    let Some(l) = lay(&page("<div id=d>a<span id=s style='margin-left:10px;padding-left:5px;border-left:2px solid'>b</span>c</div>"), 400.0) else { return };
    eprintln!("P13 {}", l.dump());
}

#[test]
fn p20_overflow_hidden_contains_float() {
    let Some(l) = lay(&page("<div id=d style='overflow:hidden'><div style='float:left;width:50px;height:80px'></div>x</div>"), 400.0) else { return };
    eprintln!("P20 d={:?}", l.rect("d"));
}

#[test]
fn p22_abspos_percent() {
    let Some(l) = lay(&page("<div style='position:relative;width:200px;height:100px;padding:10px'><div id=a style='position:absolute;width:50%;height:50%;right:0;bottom:0'></div></div>"), 400.0) else { return };
    eprintln!("P22 a={:?}", l.rect("a"));
}

#[test]
fn p25_li_block_child() {
    let Some(l) = lay(&page("<ul style='margin:0'><li id=li><p style='margin:10px 0'>x</p></li></ul>"), 400.0) else { return };
    eprintln!("P25 {}", l.dump());
}

#[test]
fn p26_reversed_hidden() {
    let Some(l) = lay(&page("<ol reversed id=o><li>a</li><li style='display:none'>b</li><li>c</li></ol>"), 400.0) else { return };
    eprintln!("P26 {}", l.dump());
}

#[test]
fn p27_abspos_inline_static() {
    let Some(l) = lay(&page("<p style='margin:0'>hello <span id=a style='position:absolute'>abs</span> world</p>"), 400.0) else { return };
    eprintln!("P27 a={:?}\n{}", l.rect("a"), l.outline());
}

#[test]
fn p28_float_in_inline_line() {
    let Some(l) = lay(&page("<p id=p style='margin:0;width:200px'>one two three four five six seven <span id=f style='float:right;width:50px;height:30px'></span>eight nine ten eleven twelve</p>"), 400.0) else { return };
    eprintln!("P28 f={:?}\n{}", l.rect("f"), l.outline());
}

#[test]
fn p29_table_spans_quadratic() {
    let mut s = String::from("<table>");
    for _ in 0..3000 { s.push_str("<tr><td colspan=1000>x</td></tr>"); }
    s.push_str("</table>");
    let t = std::time::Instant::now();
    let Some(_l) = lay(&page(&s), 400.0) else { return };
    eprintln!("P29 {:?}", t.elapsed());
}
