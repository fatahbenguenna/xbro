//! Custom properties and `var()`.

mod common;

use common::style;
use xbro_style::*;

const RED: Color = Color::rgb(255, 0, 0);
const GREEN: Color = Color::rgb(0, 128, 0);

#[test]
fn basic_substitution_and_inheritance() {
    let s = style(
        "<div id=d><p id=p>x<span id=s>y</span></p></div>",
        ":root { --main: green; --gap: 4px } p { color: var(--main); margin-left: var(--gap) }
         span { --main: red; color: var(--main) }",
    );
    assert_eq!(s.get("p").color, GREEN);
    assert_eq!(s.get("p").margin.left, LengthPercentageAuto::px(4.0));
    assert_eq!(s.get("s").color, RED);
}

#[test]
fn fallbacks() {
    let s = style(
        "<p id=a>x</p><p id=b>x</p><p id=c>x</p>",
        ":root { --w: 10px }
         #a { width: var(--missing, 20px) }
         #b { width: var(--missing, var(--w)) }
         #c { width: var(--missing, var(--also-missing, calc(1px + 2px))) }",
    );
    assert_eq!(s.get("a").width, LengthPercentageAuto::px(20.0));
    assert_eq!(s.get("b").width, LengthPercentageAuto::px(10.0));
    assert_eq!(s.get("c").width, LengthPercentageAuto::px(3.0));
}

#[test]
fn invalid_at_computed_value_time_behaves_as_unset() {
    let s = style(
        "<div id=d><p id=p>x</p></div>",
        "div { color: green; width: 50px } :root { --bad: frobnicate }
         p { color: red; color: var(--bad); width: 10px; width: var(--bad) }",
    );
    // Unlike a parse-time invalid declaration, the earlier value is NOT used.
    assert_eq!(s.get("p").color, GREEN, "inherited property → inherit");
    assert!(s.get("p").width.is_auto(), "non-inherited → initial");
    let s = style("<p id=p>x</p>", "p { color: red; color: var(--undefined) }");
    assert_eq!(s.get("p").color, Color::BLACK);
}

#[test]
fn cycles() {
    let s = style(
        "<p id=p>x</p>",
        "p { --a: var(--b); --b: var(--a); --c: var(--c);
             width: var(--a, 7px); height: var(--c, 9px); margin-left: var(--b, 3px) }",
    );
    let p = s.get("p");
    assert_eq!(p.width, LengthPercentageAuto::px(7.0));
    assert_eq!(p.height, LengthPercentageAuto::px(9.0));
    assert_eq!(p.margin.left, LengthPercentageAuto::px(3.0));
}

#[test]
fn var_in_shorthands() {
    let s = style(
        "<p id=a>x</p><p id=b>x</p><p id=c>x</p>",
        ":root { --b: 2px solid; --c: red; --m: 1px 2px }
         #a { border: var(--b) var(--c) }
         #b { margin: var(--m) 3px; margin-left: 9px }
         #c { font: bold var(--size, 20px)/1.5 serif; background: var(--c) }",
    );
    let a = s.get("a");
    assert_eq!(a.border.top.width, 2.0);
    assert_eq!(a.border.left.style, BorderStyle::Solid);
    assert_eq!(a.border.bottom.color, RED);
    let b = s.get("b");
    assert_eq!(b.margin.top, LengthPercentageAuto::px(1.0));
    assert_eq!(b.margin.right, LengthPercentageAuto::px(2.0));
    assert_eq!(b.margin.bottom, LengthPercentageAuto::px(3.0));
    assert_eq!(
        b.margin.left,
        LengthPercentageAuto::px(9.0),
        "later longhand wins"
    );
    let c = s.get("c");
    assert_eq!(c.font_size, 20.0);
    assert_eq!(c.font_weight, 700);
    assert_eq!(c.line_height, LineHeight::Number(1.5));
    assert_eq!(c.background_color, RED);
}

#[test]
fn custom_property_keywords_and_case() {
    let s = style(
        "<div id=d><p id=p>x</p><p id=q>x</p></div>",
        "div { --Color: green; --x: red } p { --x: initial; color: var(--x, var(--Color)) }
         #q { --x: inherit; color: var(--x) } #p { background-color: var(--color, blue) }",
    );
    assert_eq!(s.get("p").color, GREEN);
    assert_eq!(
        s.get("p").background_color,
        Color::rgb(0, 0, 255),
        "names are case-sensitive"
    );
    assert_eq!(s.get("q").color, RED);
}

#[test]
fn var_referencing_inherited_values() {
    let s = style(
        "<div id=d><p id=p>x</p></div>",
        "div { --size: 10px } p { --double: calc(var(--size) * 2); width: var(--double) }",
    );
    assert_eq!(s.get("p").width, LengthPercentageAuto::px(20.0));
}

#[test]
fn var_in_style_attribute_and_important() {
    let s = style(
        "<p id=p style='--c: green; color: var(--c)'>x</p>",
        "p { --c: red !important }",
    );
    // `--c: red !important` beats the inline normal declaration.
    assert_eq!(s.get("p").color, RED);
}

#[test]
fn many_custom_properties_are_shared() {
    let mut css = String::from(":root {");
    for i in 0..200 {
        css.push_str(&format!("--v{i}: {i}px;"));
    }
    css.push_str("} p { width: var(--v150) }");
    let mut html = String::new();
    for i in 0..300 {
        html.push_str(&format!("<p id=p{i}>x</p>"));
    }
    let s = style(&html, &css);
    assert_eq!(s.get("p299").width, LengthPercentageAuto::px(150.0));
}
