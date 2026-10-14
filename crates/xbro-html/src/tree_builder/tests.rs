//! Tree builder tests (html5lib-style tree assertions).

use std::fmt::Write as _;

use xbro_dom::{Document, NodeData, NodeId};

use crate::parse_document;

/// Compact serialisation of the children of `node`: elements as
/// `<name a="v">...</name>` (void elements included), text verbatim,
/// comments as `<!--x-->`.
fn serialize_children(doc: &Document, node: NodeId, out: &mut String) {
    for &child in doc.children(node) {
        match &doc.node(child).data {
            NodeData::Element(e) => {
                let _ = write!(out, "<{}", e.name);
                for a in &e.attrs {
                    let _ = write!(out, " {}=\"{}\"", a.name, a.value);
                }
                out.push('>');
                serialize_children(doc, child, out);
                let _ = write!(out, "</{}>", e.name);
            }
            NodeData::Text(t) => out.push_str(t),
            NodeData::Comment(c) => {
                let _ = write!(out, "<!--{c}-->");
            }
            NodeData::Doctype { name } => {
                let _ = write!(out, "<!DOCTYPE {name}>");
            }
            NodeData::Document => {}
        }
    }
}

fn ser(doc: &Document, node: NodeId) -> String {
    let mut out = String::new();
    serialize_children(doc, node, &mut out);
    out
}

/// Serialisation of the body's children.
fn body(input: &str) -> String {
    let doc = parse_document(input);
    let body = doc.body().expect("body");
    ser(&doc, body)
}

/// Serialisation of the head's children.
fn head(input: &str) -> String {
    let doc = parse_document(input);
    let head = doc.head().expect("head");
    ser(&doc, head)
}

/// Serialisation of the whole document.
fn document(input: &str) -> String {
    let doc = parse_document(input);
    ser(&doc, doc.root())
}

/// Checks the structural invariants every parse must satisfy.
fn assert_well_formed(doc: &Document) {
    let html = doc.document_element().expect("html element");
    assert_eq!(doc.tag_name(html), Some("html"));
    assert!(doc.head().is_some(), "head missing");
    assert!(doc.body().is_some(), "body missing");
    assert_eq!(doc.parent(doc.head().unwrap()), Some(html));
    // Parent/child links are consistent and the document is a tree.
    let mut seen = vec![false; doc.len()];
    for n in doc.descendants(doc.root()) {
        assert!(!seen[n.index()], "node reachable twice");
        seen[n.index()] = true;
        for &c in doc.children(n) {
            assert_eq!(doc.parent(c), Some(n));
        }
    }
    assert!(max_element_depth(doc) <= crate::MAX_DEPTH);
}

fn max_element_depth(doc: &Document) -> usize {
    let mut max = 0;
    let mut stack = vec![(doc.root(), 0usize)];
    while let Some((n, d)) = stack.pop() {
        if doc.node(n).is_element() {
            max = max.max(d);
        }
        for &c in doc.children(n) {
            stack.push((c, d + 1));
        }
    }
    max
}

// ---------------------------------------------------------------------------
// Document structure, implied elements, comments.

#[test]
fn empty_document_has_html_head_body() {
    for input in [
        "",
        " ",
        "\n\t",
        "<!---->",
        "<!DOCTYPE html>",
        "</p>",
        "</html>",
    ] {
        let doc = parse_document(input);
        assert_well_formed(&doc);
    }
    assert_eq!(document(""), "<html><head></head><body></body></html>");
}

#[test]
fn implied_elements() {
    assert_eq!(
        document("<!DOCTYPE html><title>T</title><p>x"),
        "<!DOCTYPE html><html><head><title>T</title></head><body><p>x</p></body></html>"
    );
    assert_eq!(
        document("hello"),
        "<html><head></head><body>hello</body></html>"
    );
    assert_eq!(
        document("<html><head></head><body></body></html>"),
        "<html><head></head><body></body></html>"
    );
}

#[test]
fn whitespace_before_html_and_head_is_ignored() {
    assert_eq!(
        document("  \n<html>  <head>  </head>  <body>x</body></html>"),
        "<html><head>  </head>  <body>x</body></html>"
    );
    assert_eq!(
        document(" \n x"),
        "<html><head></head><body>x</body></html>"
    );
}

#[test]
fn comments_placement() {
    assert_eq!(
        document("<!--a--><!DOCTYPE html><!--b--><html><!--c--><head><!--d--></head><body><!--e--></body></html><!--f-->"),
        "<!--a--><!DOCTYPE html><!--b--><html><!--c--><head><!--d--></head><body><!--e--></body></html><!--f-->"
    );
    // A comment after </body> goes into <html>.
    assert_eq!(
        document("<body></body><!--x-->"),
        "<html><head></head><body></body><!--x--></html>"
    );
}

#[test]
fn content_after_body_and_html_goes_into_body() {
    assert_eq!(body("<p>a</body><p>b"), "<p>a</p><p>b</p>");
    assert_eq!(body("<p>a</body></html><p>b"), "<p>a</p><p>b</p>");
    assert_eq!(body("a</html>  b"), "a  b");
    assert_eq!(
        document("a</body>\n</html>\n"),
        "<html><head></head><body>a\n\n</body></html>"
    );
}

#[test]
fn stray_html_and_body_attributes_are_merged() {
    let doc = parse_document("<html a=1><body b=2><html a=3 c=4><body b=5 d=6>");
    let html = doc.document_element().unwrap();
    assert_eq!(doc.attr(html, "a"), Some("1"));
    assert_eq!(doc.attr(html, "c"), Some("4"));
    let body = doc.body().unwrap();
    assert_eq!(doc.attr(body, "b"), Some("2"));
    assert_eq!(doc.attr(body, "d"), Some("6"));
}

#[test]
fn head_elements() {
    assert_eq!(
        head("<base href=x><link rel=a><meta charset=utf-8><style>p{}</style><script>1<2</script><title>a&amp;b</title>"),
        "<base href=\"x\"></base><link rel=\"a\"></link><meta charset=\"utf-8\"></meta><style>p{}</style><script>1<2</script><title>a&b</title>"
    );
    // Head-only elements after </head> are moved into the head.
    assert_eq!(
        document("<head></head><style>x</style><p>"),
        "<html><head><style>x</style></head><body><p></p></body></html>"
    );
    // Elements after the head content start the body.
    assert_eq!(
        document("<title>t</title><div>d</div>"),
        "<html><head><title>t</title></head><body><div>d</div></body></html>"
    );
}

#[test]
fn noscript_in_head_and_body() {
    // Scripting disabled: in head, noscript may contain link/meta/style.
    assert_eq!(
        head("<head><noscript><link rel=s><style>a</style></noscript></head>"),
        "<noscript><link rel=\"s\"></link><style>a</style></noscript>"
    );
    // Anything else closes the noscript and the head.
    assert_eq!(
        document("<head><noscript><p>x</p></noscript>"),
        "<html><head><noscript></noscript></head><body><p>x</p></body></html>"
    );
    // In body, noscript is ordinary markup.
    assert_eq!(
        body("<body><noscript><p>x</p></noscript>"),
        "<noscript><p>x</p></noscript>"
    );
    assert_eq!(
        body("x<noscript><b>y</b></noscript>"),
        "x<noscript><b>y</b></noscript>"
    );
}

// ---------------------------------------------------------------------------
// Implied end tags, paragraphs, lists, headings.

#[test]
fn paragraphs_closed_by_block_elements() {
    for tag in [
        "address",
        "article",
        "aside",
        "blockquote",
        "center",
        "details",
        "dialog",
        "dir",
        "div",
        "dl",
        "fieldset",
        "figcaption",
        "figure",
        "footer",
        "form",
        "h1",
        "h6",
        "header",
        "hgroup",
        "main",
        "menu",
        "nav",
        "ol",
        "p",
        "pre",
        "section",
        "summary",
        "ul",
        "listing",
    ] {
        assert_eq!(
            body(&format!("<!DOCTYPE html><p>a<{tag}>b")),
            format!("<p>a</p><{tag}>b</{tag}>"),
            "tag {tag}"
        );
    }
    assert_eq!(body("<p>a<hr>b"), "<p>a</p><hr></hr>b");
    assert_eq!(body("<p>a<xmp>b</xmp>"), "<p>a</p><xmp>b</xmp>");
    assert_eq!(
        body("<p>a<plaintext>b</p>"),
        "<p>a</p><plaintext>b</p></plaintext>"
    );
    assert_eq!(body("<p>a<li>b"), "<p>a</p><li>b</li>");
    assert_eq!(body("<p>a<dd>b<dt>c"), "<p>a</p><dd>b</dd><dt>c</dt>");
    // Inline content stays inside.
    assert_eq!(body("<p>a<span>b</span>c"), "<p>a<span>b</span>c</p>");
}

#[test]
fn table_closes_p_only_in_no_quirks_mode() {
    assert_eq!(
        body("<!DOCTYPE html><p>a<table></table>"),
        "<p>a</p><table></table>"
    );
    assert_eq!(body("<p>a<table></table>"), "<p>a<table></table></p>");
}

#[test]
fn end_p_without_open_p_inserts_empty_p() {
    assert_eq!(body("<body></p>"), "<p></p>");
    assert_eq!(body("<div></p></div>"), "<div><p></p></div>");
    assert_eq!(body("a</p>b"), "a<p></p>b");
}

#[test]
fn list_items_auto_close() {
    assert_eq!(
        body("<ul><li>a<li>b<li>c</ul>"),
        "<ul><li>a</li><li>b</li><li>c</li></ul>"
    );
    assert_eq!(
        body("<ul><li>a<ul><li>b</ul><li>c</ul>"),
        "<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>"
    );
    // A div between does not stop the li loop; other special elements do.
    assert_eq!(body("<li>a<div><li>b"), "<li>a<div></div></li><li>b</li>");
    assert_eq!(
        body("<li>a<section><li>b"),
        "<li>a<section><li>b</li></section></li>"
    );
    assert_eq!(
        body("<dl><dt>a<dd>b<dt>c<dd>d</dl>"),
        "<dl><dt>a</dt><dd>b</dd><dt>c</dt><dd>d</dd></dl>"
    );
    assert_eq!(body("<li>a</li></li>b"), "<li>a</li>b");
}

#[test]
fn headings_close_previous_heading() {
    assert_eq!(body("<h1>a<h2>b"), "<h1>a</h1><h2>b</h2>");
    assert_eq!(body("<h1>a</h2>b"), "<h1>a</h1>b");
    assert_eq!(
        body("<h3><span>a<h4>b"),
        "<h3><span>a<h4>b</h4></span></h3>"
    );
}

#[test]
fn void_elements() {
    let voids = [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr", "keygen", "basefont", "bgsound",
    ];
    for tag in voids {
        let html = format!("<div><{tag}>x</div>");
        let out = body(&html);
        assert!(
            out.contains(&format!("<{tag}></{tag}>")) || tag == "col",
            "void {tag}: {out}"
        );
        assert!(
            !out.contains(&format!("<{tag}>x")),
            "void {tag} got children: {out}"
        );
    }
    assert_eq!(body("<br/>a"), "<br></br>a");
}

#[test]
fn end_br_and_image() {
    assert_eq!(body("a</br>b"), "a<br></br>b");
    assert_eq!(body("<image src=x>"), "<img src=\"x\"></img>");
}

#[test]
fn nested_forms_are_ignored() {
    assert_eq!(
        body("<form a=1><form a=2><input></form>x"),
        "<form a=\"1\"><input></input></form>x"
    );
    // After </form>, a new form may start.
    assert_eq!(body("<form></form><form>"), "<form></form><form></form>");
}

#[test]
fn buttons_do_not_nest() {
    assert_eq!(
        body("<button>a<button>b"),
        "<button>a</button><button>b</button>"
    );
}

#[test]
fn leading_newline_dropped_in_pre_listing_textarea() {
    assert_eq!(body("<pre>\nx</pre>"), "<pre>x</pre>");
    assert_eq!(body("<pre>\n\nx</pre>"), "<pre>\nx</pre>");
    assert_eq!(body("<listing>\nx</listing>"), "<listing>x</listing>");
    assert_eq!(body("<textarea>\nx</textarea>"), "<textarea>x</textarea>");
    assert_eq!(body("<pre>x\n</pre>"), "<pre>x\n</pre>");
    assert_eq!(body("<pre>\r\nx</pre>"), "<pre>x</pre>");
    // Only directly after the start tag.
    assert_eq!(body("<pre><b>\nx</b></pre>"), "<pre><b>\nx</b></pre>");
}

// ---------------------------------------------------------------------------
// Raw text elements.

#[test]
fn raw_text_elements() {
    // Before any body content, scripts go to the head.
    assert_eq!(
        head("<script>if (a<b && c>d) {}</script>x"),
        "<script>if (a<b && c>d) {}</script>"
    );
    assert_eq!(
        body("<body><script>if (a<b && c>d) {}</script>x"),
        "<script>if (a<b && c>d) {}</script>x"
    );
    assert_eq!(body("<body><script>a</script >b"), "<script>a</script>b");
    assert_eq!(body("<body><script>a</SCRIPT>b"), "<script>a</script>b");
    assert_eq!(
        body("<body><style><p>&amp;</style>"),
        "<style><p>&amp;</style>"
    );
    assert_eq!(body("<xmp><b>x</b></xmp>"), "<xmp><b>x</b></xmp>");
    assert_eq!(body("<iframe><p>x</iframe>"), "<iframe><p>x</iframe>");
    assert_eq!(body("<noembed><p>x</noembed>"), "<noembed><p>x</noembed>");
    assert_eq!(
        body("<body><noframes><p>x</noframes>"),
        "<noframes><p>x</noframes>"
    );
    assert_eq!(
        body("<textarea><b>&amp;</b></textarea>"),
        "<textarea><b>&</b></textarea>"
    );
    assert_eq!(
        head("<title><b>&lt;</b></title>"),
        "<title><b><</b></title>"
    );
    assert_eq!(
        body("<plaintext><b></plaintext>&amp;"),
        "<plaintext><b></plaintext>&amp;</plaintext>"
    );
}

#[test]
fn unterminated_raw_text() {
    assert_eq!(body("<body><script>alert(1)"), "<script>alert(1)</script>");
    assert_eq!(head("<script>alert(1)"), "<script>alert(1)</script>");
    assert_eq!(body("<textarea>abc"), "<textarea>abc</textarea>");
    assert_eq!(head("<title>abc"), "<title>abc</title>");
    assert_eq!(head("<style>"), "<style></style>");
}

#[test]
fn script_escapes_in_tree() {
    assert_eq!(
        body("<body><script><!--<script>x='</script>'-->y</script>z"),
        "<script><!--<script>x='</script>'-->y</script>z"
    );
}

// ---------------------------------------------------------------------------
// Formatting elements and the adoption agency algorithm.

#[test]
fn misnested_formatting() {
    assert_eq!(body("<b><i></b></i>"), "<b><i></i></b>");
    assert_eq!(body("<b><i>a</b>b</i>"), "<b><i>a</i></b><i>b</i>");
    assert_eq!(body("<b>1<p>2</b>3</p>"), "<b>1</b><p><b>2</b>3</p>");
    assert_eq!(body("<b><p></b>x</p>"), "<b></b><p><b></b>x</p>");
    assert_eq!(
        body("<a href=x><div>y</a>z"),
        "<a href=\"x\"></a><div><a href=\"x\">y</a>z</div>"
    );
}

#[test]
fn nested_anchor_closes_first() {
    assert_eq!(body("<a>1<a>2</a>3"), "<a>1</a><a>2</a>3");
    assert_eq!(
        body("<a href=1><b>x<a href=2>y"),
        "<a href=\"1\"><b>x</b></a><b><a href=\"2\">y</a></b>"
    );
}

#[test]
fn formatting_reconstructed_after_blocks() {
    assert_eq!(body("<b>a<p>b</p>c"), "<b>a<p>b</p>c</b>");
    assert_eq!(body("<p><b>a</p>b"), "<p><b>a</b></p><b>b</b>");
    assert_eq!(
        body("<p><b><i>a</p>b"),
        "<p><b><i>a</i></b></p><b><i>b</i></b>"
    );
    assert_eq!(
        body("<font color=red><p>a</p></font>b"),
        "<font color=\"red\"><p>a</p></font>b"
    );
}

#[test]
fn noahs_ark_limits_identical_formatting() {
    let out = body("<p><b><b><b><b><b>x</p>y");
    // Only three identical <b> elements are reconstructed.
    assert!(out.ends_with("<b><b><b>y</b></b></b>"), "{out}");
}

#[test]
fn nobr_nesting() {
    assert_eq!(body("<nobr>a<nobr>b"), "<nobr>a</nobr><nobr>b</nobr>");
}

#[test]
fn end_tag_for_unopened_formatting_is_ignored() {
    assert_eq!(body("a</b>b</i>c"), "abc");
    assert_eq!(body("<div>a</em>b</div>"), "<div>ab</div>");
}

#[test]
fn any_other_end_tag() {
    assert_eq!(
        body("<span>a<em>b</span>c"),
        "<span>a<em>b</em></span><em>c</em>"
    );
    assert_eq!(body("<x-a>1<x-b>2</x-a>3"), "<x-a>1<x-b>2</x-b></x-a>3");
    // A special element stops the walk: the end tag is ignored.
    assert_eq!(
        body("<span>a<div>b</span>c</div>"),
        "<span>a<div>bc</div></span>"
    );
}

#[test]
fn objects_insert_markers() {
    assert_eq!(
        body("<b><object><i>x</object>y"),
        "<b><object><i>x</i></object>y</b>"
    );
}

// ---------------------------------------------------------------------------
// Select.

#[test]
fn select_content_model() {
    assert_eq!(
        body("<select><option>a<option>b<optgroup label=g><option>c<optgroup><option>d</select>"),
        "<select><option>a</option><option>b</option><optgroup label=\"g\"><option>c</option></optgroup><optgroup><option>d</option></optgroup></select>"
    );
    // Nested select closes the first one.
    assert_eq!(
        body("<select><option>a<select>b"),
        "<select><option>a</option></select>b"
    );
    // Most tags are ignored inside select.
    assert_eq!(
        body("<select><div>a</div><b>b</b></select>"),
        "<select>ab</select>"
    );
    // input closes the select.
    assert_eq!(body("<select><input>"), "<select></select><input></input>");
    assert_eq!(
        body("<select><option>a</option><hr><option>b"),
        "<select><option>a</option><hr></hr><option>b</option></select>"
    );
}

#[test]
fn select_in_table() {
    assert_eq!(
        body("<table><tr><td><select><option>a<td>b</table>"),
        "<table><tbody><tr><td><select><option>a</option></select></td><td>b</td></tr></tbody></table>"
    );
}

// ---------------------------------------------------------------------------
// Tables.

#[test]
fn implied_table_structure() {
    assert_eq!(
        body("<table><tr><td>a</table>"),
        "<table><tbody><tr><td>a</td></tr></tbody></table>"
    );
    assert_eq!(
        body("<table><td>a</td></table>"),
        "<table><tbody><tr><td>a</td></tr></tbody></table>"
    );
    assert_eq!(
        body("<table><col><col></table>"),
        "<table><colgroup><col></col><col></col></colgroup></table>"
    );
    assert_eq!(
        body("<table><caption>c</caption><thead><tr><th>h<tbody><tr><td>d<tfoot><tr><td>f</table>"),
        "<table><caption>c</caption><thead><tr><th>h</th></tr></thead><tbody><tr><td>d</td></tr></tbody><tfoot><tr><td>f</td></tr></tfoot></table>"
    );
}

#[test]
fn cells_and_rows_close_previous() {
    assert_eq!(
        body("<table><tr><td>a<td>b<th>c<tr><td>d</table>"),
        "<table><tbody><tr><td>a</td><td>b</td><th>c</th></tr><tr><td>d</td></tr></tbody></table>"
    );
    assert_eq!(
        body("<table><tr><td><p>a<td>b</table>"),
        "<table><tbody><tr><td><p>a</p></td><td>b</td></tr></tbody></table>"
    );
}

#[test]
fn end_table_closes_everything() {
    assert_eq!(
        body("<table><tr><td><div><b>a</table>b"),
        "<table><tbody><tr><td><div><b>a</b></div></td></tr></tbody></table>b"
    );
    // A nested <table> start tag closes the open table.
    assert_eq!(
        body("<table><tr><td>a</td></tr><table><tr><td>b</table>"),
        "<table><tbody><tr><td>a</td></tr></tbody></table><table><tbody><tr><td>b</td></tr></tbody></table>"
    );
}

#[test]
fn tables_nest_through_cells() {
    assert_eq!(
        body("<table><tr><td><table><tr><td>x</table></table>"),
        "<table><tbody><tr><td><table><tbody><tr><td>x</td></tr></tbody></table></td></tr></tbody></table>"
    );
}

#[test]
fn foster_parenting() {
    assert_eq!(body("<table>x</table>"), "x<table></table>");
    assert_eq!(body("<table> </table>"), "<table> </table>");
    assert_eq!(
        body("<table><tr>a<td>b</table>"),
        "a<table><tbody><tr><td>b</td></tr></tbody></table>"
    );
    assert_eq!(
        body("<table><div>a</div><tr><td>b</table>"),
        "<div>a</div><table><tbody><tr><td>b</td></tr></tbody></table>"
    );
    assert_eq!(
        body("<div><table><p>x</table></div>"),
        "<div><p>x</p><table></table></div>"
    );
    assert_eq!(
        body("<table><b>x</b><tr><td>y</table>"),
        "<b>x</b><table><tbody><tr><td>y</td></tr></tbody></table>"
    );
    // Hidden inputs and forms stay inside the table.
    assert_eq!(
        body("<table><input type=hidden><input type=text></table>"),
        "<input type=\"text\"></input><table><input type=\"hidden\"></input></table>"
    );
    assert_eq!(
        body("<table><form><tr><td>a</table>"),
        "<table><form></form><tbody><tr><td>a</td></tr></tbody></table>"
    );
    // Scripts and styles stay in the table.
    assert_eq!(
        body("<table><style>s</style></table>"),
        "<table><style>s</style></table>"
    );
}

#[test]
fn foster_parented_text_merges() {
    assert_eq!(
        body("a<table>b<tr>c</table>"),
        "abc<table><tbody><tr></tr></tbody></table>"
    );
}

#[test]
fn foster_parenting_many_nodes_is_linear() {
    let mut input = String::from("<div><table>");
    for _ in 0..20_000 {
        input.push_str("<span>x</span>");
    }
    input.push_str("</table></div>");
    let doc = parse_document(&input);
    assert_well_formed(&doc);
    let div = doc.get_elements_by_tag_name("div")[0];
    assert_eq!(doc.children(div).len(), 20_001);
}

#[test]
fn caption_and_colgroup_edge_cases() {
    assert_eq!(
        body("<table><caption>a<tr><td>b</table>"),
        "<table><caption>a</caption><tbody><tr><td>b</td></tr></tbody></table>"
    );
    assert_eq!(
        body("<table><colgroup><col>x</table>"),
        "x<table><colgroup><col></col></colgroup></table>"
    );
}

#[test]
fn stray_table_tags_in_body_are_ignored() {
    assert_eq!(body("<tr><td>a</td></tr>"), "a");
    assert_eq!(body("<caption>x"), "x");
}

// ---------------------------------------------------------------------------
// Templates and framesets.

#[test]
fn templates_contain_children() {
    assert_eq!(
        head("<template><p>x</template>"),
        "<template><p>x</p></template>"
    );
    assert_eq!(
        body("<body><template><tr><td>a</template>b"),
        "<template><tr><td>a</td></tr></template>b"
    );
    assert_eq!(
        body("<body><template><template>x</template></template>"),
        "<template><template>x</template></template>"
    );
    // Unclosed template at EOF.
    assert_eq!(
        body("<body><template><div>a"),
        "<template><div>a</div></template>"
    );
}

#[test]
fn frameset_documents() {
    let doc =
        parse_document("<frameset cols=2><frame src=a><frameset><frame></frameset></frameset>");
    assert_well_formed(&doc);
    let fs = doc.body().unwrap();
    assert_eq!(doc.tag_name(fs), Some("frameset"));
    assert_eq!(
        ser(&doc, doc.root()),
        "<html><head></head><frameset cols=\"2\"><frame src=\"a\"></frame><frameset><frame></frame></frameset></frameset></html>"
    );
    // Frameset after body content is ignored.
    assert_eq!(body("<p>x<frameset>"), "<p>x</p>");
    // A frameset replaces a body that only holds "frameset-ok" content.
    let doc = parse_document("<div></div><frameset><frame>");
    assert_eq!(doc.tag_name(doc.body().unwrap()), Some("frameset"));
    assert!(doc.get_elements_by_tag_name("div").is_empty());
    // An explicit <body> start tag makes later framesets ignored.
    let doc = parse_document("<body><frameset>");
    assert_eq!(doc.tag_name(doc.body().unwrap()), Some("body"));
}

// ---------------------------------------------------------------------------
// Foreign content.

#[test]
fn svg_case_adjustment() {
    assert_eq!(
        body("<svg viewbox='0 0 10 10' preserveaspectratio=none><lineargradient gradientunits=x><stop/></lineargradient><foreignobject><div>h</div></foreignobject><clippath/><fegaussianblur stddeviation='2'/><use xlink:href=#a /></svg>"),
        "<svg viewBox=\"0 0 10 10\" preserveAspectRatio=\"none\"><linearGradient gradientUnits=\"x\"><stop></stop></linearGradient><foreignObject><div>h</div></foreignObject><clipPath></clipPath><feGaussianBlur stdDeviation=\"2\"></feGaussianBlur><use xlink:href=\"#a\"></use></svg>"
    );
}

#[test]
fn svg_self_closing_honoured() {
    assert_eq!(
        body("<svg><circle/><rect/></svg>x"),
        "<svg><circle></circle><rect></rect></svg>x"
    );
    assert_eq!(body("<svg/>x"), "<svg></svg>x");
    assert_eq!(body("<math/>x"), "<math></math>x");
    // In HTML, self-closing non-void elements are not closed.
    assert_eq!(body("<div/>x"), "<div>x</div>");
}

#[test]
fn svg_breakout() {
    assert_eq!(body("<svg><p>x"), "<svg></svg><p>x</p>");
    assert_eq!(
        body("<svg><g><div>x</div>"),
        "<svg><g></g></svg><div>x</div>"
    );
    assert_eq!(
        body("<svg><font color=red>x"),
        "<svg></svg><font color=\"red\">x</font>"
    );
    // <font> without presentational attributes stays foreign.
    assert_eq!(
        body("<svg><font>x</font></svg>"),
        "<svg><font>x</font></svg>"
    );
    assert_eq!(body("<svg></p>x"), "<svg></svg><p></p>x");
    assert_eq!(
        body("<math><mi>x</mi><b>y</b></math>"),
        "<math><mi>x</mi></math><b>y</b>"
    );
}

#[test]
fn integration_points() {
    assert_eq!(
        body("<svg><desc><b>x</b></desc><title><i>t</i></title></svg>"),
        "<svg><desc><b>x</b></desc><title><i>t</i></title></svg>"
    );
    assert_eq!(
        body("<math><mi><b>x</b></mi></math>"),
        "<math><mi><b>x</b></mi></math>"
    );
    assert_eq!(
        body("<math><annotation-xml encoding='text/html'><div>x</div></annotation-xml></math>"),
        "<math><annotation-xml encoding=\"text/html\"><div>x</div></annotation-xml></math>"
    );
    assert_eq!(
        body("<math><annotation-xml><svg><circle/></svg></annotation-xml></math>"),
        "<math><annotation-xml><svg><circle></circle></svg></annotation-xml></math>"
    );
}

#[test]
fn foreign_end_tags_and_raw_text() {
    // Foreign end tags match case-insensitively.
    assert_eq!(
        body("<svg><lineargradient></linearGradient><g></g></svg>"),
        "<svg><linearGradient></linearGradient><g></g></svg>"
    );
    // <style> and <script> in SVG are not raw text.
    assert_eq!(
        body("<svg><style><g/></style></svg>"),
        "<svg><style><g></g></style></svg>"
    );
    assert_eq!(
        body("<svg><script>1</script></svg>"),
        "<svg><script>1</script></svg>"
    );
    // An unmatched end tag inside svg is ignored.
    assert_eq!(body("<svg><g></x></g></svg>a"), "<svg><g></g></svg>a");
}

#[test]
fn cdata_only_in_foreign_content() {
    assert_eq!(body("<svg><![CDATA[a<b>]]></svg>"), "<svg>a<b></svg>");
    assert_eq!(
        body("<div><![CDATA[x]]></div>"),
        "<div><!--[CDATA[x]]--></div>"
    );
}

#[test]
fn mathml_attributes() {
    assert_eq!(
        body("<math definitionurl=x><mi>y</mi></math>"),
        "<math definitionURL=\"x\"><mi>y</mi></math>"
    );
}

#[test]
fn svg_in_table() {
    assert_eq!(
        body("<table><tr><td><svg><g/></svg></td></tr></table>"),
        "<table><tbody><tr><td><svg><g></g></svg></td></tr></tbody></table>"
    );
}

// ---------------------------------------------------------------------------
// Quirks mode.

#[test]
fn quirks_detection() {
    assert!(parse_document("<p>no doctype").quirks_mode);
    assert!(!parse_document("<!DOCTYPE html><p>").quirks_mode);
    assert!(!parse_document("\n<!doctype html>").quirks_mode);
    assert!(parse_document("<!DOCTYPE foo>").quirks_mode);
    assert!(parse_document("<!DOCTYPE>").quirks_mode);
    assert!(
        parse_document(r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN">"#)
            .quirks_mode
    );
    assert!(!parse_document(
        r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#
    )
    .quirks_mode);
    assert!(parse_document(r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 3.2//EN">"#).quirks_mode);
    assert!(parse_document(r#"<!DOCTYPE html PUBLIC "-//IETF//DTD HTML 2.0//EN">"#).quirks_mode);
    assert!(!parse_document(r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd">"#).quirks_mode);
    // A doctype after content does not count.
    assert!(parse_document("<p>x</p><!DOCTYPE html>").quirks_mode);
    // Doctype node is created with the lowercase name.
    let doc = parse_document("<!DOCTYPE HTML>");
    assert_eq!(
        doc.node(doc.children(doc.root())[0]).data,
        NodeData::Doctype {
            name: "html".into()
        }
    );
}

// ---------------------------------------------------------------------------
// Entities and attributes through the whole pipeline.

#[test]
fn entities_in_document() {
    assert_eq!(
        body("&lt;p&gt; caf&eacute; &copy 2024 &notit; &#x263A; &#128;"),
        "<p> café © 2024 ¬it; ☺ €"
    );
    let doc = parse_document("<a href='?x=1&copy=2&amp;y=&lt;'>");
    let a = doc.get_elements_by_tag_name("a")[0];
    assert_eq!(doc.attr(a, "href"), Some("?x=1&copy=2&y=<"));
}

#[test]
fn attribute_edge_cases_in_tree() {
    let doc = parse_document("<div a=b/c CLASS=x class=y data-z checked id='q'>");
    let div = doc.get_elements_by_tag_name("div")[0];
    let e = doc.element(div).unwrap();
    assert_eq!(e.attr("a"), Some("b/c"));
    assert_eq!(e.attr("class"), Some("x"));
    assert_eq!(e.attr("data-z"), Some(""));
    assert_eq!(e.attrs.len(), 5);
    assert_eq!(doc.get_element_by_id("q"), Some(div));
}

#[test]
fn null_characters_replaced() {
    assert_eq!(body("a\0b<p\0>"), "a\u{FFFD}b<p\u{FFFD}></p\u{FFFD}>");
}

// ---------------------------------------------------------------------------
// Depth limit and robustness.

#[test]
fn deep_nesting_is_capped() {
    let n = 100_000;
    let mut input = String::with_capacity(n * 11);
    for _ in 0..n {
        input.push_str("<div>");
    }
    input.push_str("deep");
    for _ in 0..n {
        input.push_str("</div>");
    }
    input.push_str("<p>after</p>");
    let start = std::time::Instant::now();
    let doc = parse_document(&input);
    let elapsed = start.elapsed();
    assert!(elapsed.as_secs_f64() < 1.0, "took {elapsed:?}");
    assert_well_formed(&doc);
    assert_eq!(max_element_depth(&doc), crate::MAX_DEPTH);
    assert_eq!(doc.get_elements_by_tag_name("div").len(), n);
    assert!(doc.text_content(doc.root()).contains("deep"));
    let p = doc.get_elements_by_tag_name("p")[0];
    assert_eq!(doc.text_content(p), "after");
    // The debug dump (recursive) works on the capped tree.
    assert!(doc.to_debug_string().contains("\"deep\""));
}

#[test]
fn deep_nesting_of_various_elements() {
    let patterns = [
        "<span>",
        "<b>",
        "<a>",
        "<p>",
        "<li>",
        "<ul>",
        "<table><tr><td>",
        "<svg><g>",
        "<math>",
        "<select>",
        "<template>",
        "<object>",
        "<i id=1><i id=2><u>",
        "<em><div>",
        "<table>",
        "<frameset>",
        "<blockquote>",
        "<button>",
        "<font color=x>",
        "<dl><dd>",
    ];
    for pattern in patterns {
        let input = pattern.repeat(20_000) + "x";
        let start = std::time::Instant::now();
        let doc = parse_document(&input);
        let elapsed = start.elapsed();
        assert!(elapsed.as_secs_f64() < 2.0, "{pattern}: took {elapsed:?}");
        assert_well_formed(&doc);
    }
}

#[test]
fn many_misnested_formatting_end_tags() {
    let input = "<b>".to_string() + &"<div>".repeat(20_000) + &"</b>".repeat(20_000) + "x";
    let start = std::time::Instant::now();
    let doc = parse_document(&input);
    assert!(
        start.elapsed().as_secs_f64() < 2.0,
        "took {:?}",
        start.elapsed()
    );
    assert_well_formed(&doc);

    let input = "<p><object>".to_string() + &"<b id=a>".repeat(5_000) + &"<div>".repeat(20_000);
    let start = std::time::Instant::now();
    let doc = parse_document(&input);
    assert!(
        start.elapsed().as_secs_f64() < 2.0,
        "took {:?}",
        start.elapsed()
    );
    assert_well_formed(&doc);
}

#[test]
fn unique_formatting_elements_are_bounded() {
    let mut input = String::new();
    for i in 0..20_000 {
        let _ = write!(input, "<font face=f{i}>");
    }
    input.push_str("<p>x</p>y");
    let start = std::time::Instant::now();
    let doc = parse_document(&input);
    assert!(
        start.elapsed().as_secs_f64() < 2.0,
        "took {:?}",
        start.elapsed()
    );
    assert_well_formed(&doc);
}

#[test]
fn reprocessing_terminates_on_deep_templates() {
    let input = "<template>".repeat(3000);
    let doc = parse_document(&input);
    assert_well_formed(&doc);
}
