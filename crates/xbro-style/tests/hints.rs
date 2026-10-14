//! HTML presentational hints.

mod common;

use common::style;
use xbro_style::*;

const RED: Color = Color::rgb(255, 0, 0);
const GRAY: Color = Color::rgb(128, 128, 128);

#[test]
fn table_attributes() {
    let s = style(
        "<table id=t bgcolor=#ff0000 width=80% border=2 cellpadding=5 cellspacing=0 align=center>\
         <tr id=r valign=top bgcolor=blue><td id=c width=100 height=20 align=right nowrap>x</td>\
         <th id=h bgcolor=lime>y</th></tr></table>\
         <table id=t0 width=0 border=0><tr><td id=c0>z</td></tr></table>",
        "",
    );
    let t = s.get("t");
    assert_eq!(t.background_color, RED);
    assert_eq!(t.width, LengthPercentageAuto::percent(80.0));
    assert_eq!(t.border.top.width, 2.0);
    assert_eq!(t.border.top.style, BorderStyle::Outset);
    assert_eq!(t.border.top.color, GRAY);
    assert_eq!(t.border_spacing, (0.0, 0.0));
    assert!(t.margin.left.is_auto() && t.margin.right.is_auto());
    let r = s.get("r");
    assert_eq!(r.background_color, Color::rgb(0, 0, 255));
    assert_eq!(r.vertical_align, VerticalAlign::Top);
    let c = s.get("c");
    assert_eq!(c.width, LengthPercentageAuto::px(100.0));
    assert_eq!(c.height, LengthPercentageAuto::px(20.0));
    assert_eq!(c.text_align, TextAlign::Right);
    assert_eq!(c.white_space, WhiteSpace::Nowrap);
    assert_eq!(c.padding, Sides::all(LengthPercentage::px(5.0)));
    assert_eq!(c.border.left.width, 1.0);
    assert_eq!(c.border.left.style, BorderStyle::Inset);
    assert_eq!(
        c.vertical_align,
        VerticalAlign::Top,
        "td inherits the row's valign"
    );
    assert_eq!(s.get("h").background_color, Color::rgb(0, 255, 0));
    assert!(s.get("t0").width.is_auto(), "width=0 is ignored on tables");
    assert_eq!(s.get("t0").border.top.width, 0.0);
    assert_eq!(s.get("c0").border.top.width, 0.0);
    assert_eq!(
        s.get("c0").padding.top,
        LengthPercentage::px(1.0),
        "UA padding"
    );
}

#[test]
fn table_border_without_value_and_bordercolor() {
    let s = style(
        "<table id=t border bordercolor=red><tr><td id=c>x</td></tr></table>",
        "",
    );
    assert_eq!(s.get("t").border.top.width, 1.0);
    assert_eq!(s.get("t").border.top.color, RED);
    assert_eq!(s.get("c").border.top.color, RED);
}

#[test]
fn hints_lose_to_author_rules() {
    let s = style(
        "<table id=t bgcolor=red width=50><tr><td>x</td></tr></table>",
        "table { background-color: blue }",
    );
    assert_eq!(s.get("t").background_color, Color::rgb(0, 0, 255));
    assert_eq!(s.get("t").width, LengthPercentageAuto::px(50.0));
}

#[test]
fn font_element() {
    let s = style(
        "<font id=a color=red size=5 face='Verdana, Arial'>x</font><font id=b size=+1>y</font>\
         <font id=c size=-2>z</font><font id=d size=1>w</font><font id=e color=00ff00>v</font>",
        "",
    );
    let a = s.get("a");
    assert_eq!(a.color, RED);
    assert_eq!(a.font_size, 24.0);
    assert_eq!(*a.font_family, vec!["verdana".to_string(), "arial".into()]);
    assert_eq!(s.get("b").font_size, 18.0);
    assert_eq!(s.get("c").font_size, 10.0);
    assert_eq!(s.get("d").font_size, 10.0);
    assert_eq!(s.get("e").color, Color::rgb(0, 255, 0));
}

#[test]
fn align_and_hidden() {
    let s = style(
        "<div id=d align=center>x</div><p id=p align=JUSTIFY>y</p><h2 id=h align=right>z</h2>\
         <div id=hid hidden>gone</div><img id=img align=left><img id=img2 align=middle>\
         <table id=tl align=right><tr><td>t</td></tr></table>",
        "",
    );
    assert_eq!(s.get("d").text_align, TextAlign::Center);
    assert_eq!(s.get("p").text_align, TextAlign::Justify);
    assert_eq!(s.get("h").text_align, TextAlign::Right);
    assert!(s.try_get("hid").is_none());
    assert_eq!(s.get("img").float, Float::Left);
    assert_eq!(s.get("img2").vertical_align, VerticalAlign::Middle);
    assert_eq!(s.get("tl").float, Float::Right);
    // Author rules can show `hidden` elements again.
    let s = style("<div id=hid hidden>x</div>", "[hidden] { display: block }");
    assert_eq!(s.get("hid").display, Display::Block);
}

#[test]
fn image_dimensions_and_spacing() {
    let s = style(
        "<img id=i width=120 height=50% hspace=4 vspace=2 border=3>\
         <iframe id=f width=300 height=150></iframe><canvas id=c width=0></canvas>",
        "",
    );
    let i = s.get("i");
    assert_eq!(i.width, LengthPercentageAuto::px(120.0));
    assert_eq!(i.height, LengthPercentageAuto::percent(50.0));
    assert_eq!(i.margin.left, LengthPercentageAuto::px(4.0));
    assert_eq!(i.margin.top, LengthPercentageAuto::px(2.0));
    assert_eq!(i.border.top.width, 3.0);
    assert_eq!(i.border.top.style, BorderStyle::Solid);
    assert_eq!(s.get("f").width, LengthPercentageAuto::px(300.0));
    assert_eq!(s.get("c").width, LengthPercentageAuto::px(0.0));
}

#[test]
fn body_attributes() {
    let s = style(
        "<body id=b bgcolor=black text=white link=red marginwidth=0 topmargin=3>\
         <a id=a href=x>l</a><a id=n>no href</a></body>",
        "",
    );
    let b = s.get("b");
    assert_eq!(b.background_color, Color::BLACK);
    assert_eq!(b.color, Color::WHITE);
    assert_eq!(b.margin.left, LengthPercentageAuto::px(0.0));
    assert_eq!(b.margin.right, LengthPercentageAuto::px(0.0));
    assert_eq!(b.margin.top, LengthPercentageAuto::px(3.0));
    assert_eq!(b.margin.bottom, LengthPercentageAuto::px(8.0));
    assert_eq!(s.get("a").color, RED);
    assert_eq!(s.get("n").color, Color::WHITE);
}

#[test]
fn lists_hr_br() {
    let s = style(
        "<ol id=o type=A><li id=l>x</li><li id=l2 type=i>y</li></ol><ul id=u type=square></ul>\
         <hr id=hr size=1 noshade width=50% align=left><hr id=hr2 size=10><br id=br clear=all>",
        "",
    );
    assert_eq!(s.get("o").list_style_type, ListStyleType::UpperAlpha);
    assert_eq!(
        s.get("l").list_style_type,
        ListStyleType::UpperAlpha,
        "inherited"
    );
    assert_eq!(s.get("l2").list_style_type, ListStyleType::LowerRoman);
    assert_eq!(s.get("u").list_style_type, ListStyleType::Square);
    let hr = s.get("hr");
    assert_eq!(hr.border.top.style, BorderStyle::Solid);
    assert_eq!(hr.border.top.width, 0.5);
    assert_eq!(hr.width, LengthPercentageAuto::percent(50.0));
    assert_eq!(hr.margin.left, LengthPercentageAuto::px(0.0));
    assert!(hr.margin.right.is_auto());
    assert_eq!(s.get("hr2").height, LengthPercentageAuto::px(8.0));
    assert_eq!(s.get("br").clear, Clear::Both);
}

#[test]
fn nested_tables_use_their_own_cellpadding() {
    let s = style(
        "<table cellpadding=7><tr><td id=outer><table><tr><td id=inner>x</td></tr></table></td></tr></table>",
        "",
    );
    assert_eq!(s.get("outer").padding.left, LengthPercentage::px(7.0));
    assert_eq!(s.get("inner").padding.left, LengthPercentage::px(1.0));
}
