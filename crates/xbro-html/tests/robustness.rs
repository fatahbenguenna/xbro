//! Robustness and performance tests through the public API: deterministic
//! fuzzing, pathological inputs and a large synthetic document.

use std::fmt::Write as _;
use std::time::Instant;

use xbro_html::{decode_html_bytes, parse_document, Document, MAX_DEPTH};

/// Deterministic xorshift64* generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
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

fn assert_well_formed(doc: &Document, input: &str) {
    let html = doc
        .document_element()
        .unwrap_or_else(|| panic!("no html for {input:?}"));
    assert_eq!(doc.tag_name(html), Some("html"), "input {input:?}");
    let head = doc
        .head()
        .unwrap_or_else(|| panic!("no head for {input:?}"));
    let body = doc
        .body()
        .unwrap_or_else(|| panic!("no body for {input:?}"));
    assert_eq!(doc.parent(head), Some(html));
    assert_eq!(doc.parent(body), Some(html));
    for n in doc.descendants(doc.root()) {
        for &c in doc.children(n) {
            assert_eq!(doc.parent(c), Some(n), "broken parent link for {input:?}");
        }
    }
    assert!(
        max_element_depth(doc) <= MAX_DEPTH,
        "too deep for {input:?}"
    );
}

const FRAGMENTS: &[&str] = &[
    "<",
    ">",
    "/",
    "=",
    "\"",
    "'",
    "&",
    ";",
    "!",
    "-",
    "a",
    "b",
    "c",
    "d",
    "i",
    "v",
    "p",
    "div",
    "table",
    " ",
    "tr",
    "td",
    "th",
    "svg",
    "\n",
    "math",
    "mi",
    "select",
    "option",
    "optgroup",
    "template",
    "script",
    "style",
    "title",
    "textarea",
    "plaintext",
    "frameset",
    "frame",
    "li",
    "ul",
    "dl",
    "dd",
    "amp",
    "lt",
    "#x",
    "#",
    "1",
    "9",
    "<!--",
    "-->",
    "<![CDATA[",
    "]]>",
    "foreignObject",
    "desc",
    "form",
    "body",
    "html",
    "head",
    "caption",
    "colgroup",
    "col",
    "tbody",
    "noscript",
    "font",
    "color",
    "nobr",
    "button",
    "h1",
    "h2",
    "pre",
    "image",
    "br",
    "input",
    "hidden",
    "type",
    "annotation-xml",
    "encoding",
    "text/html",
    "object",
    "marquee",
    "xmp",
    "iframe",
    "\0",
    "\r",
    "\t",
    "é",
    "日本",
    "<!DOCTYPE html>",
    "&amp;",
    "&notin;",
];

fn random_markup(rng: &mut Rng) -> String {
    let len = rng.below(200);
    let mut s = String::new();
    for _ in 0..len {
        s.push_str(FRAGMENTS[rng.below(FRAGMENTS.len())]);
    }
    s
}

fn random_bytes(rng: &mut Rng) -> Vec<u8> {
    let len = rng.below(400);
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        if rng.below(3) == 0 {
            bytes.push(rng.next() as u8);
        } else {
            let frag = FRAGMENTS[rng.below(FRAGMENTS.len())];
            bytes.extend_from_slice(frag.as_bytes());
        }
    }
    bytes
}

#[test]
fn fuzz_random_markup() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..3000 {
        let input = random_markup(&mut rng);
        let doc = parse_document(&input);
        assert_well_formed(&doc, &input);
    }
}

#[test]
fn fuzz_random_bytes() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    let charsets = [
        None,
        Some("utf-8"),
        Some("windows-1252"),
        Some("utf-16le"),
        Some("shift_jis"),
        Some("bogus"),
    ];
    for i in 0..3000 {
        let bytes = random_bytes(&mut rng);
        let text = decode_html_bytes(&bytes, charsets[i % charsets.len()]);
        let doc = parse_document(&text);
        assert_well_formed(&doc, &text);
    }
}

#[test]
fn fuzz_mutated_real_markup() {
    let base = "<!DOCTYPE html><html><head><title>T &amp; t</title><style>p{}</style></head>\
        <body><table><tr><td>a<td><svg viewBox='0 0 1 1'><circle r=1/></svg></table>\
        <ul><li><b>x<i>y</b>z</i><li>w</ul><select><option>o</select><form><input></form>\
        <math><mi>m</mi></math><p>para<div>d</div><!-- c --></body></html>";
    let bytes = base.as_bytes();
    let mut rng = Rng(42);
    for _ in 0..2000 {
        let mut s = bytes.to_vec();
        for _ in 0..1 + rng.below(8) {
            let pos = rng.below(s.len());
            match rng.below(3) {
                0 => {
                    s.remove(pos);
                }
                1 => s.insert(pos, b"<>/\"'&=-!"[rng.below(9)]),
                _ => {
                    let end = (pos + rng.below(20)).min(s.len());
                    let chunk: Vec<u8> = s[pos..end].to_vec();
                    let at = rng.below(s.len());
                    for (k, b) in chunk.into_iter().enumerate() {
                        s.insert(at + k, b);
                    }
                }
            }
        }
        let text = String::from_utf8_lossy(&s).into_owned();
        let doc = parse_document(&text);
        assert_well_formed(&doc, &text);
    }
}

#[test]
fn pathological_inputs() {
    let cases: Vec<String> = vec![
        "<".repeat(50_000),
        "&".repeat(50_000),
        "&#".repeat(30_000),
        "&amp".repeat(20_000),
        "<!--".repeat(20_000),
        "</".repeat(30_000),
        "<a ".repeat(20_000),
        "<p a=".repeat(20_000),
        "<table>".repeat(20_000),
        "<tr>".repeat(20_000),
        "<td>".repeat(20_000),
        "</table>".repeat(20_000),
        "<table><caption>".repeat(10_000),
        "<svg>".repeat(20_000) + &"</svg>".repeat(20_000),
        "<math><mi>".repeat(10_000),
        "<select><option>".repeat(10_000),
        "<li><ul>".repeat(10_000),
        "<p><button>".to_string() + &"<div>".repeat(30_000),
        "<p><object>".to_string() + &"<span>".repeat(30_000) + &"</em>".repeat(30_000),
        "<b><i><u>".repeat(10_000) + &"</b></i></u>".repeat(10_000),
        "<a><div>".repeat(10_000),
        "<table><tr><td><b><table>".repeat(5_000),
        "<frameset>".repeat(10_000) + &"</frameset>".repeat(10_000),
        "<template><tr>".repeat(10_000),
        "<!DOCTYPE html><p>".to_string() + &"<p>".repeat(20_000),
        "<div>".repeat(1000) + &"<table>x".repeat(1000),
        "<script>".to_string() + &"<!--<script>".repeat(10_000),
        "<textarea>".to_string() + &"&lt;".repeat(20_000),
        "x".repeat(1_000_000),
    ];
    for input in &cases {
        let start = Instant::now();
        let doc = parse_document(input);
        let elapsed = start.elapsed();
        assert_well_formed(&doc, &input[..input.len().min(40)]);
        assert!(
            elapsed.as_secs_f64() < 3.0,
            "{:?}...: took {elapsed:?}",
            &input[..input.len().min(40)]
        );
    }
}

/// Builds a synthetic article of roughly `target` bytes: nested lists,
/// tables, paragraphs with formatting and entities.
fn synthetic_document(target: usize) -> String {
    let mut s = String::with_capacity(target + 4096);
    s.push_str(
        "<!DOCTYPE html><html><head><meta charset=utf-8><title>Synthetic &amp; test</title>",
    );
    s.push_str("<style>body { margin: 0 } .c > p { color: red }</style></head><body>");
    let mut i = 0usize;
    while s.len() < target {
        let _ = write!(
            s,
            "<h2 id=s{i}>Section {i} &mdash; caf&eacute;</h2>\n\
             <p class=lead>Lorem <b>ipsum</b> dolor <a href='/wiki/{i}?a=1&amp;b=2'>sit</a> amet, \
             <i>consectetur</i> &lt;adipiscing&gt; elit &copy; 2024 &#8212; &#x263A;.<br>\n\
             Sed <em>do <strong>eiusmod</strong></em> tempor&nbsp;incididunt.</p>\n\
             <ul><li>One<ul><li>Nested <code>a&lt;b</code><li>Two</ul><li>Three<ol><li>x<li>y</ol></ul>\n\
             <table class=wikitable><caption>T{i}</caption><tr><th>A<th>B<th>C\
             <tr><td>1<td><span>2</span><td>3 &euro;<tr><td colspan=2>4<td><p>5</table>\n\
             <div class=c><p>Para <sup>[{i}]</sup><p>Another &hellip;</div>\n\
             <!-- comment {i} --><img src=x{i}.png alt=\"img &quot;{i}&quot;\">\n"
        );
        i += 1;
    }
    s.push_str("</body></html>");
    s
}

#[test]
fn large_document_performance() {
    // The debug build is much slower: keep the size and bound reasonable.
    let (size, limit) = if cfg!(debug_assertions) {
        (250_000, 2.0)
    } else {
        (1_000_000, 1.0)
    };
    let input = synthetic_document(size);
    let start = Instant::now();
    let doc = parse_document(&input);
    let elapsed = start.elapsed();
    eprintln!(
        "parsed {} bytes into {} nodes in {elapsed:?}",
        input.len(),
        doc.len()
    );
    assert_well_formed(&doc, "synthetic");
    assert!(
        doc.len() > size / 50,
        "suspiciously few nodes: {}",
        doc.len()
    );
    assert!(
        elapsed.as_secs_f64() < limit,
        "parsing {} bytes took {elapsed:?}",
        input.len()
    );
    let tables = doc.get_elements_by_tag_name("table");
    assert!(!tables.is_empty());
    assert_eq!(doc.title().as_deref(), Some("Synthetic & test"));
}

#[test]
fn decoding_then_parsing() {
    let bytes = b"<html><head><meta charset=windows-1252><title>Caf\xE9</title></head><body>\x93q\x94</body></html>";
    let text = decode_html_bytes(bytes, None);
    let doc = parse_document(&text);
    assert_eq!(doc.title().as_deref(), Some("Café"));
    assert_eq!(doc.text_content(doc.body().unwrap()), "\u{201C}q\u{201D}");

    let utf16: Vec<u8> = std::iter::once([0xFF, 0xFE])
        .chain("<p>é</p>".encode_utf16().map(u16::to_le_bytes))
        .flatten()
        .collect();
    let doc = parse_document(&decode_html_bytes(&utf16, Some("windows-1252")));
    assert_eq!(doc.text_content(doc.body().unwrap()), "é");

    let doc = parse_document(&decode_html_bytes(b"<p>\xE9</p>", Some("ISO-8859-1")));
    assert_eq!(doc.text_content(doc.body().unwrap()), "é");
}
