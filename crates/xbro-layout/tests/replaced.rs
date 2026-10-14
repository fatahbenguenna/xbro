//! Lists, replaced elements and form controls.

mod common;

use common::*;
use xbro_layout::{BoxKind, ControlKind, Fragment, MarkerContent};

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}p{margin:0}</style>";

fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

fn marker_text(l: &Laid, id: &str) -> String {
    match &l.box_id(id).marker.as_ref().expect("marker").content {
        MarkerContent::Text(t) => t.text.clone(),
        MarkerContent::Disc => "disc".into(),
        MarkerContent::Circle => "circle".into(),
        MarkerContent::Square => "square".into(),
        MarkerContent::Image(u) => format!("image {u}"),
    }
}

// ---------------------------------------------------------------- lists

#[test]
fn bullets_and_nested_list_styles() {
    let Some(l) = lay(
        &page("<ul><li id=a>a<ul><li id=b>b<ul><li id=c>c</li></ul></li></ul></li></ul><ul style='list-style:none'><li id=n>n</li></ul>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(marker_text(&l, "a"), "disc");
    assert_eq!(marker_text(&l, "b"), "circle");
    assert_eq!(marker_text(&l, "c"), "square");
    assert!(l.box_id("n").marker.is_none());
    // Outside markers sit left of the content, centered around the x-height.
    let a = l.box_id("a");
    let m = a.marker.as_ref().unwrap();
    assert!(m.rect.right() <= a.content.x - 4.0);
    close(m.rect.width, 0.33 * 16.0, 0.01);
    // The first line is in an anonymous block (the item also holds a list).
    let baseline = a.children[0].lines[0].baseline;
    assert!(m.rect.bottom() < baseline && m.rect.y > baseline - 16.0);
}

#[test]
fn ordinal_values_start_reversed_and_value() {
    let Some(l) = lay(
        &page(
            "<ol start=3><li id=a>a</li><li id=b>b</li><li id=c value=10>c</li><li id=d>d</li></ol>\
             <ol reversed><li id=r1>x</li><li id=r2>y</li><li id=r3>z</li></ol>\
             <ol type=i><li>1</li><li>2</li><li>3</li><li id=iv>4</li></ol>\
             <ol style='list-style-type:upper-alpha'><li id=ua>A</li><li id=ub>B</li></ol>\
             <ol style='list-style-type:lower-greek'><li id=g>α</li></ol>\
             <ol style='list-style-type:decimal-leading-zero'><li id=z>0</li></ol>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(marker_text(&l, "a"), "3.");
    assert_eq!(marker_text(&l, "b"), "4.");
    assert_eq!(marker_text(&l, "c"), "10.");
    assert_eq!(marker_text(&l, "d"), "11.");
    assert_eq!(marker_text(&l, "r1"), "3.");
    assert_eq!(marker_text(&l, "r3"), "1.");
    assert_eq!(marker_text(&l, "iv"), "iv.");
    assert_eq!(marker_text(&l, "ub"), "B.");
    assert_eq!(marker_text(&l, "g"), "α.");
    assert_eq!(marker_text(&l, "z"), "01.");
    // Text markers are right-aligned before the content, on the first baseline.
    let a = l.box_id("a");
    let m = a.marker.as_ref().unwrap();
    let MarkerContent::Text(t) = &m.content else {
        panic!()
    };
    close(t.rect.right(), a.content.x - 8.0, 0.01);
    assert_eq!(t.baseline, a.lines[0].baseline);
    assert_eq!(t.node, l.id("a"));
}

#[test]
fn inside_markers_are_inline_text() {
    let Some(l) = lay(
        &page("<ol style='list-style-position:inside'><li id=a>x</li></ol><ul style='list-style-position:inside'><li id=b>y</li></ul>"),
        400.0,
    ) else {
        return;
    };
    assert!(l.box_id("a").marker.is_none());
    assert_eq!(l.line_texts("a"), vec!["1.\u{A0}x"]);
    assert_eq!(l.line_texts("b"), vec!["\u{2022}\u{A0}y"]);
}

#[test]
fn marker_uses_first_block_baseline_and_empty_items_keep_a_line() {
    let Some(l) = lay(
        &page(
            "<ul><li id=a><div style='margin-top:10px'><p>first</p></div></li><li id=e></li></ul>",
        ),
        400.0,
    ) else {
        return;
    };
    let a = l.box_id("a");
    let p = &a.children[0].children[0];
    let m = a.marker.as_ref().unwrap();
    assert!(m.rect.y > p.lines[0].rect.y && m.rect.bottom() < p.lines[0].baseline);
    assert_eq!(l.rect("e").height, 20.0);
}

#[test]
fn list_style_image_marker() {
    let images = FakeImages::new().url("dot.png", 8.0, 6.0);
    let Some(l) = lay_images(
        &page("<ul style='list-style-image:url(dot.png)'><li id=a>x</li></ul><ul style='list-style-image:url(missing.png)'><li id=b>y</li></ul>"),
        400.0,
        images,
    ) else {
        return;
    };
    let m = l.box_id("a").marker.as_ref().unwrap();
    assert!(matches!(&m.content, MarkerContent::Image(u) if u == "dot.png"));
    assert_eq!((m.rect.width, m.rect.height), (8.0, 6.0));
    // Unknown size: falls back to the list-style-type.
    assert_eq!(marker_text(&l, "b"), "disc");
}

// ---------------------------------------------------------------- replaced

#[test]
fn image_natural_size_and_specified_dimensions() {
    let images = FakeImages::new()
        .id("a", 200.0, 100.0)
        .id("b", 200.0, 100.0)
        .id("c", 200.0, 100.0)
        .id("d", 200.0, 100.0)
        .id("e", 800.0, 400.0)
        .id("f", 200.0, 100.0);
    let Some(l) = lay_images(
        &page(
            "<img id=a><img id=b width=50><img id=c style='height:25px'><img id=d width=10 height=10>\
             <div style='width:300px'><img id=e style='max-width:100%'></div>\
             <img id=f style='min-width:400px;max-height:150px'>",
        ),
        1000.0,
        images,
    ) else {
        return;
    };
    let size = |id: &str| {
        let b = l.box_id(id);
        assert_eq!(b.kind, BoxKind::Image);
        (b.content.width, b.content.height)
    };
    assert_eq!(size("a"), (200.0, 100.0));
    assert_eq!(size("b"), (50.0, 25.0));
    assert_eq!(size("c"), (50.0, 25.0));
    assert_eq!(size("d"), (10.0, 10.0));
    assert_eq!(size("e"), (300.0, 150.0));
    // CSS 2.1 §10.4: min-width wins, max-height then limits the height.
    assert_eq!(size("f"), (400.0, 150.0));
}

#[test]
fn image_sits_on_the_baseline() {
    let images = FakeImages::new().id("i", 30.0, 30.0);
    let Some(l) = lay_images(&page("<p id=p>x<img id=i>y</p>"), 400.0, images) else {
        return;
    };
    let line = &l.box_id("p").lines[0];
    close(l.rect("i").bottom(), line.baseline, 0.01);
    assert!(line.rect.height > 30.0);
}

#[test]
fn image_ratio_from_attributes_when_not_loaded() {
    let Some(l) = lay(
        &page("<div style='width:300px'><img id=a width=600 height=400 style='max-width:100%;height:auto'></div>"),
        400.0,
    ) else {
        return;
    };
    let a = l.box_id("a");
    assert_eq!((a.content.width, a.content.height), (300.0, 200.0));
}

#[test]
fn image_alt_text_fallback() {
    let Some(l) = lay(
        &page("<p id=p>see <img id=a alt='broken picture'> and <img id=b> or <img id=c alt='alt' width=80 height=30></p>"),
        400.0,
    ) else {
        return;
    };
    let a = l.box_id("a");
    assert_eq!(a.kind, BoxKind::Image);
    assert_eq!(a.lines.len(), 1);
    let t = match &a.lines[0].fragments[0] {
        Fragment::Text(t) => t,
        _ => panic!(),
    };
    assert_eq!(t.text, "broken picture");
    assert_eq!(t.node, l.id("a"));
    close(a.content.width, t.rect.width, 0.01);
    // The alt text's baseline is the line's baseline.
    close(a.lines[0].baseline, l.box_id("p").lines[0].baseline, 0.01);
    // No alt, no size: nothing.
    let b = l.box_id("b");
    assert_eq!((b.content.width, b.content.height), (0.0, 0.0));
    // Specified size wins, alt text inside.
    let c = l.box_id("c");
    assert_eq!((c.content.width, c.content.height), (80.0, 30.0));
    assert_eq!(c.lines.len(), 1);
}

#[test]
fn svg_and_media_default_sizes() {
    let images = FakeImages::new().id("poster", 640.0, 360.0);
    let Some(l) = lay_images(
        &page(
            "<svg id=s1 width=40 height=20><rect width=10 height=10/></svg>\
             <svg id=s2></svg>\
             <div style='width:200px'><svg id=s3 viewBox='0 0 10 5'></svg></div>\
             <iframe id=f></iframe><video id=v width=150></video><canvas id=c></canvas>\
             <video id=poster></video><iframe id=fw style='width:600px'></iframe>",
        ),
        1000.0,
        images,
    ) else {
        return;
    };
    let size = |id: &str| {
        let b = l.box_id(id);
        (b.content.width, b.content.height)
    };
    assert_eq!(size("s1"), (40.0, 20.0));
    assert_eq!(size("s2"), (300.0, 150.0));
    assert_eq!(size("s3"), (200.0, 100.0));
    assert_eq!(l.box_id("s1").kind, BoxKind::Image);
    assert!(
        l.box_id("s1").children.is_empty(),
        "svg subtree is not laid out"
    );
    assert_eq!(size("f"), (300.0, 150.0));
    assert_eq!(l.box_id("f").kind, BoxKind::Placeholder);
    assert_eq!(size("v"), (150.0, 75.0));
    assert_eq!(size("c"), (300.0, 150.0));
    assert_eq!(size("poster"), (640.0, 360.0));
    assert_eq!(l.box_id("poster").kind, BoxKind::Image);
    assert_eq!(size("fw"), (600.0, 150.0));
}

#[test]
fn block_replaced_and_auto_margins() {
    let images = FakeImages::new().id("i", 100.0, 50.0);
    let Some(l) = lay_images(
        &page("<img id=i style='display:block;margin:0 auto'>"),
        400.0,
        images,
    ) else {
        return;
    };
    assert_eq!(l.rect("i").x, 150.0);
    assert_eq!(l.rect("i").height, 50.0);
}

// ---------------------------------------------------------------- controls

fn control(l: &Laid, id: &str) -> ControlKind {
    match &l.box_id(id).kind {
        BoxKind::Control(k) => k.clone(),
        k => panic!("not a control: {k:?}"),
    }
}

fn control_text(l: &Laid, id: &str) -> Option<(String, xbro_style::Color)> {
    l.box_id(id).lines.first().and_then(|line| {
        line.fragments.iter().find_map(|f| match f {
            Fragment::Text(t) => Some((t.text.clone(), t.style.color)),
            _ => None,
        })
    })
}

#[test]
fn text_inputs_value_placeholder_password() {
    let Some(l) = lay(
        &page(
            "<input id=a value='hello'><input id=b placeholder='type here'>\
             <input id=c type=password value='secret'><input id=d size=10>\
             <input id=e type=email value='a@b.c' placeholder='x'><input id=f style='width:300px'>",
        ),
        1000.0,
    ) else {
        return;
    };
    assert_eq!(control(&l, "a"), ControlKind::TextField);
    let (t, c) = control_text(&l, "a").unwrap();
    assert_eq!(t, "hello");
    assert_eq!(c, xbro_style::Color::BLACK);
    let (t, c) = control_text(&l, "b").unwrap();
    assert_eq!(t, "type here");
    assert_eq!(c, xbro_style::Color::rgb(0x75, 0x75, 0x75));
    assert_eq!(control_text(&l, "c").unwrap().0, "••••••");
    assert_eq!(control_text(&l, "e").unwrap().0, "a@b.c");
    assert!(control_text(&l, "d").is_none());
    // Default width: size × 0.55em (13.333px font), one line high.
    let a = l.box_id("a");
    close(a.content.width, 20.0 * 0.55 * 13.333, 0.05);
    close(l.box_id("d").content.width, 10.0 * 0.55 * 13.333, 0.05);
    assert!(a.content.height > 13.0 && a.content.height < 18.0);
    // UA padding/border.
    assert_eq!(a.border, xbro_layout::Edges::all(1.0));
    assert_eq!(a.padding, xbro_layout::Edges::new(1.0, 2.0, 1.0, 2.0));
    assert_eq!(l.box_id("f").content.width, 300.0);
    // Text is a single line, vertically inside the content box.
    let line = &a.lines[0];
    assert!(line.rect.y >= a.content.y - 0.01 && line.rect.bottom() <= a.content.bottom() + 0.01);
}

#[test]
fn long_values_do_not_wrap() {
    let Some(l) = lay(
        &page("<input id=a value='a very long value that does not fit at all in the field'>"),
        1000.0,
    ) else {
        return;
    };
    assert_eq!(l.box_id("a").lines.len(), 1);
}

#[test]
fn buttons_checkboxes_and_others() {
    let Some(l) = lay(
        &page(
            "<input id=s type=submit><input id=r type=reset><input id=b type=button value='Go'>\
             <input id=cb type=checkbox checked><input id=cu type=checkbox><input id=ra type=radio checked>\
             <input id=rg type=range min=0 max=200 value=50><input id=co type=color value='#ff8000'>\
             <input id=fi type=file><progress id=p value=3 max=4></progress><meter id=m value=0.25></meter>\
             <button id=btn>Click <b>me</b></button>",
        ),
        1000.0,
    ) else {
        return;
    };
    assert_eq!(control(&l, "s"), ControlKind::Button);
    assert_eq!(control_text(&l, "s").unwrap().0, "Submit");
    assert_eq!(control_text(&l, "r").unwrap().0, "Reset");
    assert_eq!(control_text(&l, "b").unwrap().0, "Go");
    assert_eq!(control_text(&l, "fi").unwrap().0, "Choose File");
    // Label is shrink-wrapped.
    let s = l.box_id("s");
    let t = match &s.lines[0].fragments[0] {
        Fragment::Text(t) => t.rect.width,
        _ => panic!(),
    };
    close(s.content.width, t, 0.01);
    assert_eq!(control(&l, "cb"), ControlKind::Checkbox { checked: true });
    assert_eq!(control(&l, "cu"), ControlKind::Checkbox { checked: false });
    assert_eq!(control(&l, "ra"), ControlKind::Radio { checked: true });
    let cb = l.box_id("cb");
    assert_eq!((cb.content.width, cb.content.height), (13.0, 13.0));
    assert_eq!(control(&l, "rg"), ControlKind::Range { value: 0.25 });
    let rg = l.box_id("rg");
    assert_eq!((rg.content.width, rg.content.height), (129.0, 16.0));
    assert_eq!(
        control(&l, "co"),
        ControlKind::Color(xbro_style::Color::rgb(0xff, 0x80, 0))
    );
    assert_eq!(l.rect("co").width, 50.0);
    assert_eq!(l.rect("co").height, 27.0);
    assert_eq!(control(&l, "p"), ControlKind::Progress { value: 0.75 });
    let p = l.box_id("p");
    assert_eq!((p.content.width, p.content.height), (160.0, 16.0));
    assert_eq!(control(&l, "m"), ControlKind::Progress { value: 0.25 });
    assert_eq!(l.box_id("m").content.width, 80.0);
    // <button> is an ordinary inline-block with its children.
    let btn = l.box_id("btn");
    assert_eq!(btn.kind, BoxKind::Block);
    assert_eq!(l.line_texts("btn"), vec!["Click me"]);
}

#[test]
fn select_and_textarea() {
    let Some(l) = lay(
        &page(
            "<select id=s><option>short</option><option selected>the longest option</option><option>mid</option></select>\
             <select id=f><option>first</option><option>second</option></select>\
             <select id=e></select>\
             <textarea id=t cols=10 rows=3>line one\nline two is longer than ten</textarea>\
             <textarea id=d></textarea>",
        ),
        1000.0,
    ) else {
        return;
    };
    assert_eq!(control(&l, "s"), ControlKind::Select);
    assert_eq!(control_text(&l, "s").unwrap().0, "the longest option");
    assert_eq!(control_text(&l, "f").unwrap().0, "first");
    assert!(control_text(&l, "e").is_none());
    let s = l.box_id("s");
    let t = match &s.lines[0].fragments[0] {
        Fragment::Text(t) => t.rect.width,
        _ => panic!(),
    };
    close(s.content.width, t + 24.0, 0.01);
    assert_eq!(control(&l, "t"), ControlKind::TextArea);
    let ta = l.box_id("t");
    let line_h = ta.lines[0].rect.height;
    close(ta.content.height, 3.0 * line_h, 0.01);
    let d = l.box_id("d");
    assert!(d.content.width > 100.0);
    // Pre-wrap: newline preserved, long line wrapped in the 10-column width.
    assert!(ta.lines.len() >= 3, "{}", l.dump());
    assert_eq!(super_text(ta, 0), "line one");
}

fn super_text(b: &xbro_layout::LayoutBox, i: usize) -> String {
    line_text(&b.lines[i])
}

#[test]
fn image_input_is_an_image() {
    let images = FakeImages::new().id("i", 20.0, 10.0);
    let Some(l) = lay_images(&page("<input id=i type=image src=x.png>"), 400.0, images) else {
        return;
    };
    assert_eq!(l.box_id("i").kind, BoxKind::Image);
    assert_eq!(l.box_id("i").content.width, 20.0);
}

#[test]
fn empty_and_filled_text_fields_share_the_baseline() {
    let Some(l) = lay(
        &page("<p id=p>Label <input id=a> <input id=b value=x> <select id=s></select></p>"),
        800.0,
    ) else {
        return;
    };
    let a = l.box_id("a");
    let b = l.box_id("b");
    assert_eq!(a.border_box().y, b.border_box().y);
    assert_eq!(l.box_id("s").border_box().y, a.border_box().y);
    let line = &l.box_id("p").lines[0];
    close(b.lines[0].baseline, line.baseline, 0.01);
}
