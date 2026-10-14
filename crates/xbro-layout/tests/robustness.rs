//! Odd, empty, huge and hostile documents never panic and give sane geometry.

mod common;

use common::*;
use xbro_layout::{Fragment, LayoutBox, Point, Size};

/// Checks that every rect of the tree is finite with non-negative sizes.
fn check_sane(b: &LayoutBox, depth: usize) {
    assert!(depth < 4096);
    for r in [b.content, b.border_box(), b.margin_box()] {
        assert!(r.x.is_finite() && r.y.is_finite(), "{r:?}");
        assert!(
            r.width >= 0.0 && r.height >= 0.0 && r.width.is_finite() && r.height.is_finite(),
            "{r:?}"
        );
    }
    for l in &b.lines {
        assert!(l.rect.width >= 0.0 && l.rect.height >= 0.0 && l.baseline.is_finite());
        for f in &l.fragments {
            match f {
                Fragment::Text(t) => {
                    assert!(t.rect.x.is_finite() && t.rect.width >= 0.0 && t.baseline.is_finite());
                    assert!(t
                        .glyphs
                        .iter()
                        .all(|g| g.x.is_finite() && g.advance.is_finite()));
                }
                Fragment::InlineBox(ib) => assert!(ib.rect.width >= 0.0 && ib.rect.height >= 0.0),
                Fragment::Atomic(a) => check_sane(a, depth + 1),
            }
        }
    }
    if let Some(m) = &b.marker {
        assert!(m.rect.x.is_finite() && m.rect.width >= 0.0);
    }
    for c in &b.children {
        check_sane(c, depth + 1);
    }
}

fn sane_layout(l: &Laid) {
    if let Some(root) = &l.tree.root {
        check_sane(root, 0);
    }
    let ds = l.tree.document_size;
    assert!(ds.width.is_finite() && ds.height.is_finite());
    assert!(ds.width >= l.tree.viewport.width && ds.height >= l.tree.viewport.height);
    // Queries never panic either.
    let _ = l.tree.hit_test(Point::new(10.0, 10.0), Point::ZERO);
    let _ = l
        .tree
        .hit_test(Point::new(f32::NAN, 1.0), Point::new(f32::INFINITY, 0.0));
    let _ = l.tree.text_fragments();
    let _ = l.tree.node_rect(l.doc.root());
    let _ = l.dump();
    let _ = l.outline();
}

/// Runs `f` on a thread with a 2 MiB stack (the default for spawned
/// threads), as the engine's loader threads would.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("layout panicked or overflowed its stack");
}

fn lay_any(html: &str, width: f32) -> Laid {
    let fonts = fonts().unwrap_or_else(no_fonts);
    lay_full(html, Size::new(width, 600.0), fonts, None)
}

#[test]
fn empty_and_odd_documents() {
    for html in [
        "",
        " ",
        "just text",
        "<html></html>",
        "<html style='display:none'><body>x</body></html>",
        "<body style='display:none'>x</body>",
        "<html style='display:contents'><body>x</body></html>",
        "<html style='display:inline'>inline root</html>",
        "<html style='display:flex'><body>x</body></html>",
        "<html style='display:table'><body>x</body></html>",
        "<html style='position:absolute;left:10px'><body>x</body></html>",
        "<html style='float:left'>x</html>",
        "<frameset><frame></frameset>",
        "<table><td>cell<tr><td>x</table>",
        "<tr><td>orphan cell</td></tr>",
        "<body><td>orphan</td> text <th>b</th></body>",
        "<ul><li><li><li></ul><ol reversed start=-5><li>a<li>b</ol>",
        "<br><br><wbr><br>",
        "<p>\u{AD}\u{200B}\u{AD}</p>",
        "<pre>\t\t\t\n\n\n</pre>",
        "<svg><foreignObject><div>x</div></foreignObject></svg>",
        "<select><option></select><textarea></textarea><input type=range min=5 max=1>",
        "<img alt=''><img alt=' '><img src=x width=-5 height=abc>",
        "<div style='display:contents'><li>contents item</li></div>",
        "<span style='display:block;float:right;position:fixed'>x</span>",
        "<q></q><q><q>nested</q></q>",
    ] {
        let l = lay_any(html, 300.0);
        sane_layout(&l);
    }
    let l = lay_any("", 300.0);
    // The parser always creates html/head/body.
    assert!(l.tree.root.is_some());
    let none = lay_any("<html style='display:none'><body>x</body></html>", 300.0);
    assert!(none.tree.root.is_none());
    assert_eq!(none.tree.document_size, Size::new(300.0, 600.0));
    assert!(none
        .tree
        .hit_test(Point::new(1.0, 1.0), Point::ZERO)
        .is_none());
}

#[test]
fn zero_and_hostile_viewports() {
    let fonts = fonts().unwrap_or_else(no_fonts);
    for vp in [
        Size::new(0.0, 0.0),
        Size::new(-100.0, -5.0),
        Size::new(f32::NAN, f32::INFINITY),
        Size::new(1.0e30, 1.0e30),
        Size::new(1.0, 1.0),
    ] {
        let l = lay_full(
            "<p>Some text <b>here</b><img width=50 height=50></p>",
            vp,
            fonts,
            None,
        );
        sane_layout(&l);
    }
}

#[test]
fn hostile_css_values() {
    let html = "<style>
        div { width: 1e30px; height: -5px; margin: 1e30px -1e30px; padding: 1e20px; }
        p { font-size: 1e9px; line-height: 1e9; letter-spacing: -1e9px; word-spacing: 1e9px; text-indent: -1e30px; }
        span { vertical-align: 1e30px; display: inline-block; width: calc(100% + 1e30px); }
        b { position: relative; left: 1e30px; top: -1e30px; }
        i { position: absolute; inset: 1e30px; }
        img { width: 1e10px; aspect-ratio: 1e-30; }
        ul { list-style-type: lower-roman; }
        em { font-size: 0; line-height: 0; }
      </style>
      <div><p>text <span>ib</span> <b>rel</b> <i>abs</i> <em>zero</em></p></div>
      <img><ul><li value=-99999999999999999999>x</li><li value=99999999999999999999>y</li></ul>
      <p style='width:0;padding:0'>narrow narrow</p>
      <p style='font-size:0.0001px'>tiny</p>";
    let l = lay_any(html, 500.0);
    sane_layout(&l);
}

#[test]
fn deeply_nested_blocks_on_a_small_stack() {
    on_small_stack(|| {
        for wrap in [
            ("<div style='padding:1px'>", "</div>"),
            ("<span>", "</span>"),
            ("<div style='display:inline-block'>", "</div>"),
            ("<div style='float:left'>", "</div>"),
            ("<table><tr><td>", "</td></tr></table>"),
            ("<ul><li>", "</li></ul>"),
            (
                "<div style='position:relative'><div style='position:absolute'>",
                "</div></div>",
            ),
            ("<div style='display:flex'>", "</div>"),
            ("<a href=x><div>", "</div></a>"),
        ] {
            let mut html = String::new();
            for _ in 0..300 {
                html.push_str(wrap.0);
            }
            html.push_str("deep text");
            for _ in 0..300 {
                html.push_str(wrap.1);
            }
            let l = lay_any(&html, 800.0);
            sane_layout(&l);
            assert!(
                l.texts().iter().any(|t| t.text.contains("deep")),
                "text lost for {:?}",
                wrap.0
            );
        }
    });
}

#[test]
fn ten_thousand_siblings() {
    let mut html = String::from("<body>");
    for i in 0..10_000 {
        html.push_str(&format!("<div>item {i}</div>"));
    }
    html.push_str("<p>");
    for i in 0..10_000 {
        html.push_str(&format!("<span>w{i}</span> "));
    }
    html.push_str("</p></body>");
    let l = lay_any(&html, 800.0);
    sane_layout(&l);
    let body = l.box_of(l.doc.body().unwrap()).unwrap();
    assert_eq!(body.children.len(), 10_001);
    assert!(l.tree.document_size.height > 10_000.0 * 10.0);
}

#[test]
fn huge_single_word_with_break_anywhere_is_linear() {
    let word = "x".repeat(200_000);
    let html = format!(
        "<p style='width:100px;overflow-wrap:anywhere'>{word}</p><p style='width:100px'>{word}</p>"
    );
    let t = std::time::Instant::now();
    let l = lay_any(&html, 400.0);
    sane_layout(&l);
    // Generous bound (debug builds): quadratic behaviour would take minutes.
    assert!(t.elapsed().as_secs() < 30, "{:?}", t.elapsed());
    // No content is lost: every character is on some line.
    let p = l.tag("p", 0);
    let chars: usize = l
        .box_of(p)
        .unwrap()
        .lines
        .iter()
        .map(|line| line_text(line).len())
        .sum();
    assert_eq!(chars, 200_000);
    assert!(l.box_of(p).unwrap().lines.len() > 1000);
    // Without break-anywhere: a single overflowing line.
    assert_eq!(l.box_of(l.tag("p", 1)).unwrap().lines.len(), 1);
}

#[test]
fn layout_without_fonts() {
    let l = lay_full(
        "<h1>Title</h1><p>Some <b>text</b> with <input value=x> and <ol><li>item</li></ol></p>",
        Size::new(400.0, 300.0),
        no_fonts(),
        None,
    );
    sane_layout(&l);
    assert!(!l.texts().is_empty());
}

#[test]
fn nested_shrink_to_fit_is_not_exponential() {
    // 60 levels of floats inside inline-blocks inside table cells: each
    // level measures its content; caching keeps this linear.
    let mut html = String::new();
    for i in 0..60 {
        match i % 3 {
            0 => html.push_str("<div style='float:left'>"),
            1 => html.push_str("<span style='display:inline-block'>"),
            _ => html.push_str("<table><tr><td>"),
        }
        html.push_str("word ");
    }
    for i in (0..60).rev() {
        match i % 3 {
            0 => html.push_str("</div>"),
            1 => html.push_str("</span>"),
            _ => html.push_str("</td></tr></table>"),
        }
    }
    let t = std::time::Instant::now();
    let l = lay_any(&html, 800.0);
    sane_layout(&l);
    assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
}

#[test]
fn many_positioned_and_floating_boxes() {
    let mut html = String::from("<div style='position:relative'>");
    for i in 0..2000 {
        html.push_str(&format!(
            "<div style='position:absolute;left:{}px;top:{}px'>a{i}</div><div style='float:{}'>f</div><span style='position:fixed;bottom:{}px'>x</span>",
            i % 300,
            i,
            if i % 2 == 0 { "left" } else { "right" },
            i % 50
        ));
    }
    html.push_str("</div>");
    let l = lay_any(&html, 800.0);
    sane_layout(&l);
}

#[test]
fn hostile_flex_grid_and_table_values() {
    let html = "<style>
        .f { display:flex; flex-wrap:wrap; gap: 1e30px 1e30%; }
        .g { display:grid; grid-template-columns: repeat(1000000, 1fr); gap: -5px; }
        .h { display:inline-grid; grid-template-columns: repeat(auto-fill, minmax(0, 1fr)); }
        .t { border-spacing: 1e30px; width: 1e30px; height: -1e30px; }
      </style>
      <div class=f style='flex-direction:column-reverse;height:1e30px;min-height:-5px;max-height:0'>
        <div style='flex:1e30 1e30 1e30px;min-width:1e30px;max-width:-1px'>a</div>
        <div style='flex:-1 -1 -100%;margin:auto;order:-2147483648'>b</div>
        <div style='flex-basis:calc(50% + 1e30px);align-self:baseline;margin-top:auto'>c</div>
        <div style='order:2147483647;width:0;height:0;padding:1e30px'></div>
      </div>
      <div style='display:flex;flex-direction:row-reverse;flex-wrap:wrap-reverse;align-content:space-evenly;justify-content:space-around;width:0;height:0'>
        <div style='width:100px;align-self:stretch;margin:-1e30px'>x</div><span>y</span>text
      </div>
      <div class=g><div>1</div><div>2</div><div style='margin:auto;width:150%'>3</div></div>
      <div class=h><div></div><div></div><span></span></div>
      <div style='display:grid;gap:10%;height:auto'></div>
      <table class=t style='table-layout:fixed;border-collapse:collapse'>
        <colgroup span=0><col span=99999 style='width:1e30px'><col span=-3 width=50%></colgroup>
        <caption style='margin:-1e30px'>cap</caption>
        <tr style='height:1e30px'><td colspan=-1 rowspan=-7 style='width:200%;padding:1e30px'>a</td><td rowspan=0>b</td></tr>
        <tr><td style='vertical-align:baseline;height:-5px'>c</td></tr>
      </table>
      <table style='border-collapse:collapse;border:1e30px solid'><tr><td style='border:1e30px solid;width:calc(10% + 5px)'>d</td></tr></table>
      <table><tr><td style='width:120%'>p1</td><td style='width:90%'>p2</td><td style='width:0%'>p3</td></tr></table>
      <div style='display:table-row-group'><div style='display:table-column'></div>loose text</div>
      <span style='display:table-caption'>orphan caption</span>
      <div style='display:table;height:100px'></div>";
    for width in [0.0, 1.0, 320.0, 1024.0] {
        let l = lay_any(html, width);
        sane_layout(&l);
    }
}

#[test]
fn flex_and_tables_with_many_children() {
    let mut html = String::from("<div style='display:flex;flex-wrap:wrap'>");
    for i in 0..5000 {
        html.push_str(&format!(
            "<div style='flex:{} 1 {}px'>i{i}</div>",
            i % 3,
            i % 97
        ));
    }
    html.push_str("</div><div style='display:flex'>");
    for i in 0..3000 {
        html.push_str(&format!(
            "<div style='flex:1;min-width:{}px;max-width:{}px'>n</div>",
            i % 7,
            10 + i % 13
        ));
    }
    html.push_str("</div><table><tr>");
    for i in 0..1200 {
        html.push_str(&format!("<td colspan={}>c</td>", 1 + i % 3));
    }
    html.push_str("</tr></table>");
    let t = std::time::Instant::now();
    let l = lay_any(&html, 800.0);
    sane_layout(&l);
    assert!(t.elapsed().as_secs() < 30, "{:?}", t.elapsed());
}

#[test]
fn deeply_nested_grid_column_flex_and_css_tables_on_a_small_stack() {
    on_small_stack(|| {
        for wrap in [
            (
                "<div style='display:grid;grid-template-columns:1fr 1fr'><div>",
                "</div><div>s</div></div>",
            ),
            (
                "<div style='display:flex;flex-direction:column-reverse;flex-wrap:wrap'><div style='flex:1'>",
                "</div><b>s</b></div>",
            ),
            (
                "<div style='display:table'><div style='display:table-cell'>",
                "</div></div>",
            ),
            ("<span style='display:inline-flex'>", "</span>"),
            ("<table><caption>c</caption><tr><td rowspan=2>", "</td></tr><tr><td>x</td></tr></table>"),
        ] {
            let mut html = String::new();
            for _ in 0..300 {
                html.push_str(wrap.0);
            }
            html.push_str("deep text");
            for _ in 0..300 {
                html.push_str(wrap.1);
            }
            let l = lay_any(&html, 800.0);
            sane_layout(&l);
            assert!(
                l.texts().iter().any(|t| t.text.contains("deep")),
                "text lost for {:?}",
                wrap.0
            );
        }
    });
}

/// A tiny deterministic generator (xorshift) of random documents mixing
/// every box kind with hostile property values.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        let i = (self.next() % xs.len() as u64) as usize;
        xs.get(i).copied().unwrap_or("")
    }
}

const FUZZ_TAGS: &[&str] = &[
    "div", "span", "p", "a", "ul", "li", "ol", "table", "tr", "td", "th", "caption", "img",
    "input", "button", "select", "textarea", "br", "wbr", "svg", "pre", "center", "colgroup",
];
const FUZZ_PROPS: &[&str] = &[
    "display:block",
    "display:inline",
    "display:inline-block",
    "display:flex",
    "display:inline-flex",
    "display:grid",
    "display:table",
    "display:table-cell",
    "display:table-row",
    "display:contents",
    "display:list-item",
    "display:none",
    "float:left",
    "float:right",
    "clear:both",
    "position:absolute",
    "position:relative",
    "position:fixed",
    "position:sticky",
    "top:10px",
    "left:-20px",
    "right:0",
    "bottom:0",
    "inset:0",
    "width:100px",
    "width:50%",
    "width:0",
    "width:1e30px",
    "min-width:300px",
    "max-width:10px",
    "height:50px",
    "height:100%",
    "min-height:100px",
    "max-height:5px",
    "margin:10px",
    "margin:-30px",
    "margin:auto",
    "margin-top:-1e9px",
    "padding:10%",
    "border:3px solid red",
    "box-sizing:border-box",
    "overflow:hidden",
    "overflow-x:hidden",
    "white-space:pre",
    "white-space:nowrap",
    "white-space:pre-wrap",
    "white-space:pre-line",
    "text-align:justify",
    "text-align:center",
    "text-indent:-50px",
    "letter-spacing:-20px",
    "word-spacing:1e9px",
    "line-height:0",
    "line-height:1e30px",
    "font-size:0",
    "font-size:200px",
    "vertical-align:top",
    "vertical-align:bottom",
    "vertical-align:middle",
    "vertical-align:-1e9px",
    "flex:1",
    "flex:0 0 100px",
    "flex-grow:1e30",
    "flex-basis:50%",
    "flex-direction:column",
    "flex-direction:row-reverse",
    "flex-wrap:wrap",
    "flex-wrap:wrap-reverse",
    "justify-content:space-between",
    "align-items:baseline",
    "align-content:space-around",
    "gap:1e9px",
    "order:-1",
    "grid-template-columns:repeat(3,1fr)",
    "aspect-ratio:16/9",
    "list-style-position:inside",
    "border-collapse:collapse",
    "border-spacing:1e9px",
    "visibility:hidden",
    "opacity:0.5",
    "z-index:5",
    "z-index:-1",
    "overflow-wrap:anywhere",
];
const FUZZ_TEXTS: &[&str] = &[
    "hello world",
    " ",
    "\n  \t",
    "a-b-c long-hyphenated-word",
    "supercalifragilisticexpialidocious",
    "日本語のテキスト。",
    "x\u{AD}y\u{200B}z",
    "\t\ttabs\tand  spaces  ",
    "😀 emoji",
    "(",
    ").",
];
const FUZZ_ATTRS: &[&str] = &[
    "",
    " colspan=3",
    " rowspan=0",
    " alt='alt text'",
    " type=checkbox checked",
    " type=password value=secret",
    " size=1000000",
    " reversed start=3",
    " span=1000",
    " placeholder=hi",
    " width=50 height=20",
];

fn random_children(r: &mut Rng, depth: usize, out: &mut String) {
    let n = r.next() % 6;
    for _ in 0..n {
        if depth > 8 || r.next() % 4 == 0 {
            out.push_str(r.pick(FUZZ_TEXTS));
            continue;
        }
        let tag = r.pick(FUZZ_TAGS);
        let mut style = String::new();
        for _ in 0..r.next() % 5 {
            style.push_str(r.pick(FUZZ_PROPS));
            style.push(';');
        }
        let attr = r.pick(FUZZ_ATTRS);
        out.push_str(&format!("<{tag}{attr} style='{style}'>"));
        if !matches!(tag, "br" | "wbr" | "img" | "input") {
            random_children(r, depth + 1, out);
            out.push_str(&format!("</{tag}>"));
        }
    }
}

#[test]
fn random_documents_never_panic() {
    for seed in 1..=150u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut html = String::from(
            "<style>.x::after{content:open-quote 'A' url(a.png)}</style><body class=x>",
        );
        random_children(&mut r, 0, &mut html);
        let width = [0.0, 50.0, 360.0, 1024.0][(r.next() % 4) as usize];
        let t = std::time::Instant::now();
        let l = lay_any(&html, width);
        sane_layout(&l);
        assert!(t.elapsed().as_secs() < 10, "seed {seed}: {:?}", t.elapsed());
    }
}

#[test]
fn hostile_grids_stay_bounded_and_fast() {
    let mut items = String::new();
    for i in 0..400 {
        items.push_str(&format!(
            "<div style='grid-column:span {} / {};grid-row:{} / span 900'>x</div>",
            i % 7 + 1,
            -(i % 5) - 1,
            i * 37 % 20_000 + 1
        ));
    }
    let html = format!(
        "<div style='display:grid;grid-template-columns:repeat(auto-fill,0px) 1e9px minmax(1e30px,1fr) fit-content(-5px);gap:1e20px'>{items}</div>\
         <div style='display:grid;grid-template-columns:repeat(auto-fit,minmax(1px,1fr));grid-template-rows:repeat(auto-fill,1px);height:1e9px;width:1e6px'>{items}</div>\
         <div style='display:inline-grid;grid-template-columns:repeat(1000,min-content) repeat(1000,max-content);grid-auto-flow:column'>\
           <div style='grid-column:1 / span 10000;grid-row:-10000 / 10000'>y</div><div style='grid-column:-1 / 1'>z</div></div>"
    );
    let t = std::time::Instant::now();
    let l = lay_any(&html, 800.0);
    sane_layout(&l);
    assert!(t.elapsed().as_secs() < 30, "{:?}", t.elapsed());
}

#[test]
fn many_wide_spanning_grid_items_stay_bounded() {
    // Thousands of items each spanning hundreds of intrinsic tracks: the
    // spanning-item distribution has a work budget, beyond which the
    // remaining items no longer grow the tracks.
    let mut items = String::new();
    for i in 0..6000 {
        items.push_str(&format!(
            "<i style='display:block;grid-column:span {};min-width:{}px'>x</i>",
            900 + i % 100,
            i % 3000
        ));
    }
    let html = format!(
        "<div style='display:inline-grid;grid-template-columns:repeat(500,auto) repeat(500,fit-content(20px))'>{items}</div>\
         <div style='display:grid;grid-template-columns:repeat(1000,minmax(min-content,1fr))'>{items}</div>"
    );
    let t = std::time::Instant::now();
    let l = lay_any(&html, 800.0);
    sane_layout(&l);
    assert!(t.elapsed().as_secs() < 30, "{:?}", t.elapsed());
}
