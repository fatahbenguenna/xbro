//! Box generation: display, blockification, display:none subtrees, text
//! nodes, decorations and generated content.

mod common;

use std::sync::Arc;

use common::style;
use xbro_dom::NodeData;
use xbro_style::*;

const RED: Color = Color::rgb(255, 0, 0);
const BLUE: Color = Color::rgb(0, 0, 255);

#[test]
fn blockification() {
    let s = style(
        "<div id=flex><span id=item>a</span><span id=iflex>b</span><span id=abs>c</span></div>\
         <span id=float>d</span><span id=fixed>e</span><span id=itable>f</span>\
         <div id=grid><a id=gitem>g</a></div><td id=cell>h</td>",
        "#flex { display: flex } #iflex { display: inline-flex } #abs { position: absolute; float: left }
         #float { float: left } #fixed { position: fixed } #itable { display: inline-table; float: right }
         #grid { display: grid } #gitem { float: right }",
    );
    assert_eq!(
        s.get("item").display,
        Display::Block,
        "flex items are blockified"
    );
    assert_eq!(s.get("iflex").display, Display::Flex);
    let abs = s.get("abs");
    assert_eq!(abs.display, Display::Block);
    assert_eq!(abs.float, Float::None, "float computes to none for abspos");
    assert_eq!(s.get("float").display, Display::Block);
    assert_eq!(s.get("float").float, Float::Left);
    assert_eq!(s.get("fixed").display, Display::Block);
    assert_eq!(s.get("itable").display, Display::Table);
    assert_eq!(s.get("gitem").display, Display::Block);
    assert_eq!(
        s.get("gitem").float,
        Float::None,
        "float does not apply to grid items"
    );
    // Root element is blockified too.
    let s = style("<p>x</p>", "html { display: inline }");
    assert_eq!(s.tag("html").display, Display::Block);
}

#[test]
fn contents_is_transparent_for_flex_items() {
    let s = style(
        "<div id=f><div id=c><span id=s>x</span></div></div>",
        "#f { display: flex } #c { display: contents }",
    );
    assert_eq!(s.get("c").display, Display::Contents);
    assert_eq!(s.get("s").display, Display::Block);
}

#[test]
fn display_none_subtrees() {
    let s = style(
        "<div id=hidden><p id=inner>text</p></div><div id=shown>t</div><script id=sc>x</script>",
        "#hidden { display: none } #inner { display: block }",
    );
    assert!(s.try_get("hidden").is_none());
    assert!(s.try_get("inner").is_none());
    assert!(s.try_get("sc").is_none());
    assert!(s.try_get("shown").is_some());
    // Text inside the hidden subtree has no style.
    let inner = s.node("inner");
    let text = s.doc.children(inner)[0];
    assert!(s.map.get(text).is_none());
    // head and its children are not rendered.
    let head = s.doc.head().unwrap();
    assert!(s.map.get(head).is_none());
    // Document node and doctype have no style.
    assert!(s.map.get(s.doc.root()).is_none());
}

#[test]
fn text_nodes_share_parent_style() {
    let s = style(
        "<p id=p>hello <b id=b>bold</b> world</p>",
        "p { color: red }",
    );
    let p = s.node("p");
    let p_style = s.map.get(p).unwrap();
    let mut texts = 0;
    for &c in s.doc.children(p) {
        if matches!(s.doc.node(c).data, NodeData::Text(_)) {
            assert!(Arc::ptr_eq(s.map.get(c).unwrap(), p_style));
            texts += 1;
        }
    }
    assert_eq!(texts, 2);
    let b = s.node("b");
    let b_text = s.doc.children(b)[0];
    assert!(Arc::ptr_eq(
        s.map.get(b_text).unwrap(),
        s.map.get(b).unwrap()
    ));
    assert_eq!(s.map.styles.len(), s.doc.len());
}

#[test]
fn decorations_propagate() {
    let s = style(
        "<div id=d><span id=s>a<em id=e>b</em></span><span id=ib>c<i id=ii>d</i></span>\
         <span id=f>e</span><span id=abs>f</span><span id=own>g</span></div>",
        "#d { text-decoration: underline red; color: blue }
         #ib { display: inline-block } #f { float: left } #abs { position: absolute }
         #own { text-decoration: line-through; color: green }",
    );
    let line = |u: bool, t: bool| TextDecorationLine {
        underline: u,
        overline: false,
        line_through: t,
    };
    assert_eq!(s.get("d").decorations_in_effect, line(true, false));
    assert_eq!(s.get("d").decoration_color, RED);
    let e = s.get("e");
    assert_eq!(e.decorations_in_effect, line(true, false));
    assert_eq!(e.decoration_color, RED);
    assert_eq!(
        e.text_decoration_line,
        TextDecorationLine::NONE,
        "own line not inherited"
    );
    assert_eq!(s.get("ib").decorations_in_effect, TextDecorationLine::NONE);
    assert_eq!(s.get("ii").decorations_in_effect, TextDecorationLine::NONE);
    assert_eq!(s.get("f").decorations_in_effect, TextDecorationLine::NONE);
    assert_eq!(s.get("abs").decorations_in_effect, TextDecorationLine::NONE);
    let own = s.get("own");
    assert_eq!(own.decorations_in_effect, line(true, true));
    assert_eq!(own.decoration_color, Color::rgb(0, 128, 0));
    // Links: UA underline with the link color.
    let s = style("<a id=a href=x>l</a>", "");
    assert_eq!(s.get("a").decorations_in_effect, line(true, false));
    assert_eq!(s.get("a").decoration_color, Color::rgb(0, 0, 0xee));
}

/// CSS Text Decoration 3: a decoration is drawn in the color of the box
/// that declares it, not in the color of the descendants it propagates to
/// (tests/fixtures/text.html, `.nested-deco`).
#[test]
fn decoration_color_comes_from_the_decorating_box() {
    let s = style(
        "<p id=p>souligné <em id=em>dans les enfants</em> et \
         <span id=ib style=\"display:inline-block\">pas ici</span>\
         <span id=c style=\"display: contents; color: green\"><b id=cb>x</b></span>\
         <i id=tdc>y</i><a id=link href=x>z</a></p>\
         <div id=d><p id=inner>w <u id=u>v</u></p></div>",
        ".nested { } p#p { text-decoration: underline }
         #em { color: #1971c2 }
         #ib { color: red }
         #tdc { color: red; text-decoration-color: blue }
         #d { color: purple; text-decoration: line-through }
         #inner { color: orange }
         #u { color: teal }",
    );
    let line = TextDecorationLine {
        underline: true,
        ..TextDecorationLine::NONE
    };
    assert_eq!(s.get("p").decoration_color, Color::BLACK);
    let em = s.get("em");
    assert_eq!(em.color, Color::rgb(0x19, 0x71, 0xc2));
    assert_eq!(em.decorations_in_effect, line);
    assert_eq!(
        em.decoration_color,
        Color::BLACK,
        "the <p>'s color, not the <em>'s"
    );
    // Atomic inlines do not get the decoration at all.
    assert_eq!(s.get("ib").decorations_in_effect, TextDecorationLine::NONE);
    // `display: contents` and its children: still the <p>'s color.
    assert_eq!(s.get("c").decoration_color, Color::BLACK);
    assert_eq!(s.get("cb").decoration_color, Color::BLACK);
    // `text-decoration-color` only colors the element's own decorations.
    assert_eq!(s.get("tdc").decoration_color, Color::BLACK);
    // A link adds its own underline: one color (the link's) for all lines.
    let link = s.get("link");
    assert_eq!(link.decorations_in_effect, line);
    assert_eq!(link.decoration_color, Color::rgb(0, 0, 0xee));
    // Propagation into block descendants keeps the ancestor's color.
    let inner = s.get("inner");
    assert_eq!(inner.color, Color::rgb(255, 165, 0));
    assert_eq!(inner.decoration_color, Color::rgb(128, 0, 128));
    // A box adding a line uses its own color for all its lines (approximation).
    let u = s.get("u");
    assert_eq!(
        u.decorations_in_effect,
        TextDecorationLine {
            underline: true,
            line_through: true,
            ..TextDecorationLine::NONE
        }
    );
    assert_eq!(u.decoration_color, Color::rgb(0, 128, 128));
    // Text runs share their parent's style, hence its decoration color.
    let em_node = s.node("em");
    let text = s.doc.children(em_node)[0];
    assert_eq!(s.map.get(text).unwrap().decoration_color, Color::BLACK);
    // Anonymous boxes / text runs built by layout inherit it too.
    assert_eq!(
        ComputedStyle::inherit_from(&em).decoration_color,
        Color::BLACK
    );
}

#[test]
fn before_and_after() {
    let s = style(
        "<p id=p data-x=hi>x</p><p id=none>y</p><p id=hidden>z</p><div id=f>w</div><img id=img>",
        "#p { color: blue }
         #p::before { content: '[' attr(data-x) ']'; color: red }
         #p::after { content: url(a.png) }
         #none::before { content: none } #none::after { color: red }
         #hidden::before { content: 'x'; display: none }
         #f { display: flex } #f::before { content: ''; display: inline }
         img::before { content: 'no' }",
    );
    let before = s.before("p").expect("::before generated");
    assert_eq!(before.color, RED);
    assert_eq!(
        before.content,
        Content::Items(vec![
            ContentItem::Text("[".into()),
            ContentItem::Text("hi".into()),
            ContentItem::Text("]".into())
        ])
    );
    let after = s.after("p").expect("::after generated");
    assert_eq!(after.color, BLUE, "inherits from the element");
    assert_eq!(
        after.content,
        Content::Items(vec![ContentItem::Image("a.png".into())])
    );
    assert!(s.before("none").is_none());
    assert!(s.after("none").is_none(), "no content → no box");
    assert!(s.before("hidden").is_none());
    assert_eq!(
        s.before("f").unwrap().display,
        Display::Block,
        "flex item pseudo is blockified"
    );
    assert!(s.before("img").is_none());
}

#[test]
fn pseudo_elements_without_content_are_skipped() {
    let s = style(
        "<p id=p>x</p><p id=q class=c>y</p><div id=d>z</div>",
        "*, *::before, *::after { box-sizing: border-box }
         .c::after { content: inherit } #q { content: 'own' }
         #d::before { content: var(--missing, 'fb') }",
    );
    assert!(s.before("p").is_none());
    assert!(s.after("p").is_none());
    assert_eq!(
        s.after("q").unwrap().content,
        Content::Items(vec![ContentItem::Text("own".into())]),
        "content: inherit takes the element's value"
    );
    assert_eq!(
        s.before("d").unwrap().content,
        Content::Items(vec![ContentItem::Text("fb".into())])
    );
}

#[test]
fn quotes_from_ua_sheet() {
    let s = style("<q id=q>x</q>", "");
    assert_eq!(
        s.before("q").unwrap().content,
        Content::Items(vec![ContentItem::OpenQuote])
    );
    assert_eq!(
        s.after("q").unwrap().content,
        Content::Items(vec![ContentItem::CloseQuote])
    );
}

#[test]
fn pseudo_elements_with_custom_properties() {
    let s = style(
        "<p id=p>x</p>",
        "p { --c: green } p::before { content: 'x'; color: var(--c) }",
    );
    assert_eq!(s.before("p").unwrap().color, Color::rgb(0, 128, 0));
}

#[test]
fn deep_trees_do_not_overflow() {
    // Built directly in the DOM to exceed the parser's depth limit.
    let mut doc = xbro_dom::Document::new();
    let html = doc.create_element("html", vec![]);
    doc.append_child(doc.root(), html);
    let mut parent = html;
    for _ in 0..20_000 {
        let div = doc.create_element("div", vec![]);
        doc.append_child(parent, div);
        parent = div;
    }
    doc.append_text(parent, "deep");
    let sheet = xbro_css::parse_stylesheet("div div div { color: red } div > div { margin: 1px }");
    let map = compute_styles(&doc, &[sheet], &StyleContext::default());
    assert!(map.get(parent).is_some());
}
