//! Cascade order, inheritance, initial values and CSS-wide keywords.

mod common;

use common::{style, style_with};
use xbro_style::*;

const RED: Color = Color::rgb(255, 0, 0);
const GREEN: Color = Color::rgb(0, 128, 0);
const BLUE: Color = Color::rgb(0, 0, 255);

#[test]
fn specificity_beats_source_order() {
    let s = style(
        "<p id=a class=c>x</p>",
        "#a { color: green } .c { color: red } p { color: blue }",
    );
    assert_eq!(s.get("a").color, GREEN);
    let s = style(
        "<div class=x><p id=a class=c>x</p></div>",
        ".x .c { color: green } .c { color: red }",
    );
    assert_eq!(s.get("a").color, GREEN);
    // :where() has zero specificity, :is() takes its most specific argument.
    let s = style(
        "<p id=a class=c>x</p>",
        "p { color: green } :where(#a) { color: red } :is(.c, #zz) { color: blue }",
    );
    assert_eq!(s.get("a").color, BLUE);
}

#[test]
fn source_order_breaks_ties() {
    let s = style("<p id=a>x</p>", "p { color: red } p { color: green }");
    assert_eq!(s.get("a").color, GREEN);
    // Later sheets win too.
    let s = style_with(
        "<p id=a>x</p>",
        &["p { color: red }", "p { color: green }"],
        &StyleContext::default(),
    );
    assert_eq!(s.get("a").color, GREEN);
}

#[test]
fn important_and_inline_styles() {
    let s = style(
        "<p id=a style='color: red'>x</p>",
        "#a { color: blue } p { color: green !important }",
    );
    assert_eq!(s.get("a").color, GREEN, "author !important beats inline");
    let s = style(
        "<p id=a style='color: green'>x</p>",
        "#a#a#a { color: red }",
    );
    assert_eq!(s.get("a").color, GREEN, "inline beats any selector");
    let s = style(
        "<p id=a style='color: green !important'>x</p>",
        "#a { color: red !important }",
    );
    assert_eq!(
        s.get("a").color,
        GREEN,
        "inline !important beats sheet !important"
    );
    let s = style("<p id=a style='color: red; color: green'>x</p>", "");
    assert_eq!(s.get("a").color, GREEN);
}

#[test]
fn origins() {
    // Author normal beats UA normal.
    let s = style("<h1 id=h>x</h1>", "h1 { font-weight: 300 }");
    assert_eq!(s.get("h").font_weight, 300);
    // UA rules apply when nothing overrides them.
    let s = style("<h1 id=h>x</h1>", "");
    assert_eq!(s.get("h").font_weight, 700);
    // UA !important cannot be beaten... (xbro's UA sheet has none, so check
    // author !important vs UA normal instead).
    let s = style("<b id=b>x</b>", "b { font-weight: normal !important }");
    assert_eq!(s.get("b").font_weight, 400);
}

#[test]
fn invalid_declarations_are_ignored() {
    let s = style(
        "<p id=a>x</p>",
        "p { color: green; color: notacolor; width: 10px; width: -5px; margin-left: 12 }",
    );
    let a = s.get("a");
    assert_eq!(a.color, GREEN);
    assert_eq!(a.width, LengthPercentageAuto::px(10.0));
    assert_eq!(a.margin.left, LengthPercentageAuto::ZERO);
}

#[test]
fn inheritance_and_initial_values() {
    let s = style(
        "<div id=d><span id=s>x</span></div>",
        "div { color: red; font-size: 20px; width: 50px; padding: 3px; \
         letter-spacing: 2px; visibility: hidden; cursor: pointer; text-indent: 5px; \
         list-style-type: square; border-collapse: collapse; border-spacing: 4px; \
         white-space: pre; word-break: break-all; opacity: 0.5 }",
    );
    let span = s.get("s");
    assert_eq!(span.color, RED);
    assert_eq!(span.font_size, 20.0);
    assert_eq!(span.letter_spacing, 2.0);
    assert_eq!(span.visibility, Visibility::Hidden);
    assert_eq!(span.cursor, Cursor::Pointer);
    assert_eq!(span.text_indent, LengthPercentage::px(5.0));
    assert_eq!(span.list_style_type, ListStyleType::Square);
    assert_eq!(span.border_collapse, BorderCollapse::Collapse);
    assert_eq!(span.border_spacing, (4.0, 4.0));
    assert_eq!(span.white_space, WhiteSpace::Pre);
    assert!(span.break_anywhere);
    // Not inherited:
    assert!(span.width.is_auto());
    assert_eq!(span.padding, Sides::all(LengthPercentage::ZERO));
    assert_eq!(span.opacity, 1.0);
}

#[test]
fn css_wide_keywords() {
    let s = style(
        "<div id=d><p id=i>x</p><p id=n>x</p><p id=u>x</p><p id=w>x</p><p id=r>x</p></div>",
        "div { color: red; width: 100px; border: 2px solid blue }
         p { color: green; width: 10px }
         #i { width: inherit; border-top-width: inherit; border-top-style: inherit }
         #n { color: initial; width: initial }
         #u { color: unset; width: unset }
         #w { all: initial }
         #r { color: revert; width: revert }",
    );
    let i = s.get("i");
    assert_eq!(i.width, LengthPercentageAuto::px(100.0));
    assert_eq!(i.border.top.width, 2.0);
    assert_eq!(i.border.top.style, BorderStyle::Solid);
    let n = s.get("n");
    assert_eq!(n.color, Color::BLACK);
    assert!(n.width.is_auto());
    let u = s.get("u");
    assert_eq!(u.color, RED, "unset inherits inherited properties");
    assert!(u.width.is_auto(), "unset resets the others");
    let w = s.get("w");
    assert_eq!(w.display, Display::Inline, "all: initial resets display");
    assert_eq!(w.color, Color::BLACK);
    let r = s.get("r");
    assert_eq!(r.color, RED);
    assert!(r.width.is_auto());
}

#[test]
fn revert_rolls_back_to_the_user_agent_value() {
    let s = style(
        "<div id=d>x</div><a id=l href=x>l</a><p id=p>y</p>",
        "div { display: inline } #d { display: revert }
         a { color: red; text-decoration: none } #l { color: revert; text-decoration: revert-layer }
         body { color: green } p { color: blue } #p { color: revert; margin: revert }",
    );
    assert_eq!(s.get("d").display, Display::Block);
    assert_eq!(s.get("l").color, Color::rgb(0, 0, 0xee));
    assert!(s.get("l").text_decoration_line.underline);
    assert_eq!(
        s.get("p").color,
        GREEN,
        "no UA value: behaves as unset (inherit)"
    );
    assert_eq!(s.get("p").margin.top, LengthPercentageAuto::px(16.0));
}

#[test]
fn shorthand_and_longhand_interplay() {
    let s = style(
        "<p id=a>x</p><p id=b>x</p>",
        "#a { margin: 1px; margin-left: 5px } #b { margin-left: 5px; margin: 1px }",
    );
    assert_eq!(s.get("a").margin.left, LengthPercentageAuto::px(5.0));
    assert_eq!(s.get("a").margin.top, LengthPercentageAuto::px(1.0));
    assert_eq!(s.get("b").margin.left, LengthPercentageAuto::px(1.0));
    // Logical and physical properties share a slot: the later one wins.
    let s = style(
        "<p id=a>x</p><p id=b>x</p>",
        "#a { margin-left: 3px; margin-inline-start: 7px } #b { margin-inline-start: 7px; margin-left: 3px }",
    );
    assert_eq!(s.get("a").margin.left, LengthPercentageAuto::px(7.0));
    assert_eq!(s.get("b").margin.left, LengthPercentageAuto::px(3.0));
}

#[test]
fn selector_list_with_invalid_selector_is_dropped() {
    let s = style(
        "<p id=a>x</p>",
        "p, p:frobnicate { color: red } p { color: green }",
    );
    assert_eq!(s.get("a").color, GREEN);
}

#[test]
fn nested_rules_apply() {
    let s = style(
        "<div class=card><p id=a>x</p></div>",
        ".card { color: red; & p { color: green } }",
    );
    assert_eq!(s.get("a").color, GREEN);
}

#[test]
fn quirks_mode_unitless_lengths_and_table_reset() {
    // No doctype: quirks mode.
    let doc = xbro_html::parse_document(
        "<body><div id=d style='width: 100; margin-left: 5'>x</div>\
         <font size=7><table><tr><td id=c>x</td></tr></table></font></body>",
    );
    assert!(doc.quirks_mode);
    let map = compute_styles(&doc, &[], &StyleContext::default());
    let d = map.get(doc.get_element_by_id("d").unwrap()).unwrap();
    assert_eq!(d.width, LengthPercentageAuto::px(100.0));
    assert_eq!(d.margin.left, LengthPercentageAuto::px(5.0));
    let c = map.get(doc.get_element_by_id("c").unwrap()).unwrap();
    assert_eq!(
        c.font_size, 16.0,
        "tables do not inherit font size in quirks mode"
    );
    // Standards mode: unitless lengths are invalid.
    let s = style("<div id=d style='width: 100'>x</div>", "");
    assert!(s.get("d").width.is_auto());
}
