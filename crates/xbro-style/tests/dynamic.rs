//! Dynamic pseudo-classes, `@media` and `@supports`.

mod common;

use common::{style, style_with, viewport};
use xbro_style::*;

const RED: Color = Color::rgb(255, 0, 0);
const GREEN: Color = Color::rgb(0, 128, 0);

#[test]
fn hover_focus_active() {
    let html = "<div id=menu><a id=link href=#>x<span id=icon>i</span></a></div><input id=field>";
    let css = "a:hover { color: green } div:hover { background-color: red }
               input:focus { outline: 2px solid } div:focus-within { color: red }
               a:active { font-weight: bold } span:hover { cursor: wait }";
    let s = style(html, css);
    assert_eq!(s.get("link").color, Color::rgb(0, 0, 0xee));
    assert_eq!(s.get("field").outline_width, 0.0);
    let doc = xbro_html::parse_document(&format!("<!DOCTYPE html>{html}"));
    let ctx = StyleContext {
        hover: doc.get_element_by_id("icon"),
        focus: doc.get_element_by_id("field"),
        active: doc.get_element_by_id("link"),
        ..StyleContext::default()
    };
    let s = style_with(html, &[css], &ctx);
    assert_eq!(
        s.get("link").color,
        GREEN,
        "ancestors of the hovered element match :hover"
    );
    assert_eq!(s.get("menu").background_color, RED);
    assert_eq!(s.get("icon").cursor, Cursor::Wait);
    assert_eq!(s.get("field").outline_width, 2.0);
    assert_eq!(
        s.get("menu").color,
        Color::BLACK,
        "focus is not inside #menu"
    );
    assert_eq!(s.get("link").font_weight, 700);
}

#[test]
fn media_queries_follow_viewport() {
    let css = "p { color: red } @media (min-width: 600px) { p { color: green } }
               @media (max-width: 400px) { p { width: 10px } }
               @media print { p { display: none } }
               div { width: 50vw }";
    let wide = style_with(
        "<p id=p>x</p><div id=d></div>",
        &[css],
        &viewport(800.0, 600.0),
    );
    assert_eq!(wide.get("p").color, GREEN);
    assert!(wide.get("p").width.is_auto());
    assert_eq!(wide.get("d").width, LengthPercentageAuto::px(400.0));
    let narrow = style_with(
        "<p id=p>x</p><div id=d></div>",
        &[css],
        &viewport(300.0, 600.0),
    );
    assert_eq!(narrow.get("p").color, RED);
    assert_eq!(narrow.get("p").width, LengthPercentageAuto::px(10.0));
    assert_eq!(narrow.get("d").width, LengthPercentageAuto::px(150.0));
}

#[test]
fn supports_rules() {
    let s = style(
        "<p id=p>x</p>",
        "@supports (display: flex) { p { color: green } }
         @supports (frobnicate: yes) { p { color: red } }
         @supports not (display: flex) { p { width: 1px } }",
    );
    assert_eq!(s.get("p").color, GREEN);
    assert!(s.get("p").width.is_auto());
}

#[test]
fn import_rules_are_ignored() {
    let s = style(
        "<p id=p>x</p>",
        "@import url(other.css); p { color: green }",
    );
    assert_eq!(s.get("p").color, GREEN);
}

#[test]
fn form_pseudo_classes() {
    let s = style(
        "<input id=c type=checkbox checked><input id=d disabled><input id=e>",
        "input:checked { width: 5px } input:disabled { width: 6px } input:enabled { height: 7px }",
    );
    assert_eq!(s.get("c").width, LengthPercentageAuto::px(5.0));
    assert_eq!(s.get("d").width, LengthPercentageAuto::px(6.0));
    assert_eq!(s.get("e").height, LengthPercentageAuto::px(7.0));
    assert!(s.get("d").height.is_auto());
}
