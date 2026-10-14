//! Serialization of inline `<svg>` elements to standalone SVG documents,
//! which `xbro_paint::Image::from_svg_str` renders.
//!
//! The DOM does not record namespaces, so the serializer declares the SVG
//! and XLink namespaces on the root and drops prefixed names it cannot
//! declare (`sodipodi:*`, `inkscape:*`…) to keep the XML well-formed.
//! `<script>` and `<foreignObject>` subtrees are skipped. The root gets a
//! `color` attribute with the element's computed color so that
//! `currentColor` paints like in the page. Elements referenced by
//! `<use href="#id">` or `url(#id)` that live outside the `<svg>` (icon
//! sprite sheets) are copied into a `<defs>` block.

use std::collections::{HashMap, HashSet};

use xbro_dom::{Document, NodeData, NodeId};
use xbro_style::Color;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// Largest serialized document (bytes); larger inline SVGs are not rendered.
const MAX_SVG_BYTES: usize = 4 << 20;

/// Maximum number of external elements copied for `#id` references.
const MAX_EXTERNAL_REFS: usize = 64;

/// Maximum element nesting followed by the serializer (the DOM is bounded
/// to 256 levels anyway).
const MAX_DEPTH: usize = 300;

/// Lazily built `id` → element index, shared by the serialization of all the
/// inline SVGs of a document.
#[derive(Default)]
pub(crate) struct IdIndex {
    map: Option<HashMap<String, NodeId>>,
}

impl IdIndex {
    fn get(&mut self, doc: &Document, id: &str) -> Option<NodeId> {
        let map = self.map.get_or_insert_with(|| {
            let mut map = HashMap::new();
            for n in doc.descendants(doc.root()) {
                if let Some(id) = doc.element(n).and_then(|e| e.id()) {
                    map.entry(id.to_string()).or_insert(n);
                }
            }
            map
        });
        map.get(id).copied()
    }
}

/// CSS-ish color for the root `color` attribute.
fn color_value(c: Color) -> String {
    if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!(
            "rgba({},{},{},{:.3})",
            c.r,
            c.g,
            c.b,
            f32::from(c.a) / 255.0
        )
    }
}

/// Valid XML name (ASCII subset is enough for SVG vocabularies).
fn is_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || !first.is_ascii())
        && chars.all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':') || !c.is_ascii()
        })
        && name.matches(':').count() <= 1
        && !name.ends_with(':')
}

/// Whether an attribute can be written as-is (declared or no prefix).
fn keep_attribute(name: &str) -> bool {
    if !is_xml_name(name) {
        return false;
    }
    match name.split_once(':') {
        None => name != "xmlns",
        Some(("xlink", _)) | Some(("xml", _)) => true,
        _ => false,
    }
}

fn push_escaped(out: &mut String, text: &str, attr: bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\n' if attr => out.push_str("&#10;"),
            '\t' if attr => out.push_str("&#9;"),
            '\r' if attr => out.push_str("&#13;"),
            // Characters not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t' | '\r') => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
}

/// Element names whose subtrees are not serialized.
fn skipped_element(name: &str) -> bool {
    name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("foreignobject")
}

/// `#id` targets referenced by an attribute value (`href="#a"`, `url(#b)`).
fn references(name: &str, value: &str, out: &mut Vec<String>) {
    let v = value.trim();
    if name == "href" || name == "xlink:href" {
        if let Some(id) = v.strip_prefix('#') {
            if !id.is_empty() {
                out.push(id.to_string());
            }
        }
        return;
    }
    let mut rest = v;
    while let Some(pos) = rest.find("url(") {
        rest = &rest[pos + 4..];
        let inner = rest.trim_start().trim_start_matches(['"', '\'']);
        if let Some(id) = inner.strip_prefix('#') {
            let end = id
                .find(|c: char| matches!(c, ')' | '"' | '\'') || c.is_whitespace())
                .unwrap_or(id.len());
            if end > 0 {
                out.push(id[..end].to_string());
            }
        }
    }
}

struct Writer<'a> {
    doc: &'a Document,
    out: String,
    /// `id`s defined inside the serialized subtrees.
    defined: HashSet<String>,
    /// `#id` references found so far.
    referenced: Vec<String>,
    overflow: bool,
}

impl Writer<'_> {
    fn element(&mut self, node: NodeId, root_extra: Option<&str>, depth: usize) {
        if depth > MAX_DEPTH || self.overflow {
            return;
        }
        if self.out.len() > MAX_SVG_BYTES {
            self.overflow = true;
            return;
        }
        let doc = self.doc;
        let Some(el) = doc.element(node) else {
            return;
        };
        if skipped_element(&el.name) || !is_xml_name(&el.name) || el.name.contains(':') {
            return;
        }
        self.out.push('<');
        self.out.push_str(&el.name);
        let is_root = root_extra.is_some();
        if let Some(extra) = root_extra {
            self.out.push_str(extra);
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for a in &el.attrs {
            if !keep_attribute(&a.name) || !seen.insert(a.name.as_str()) {
                continue;
            }
            if is_root && a.name.eq_ignore_ascii_case("color") {
                continue; // Replaced by the computed color.
            }
            // Event handlers are meaningless here.
            if a.name.len() > 2
                && a.name
                    .get(..2)
                    .is_some_and(|p| p.eq_ignore_ascii_case("on"))
            {
                continue;
            }
            if a.name == "id" && !a.value.is_empty() {
                self.defined.insert(a.value.clone());
            }
            references(&a.name, &a.value, &mut self.referenced);
            self.out.push(' ');
            self.out.push_str(&a.name);
            self.out.push_str("=\"");
            push_escaped(&mut self.out, &a.value, true);
            self.out.push('"');
        }
        let children = doc.children(node);
        if children.is_empty() {
            self.out.push_str("/>");
            return;
        }
        self.out.push('>');
        for &c in children {
            match &doc.node(c).data {
                NodeData::Text(t) => push_escaped(&mut self.out, t, false),
                NodeData::Element(_) => self.element(c, None, depth + 1),
                _ => {}
            }
        }
        self.out.push_str("</");
        self.out.push_str(&el.name);
        self.out.push('>');
    }
}

/// Serializes the inline `<svg>` element `svg` to an SVG document, with
/// `color` as the value of `currentColor`. `None` when the element is not an
/// `svg` or the document would be too large.
pub(crate) fn serialize_svg(
    doc: &Document,
    svg: NodeId,
    color: Color,
    ids: &mut IdIndex,
) -> Option<String> {
    if !doc.is_element_named(svg, "svg") {
        return None;
    }
    let mut root_extra = format!(" xmlns=\"{SVG_NS}\" xmlns:xlink=\"{XLINK_NS}\"");
    root_extra.push_str(" color=\"");
    root_extra.push_str(&color_value(color));
    root_extra.push('"');
    let mut w = Writer {
        doc,
        out: String::with_capacity(1024),
        defined: HashSet::new(),
        referenced: Vec::new(),
        overflow: false,
    };
    w.element(svg, Some(&root_extra), 0);
    if w.overflow {
        return None;
    }
    // Copy referenced elements defined elsewhere in the document.
    let mut external = String::new();
    let mut copied = 0usize;
    let mut checked: HashSet<String> = HashSet::new();
    while let Some(id) = w.referenced.pop() {
        if copied >= MAX_EXTERNAL_REFS || w.overflow {
            break;
        }
        if w.defined.contains(&id) || !checked.insert(id.clone()) {
            continue;
        }
        let Some(target) = ids.get(doc, &id) else {
            continue;
        };
        // Never copy an ancestor of the svg (it would include the svg itself).
        if target == svg || doc.ancestors(svg).any(|a| a == target) {
            continue;
        }
        let before = std::mem::take(&mut w.out);
        w.element(target, None, 0);
        external.push_str(&std::mem::replace(&mut w.out, before));
        copied += 1;
    }
    if w.overflow {
        return None;
    }
    if !external.is_empty() {
        // Insert `<defs>` right after the root start tag.
        let insert_at = root_start_tag_end(&w.out)?;
        if w.out[..insert_at].ends_with("/>") {
            let head = w.out[..insert_at - 2].to_string();
            w.out = format!("{head}><defs>{external}</defs></svg>");
        } else {
            w.out
                .insert_str(insert_at, &format!("<defs>{external}</defs>"));
        }
    }
    Some(w.out)
}

/// Byte offset just past the root start tag (`<svg …>` or `<svg …/>`).
fn root_start_tag_end(xml: &str) -> Option<usize> {
    let mut in_quote = false;
    for (i, c) in xml.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            '>' if !in_quote => return Some(i + 1),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_svg(doc: &Document) -> NodeId {
        doc.descendants(doc.root())
            .find(|&n| doc.is_element_named(n, "svg"))
            .unwrap()
    }

    #[test]
    fn serializes_with_namespaces_and_color() {
        let doc = xbro_html::parse_document(
            "<p><svg viewBox='0 0 10 10' width=20 color=red onload='x()' sodipodi:docname='a'>\
             <script>alert(1)</script><title>a &lt; b</title>\
             <rect width='5' height='5' fill='currentColor' data-x='\"q\"' 日x='1'/>\
             <foreignObject><div>html</div></foreignObject></svg>",
        );
        let svg = first_svg(&doc);
        let mut ids = IdIndex::default();
        let xml = serialize_svg(&doc, svg, Color::rgb(0, 128, 255), &mut ids).unwrap();
        assert!(
            xml.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""),
            "{xml}"
        );
        assert!(xml.contains("xmlns:xlink="));
        assert!(xml.contains("color=\"#0080ff\""));
        assert!(!xml.contains("color=\"red\""));
        assert!(xml.contains("viewBox=\"0 0 10 10\""));
        assert!(!xml.contains("alert"));
        assert!(!xml.contains("html"));
        assert!(!xml.contains("onload"));
        assert!(!xml.contains("sodipodi"));
        assert!(xml.contains("a &lt; b"));
        assert!(xml.contains("data-x=\"&quot;q&quot;\""));
        assert!(xbro_paint::Image::from_svg_str(&xml).is_some(), "{xml}");
    }

    #[test]
    fn copies_external_sprite_symbols() {
        let doc = xbro_html::parse_document(
            "<svg style='display:none'><symbol id='icon' viewBox='0 0 4 4'>\
             <rect width='4' height='4' fill='url(#grad)'/></symbol>\
             <linearGradient id='grad'><stop offset='0' stop-color='red'/></linearGradient></svg>\
             <p><svg width=8 height=8 id=use-me><use xlink:href='#icon'/></svg>",
        );
        let svg = doc.get_element_by_id("use-me").unwrap();
        let mut ids = IdIndex::default();
        let xml = serialize_svg(&doc, svg, Color::BLACK, &mut ids).unwrap();
        assert!(xml.contains("<defs><"), "{xml}");
        assert!(xml.contains("<symbol id=\"icon\""), "{xml}");
        assert!(xml.contains("<linearGradient id=\"grad\""), "{xml}");
        assert!(xbro_paint::Image::from_svg_str(&xml).is_some(), "{xml}");
    }

    #[test]
    fn empty_svg_and_references() {
        let doc = xbro_html::parse_document("<svg width=3 height=4></svg>");
        let svg = first_svg(&doc);
        let xml = serialize_svg(&doc, svg, Color::BLACK, &mut IdIndex::default()).unwrap();
        assert!(xml.ends_with("/>"), "{xml}");
        let mut refs = Vec::new();
        references("fill", "url( '#a' ) url(#b)", &mut refs);
        references("href", "#c", &mut refs);
        references("href", "other.svg#d", &mut refs);
        assert_eq!(refs, vec!["a", "b", "c"]);
        assert!(is_xml_name("xlink:href"));
        assert!(!is_xml_name("a:b:c"));
        assert!(!is_xml_name("\"x"));
        assert!(!keep_attribute("inkscape:label"));
        assert!(keep_attribute("xml:space"));
    }
}
