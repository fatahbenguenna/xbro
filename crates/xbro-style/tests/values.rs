//! Computed values: units, calc, fonts, shorthands, colors.

mod common;

use common::{approx, style, style_with, viewport};
use xbro_style::*;

const RED: Color = Color::rgb(255, 0, 0);
const BLUE: Color = Color::rgb(0, 0, 255);

fn lp(px: f32, percent: f32) -> LengthPercentage {
    LengthPercentage { px, percent }
}

#[test]
fn length_units() {
    let s = style_with(
        "<div id=d><p id=p>x</p></div>",
        &["div { font-size: 20px } p { font-size: 10px;
           margin-top: 2em; margin-right: 1rem; margin-bottom: 2ex; margin-left: 2ch;
           padding-top: 12pt; padding-right: 1pc; padding-bottom: 1in; padding-left: 2.54cm;
           border-top: 25.4mm solid; border-right: 4q solid;
           top: 10vw; right: 10vh; bottom: 10vmin; left: 10vmax; }"],
        &viewport(800.0, 600.0),
    );
    let p = s.get("p");
    assert_px!(p.margin.top.resolve(0.0).unwrap(), 20.0);
    assert_px!(p.margin.right.resolve(0.0).unwrap(), 16.0);
    assert_px!(p.margin.bottom.resolve(0.0).unwrap(), 10.0);
    assert_px!(p.margin.left.resolve(0.0).unwrap(), 10.0);
    assert_px!(p.padding.top.px, 16.0);
    assert_px!(p.padding.right.px, 16.0);
    assert_px!(p.padding.bottom.px, 96.0);
    assert_px!(p.padding.left.px, 96.0);
    assert_px!(p.border.top.width, 96.0);
    assert_px!(p.border.right.width, 4.0 * 96.0 / 101.6);
    assert_px!(p.top.resolve(0.0).unwrap(), 80.0);
    assert_px!(p.right.resolve(0.0).unwrap(), 60.0);
    assert_px!(p.bottom.resolve(0.0).unwrap(), 60.0);
    assert_px!(p.left.resolve(0.0).unwrap(), 80.0);
}

#[test]
fn monospace_ch_and_percentages() {
    let s = style(
        "<pre id=p>x</pre><div id=d>x</div>",
        "pre { font-size: 10px; width: 10ch } div { width: 50%; height: calc(100% - 10px) }",
    );
    assert_eq!(s.get("p").width, LengthPercentageAuto::px(60.0));
    assert_eq!(s.get("d").width, LengthPercentageAuto::percent(50.0));
    assert_eq!(
        s.get("d").height,
        LengthPercentageAuto::Length(lp(-10.0, 100.0))
    );
}

#[test]
fn calc_min_max_clamp() {
    let s = style_with(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div>",
        &["div { font-size: 10px }
           #a { width: calc(2em + 3px * 2); margin-left: calc((50% - 10px) / 2) }
           #b { width: min(100px, 5em); padding-left: max(1px, 2px, 3px) }
           #c { width: clamp(10px, 50vw, 300px); font-size: clamp(1rem, 2vw, 3rem) }
           #d { width: min(50%, 300px); opacity: calc(0.25 * 2); z-index: calc(1 + 2) }
           #e { width: calc(-50px); padding-top: calc(10px - 20px); line-height: calc(1 + 0.5) }"],
        &viewport(1000.0, 800.0),
    );
    assert_eq!(s.get("a").width, LengthPercentageAuto::px(26.0));
    assert_eq!(
        s.get("a").margin.left,
        LengthPercentageAuto::Length(lp(-5.0, 25.0))
    );
    assert_eq!(s.get("b").width, LengthPercentageAuto::px(50.0));
    assert_eq!(s.get("b").padding.left, LengthPercentage::px(3.0));
    assert_eq!(s.get("c").width, LengthPercentageAuto::px(300.0));
    assert_px!(s.get("c").font_size, 20.0);
    // Mixed px/% in min(): percentages resolved against the viewport width.
    assert_eq!(s.get("d").width, LengthPercentageAuto::px(300.0));
    assert_eq!(s.get("d").opacity, 0.5);
    assert_eq!(s.get("d").z_index, Some(3));
    // calc() results are clamped for properties that cannot be negative.
    assert_eq!(s.get("e").width, LengthPercentageAuto::px(0.0));
    assert_eq!(s.get("e").padding.top, LengthPercentage::px(0.0));
    assert_eq!(s.get("e").line_height, LineHeight::Number(1.5));
}

#[test]
fn font_sizes() {
    let s = style(
        "<div id=root><p id=em>x<span id=nested>y</span></p><p id=pct>x</p><p id=rem>x</p>\
         <p id=kw>x</p><p id=sm>x</p><p id=lg>x</p></div>",
        "html { font-size: 20px } #root { font-size: 10px }
         #em { font-size: 2em } #nested { font-size: 1.5em } #pct { font-size: 150% }
         #rem { font-size: 2rem } #kw { font-size: xx-large } #sm { font-size: smaller }
         #lg { font-size: larger }",
    );
    assert_eq!(s.get("em").font_size, 20.0);
    assert_eq!(s.get("nested").font_size, 30.0);
    assert_eq!(s.get("pct").font_size, 15.0);
    assert_eq!(
        s.get("rem").font_size,
        40.0,
        "rem uses the root element's size"
    );
    assert_eq!(s.get("kw").font_size, 32.0);
    assert_px!(s.get("sm").font_size, 10.0 / 1.2);
    assert_px!(s.get("lg").font_size, 12.0);
    // rem on the root itself refers to the initial 16px.
    let s = style(
        "<p id=p>x</p>",
        "html { font-size: 2rem } p { width: 1rem }",
    );
    assert_eq!(s.tag("html").font_size, 32.0);
    assert_eq!(s.get("p").width, LengthPercentageAuto::px(32.0));
    let keywords = [
        ("xx-small", 9.0),
        ("x-small", 10.0),
        ("small", 13.0),
        ("medium", 16.0),
        ("large", 18.0),
        ("x-large", 24.0),
        ("xx-large", 32.0),
        ("xxx-large", 48.0),
    ];
    for (kw, px) in keywords {
        let s = style("<p id=p>x</p>", &format!("p {{ font-size: {kw} }}"));
        assert_eq!(s.get("p").font_size, px, "{kw}");
    }
}

#[test]
fn em_in_other_properties_uses_own_font_size() {
    let s = style(
        "<div id=d><p id=p>x</p></div>",
        "div { font-size: 10px } p { font-size: 20px; margin-left: 1em; text-indent: 2em }",
    );
    assert_eq!(s.get("p").margin.left, LengthPercentageAuto::px(20.0));
    assert_eq!(s.get("p").text_indent, LengthPercentage::px(40.0));
}

#[test]
fn font_weights() {
    let s = style(
        "<p id=a>x</p><p id=b>x<b id=bb>y<b id=bbb>z</b></b></p><p id=c>x</p><p id=d>x</p><p id=e>x</p>",
        "#a { font-weight: bold } #b { font-weight: 500 } #c { font-weight: 800 }
         #c { font-weight: lighter } #d { font-weight: 1 } #e { font-weight: 1000 }",
    );
    assert_eq!(s.get("a").font_weight, 700);
    assert_eq!(s.get("b").font_weight, 500);
    assert_eq!(s.get("bb").font_weight, 700, "bolder from 500");
    assert_eq!(s.get("bbb").font_weight, 900, "bolder from 700");
    assert_eq!(s.get("c").font_weight, 100, "lighter from the parent's 400");
    assert_eq!(s.get("d").font_weight, 100, "clamped to 100..=900");
    assert_eq!(s.get("e").font_weight, 900);
}

#[test]
fn font_shorthand_and_families() {
    let s = style(
        "<p id=a>x</p><p id=b>x</p><p id=c>x</p>",
        "#a { font: italic bold 12px/30px Georgia, 'Times New Roman', serif }
         #b { line-height: 3; font: 20px sans-serif }
         #c { font: caption }",
    );
    let a = s.get("a");
    assert_eq!(a.font_style, FontStyle::Italic);
    assert_eq!(a.font_weight, 700);
    assert_eq!(a.font_size, 12.0);
    assert_eq!(a.line_height, LineHeight::Px(30.0));
    assert_eq!(
        *a.font_family,
        vec![
            "georgia".to_string(),
            "times new roman".into(),
            "serif".into()
        ]
    );
    let b = s.get("b");
    assert_eq!(b.line_height, LineHeight::Normal, "font resets line-height");
    assert_eq!(b.font_weight, 400);
    let c = s.get("c");
    assert_eq!(c.font_size, 13.0);
    assert_eq!(*c.font_family, vec!["sans-serif".to_string()]);
}

#[test]
fn font_family_arc_is_shared() {
    let s = style(
        "<div id=d><p id=a>x</p><p id=b>x</p></div>",
        "div { font-family: Arial, sans-serif } #a { color: red }",
    );
    assert!(std::sync::Arc::ptr_eq(
        &s.get("a").font_family,
        &s.get("b").font_family
    ));
}

#[test]
fn line_heights() {
    let s = style(
        "<div id=n>x</div><div id=num>x<span id=child>y</span></div><div id=len>x</div>\
         <div id=pct>x</div><div id=em>x</div>",
        "div { font-size: 10px } #n { line-height: normal } #num { line-height: 1.5 }
         #child { font-size: 20px } #len { line-height: 22px } #pct { line-height: 150% }
         #em { line-height: 2em }",
    );
    assert_eq!(s.get("n").line_height, LineHeight::Normal);
    assert_eq!(s.get("num").line_height, LineHeight::Number(1.5));
    assert_eq!(
        s.get("child").line_height,
        LineHeight::Number(1.5),
        "numbers inherit as numbers"
    );
    assert_eq!(s.get("child").line_height_px(0.0), 30.0);
    assert_eq!(s.get("len").line_height, LineHeight::Px(22.0));
    assert_eq!(s.get("pct").line_height, LineHeight::Px(15.0));
    assert_eq!(s.get("em").line_height, LineHeight::Px(20.0));
}

#[test]
fn borders() {
    let s = style(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div>",
        "div { color: blue }
         #a { border: 2px solid }
         #b { border-width: 5px; border-style: none solid hidden dotted }
         #c { border-style: solid; border-color: red currentcolor }
         #d { border-top: thick double red; border-left: thin solid; border-right-style: solid }
         #e { border-inline: 4px dashed; border-block-start: 1px groove }",
    );
    let a = s.get("a");
    assert_eq!(a.border.left.width, 2.0);
    assert_eq!(a.border.left.color, BLUE, "currentcolor by default");
    let b = s.get("b");
    assert_eq!(b.border.top.width, 0.0, "style none → width 0");
    assert_eq!(b.border.right.width, 5.0);
    assert_eq!(b.border.bottom.width, 0.0, "style hidden → width 0");
    assert_eq!(b.border.left.width, 5.0);
    let c = s.get("c");
    assert_eq!(c.border.top.width, 3.0, "medium by default");
    assert_eq!(c.border.top.color, RED);
    assert_eq!(c.border.right.color, BLUE);
    let d = s.get("d");
    assert_eq!(d.border.top.width, 5.0);
    assert_eq!(d.border.top.style, BorderStyle::Double);
    assert_eq!(d.border.left.width, 1.0);
    assert_eq!(d.border.right.width, 3.0);
    assert_eq!(d.border.bottom.width, 0.0);
    let e = s.get("e");
    assert_eq!(e.border.left.width, 4.0);
    assert_eq!(e.border.right.style, BorderStyle::Dashed);
    assert_eq!(e.border.top.style, BorderStyle::Groove);
    assert_eq!(e.border.bottom.width, 0.0);
    assert_eq!(e.border_widths().left, 4.0);
}

#[test]
fn currentcolor_follows_own_color() {
    let s = style(
        "<div id=d><p id=p>x</p></div>",
        "div { color: red; border: 1px solid currentcolor; outline: 1px solid }
         p { color: blue; border: 1px solid; background-color: currentColor;
             text-decoration: underline }",
    );
    let d = s.get("d");
    assert_eq!(d.border.top.color, RED);
    assert_eq!(d.outline_color, RED);
    let p = s.get("p");
    assert_eq!(p.border.top.color, BLUE);
    assert_eq!(p.background_color, BLUE);
    assert_eq!(p.text_decoration_color, None);
    assert_eq!(p.decoration_color, BLUE);
}

#[test]
fn color_currentcolor_inherits() {
    let s = style(
        "<div id=d><p id=p>x</p></div>",
        "div { color: red } p { color: currentcolor }",
    );
    assert_eq!(s.get("p").color, RED);
}

#[test]
fn border_radius_and_outline() {
    let s = style(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div>",
        "#a { border-radius: 1px 2px 3px 4px / 9px }
         #b { border-radius: 50%; border-top-left-radius: 5px }
         #c { outline: 3px dashed red }
         #d { outline: 5px solid; outline: 0 }",
    );
    assert_eq!(
        s.get("a").border_radius,
        [
            LengthPercentage::px(1.0),
            LengthPercentage::px(2.0),
            LengthPercentage::px(3.0),
            LengthPercentage::px(4.0)
        ]
    );
    let b = s.get("b").border_radius;
    assert_eq!(b[0], LengthPercentage::px(5.0));
    assert_eq!(b[2], LengthPercentage::percent(50.0));
    assert_eq!(s.get("c").outline_width, 3.0);
    assert_eq!(s.get("c").outline_color, RED);
    assert_eq!(s.get("d").outline_width, 0.0);
}

#[test]
fn margins_paddings_insets() {
    let s = style(
        "<div id=a></div><div id=b></div><div id=c></div>",
        "#a { margin: 1px 2px; padding: 1px 2px 3px; position: relative; inset: 1px 2px 3px 4px }
         #b { margin: 0 auto; padding-inline: 5px 6px; padding-block: 7px; inset-block: auto 2px }
         #c { margin-block: 1em 2em; margin-inline: 3px }",
    );
    let a = s.get("a");
    assert_eq!(a.margin.bottom, LengthPercentageAuto::px(1.0));
    assert_eq!(a.margin.left, LengthPercentageAuto::px(2.0));
    assert_eq!(a.padding.left, LengthPercentage::px(2.0));
    assert_eq!(a.padding.bottom, LengthPercentage::px(3.0));
    assert_eq!(a.left, LengthPercentageAuto::px(4.0));
    let b = s.get("b");
    assert!(b.margin.left.is_auto() && b.margin.right.is_auto());
    assert_eq!(b.padding.left, LengthPercentage::px(5.0));
    assert_eq!(b.padding.right, LengthPercentage::px(6.0));
    assert_eq!(b.padding.bottom, LengthPercentage::px(7.0));
    assert!(b.top.is_auto());
    assert_eq!(b.bottom, LengthPercentageAuto::px(2.0));
    let c = s.get("c");
    assert_eq!(c.margin.top, LengthPercentageAuto::px(16.0));
    assert_eq!(c.margin.bottom, LengthPercentageAuto::px(32.0));
    assert_eq!(c.margin.right, LengthPercentageAuto::px(3.0));
}

#[test]
fn sizes() {
    let s = style(
        "<div id=a></div><div id=b></div>",
        "#a { width: fit-content; min-width: min-content; max-width: 100%; height: 10px; \
              min-height: auto; max-height: none; box-sizing: border-box; aspect-ratio: 16/9 }
         #b { inline-size: 30px; block-size: 40%; max-inline-size: 50px; min-block-size: 5px }",
    );
    let a = s.get("a");
    assert!(a.width.is_auto());
    assert!(a.min_width.is_auto());
    assert_eq!(
        a.max_width,
        MaxSize::Length(LengthPercentage::percent(100.0))
    );
    assert_eq!(a.max_height, MaxSize::None);
    assert_eq!(a.box_sizing, BoxSizing::BorderBox);
    assert!(approx(a.aspect_ratio.unwrap(), 16.0 / 9.0));
    let b = s.get("b");
    assert_eq!(b.width, LengthPercentageAuto::px(30.0));
    assert_eq!(b.height, LengthPercentageAuto::percent(40.0));
    assert_eq!(b.max_width, MaxSize::Length(LengthPercentage::px(50.0)));
    assert_eq!(b.min_height, LengthPercentageAuto::px(5.0));
}

#[test]
fn backgrounds() {
    let s = style(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div>",
        "#a { background: url(img.png) no-repeat right 10px bottom / cover, rgb(1, 2, 3) }
         #b { background-color: red; background-image: linear-gradient(to right, red, 25%, blue);
              background-position: center; background-size: 10px 20%; background-repeat: repeat-y }
         #c { background: radial-gradient(circle, blue, red) }
         #d { background-color: red; background-image: radial-gradient(blue, red) }
         #e { background: url(a.png); background-position-x: 30%; background-position-y: bottom }",
    );
    let a = s.get("a");
    assert_eq!(a.background_color, Color::rgb(1, 2, 3));
    assert_eq!(
        a.background_image,
        Some(BackgroundImage::Url("img.png".into()))
    );
    assert_eq!(a.background_repeat, BackgroundRepeat::NoRepeat);
    assert_eq!(a.background_size, BackgroundSize::Cover);
    assert_eq!(a.background_position, (lp(-10.0, 100.0), lp(0.0, 100.0)));
    let b = s.get("b");
    assert_eq!(b.background_color, RED);
    assert_eq!(
        b.background_image,
        Some(BackgroundImage::LinearGradient {
            angle_deg: 90.0,
            stops: vec![(RED, 0.0), (BLUE, 1.0)]
        })
    );
    assert_eq!(
        b.background_position,
        (
            LengthPercentage::percent(50.0),
            LengthPercentage::percent(50.0)
        )
    );
    assert_eq!(
        b.background_size,
        BackgroundSize::Explicit(
            Some(LengthPercentage::px(10.0)),
            Some(LengthPercentage::percent(20.0))
        )
    );
    assert_eq!(b.background_repeat, BackgroundRepeat::RepeatY);
    let c = s.get("c");
    assert_eq!(c.background_image, None);
    assert_eq!(
        c.background_color, BLUE,
        "radial gradients paint their first color"
    );
    assert_eq!(
        s.get("d").background_color,
        RED,
        "an explicit color is kept"
    );
    let e = s.get("e");
    assert_eq!(
        e.background_position,
        (
            LengthPercentage::percent(30.0),
            LengthPercentage::percent(100.0)
        )
    );
}

#[test]
fn gradient_angles_and_stops() {
    let s = style(
        "<div id=a></div><div id=b></div>",
        "#a { background-image: linear-gradient(45deg, red, blue 50%, red) }
         #b { color: blue; background-image: linear-gradient(0.5turn, currentcolor, red) }",
    );
    match &s.get("a").background_image {
        Some(BackgroundImage::LinearGradient { angle_deg, stops }) => {
            assert_eq!(*angle_deg, 45.0);
            let positions: Vec<f32> = stops.iter().map(|s| s.1).collect();
            assert_eq!(positions, vec![0.0, 0.5, 1.0]);
        }
        other => panic!("{other:?}"),
    }
    match &s.get("b").background_image {
        Some(BackgroundImage::LinearGradient { angle_deg, stops }) => {
            assert_eq!(*angle_deg, 180.0);
            assert_eq!(stops[0].0, BLUE);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn text_properties() {
    let s = style(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div>",
        "#a { text-align: end; text-transform: uppercase; letter-spacing: 0.1em; word-spacing: 3px;
              white-space: nowrap; overflow-wrap: break-word; font-size: 20px }
         #b { text-align: -webkit-center; white-space: pre-line; word-break: keep-all }
         #c { text-align: justify; white-space: preserve; text-wrap: nowrap }
         #d { white-space: pre-wrap; text-wrap-mode: nowrap; white-space-collapse: collapse }
         #e { white-space: break-spaces; word-wrap: anywhere }",
    );
    let a = s.get("a");
    assert_eq!(a.text_align, TextAlign::Right);
    assert_eq!(a.text_transform, TextTransform::Uppercase);
    assert_eq!(a.letter_spacing, 2.0);
    assert_eq!(a.word_spacing, 3.0);
    assert_eq!(a.white_space, WhiteSpace::Nowrap);
    assert!(a.break_anywhere);
    let b = s.get("b");
    assert_eq!(b.text_align, TextAlign::Center);
    assert_eq!(b.white_space, WhiteSpace::PreLine);
    assert!(!b.break_anywhere);
    assert_eq!(s.get("c").text_align, TextAlign::Justify);
    assert_eq!(s.get("c").white_space, WhiteSpace::Pre);
    assert_eq!(s.get("d").white_space, WhiteSpace::Nowrap);
    assert_eq!(s.get("e").white_space, WhiteSpace::BreakSpaces);
    assert!(s.get("e").break_anywhere);
}

#[test]
fn text_align_match_parent() {
    let s = style(
        "<div id=d><p id=p>x</p></div>",
        "div { text-align: right } p { text-align: left; text-align: match-parent }",
    );
    assert_eq!(s.get("p").text_align, TextAlign::Right);
}

#[test]
fn vertical_align_values() {
    let s = style(
        "<span id=a>x</span><span id=b>x</span><span id=c>x</span><span id=d>x</span>",
        "span { font-size: 10px; line-height: 20px } #a { vertical-align: super }
         #b { vertical-align: -3px } #c { vertical-align: 50% } #d { vertical-align: 1em }",
    );
    assert_eq!(s.get("a").vertical_align, VerticalAlign::Super);
    assert_eq!(s.get("b").vertical_align, VerticalAlign::Length(-3.0));
    assert_eq!(s.get("c").vertical_align, VerticalAlign::Length(10.0));
    assert_eq!(s.get("d").vertical_align, VerticalAlign::Length(10.0));
}

#[test]
fn lists_and_tables() {
    let s = style(
        "<ul id=u><li id=l>x</li></ul><table id=t><tr><td>x</td></tr></table><ol id=o></ol>",
        "#u { list-style: square inside url(dot.png) } #o { list-style: lower-roman }
         #t { border-collapse: collapse; border-spacing: 3px 4px }",
    );
    let l = s.get("l");
    assert_eq!(l.list_style_type, ListStyleType::Square);
    assert_eq!(l.list_style_position, ListStylePosition::Inside);
    assert_eq!(l.list_style_image.as_deref(), Some("dot.png"));
    assert_eq!(s.get("o").list_style_type, ListStyleType::LowerRoman);
    assert_eq!(s.get("t").border_collapse, BorderCollapse::Collapse);
    assert_eq!(s.get("t").border_spacing, (3.0, 4.0));
}

#[test]
fn flex_properties() {
    let s = style(
        "<div id=c><p id=a>x</p><p id=b>x</p><p id=d>x</p><p id=e>x</p></div>",
        "#c { display: flex; flex-flow: column-reverse wrap; justify-content: space-evenly;
              align-items: baseline; align-content: center; gap: 10px 5% }
         #a { flex: 1; order: -1; align-self: flex-end }
         #b { flex: auto }
         #d { flex: none }
         #e { flex: 2 3 10% }",
    );
    let c = s.get("c");
    assert_eq!(c.flex_direction, FlexDirection::ColumnReverse);
    assert_eq!(c.flex_wrap, FlexWrap::Wrap);
    assert_eq!(c.justify_content, JustifyContent::SpaceEvenly);
    assert_eq!(c.align_items, AlignItems::Baseline);
    assert_eq!(c.align_content, JustifyContent::Center);
    assert_eq!(c.row_gap, LengthPercentage::px(10.0));
    assert_eq!(c.column_gap, LengthPercentage::percent(5.0));
    let a = s.get("a");
    assert_eq!((a.flex_grow, a.flex_shrink), (1.0, 1.0));
    assert_eq!(a.flex_basis, LengthPercentageAuto::percent(0.0));
    assert_eq!(a.order, -1);
    assert_eq!(a.align_self, AlignItems::FlexEnd);
    let b = s.get("b");
    assert_eq!(
        (b.flex_grow, b.flex_shrink, b.flex_basis),
        (1.0, 1.0, LengthPercentageAuto::Auto)
    );
    let d = s.get("d");
    assert_eq!(
        (d.flex_grow, d.flex_shrink, d.flex_basis),
        (0.0, 0.0, LengthPercentageAuto::Auto)
    );
    let e = s.get("e");
    assert_eq!((e.flex_grow, e.flex_shrink), (2.0, 3.0));
    assert_eq!(e.flex_basis, LengthPercentageAuto::percent(10.0));
}

#[test]
fn grid_columns_and_place() {
    let s = style(
        "<div id=g></div><div id=h></div><div id=f></div>",
        "#g { display: grid; grid-template-columns: repeat(3, 1fr) 200px; place-content: center end;
              place-items: start }
         #h { display: block; grid-template-columns: 1fr 1fr }
         #f { display: inline-grid; grid-template-columns: repeat(auto-fit, minmax(10px, 1fr)) }",
    );
    let g = s.get("g");
    assert_eq!(g.grid_columns, 4);
    assert_eq!(g.align_content, JustifyContent::Center);
    assert_eq!(g.justify_content, JustifyContent::FlexEnd);
    assert_eq!(g.align_items, AlignItems::FlexStart);
    assert_eq!(s.get("h").grid_columns, 0, "not a grid");
    assert_eq!(s.get("f").grid_columns, 0, "auto-fit is unknown");
}

#[test]
fn overflow_visibility_cursor_misc() {
    let s = style(
        "<div id=a></div><div id=b></div><div id=c></div>",
        "#a { overflow: hidden; visibility: collapse; cursor: url(x.cur), pointer; opacity: 40%; z-index: 5 }
         #b { overflow-x: hidden }
         #c { overflow: clip; float: inline-end; clear: both }",
    );
    let a = s.get("a");
    assert_eq!(
        (a.overflow_x, a.overflow_y),
        (Overflow::Hidden, Overflow::Hidden)
    );
    assert_eq!(a.visibility, Visibility::Collapse);
    assert_eq!(a.cursor, Cursor::Pointer);
    assert!(approx(a.opacity, 0.4));
    assert_eq!(a.z_index, Some(5));
    let b = s.get("b");
    assert_eq!(
        b.overflow_y,
        Overflow::Auto,
        "visible next to hidden computes to auto"
    );
    let c = s.get("c");
    assert_eq!(
        (c.overflow_x, c.overflow_y),
        (Overflow::Clip, Overflow::Clip)
    );
    assert_eq!(c.float, Float::Right);
    assert_eq!(c.clear, Clear::Both);
}

#[test]
fn content_property() {
    let s = style(
        "<p id=p title=hello>x</p>",
        "p { content: 'a' attr(title) }",
    );
    assert_eq!(
        s.get("p").content,
        Content::Items(vec![
            ContentItem::Text("a".into()),
            ContentItem::Text("hello".into())
        ])
    );
}

#[test]
fn rem_on_the_root_element_uses_its_own_font_size() {
    let s = style(
        "<p id=p>x</p>",
        "html { font-size: 20px; padding-left: 1rem; font-weight: bold } p { margin-left: 1rem }",
    );
    assert_eq!(s.tag("html").padding.left, LengthPercentage::px(20.0));
    assert_eq!(s.get("p").margin.left, LengthPercentageAuto::px(20.0));
}
