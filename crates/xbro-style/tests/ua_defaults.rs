//! User-agent stylesheet defaults.

mod common;

use common::style;
use xbro_style::*;

#[test]
fn root_and_body() {
    let s = style("<body id=b><p id=p>x</p></body>", "");
    let html = s.tag("html");
    assert_eq!(html.display, Display::Block);
    assert_eq!(*html.font_family, vec!["serif".to_string()]);
    assert_eq!(html.color, Color::BLACK);
    assert_eq!(html.font_size, 16.0);
    let b = s.get("b");
    assert_eq!(b.margin, Sides::all(LengthPercentageAuto::px(8.0)));
    let p = s.get("p");
    assert_eq!(p.display, Display::Block);
    assert_eq!(p.margin.top, LengthPercentageAuto::px(16.0));
    assert_eq!(p.margin.left, LengthPercentageAuto::px(0.0));
}

#[test]
fn headings() {
    let s = style(
        "<h1 id=h1>a</h1><h2 id=h2>b</h2><h3 id=h3>c</h3><h4 id=h4>d</h4><h5 id=h5>e</h5><h6 id=h6>f</h6>\
         <section><h1 id=sh1>g</h1></section>",
        "",
    );
    let h1 = s.get("h1");
    assert_eq!(h1.font_size, 32.0);
    assert_eq!(h1.font_weight, 700);
    assert_px!(h1.margin.top.resolve(0.0).unwrap(), 0.67 * 32.0);
    assert_eq!(s.get("h2").font_size, 24.0);
    assert_px!(s.get("h3").font_size, 18.72);
    assert_eq!(s.get("h4").font_size, 16.0);
    assert_px!(s.get("h5").font_size, 13.28);
    assert_px!(s.get("h6").font_size, 10.72);
    assert_eq!(s.get("sh1").font_size, 24.0);
}

#[test]
fn links_and_phrasing() {
    let s = style(
        "<a id=a href=x>l</a><a id=noref>n</a><b id=b>b</b><strong id=st>s</strong><i id=i>i</i>\
         <em id=em>e</em><u id=u>u</u><s id=s>s</s><del id=del>d</del><small id=sm>s</small>\
         <sub id=sub>s</sub><sup id=sup>s</sup><mark id=mark>m</mark><code id=code>c</code>",
        "",
    );
    let a = s.get("a");
    assert_eq!(a.color, Color::rgb(0, 0, 0xee));
    assert!(a.text_decoration_line.underline);
    assert_eq!(a.cursor, Cursor::Pointer);
    assert_eq!(s.get("noref").color, Color::BLACK);
    assert!(!s.get("noref").text_decoration_line.underline);
    assert_eq!(s.get("b").font_weight, 700);
    assert_eq!(s.get("st").font_weight, 700);
    assert_eq!(s.get("i").font_style, FontStyle::Italic);
    assert_eq!(s.get("em").font_style, FontStyle::Italic);
    assert!(s.get("u").text_decoration_line.underline);
    assert!(s.get("s").text_decoration_line.line_through);
    assert!(s.get("del").text_decoration_line.line_through);
    assert_px!(s.get("sm").font_size, 16.0 / 1.2);
    assert_eq!(s.get("sub").vertical_align, VerticalAlign::Sub);
    assert_eq!(s.get("sup").vertical_align, VerticalAlign::Super);
    assert_eq!(s.get("mark").background_color, Color::rgb(255, 255, 0));
    assert_eq!(*s.get("code").font_family, vec!["monospace".to_string()]);
}

#[test]
fn lists() {
    let s = style(
        "<ul id=u><li id=li>x<ul id=u2><li>y<ul id=u3><li>z</li></ul></li></ul></li></ul><ol id=o></ol>",
        "",
    );
    let u = s.get("u");
    assert_eq!(u.padding.left, LengthPercentage::px(40.0));
    assert_eq!(u.margin.top, LengthPercentageAuto::px(16.0));
    assert_eq!(u.list_style_type, ListStyleType::Disc);
    assert_eq!(s.get("li").display, Display::ListItem);
    let u2 = s.get("u2");
    assert_eq!(u2.list_style_type, ListStyleType::Circle);
    assert_eq!(u2.margin.top, LengthPercentageAuto::px(0.0));
    assert_eq!(s.get("u3").list_style_type, ListStyleType::Square);
    assert_eq!(s.get("o").list_style_type, ListStyleType::Decimal);
}

#[test]
fn preformatted() {
    let s = style(
        "<pre id=p>x<code id=c>y</code></pre><code id=lone>z</code>",
        "",
    );
    let p = s.get("p");
    assert_eq!(*p.font_family, vec!["monospace".to_string()]);
    assert_eq!(p.font_size, 13.0);
    assert_eq!(p.white_space, WhiteSpace::Pre);
    assert_eq!(p.margin.top, LengthPercentageAuto::px(13.0));
    assert_eq!(
        s.get("c").font_size,
        13.0,
        "nested monospace does not shrink again"
    );
    assert_eq!(s.get("lone").font_size, 13.0);
}

#[test]
fn tables() {
    let s = style(
        "<table id=t><caption id=cap>c</caption><thead id=th><tr><th id=h>h</th></tr></thead>\
         <tbody id=tb><tr id=r><td id=d>x</td></tr></tbody><tfoot id=tf></tfoot></table>",
        "",
    );
    let t = s.get("t");
    assert_eq!(t.display, Display::Table);
    assert_eq!(t.border_spacing, (2.0, 2.0));
    assert_eq!(t.border_collapse, BorderCollapse::Separate);
    assert_eq!(t.box_sizing, BoxSizing::BorderBox);
    assert_eq!(s.get("cap").display, Display::TableCaption);
    assert_eq!(s.get("cap").text_align, TextAlign::Center);
    assert_eq!(s.get("th").display, Display::TableHeaderGroup);
    assert_eq!(s.get("tb").display, Display::TableRowGroup);
    assert_eq!(s.get("tf").display, Display::TableFooterGroup);
    assert_eq!(s.get("r").display, Display::TableRow);
    let d = s.get("d");
    assert_eq!(d.display, Display::TableCell);
    assert_eq!(d.padding, Sides::all(LengthPercentage::px(1.0)));
    assert_eq!(d.vertical_align, VerticalAlign::Middle);
    let h = s.get("h");
    assert_eq!(h.font_weight, 700);
    assert_eq!(h.text_align, TextAlign::Center);
}

#[test]
fn centered_tables() {
    let s = style(
        "<center><table id=t><tr><td>x</td></tr></table></center>         <div align=center><table id=t2><tr><td>y</td></tr></table></div>         <div><table id=t3><tr><td>z</td></tr></table></div>",
        "",
    );
    assert!(s.get("t").margin.left.is_auto() && s.get("t").margin.right.is_auto());
    assert!(s.get("t2").margin.left.is_auto());
    assert_eq!(s.get("t3").margin.left, LengthPercentageAuto::px(0.0));
}

#[test]
fn form_controls() {
    let s = style(
        "<input id=i><button id=b>b</button><textarea id=t></textarea><select id=s><option id=o>x</option></select>\
         <input id=cb type=checkbox><input id=h type=HIDDEN><input id=sub type=submit>",
        "",
    );
    let i = s.get("i");
    assert_eq!(i.display, Display::InlineBlock);
    assert_px!(i.font_size, 13.333);
    assert_eq!(i.border.top.width, 1.0);
    assert_eq!(i.background_color, Color::WHITE);
    assert_eq!(i.padding.left, LengthPercentage::px(2.0));
    let b = s.get("b");
    assert_eq!(b.display, Display::InlineBlock);
    assert_eq!(b.padding.left, LengthPercentage::px(6.0));
    assert_eq!(b.text_align, TextAlign::Center);
    assert_eq!(b.background_color, Color::rgb(0xef, 0xef, 0xef));
    let t = s.get("t");
    assert_eq!(*t.font_family, vec!["monospace".to_string()]);
    assert_eq!(t.white_space, WhiteSpace::PreWrap);
    assert!(
        s.try_get("o").is_none(),
        "options are rendered by the select control"
    );
    assert_eq!(s.get("cb").border.top.width, 0.0);
    assert_eq!(s.get("cb").margin.left, LengthPercentageAuto::px(4.0));
    assert!(s.try_get("h").is_none());
    assert_eq!(s.get("sub").padding.left, LengthPercentage::px(6.0));
}

#[test]
fn misc_elements() {
    let s = style(
        "<center id=c>x</center><blockquote id=bq>q</blockquote><hr id=hr><dl><dd id=dd>d</dd></dl>\
         <details id=det><summary id=sum>s</summary><p id=hidden>h</p></details>\
         <details open><summary>s</summary><p id=shown>h</p></details>\
         <noscript id=ns>visible</noscript><dialog id=dlg>d</dialog><dialog id=dlgo open>o</dialog>\
         <iframe id=if></iframe><img id=img><fieldset id=fs></fieldset>",
        "",
    );
    assert_eq!(s.get("c").display, Display::Block);
    assert_eq!(s.get("c").text_align, TextAlign::Center);
    assert_eq!(s.get("bq").margin.left, LengthPercentageAuto::px(40.0));
    let hr = s.get("hr");
    assert_eq!(hr.color, Color::rgb(128, 128, 128));
    assert_eq!(hr.border.top.style, BorderStyle::Inset);
    assert_eq!(hr.border.top.width, 1.0);
    assert_eq!(hr.border.top.color, Color::rgb(128, 128, 128));
    assert!(hr.margin.left.is_auto());
    assert_eq!(s.get("dd").margin.left, LengthPercentageAuto::px(40.0));
    assert!(s.try_get("sum").is_some());
    assert!(s.try_get("hidden").is_none());
    assert!(s.try_get("shown").is_some());
    assert!(
        s.try_get("ns").is_some(),
        "noscript is rendered (no scripting)"
    );
    assert!(s.try_get("dlg").is_none());
    assert_eq!(s.get("dlgo").display, Display::Block);
    assert_eq!(s.get("dlgo").position, Position::Absolute);
    assert_eq!(s.get("if").border.top.width, 2.0);
    assert_eq!(s.get("img").display, Display::Inline);
    assert_eq!(s.get("fs").border.top.style, BorderStyle::Groove);
}
