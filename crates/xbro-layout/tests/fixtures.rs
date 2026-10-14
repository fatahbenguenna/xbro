//! Every fixture of `tests/fixtures` laid out at desktop and phone widths:
//! no panic, sane geometry and a few plausibility checks per page.

mod common;

use common::*;
use xbro_layout::{BoxKind, ControlKind, Fragment, LayoutBox, MarkerContent, Size};

const FIXTURES: [&str; 8] = [
    "basic.html",
    "hn.html",
    "wiki.html",
    "modern.html",
    "forms.html",
    "layout.html",
    "tables.html",
    "text.html",
];

fn load(name: &str, width: f32) -> Option<Laid> {
    let html = std::fs::read_to_string(fixtures_dir().join(name)).expect("fixture");
    Some(lay_full(&html, Size::new(width, 768.0), fonts()?, None))
}

fn visit<'a>(b: &'a LayoutBox, f: &mut dyn FnMut(&'a LayoutBox)) {
    f(b);
    for l in &b.lines {
        for fr in &l.fragments {
            if let Fragment::Atomic(a) = fr {
                visit(a, f);
            }
        }
    }
    for c in &b.children {
        visit(c, f);
    }
}

fn all_boxes(l: &Laid) -> Vec<&LayoutBox> {
    let mut out = Vec::new();
    if let Some(root) = &l.tree.root {
        visit(root, &mut |b| out.push(b));
    }
    out
}

/// The first element with the given class.
fn class(l: &Laid, c: &str) -> xbro_dom::NodeId {
    l.doc
        .descendants(l.doc.root())
        .find(|&n| l.doc.element(n).is_some_and(|e| e.has_class(c)))
        .unwrap_or_else(|| panic!("no .{c}"))
}

#[test]
fn all_fixtures_lay_out_at_desktop_and_phone_widths() {
    for name in FIXTURES {
        for width in [1024.0, 360.0] {
            let Some(l) = load(name, width) else {
                return;
            };
            let root = l.tree.root.as_ref().expect("root box");
            assert_eq!(root.border_box().x, 0.0, "{name}");
            assert_eq!(root.border_box().width, width, "{name}");
            let texts = l.texts();
            assert!(texts.len() > 5, "{name} @ {width}: no text");
            for t in &texts {
                assert!(t.rect.x.is_finite() && t.rect.y.is_finite(), "{name}");
                assert!(t.rect.width >= 0.0 && t.rect.height > 0.0, "{name}: {t:?}");
            }
            let ds = l.tree.document_size;
            assert!(ds.width >= width && ds.height >= 768.0, "{name}");
            assert!(ds.height < 100_000.0, "{name}: absurd height {}", ds.height);
            // At least most text starts inside the page horizontally.
            let inside = texts
                .iter()
                .filter(|t| t.rect.x >= -1.0 && t.rect.x < ds.width)
                .count();
            assert!(inside * 10 >= texts.len() * 9, "{name} @ {width}");
            for b in all_boxes(&l) {
                let r = b.border_box();
                assert!(
                    r.width >= 0.0 && r.height >= 0.0 && r.x.is_finite() && r.y.is_finite(),
                    "{name}"
                );
            }
            let _ = l.dump();
        }
    }
}

#[test]
fn basic_fixture_geometry() {
    let Some(l) = load("basic.html", 1024.0) else {
        return;
    };
    let h1 = l.box_of(l.tag("h1", 0)).unwrap();
    close(h1.border_box().y, 21.44, 0.05);
    assert_eq!(h1.border_box().x, 8.0);
    let body = l.box_of(l.doc.body().unwrap()).unwrap();
    assert_eq!(body.content.x, 8.0);
    assert_eq!(body.content.width, 1008.0);
    // <ol start=3> with value=10.
    let ol = l.tag("ol", 0);
    let markers: Vec<String> = l
        .doc
        .element_children(ol)
        .filter_map(|li| l.box_of(li))
        .filter_map(|b| match &b.marker.as_ref()?.content {
            MarkerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(markers, vec!["3.", "4.", "10.", "11."]);
    // <ol type=i>.
    let ol2 = l.tag("ol", 1);
    let last = l.doc.element_children(ol2).last().unwrap();
    match &l.box_of(last).unwrap().marker.as_ref().unwrap().content {
        MarkerContent::Text(t) => assert_eq!(t.text, "iv."),
        m => panic!("{m:?}"),
    }
    // The <pre> keeps its lines.
    let pre = l.box_of(l.tag("pre", 0)).unwrap();
    assert_eq!(pre.lines.len(), 4);
    // The noscript content is rendered, the script is not.
    let all: String = l.texts().iter().map(|t| t.text.as_str()).collect();
    assert!(all.contains("JavaScript"));
    assert!(!all.contains("document.write"));
}

#[test]
fn wiki_fixture_float_and_wrapping() {
    let Some(l) = load("wiki.html", 1024.0) else {
        return;
    };
    let infobox = l.box_of(class(&l, "infobox")).expect("infobox box");
    // The infobox floats right; the text next to it is narrowed.
    assert!(infobox.style.float == xbro_style::Float::Right);
    let output = l.box_of(class(&l, "mw-parser-output")).unwrap();
    close(infobox.margin_box().right(), output.content.right(), 1.0);
    let ib = infobox.border_box();
    let mut beside = 0;
    for b in all_boxes(&l) {
        for line in &b.lines {
            if line.rect.y >= ib.y && line.rect.bottom() <= ib.bottom() && line.rect.x < ib.x {
                assert!(line.rect.right() <= ib.x + 1.0, "line overlaps the float");
                beside += 1;
            }
        }
    }
    assert!(beside > 3);
    let Some(narrow) = load("wiki.html", 360.0) else {
        return;
    };
    assert!(narrow.tree.document_size.height > l.tree.document_size.height);
}

#[test]
fn forms_fixture_controls() {
    let Some(l) = load("forms.html", 1024.0) else {
        return;
    };
    let mut kinds = Vec::new();
    for b in all_boxes(&l) {
        if let BoxKind::Control(k) = &b.kind {
            kinds.push(k.clone());
        }
    }
    assert!(kinds.contains(&ControlKind::Checkbox { checked: true }));
    assert!(kinds.contains(&ControlKind::Checkbox { checked: false }));
    assert!(kinds.contains(&ControlKind::Radio { checked: true }));
    assert!(kinds.contains(&ControlKind::Select));
    assert!(kinds.contains(&ControlKind::TextArea));
    assert!(kinds.contains(&ControlKind::Range { value: 0.7 }));
    assert!(kinds
        .iter()
        .any(|k| matches!(k, ControlKind::Progress { value } if (*value - 0.4).abs() < 1e-6)));
    assert!(kinds.iter().any(|k| matches!(k, ControlKind::Color(_))));
    // The hidden input generates nothing.
    let hidden = l
        .doc
        .get_elements_by_tag_name("input")
        .into_iter()
        .find(|&n| l.doc.attr(n, "type") == Some("hidden"))
        .unwrap();
    assert!(l.box_of(hidden).is_none());
    // The password is shown as bullets, the placeholder in grey.
    let pw = l.box_of(l.id("pw")).unwrap();
    assert_eq!(line_text(&pw.lines[0]), "••••••");
    let q = l.box_of(l.id("q")).unwrap();
    match &q.lines[0].fragments[0] {
        Fragment::Text(t) => assert_eq!(t.style.color, xbro_style::Color::rgb(0x75, 0x75, 0x75)),
        f => panic!("{f:?}"),
    }
}

#[test]
fn layout_fixture_positioning() {
    let Some(l) = load("layout.html", 1024.0) else {
        return;
    };
    let fixed = l.box_of(class(&l, "fixed")).unwrap();
    assert!(fixed.is_fixed());
    let fb = fixed.border_box();
    close(fb.right(), 1024.0 - 10.0, 0.5);
    close(fb.bottom(), 768.0 - 10.0, 0.5);
    let rel = l.box_of(class(&l, "rel")).unwrap();
    let pad = rel.padding_box();
    let abs = rel
        .children
        .iter()
        .filter(|c| c.style.position == xbro_style::Position::Absolute)
        .count();
    assert_eq!(abs, 3);
    let tl = &rel
        .children
        .iter()
        .find(|c| c.style.position == xbro_style::Position::Absolute)
        .unwrap();
    assert_eq!(tl.border_box().x, pad.x + 10.0);
    assert_eq!(tl.border_box().y, pad.y + 10.0);
    let center = l.box_of(class(&l, "center")).unwrap();
    let body = l.box_of(l.doc.body().unwrap()).unwrap();
    close(center.border_box().width, body.content.width / 2.0, 0.01);
    close(
        center.border_box().x - body.content.x,
        body.content.right() - center.border_box().right(),
        0.01,
    );
    // The page is long (1200px block) and the document size says so.
    assert!(l.tree.document_size.height > 2000.0);
}
