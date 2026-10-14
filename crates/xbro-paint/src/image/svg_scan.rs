//! A cheap structural pre-scan of SVG markup, run before handing it to usvg.
//!
//! usvg (and the roxmltree parser under it) walk the element tree
//! recursively, and resolve some references (clip paths, masks) recursively
//! too: hostile structure can exhaust the stack, which aborts the whole
//! process (`catch_unwind` cannot help), or make the parser build huge
//! amounts of text. This scanner measures the structure in linear passes
//! over the bytes, without building anything, so such documents are
//! rejected up front:
//!
//! * element nesting deeper than [`MAX_SVG_DEPTH`];
//! * more than [`MAX_REFERENCE_LINKS`] clip-path/mask/marker/filter
//!   references made from inside `<clipPath>`, `<mask>`, `<marker>` or
//!   `<pattern>` definitions (the links of reference chains);
//! * DTD entities whose references would expand to more than
//!   [`MAX_ENTITY_EXPANSION`] bytes in total, or that contain markup.

use std::collections::HashMap;

/// Largest element nesting depth accepted in SVG documents (the same bound
/// the HTML parser applies to the DOM; real SVG files stay far below it).
pub(crate) const MAX_SVG_DEPTH: usize = 256;

/// Most clip-path/mask/marker/filter references accepted inside
/// clip/mask/marker/pattern definitions (real documents rarely chain
/// definitions at all).
pub(crate) const MAX_REFERENCE_LINKS: usize = 16;

/// Largest total size, in bytes, of the text produced by expanding the
/// document's DTD entity references.
pub(crate) const MAX_ENTITY_EXPANSION: usize = 1 << 20;

/// Longest entity name considered when looking for references.
const MAX_ENTITY_NAME: usize = 256;

/// Deepest entity-in-entity nesting followed (roxmltree stops at 10).
const MAX_ENTITY_NESTING: usize = 16;

/// True when the document passes every structural check of this module.
pub(crate) fn structure_ok(data: &[u8]) -> bool {
    let Some(summary) = walk(data, MAX_SVG_DEPTH) else {
        return false;
    };
    summary.links <= MAX_REFERENCE_LINKS && entities_bounded(data, &summary)
}

/// True when the element nesting depth of the XML document `data` never
/// exceeds `limit` (and its DTD declares no entity containing markup).
///
/// Comments, CDATA sections, processing instructions and quoted attribute
/// values are skipped. A DOCTYPE whose internal subset declares an entity
/// containing markup (a literal `<` inside a quoted value) is rejected as
/// well, since expanding such entities could rebuild a deep tree from a
/// shallow-looking document. Malformed input is scanned leniently (the
/// parser rejects it later); the scan is O(n) and never panics.
#[cfg(test)]
pub(crate) fn nesting_within(data: &[u8], limit: usize) -> bool {
    walk(data, limit).is_some()
}

/// What [`walk`] learnt about a document.
#[derive(Default)]
struct Summary<'a> {
    /// References made from inside clip/mask/marker/pattern definitions.
    links: usize,
    /// General entities declared in the DTD: (name, literal value).
    entities: Vec<(&'a [u8], &'a [u8])>,
    /// Index just past the DOCTYPE (entity references start after it).
    doctype_end: usize,
}

/// One pass over the document; `None` when nesting exceeds `limit` or an
/// entity declares markup.
fn walk(data: &[u8], limit: usize) -> Option<Summary<'_>> {
    let n = data.len();
    let mut summary = Summary::default();
    let mut depth = 0usize;
    // Depth of the outermost open definition element, if any.
    let mut def_depth: Option<usize> = None;
    let mut i = 0usize;
    while i < n {
        let Some(offset) = find_byte(data, i, b'<') else {
            break;
        };
        i = offset + 1;
        let rest = data.get(i..).unwrap_or_default();
        if rest.starts_with(b"!--") {
            i = skip_past(data, i + 3, b"-->");
        } else if rest.starts_with(b"![CDATA[") {
            i = skip_past(data, i + 8, b"]]>");
        } else if rest.starts_with(b"?") {
            i = skip_past(data, i + 1, b"?>");
        } else if rest.starts_with(b"!") {
            let end = skip_doctype(data, i + 1)?;
            summary.entities.extend(entity_declarations(
                data.get(i + 1..end).unwrap_or_default(),
            ));
            summary.doctype_end = summary.doctype_end.max(end);
            i = end;
        } else if rest.starts_with(b"/") {
            if def_depth == Some(depth) {
                def_depth = None;
            }
            depth = depth.saturating_sub(1);
            i = skip_past(data, i + 1, b">");
        } else {
            let (next, self_closing) = skip_tag(data, i);
            let tag = data.get(i..next).unwrap_or_default();
            let name_len = tag
                .iter()
                .position(|&b| b.is_ascii_whitespace() || b == b'/' || b == b'>')
                .unwrap_or(tag.len());
            let (name, attributes) = tag.split_at(name_len);
            let is_def = is_definition(name);
            if (def_depth.is_some() || is_def) && is_link(attributes) {
                summary.links += 1;
            }
            if !self_closing {
                depth += 1;
                if depth > limit {
                    return None;
                }
                if is_def && def_depth.is_none() {
                    def_depth = Some(depth);
                }
            }
            i = next;
        }
    }
    Some(summary)
}

/// True when the first element of the XML document `data` (after the
/// prolog: XML declaration, processing instructions, comments, DOCTYPE
/// and whitespace) is an `svg` element, with or without a namespace
/// prefix. Element names are compared ASCII case-insensitively, as the
/// sniffing of `<SVG` always was. Linear and never panics.
pub(crate) fn root_is_svg(data: &[u8]) -> bool {
    let mut i = 0usize;
    loop {
        while data.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let rest = data.get(i..).unwrap_or_default();
        if rest.starts_with(b"<?") {
            i = skip_past(data, i + 2, b"?>");
        } else if rest.starts_with(b"<!--") {
            i = skip_past(data, i + 4, b"-->");
        } else if rest.starts_with(b"<!") {
            // A DOCTYPE whose subset declares markup is not sniffed as SVG
            // (it would be rejected by the structural scan anyway).
            match skip_doctype(data, i + 2) {
                Some(next) => i = next,
                None => return false,
            }
        } else if rest.starts_with(b"<") {
            let name_len = rest
                .iter()
                .skip(1)
                .position(|&b| b.is_ascii_whitespace() || b == b'/' || b == b'>')
                .unwrap_or(rest.len().saturating_sub(1));
            let name = rest.get(1..1 + name_len).unwrap_or_default();
            let local = name.rsplit(|&b| b == b':').next().unwrap_or(name);
            return local.eq_ignore_ascii_case(b"svg");
        } else {
            return false;
        }
    }
}

/// `<clipPath>`, `<mask>`, `<marker>` and `<pattern>` (any namespace prefix).
fn is_definition(name: &[u8]) -> bool {
    let local = name.rsplit(|&b| b == b':').next().unwrap_or(name);
    matches!(local, b"clipPath" | b"mask" | b"marker" | b"pattern")
}

/// True when start-tag attributes reference another clip path, mask,
/// marker or filter (`clip-path="url(#a)"`, `style="mask:url(#m)"`…).
fn is_link(attributes: &[u8]) -> bool {
    contains_ignore_case(attributes, b"url(")
        && [&b"clip-path"[..], b"mask", b"marker", b"filter"]
            .iter()
            .any(|p| contains_ignore_case(attributes, p))
}

fn contains_ignore_case(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && hay
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
}

/// Index of the first `byte` at or after `from`.
fn find_byte(data: &[u8], from: usize, byte: u8) -> Option<usize> {
    data.get(from..)?
        .iter()
        .position(|&b| b == byte)
        .map(|p| from + p)
}

/// Index just past the first occurrence of `pattern` at or after `from`
/// (the end of the data when there is none).
fn skip_past(data: &[u8], from: usize, pattern: &[u8]) -> usize {
    let Some(hay) = data.get(from..) else {
        return data.len();
    };
    hay.windows(pattern.len())
        .position(|w| w == pattern)
        .map_or(data.len(), |p| from + p + pattern.len())
}

/// Skips a start tag whose name begins at `from`, honouring quoted
/// attribute values. Returns the index past its `>` and whether the tag is
/// self-closing (`/>`); an unterminated tag counts as self-closing.
fn skip_tag(data: &[u8], from: usize) -> (usize, bool) {
    let mut quote: Option<u8> = None;
    let mut prev = 0u8;
    for (offset, &b) in data.get(from..).unwrap_or_default().iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return (from + offset + 1, prev == b'/'),
            None => {}
        }
        if !b.is_ascii_whitespace() {
            prev = b;
        }
    }
    (data.len(), true)
}

/// Skips a `<!DOCTYPE …>` (or other `<!…>` declaration) starting after
/// its `<!`, including an internal subset in brackets. Returns `None` when
/// the subset declares markup inside a quoted value (see [`nesting_within`]).
fn skip_doctype(data: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut quote: Option<u8> = None;
    let mut subset = 0usize;
    while let Some(&b) = data.get(i) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) if b == b'<' && subset > 0 => return None,
            Some(_) => {}
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'[' => subset += 1,
                b']' => subset = subset.saturating_sub(1),
                b'<' if subset > 0 && data.get(i + 1..i + 4) == Some(b"!--".as_slice()) => {
                    i = skip_past(data, i + 4, b"-->");
                    continue;
                }
                b'>' if subset == 0 => return Some(i + 1),
                _ => {}
            },
        }
        i += 1;
    }
    Some(data.len())
}

/// General entity declarations (`<!ENTITY name "value">`) of a DOCTYPE,
/// as (name, literal value); external entities (never loaded) get an
/// empty value and parameter entities (`<!ENTITY % …>`) are skipped.
/// One linear pass (comments, other declarations and quoted strings are
/// skipped).
fn entity_declarations(doctype: &[u8]) -> Vec<(&[u8], &[u8])> {
    const DECL: &[u8] = b"<!ENTITY";
    let skip_ws = |mut j: usize| {
        while doctype.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        j
    };
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(&b) = doctype.get(i) {
        let rest = doctype.get(i..).unwrap_or_default();
        if rest.starts_with(b"<!--") {
            i = skip_past(doctype, i + 4, b"-->");
        } else if rest.starts_with(DECL) {
            let mut j = skip_ws(i + DECL.len());
            if doctype.get(j) == Some(&b'%') {
                i = skip_declaration(doctype, j);
                continue;
            }
            let name_start = j;
            while doctype
                .get(j)
                .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>')
            {
                j += 1;
            }
            let name = doctype.get(name_start..j).unwrap_or_default();
            j = skip_ws(j);
            let value = match doctype.get(j) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    let end = find_byte(doctype, j + 1, q).unwrap_or(doctype.len());
                    doctype.get(j + 1..end).unwrap_or_default()
                }
                _ => &[][..],
            };
            if !name.is_empty() {
                out.push((name, value));
            }
            i = skip_declaration(doctype, j);
        } else if rest.starts_with(b"<!") || rest.starts_with(b"<?") {
            i = skip_declaration(doctype, i + 2);
        } else if b == b'"' || b == b'\'' {
            i = find_byte(doctype, i + 1, b).map_or(doctype.len(), |e| e + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// Index past the `>` closing the declaration that contains `from`
/// (quoted values honoured).
fn skip_declaration(data: &[u8], from: usize) -> usize {
    let mut quote: Option<u8> = None;
    for (offset, &b) in data.get(from..).unwrap_or_default().iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return from + offset + 1,
            None => {}
        }
    }
    data.len()
}

/// The entity name referenced at `data[i]` (which is `&`), if it is a
/// well-formed `&name;` reference.
fn reference_at(data: &[u8], i: usize) -> Option<&[u8]> {
    let tail = data.get(i + 1..)?;
    let len = tail
        .iter()
        .take(MAX_ENTITY_NAME + 1)
        .position(|&b| b == b';' || b == b'&' || b == b'<' || b.is_ascii_whitespace())?;
    if tail.get(len) == Some(&b';') && len > 0 {
        tail.get(..len)
    } else {
        None
    }
}

/// True when the total expansion of the document's entity references
/// stays within [`MAX_ENTITY_EXPANSION`] (always true without entities).
fn entities_bounded(data: &[u8], summary: &Summary<'_>) -> bool {
    if summary.entities.is_empty() {
        return true;
    }
    let mut index: HashMap<&[u8], usize> = HashMap::new();
    for (i, (name, _)) in summary.entities.iter().enumerate() {
        // The first declaration of a name wins (XML rule).
        index.entry(name).or_insert(i);
    }
    let mut memo: Vec<Option<usize>> = vec![None; summary.entities.len()];
    let mut total = 0usize;
    let body = data.get(summary.doctype_end..).unwrap_or_default();
    let mut i = 0usize;
    while let Some(amp) = find_byte(body, i, b'&') {
        i = amp + 1;
        let Some(&entity) = reference_at(body, amp).and_then(|name| index.get(name)) else {
            continue;
        };
        total = total.saturating_add(expansion(entity, &summary.entities, &index, &mut memo, 0));
        if total > MAX_ENTITY_EXPANSION {
            return false;
        }
    }
    true
}

/// Bytes produced by expanding entity `e` (`usize::MAX` for loops and
/// excessive nesting, which roxmltree rejects anyway).
fn expansion(
    e: usize,
    entities: &[(&[u8], &[u8])],
    index: &HashMap<&[u8], usize>,
    memo: &mut [Option<usize>],
    depth: usize,
) -> usize {
    if depth > MAX_ENTITY_NESTING {
        return usize::MAX;
    }
    if let Some(Some(known)) = memo.get(e) {
        return *known;
    }
    let Some(&(_, value)) = entities.get(e) else {
        return 0;
    };
    // Marks the entity as being expanded: a reference loop yields MAX.
    if let Some(slot) = memo.get_mut(e) {
        *slot = Some(usize::MAX);
    }
    let mut total = 0usize;
    let mut i = 0usize;
    while i < value.len() {
        let child = (value.get(i) == Some(&b'&'))
            .then(|| reference_at(value, i))
            .flatten()
            .and_then(|name| index.get(name).map(|&c| (c, name.len())));
        match child {
            Some((c, name_len)) => {
                total = total.saturating_add(expansion(c, entities, index, memo, depth + 1));
                i += name_len + 2;
            }
            None => {
                total = total.saturating_add(1);
                i += 1;
            }
        }
        if total == usize::MAX {
            break;
        }
    }
    if let Some(slot) = memo.get_mut(e) {
        *slot = Some(total);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(depth: usize) -> String {
        let mut s = String::from("<svg>");
        for _ in 1..depth {
            s.push_str("<g>");
        }
        for _ in 1..depth {
            s.push_str("</g>");
        }
        s.push_str("</svg>");
        s
    }

    #[test]
    fn measures_depth() {
        assert!(nesting_within(nested(256).as_bytes(), 256));
        assert!(!nesting_within(nested(257).as_bytes(), 256));
        assert!(nesting_within(b"", 0));
        // Siblings and self-closing tags do not add depth.
        let flat = "<svg>".to_string() + &"<g><rect/></g><circle r='1' />".repeat(1000) + "</svg>";
        assert!(nesting_within(flat.as_bytes(), 2));
        assert!(!nesting_within(flat.as_bytes(), 1));
    }

    #[test]
    fn skips_comments_cdata_and_attributes() {
        let svg = br#"<?xml version="1.0"?><!-- <g><g><g> -->
            <svg a="<g>" b='/>' c="x>"><![CDATA[<g><g><g>]]><style>a>b{}</style></svg>"#;
        assert!(nesting_within(svg, 2));
        assert!(!nesting_within(svg, 1));
    }

    #[test]
    fn doctype_subsets() {
        let ok = br#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "x.dtd" [
            <!ENTITY ns_svg "http://www.w3.org/2000/svg">
            <!-- don't <g> -->
        ]><svg xmlns="&ns_svg;"/>"#;
        assert!(nesting_within(ok, 1));
        assert!(structure_ok(ok));
        let markup = br#"<!DOCTYPE svg [ <!ENTITY a "<g><g></g></g>"> ]><svg>&a;</svg>"#;
        assert!(!nesting_within(markup, 256));
    }

    #[test]
    fn entity_declarations_are_parsed() {
        let dtd = br#"<!ENTITY a "x"><!-- <!ENTITY c "no"> --><!ENTITY % p "y">
            <!ENTITY b 'z&a;'><!ENTITY ext SYSTEM "file.ent">"#;
        let decls = entity_declarations(dtd);
        let names: Vec<&[u8]> = decls.iter().map(|d| d.0).collect();
        assert_eq!(names, vec![&b"a"[..], b"b", b"ext"]);
        assert_eq!(decls[1].1, b"z&a;");
        assert_eq!(decls[2].1, b"");
    }

    #[test]
    fn entity_expansion_is_bounded() {
        // A few short references, as illustration editors emit: fine.
        let small = br#"<!DOCTYPE svg [<!ENTITY ns "http://www.w3.org/2000/svg">]>
            <svg xmlns="&ns;"><g id="&ns;"/></svg>"#;
        assert!(structure_ok(small));
        // References that would expand to megabytes of text are rejected.
        let value = "x".repeat(1000);
        let refs = "&a;".repeat(50);
        let big = format!(
            "<!DOCTYPE svg [<!ENTITY a \"{value}\"><!ENTITY b \"{refs}\">]><svg>{}</svg>",
            "&b;".repeat(30)
        );
        assert!(!structure_ok(big.as_bytes()));
        // The same declarations referenced once stay within the budget.
        let once =
            format!("<!DOCTYPE svg [<!ENTITY a \"{value}\"><!ENTITY b \"{refs}\">]><svg>&b;</svg>");
        assert!(structure_ok(once.as_bytes()));
        // Self-referencing entities are rejected.
        let looped = br#"<!DOCTYPE svg [<!ENTITY a "&b;"><!ENTITY b "&a;">]><svg>&a;</svg>"#;
        assert!(!structure_ok(looped));
        // Undeclared names and character references cost nothing.
        assert!(structure_ok(b"<svg>&amp;&#60;&unknown;&</svg>"));
    }

    #[test]
    fn reference_links_are_counted() {
        let chained = |n: usize| {
            let mut s = String::from("<svg><defs>");
            for i in 0..n {
                s.push_str(&format!(
                    "<clipPath id='c{i}' clip-path='url(#c{})'><rect/></clipPath>",
                    i + 1
                ));
            }
            s + "</defs></svg>"
        };
        assert!(structure_ok(chained(MAX_REFERENCE_LINKS).as_bytes()));
        assert!(!structure_ok(chained(MAX_REFERENCE_LINKS + 1).as_bytes()));
        // References inside definition contents count too.
        let inner = "<svg>".to_string()
            + &"<mask id='m'><rect mask='url(#n)'/></mask>".repeat(MAX_REFERENCE_LINKS + 1)
            + "</svg>";
        assert!(!structure_ok(inner.as_bytes()));
        // Paint references (gradients) and references outside definitions do not.
        let paints = "<svg>".to_string()
            + &"<pattern id='p'><rect fill='url(#g)'/></pattern>".repeat(100)
            + &"<rect clip-path='url(#c)' mask='url(#m)'/>".repeat(100)
            + "</svg>";
        assert!(structure_ok(paints.as_bytes()));
    }

    #[test]
    fn malformed_input_is_scanned_without_panicking() {
        for s in [
            &b"<"[..],
            b"<!",
            b"<!--",
            b"<![CDATA[",
            b"<?",
            b"</",
            b"<svg a=\"",
            b"<!DOCTYPE [ \"",
            b"<!DOCTYPE [ <!ENTITY",
            b"<!DOCTYPE [ <!ENTITY a",
            b"<!DOCTYPE [ <!ENTITY a \"&a",
            b"<!DOCTYPE [ <!ENTITY a \"x\">]><svg>&a",
            b"<<<<>>>>",
            b"</g></g></g><g>",
            b"<mask></mask></mask><clipPath clip-path='url(",
        ] {
            let _ = nesting_within(s, 4);
            let _ = structure_ok(s);
        }
        assert!(!nesting_within("<g>".repeat(10).as_bytes(), 4));
    }
}
