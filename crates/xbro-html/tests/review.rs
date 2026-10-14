//! Regression tests written during the adversarial review of the crate:
//! spec deviations, hostile-input performance (quadratic paths), memory
//! amplification and property-style checks, all through the public API.

use std::fmt::Write as _;
use std::time::Instant;

use xbro_html::entities;
use xbro_html::tokenizer::{TextMode, Token, Tokenizer};
use xbro_html::{decode_html_bytes, parse_document, Document, NodeData, NodeId, MAX_DEPTH};

// ---------------------------------------------------------------------------
// Helpers.

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

/// Compact serialisation of the children of `node` (void elements included).
fn ser(doc: &Document, node: NodeId) -> String {
    let mut out = String::new();
    let mut stack: Vec<(NodeId, bool)> = doc
        .children(node)
        .iter()
        .rev()
        .map(|&c| (c, false))
        .collect();
    while let Some((n, closing)) = stack.pop() {
        match &doc.node(n).data {
            NodeData::Element(e) if closing => {
                let _ = write!(out, "</{}>", e.name);
            }
            NodeData::Element(e) => {
                let _ = write!(out, "<{}", e.name);
                for a in &e.attrs {
                    let _ = write!(out, " {}=\"{}\"", a.name, a.value);
                }
                out.push('>');
                stack.push((n, true));
                stack.extend(doc.children(n).iter().rev().map(|&c| (c, false)));
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
    out
}

fn body(input: &str) -> String {
    let doc = parse_document(input);
    let body = doc.body().expect("body");
    ser(&doc, body)
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

/// Structural invariants of every parse: `html > head, body`, consistent
/// parent links, a real tree (no node reachable twice) and the depth cap.
fn assert_well_formed(doc: &Document, what: &str) {
    let html = doc.document_element().expect("html element");
    assert_eq!(doc.tag_name(html), Some("html"), "{what}");
    let head = doc.head().unwrap_or_else(|| panic!("no head: {what}"));
    let body = doc.body().unwrap_or_else(|| panic!("no body: {what}"));
    assert_eq!(doc.parent(head), Some(html), "{what}");
    assert_eq!(doc.parent(body), Some(html), "{what}");
    let mut seen = vec![false; doc.len()];
    for n in doc.descendants(doc.root()) {
        assert!(!seen[n.index()], "node reachable twice: {what}");
        seen[n.index()] = true;
        for &c in doc.children(n) {
            assert_eq!(doc.parent(c), Some(n), "broken parent link: {what}");
        }
    }
    assert!(max_element_depth(doc) <= MAX_DEPTH, "too deep: {what}");
    assert_no_parent_cycles(doc, what);
}

/// Every node's parent chain (detached nodes included) must terminate: a
/// cycle is invisible to a traversal from the root but would make any
/// `ancestors()` walk downstream loop forever. Linear (memoised walk).
fn assert_no_parent_cycles(doc: &Document, what: &str) {
    // 0 = unknown, 1 = on the current path, 2 = known to terminate.
    let mut state = vec![0u8; doc.len()];
    let mut path = Vec::new();
    for id in doc.ids() {
        let mut cur = Some(id);
        while let Some(n) = cur {
            match state[n.index()] {
                2 => break,
                1 => panic!("parent cycle through node {n:?}: {what}"),
                _ => {
                    state[n.index()] = 1;
                    path.push(n);
                    cur = doc.parent(n);
                }
            }
        }
        for n in path.drain(..) {
            state[n.index()] = 2;
        }
    }
}

/// Parses `input` `runs` times; returns the last document and the fastest
/// parse time in seconds (the minimum filters out scheduler noise when
/// other builds share the machine). The invariants are checked once.
fn parse_checked(input: &str, runs: usize, what: &str) -> (Document, f64) {
    let mut best = f64::INFINITY;
    let mut doc = Document::new();
    for _ in 0..runs.max(1) {
        let start = Instant::now();
        doc = parse_document(input);
        best = best.min(start.elapsed().as_secs_f64());
    }
    assert_well_formed(&doc, what);
    (doc, best)
}

/// Checks that parsing `make(n)` scales linearly: quadrupling the input
/// must not multiply the time by much more than four (a quadratic path
/// multiplies it by sixteen). Also bounds the absolute time of the large
/// input (`limit` seconds in release, four times that in debug builds).
fn assert_linear(make: impl Fn(usize) -> String, n: usize, limit: f64, what: &str) -> Document {
    let limit = if cfg!(debug_assertions) {
        limit * 4.0
    } else {
        limit
    };
    let (_, small) = parse_checked(&make(n), 3, what);
    let big_input = make(4 * n);
    let (doc, big) = parse_checked(&big_input, 2, what);
    assert!(
        big < 8.0 * small + 0.1,
        "{what}: superlinear, {small:.4}s for n={n} but {big:.4}s for 4n"
    );
    assert!(
        big < limit,
        "{what}: {} bytes took {big:.3}s",
        big_input.len()
    );
    doc
}

// ---------------------------------------------------------------------------
// Foreign content: breakout end tags at integration points.

#[test]
fn end_p_breakout_at_svg_html_integration_point_creates_paragraph() {
    // `</p>` is reprocessed with the "in body" rules, which insert an empty
    // <p> (the integration point is a scope boundary) instead of dropping it.
    assert_eq!(
        body("<svg><foreignObject></p>x</foreignObject></svg>"),
        "<svg><foreignObject><p></p>x</foreignObject></svg>"
    );
    assert_eq!(
        body("<svg><desc></p></desc><title></p></title></svg>"),
        "<svg><desc><p></p></desc><title><p></p></title></svg>"
    );
}

#[test]
fn end_br_breakout_at_mathml_text_integration_point_creates_br() {
    assert_eq!(
        body("<math><mi></br>x</mi></math>"),
        "<math><mi><br></br>x</mi></math>"
    );
    assert_eq!(
        body("<svg><foreignObject></br></foreignObject></svg>"),
        "<svg><foreignObject><br></br></foreignObject></svg>"
    );
    // Inside a plain foreign element the breakout still pops to HTML content.
    assert_eq!(
        body("<math><mrow></br>"),
        "<math><mrow></mrow></math><br></br>"
    );
}

// ---------------------------------------------------------------------------
// Hostile inputs that used to take quadratic time.

#[test]
fn foster_parenting_around_a_depth_clamped_table_is_linear() {
    // At the depth limit the table's rows become siblings *after* the
    // table, so foster-parented nodes no longer sit next to the end of the
    // parent's child list.
    let n = 10_000;
    let doc = assert_linear(
        |n| "<div>".repeat(300) + "<table>" + &"<tr><br>".repeat(n),
        n,
        0.5,
        "deep table + fostered <br>",
    );
    assert_eq!(doc.get_elements_by_tag_name("br").len(), 4 * n);

    let doc = assert_linear(
        |n| "<div>".repeat(300) + "<table>" + &"<tr>x".repeat(n),
        n,
        0.5,
        "deep table + fostered text",
    );
    let text = doc.text_content(doc.root());
    assert_eq!(text.chars().filter(|&c| c == 'x').count(), 4 * n);

    let doc = assert_linear(
        |n| "<span>".repeat(256) + "<table>" + &"<tbody><img>".repeat(n),
        n,
        0.5,
        "deep table + fostered <img>",
    );
    assert_eq!(doc.get_elements_by_tag_name("img").len(), 4 * n);
}

#[test]
fn cell_and_heading_closing_under_many_templates_is_linear() {
    // Templates are never closed implicitly, so they can pile up on the
    // stack of open elements; closing a cell or a heading must not walk the
    // whole stack from the bottom every time.
    let n = 8_000;
    let doc = assert_linear(
        |n| "<template>".repeat(n) + "<table><tr><td>" + &"<td>x".repeat(n),
        n,
        0.5,
        "templates + cells",
    );
    assert_eq!(doc.get_elements_by_tag_name("td").len(), 4 * n + 1);

    let doc = assert_linear(
        |n| "<template>".repeat(n) + &"<h1>x</h1>".repeat(n),
        n,
        0.5,
        "templates + headings",
    );
    assert_eq!(doc.get_elements_by_tag_name("h1").len(), 4 * n);
}

#[test]
fn unknown_end_tags_under_deep_inline_nesting_are_fast() {
    assert_linear(
        |n| "<span>".repeat(500) + &"</x-unknown>".repeat(n),
        20_000,
        0.5,
        "spans + unknown end tags",
    );
}

#[test]
fn unmatched_end_tags_inside_deep_svg_are_linear() {
    assert_linear(
        |n| "<svg>".to_string() + &"<g>".repeat(500) + &"</x>".repeat(n),
        20_000,
        0.5,
        "deep svg + unmatched end tags",
    );
}

#[test]
fn stray_body_and_html_attribute_merging_is_linear() {
    let attrs =
        |prefix: &str, n: usize| -> String { (0..n).map(|i| format!(" {prefix}{i}")).collect() };
    let doc = assert_linear(
        |n| format!("<body{}><body{}>", attrs("a", n), attrs("b", n)),
        5_000,
        0.5,
        "one big body attribute merge",
    );
    let body_element = doc.body().unwrap();
    assert_eq!(doc.element(body_element).unwrap().attrs.len(), 40_000);
    assert_linear(
        |n| format!("<html{}>", attrs("a", n)) + &"<html x>".repeat(n),
        5_000,
        0.5,
        "many small html attribute merges",
    );
    // Semantics are unchanged: existing attributes win, new ones append.
    assert_eq!(body("<body a=1><body b=2 a=3><body c=4 b=5>"), "");
    let doc = parse_document("<body a=1><body b=2 a=3><body c=4 b=5>");
    let e = doc.element(doc.body().unwrap()).unwrap();
    let pairs: Vec<(&str, &str)> = e
        .attrs
        .iter()
        .map(|a| (a.name.as_str(), a.value.as_str()))
        .collect();
    assert_eq!(pairs, [("a", "1"), ("b", "2"), ("c", "4")]);
}

#[test]
fn foreign_end_tags_match_open_foreign_elements_only() {
    // Matching end tag deeper in the foreign subtree: pops up to it.
    assert_eq!(
        body("<svg><g><rect></G>x</svg>"),
        "<svg><g><rect></rect></g>x</svg>"
    );
    // Case-insensitive match against a case-adjusted name.
    assert_eq!(
        body("<svg><feblend><feFlood></FEBLEND>y"),
        "<svg><feBlend><feFlood></feFlood></feBlend>y</svg>"
    );
    // No such element open: handed to the HTML rules, which ignore it.
    assert_eq!(body("<svg><circle></x>y"), "<svg><circle>y</circle></svg>");
    // An HTML ancestor is closed by the HTML rules, popping the svg too.
    assert_eq!(
        body("<div><svg><g></div>x"),
        "<div><svg><g></g></svg></div>x"
    );
    // From an integration point, the foreign rules still find the root.
    assert_eq!(body("<svg><title></svg>x"), "<svg><title></title></svg>x");
    assert_eq!(body("<math><mi></math>x"), "<math><mi></mi></math>x");
}

// ---------------------------------------------------------------------------
// Memory amplification.

#[test]
fn formatting_reconstruction_cannot_amplify_without_bound() {
    // 64 distinct formatting elements stay in the list of active formatting
    // elements; every paragraph would reconstruct all of them. The number
    // of reconstructed elements is budgeted so that the DOM stays linear in
    // the size of the input.
    let mut input = String::from("<div>");
    for i in 0..64 {
        let _ = write!(input, "<b id={i}>");
    }
    input.push_str("</div>");
    input.push_str(&"<p>x".repeat(100_000));
    let (doc, elapsed) = parse_checked(&input, 1, "reconstruction amplification");
    assert!(elapsed < 4.0, "took {elapsed:.3}s");
    assert!(
        doc.len() <= 2 * input.len(),
        "{} nodes for {} bytes",
        doc.len(),
        input.len()
    );
    // Small pages are unaffected: reconstruction still happens.
    assert_eq!(
        body("<div><b><i></div><p>x"),
        "<div><b><i></i></b></div><p><b><i>x</i></b></p>"
    );
}

#[test]
fn node_count_stays_linear_in_input_size_on_random_markup() {
    const PIECES: &[&str] = &[
        "<b>",
        "<i id=1>",
        "<u class=x>",
        "<s>",
        "<a href=y>",
        "<font color=red>",
        "<nobr>",
        "</b>",
        "</i>",
        "</u>",
        "</s>",
        "</a>",
        "</font>",
        "</nobr>",
        "<p>",
        "</p>",
        "<div>",
        "</div>",
        "<table>",
        "<tr>",
        "<td>",
        "</table>",
        "x",
        " ",
        "<li>",
        "<blockquote>",
        "<em>",
        "<strong>",
        "</em>",
        "</strong>",
        "<code>",
        "<big>",
        "<small>",
    ];
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut worst = 0.0f64;
    for _ in 0..300 {
        let mut input = String::new();
        for _ in 0..2_000 {
            input.push_str(PIECES[rng.below(PIECES.len())]);
        }
        let doc = parse_document(&input);
        assert_well_formed(&doc, "random formatting soup");
        worst = worst.max(doc.len() as f64 / input.len() as f64);
    }
    assert!(worst < 2.0, "worst node/byte ratio {worst}");
}

// ---------------------------------------------------------------------------
// Robustness of the depth-limit code paths.

#[test]
fn fuzz_random_markup_below_the_depth_limit() {
    // Same idea as the general fuzz test, but every input starts at the
    // depth limit so that clamping, foster parenting and the adoption
    // agency algorithm interact with flattened subtrees.
    const PIECES: &[&str] = &[
        "<table>",
        "<tr>",
        "<td>",
        "<th>",
        "</td>",
        "</tr>",
        "</table>",
        "<tbody>",
        "<caption>",
        "<colgroup>",
        "<col>",
        "<b>",
        "</b>",
        "<i>",
        "</i>",
        "<a>",
        "</a>",
        "<p>",
        "</p>",
        "x",
        " ",
        "<div>",
        "</div>",
        "<svg>",
        "</svg>",
        "<foreignObject>",
        "<math>",
        "<mi>",
        "<select>",
        "<option>",
        "<template>",
        "</template>",
        "<br>",
        "<img>",
        "<li>",
        "<ul>",
        "<button>",
        "<form>",
        "</form>",
        "<h1>",
        "</h1>",
        "<nobr>",
        "<object>",
        "</object>",
        "<textarea>",
        "</textarea>",
        "<!--c-->",
        "</p>",
        "</br>",
        "<frameset>",
        "<hr>",
    ];
    let prefixes = [
        "<div>".repeat(300),
        "<span>".repeat(255),
        "<b>".repeat(270),
        "<table><tr><td>".repeat(90),
    ];
    let mut rng = Rng(0xC0FF_EE00_1234_5678);
    for i in 0..400 {
        let mut input = prefixes[i % prefixes.len()].clone();
        for _ in 0..rng.below(120) {
            input.push_str(PIECES[rng.below(PIECES.len())]);
        }
        let doc = parse_document(&input);
        assert_well_formed(&doc, &input[input.len().saturating_sub(200)..]);
    }
}

// ---------------------------------------------------------------------------
// Input handling.

#[test]
fn leading_byte_order_mark_in_string_is_ignored() {
    let doc = parse_document("\u{FEFF}<!DOCTYPE html><p>x");
    assert!(!doc.quirks_mode, "a BOM must not force quirks mode");
    assert_eq!(doc.text_content(doc.body().unwrap()), "x");
    // Only one leading BOM is stripped; others are content.
    let doc = parse_document("\u{FEFF}\u{FEFF}x");
    assert_eq!(doc.text_content(doc.body().unwrap()), "\u{FEFF}x");
}

#[test]
fn decoding_utf8_cut_off_mid_character_stays_utf8() {
    // A document truncated in the middle of a multi-byte character is
    // still UTF-8 (only the incomplete tail is replaced).
    let mut bytes = "<p>café — été".as_bytes().to_vec();
    bytes.extend_from_slice(&"é".as_bytes()[..1]);
    assert_eq!(decode_html_bytes(&bytes, None), "<p>café — été\u{FFFD}");
    // A stray invalid byte in the middle still means windows-1252.
    assert_eq!(decode_html_bytes(b"caf\xE9 \xC3\xA9", None), "café Ã©");
}

#[test]
fn transport_charset_labels_are_trimmed_and_unquoted() {
    // KOI8-R 0xC1 is U+0430 (Cyrillic a); windows-1252 would give U+00C1.
    assert_eq!(decode_html_bytes(b"\xC1", Some("\"koi8-r\"")), "\u{430}");
    assert_eq!(decode_html_bytes(b"\xC1", Some(" 'KOI8-R' ")), "\u{430}");
    assert_eq!(decode_html_bytes(b"\xC1", Some("koi8-r")), "\u{430}");
    // An empty quoted label is ignored.
    assert_eq!(decode_html_bytes("café".as_bytes(), Some("\"\"")), "café");
}

#[test]
fn meta_charset_prescan_follows_attribute_order() {
    // `charset` after a `content` charset overrides it (the spec's
    // "get an attribute" loop), and need-pragma becomes false.
    let doc = b"<meta content='text/html; charset=koi8-r' charset=iso-8859-2>\xB1";
    assert_eq!(decode_html_bytes(doc, None).chars().last(), Some('\u{105}'));
    // A duplicate attribute is ignored (the first one wins).
    let doc = b"<meta charset=iso-8859-2 charset=koi8-r>\xB1";
    assert_eq!(decode_html_bytes(doc, None).chars().last(), Some('\u{105}'));
}

// ---------------------------------------------------------------------------
// Property-style checks of the tokenizer and the entity decoder.

#[test]
fn tokenizer_always_makes_progress_and_never_emits_empty_text() {
    const ALPHABET: &[&str] = &[
        "<", ">", "/", "=", "\"", "'", "&", ";", "!", "-", "?", "[", "]", "a", "b", "x", "1", " ",
        "\n", "\r", "\0", "é", "日", "script", "style", "CDATA", "DOCTYPE", "#x", "amp", "--",
        "<!--", "-->", "</",
    ];
    let modes = [
        TextMode::Data,
        TextMode::RcData,
        TextMode::RawText,
        TextMode::ScriptData,
        TextMode::PlainText,
    ];
    let mut rng = Rng(0xFEED_FACE_DEAD_BEEF);
    for i in 0..3_000 {
        let mut input = String::new();
        for _ in 0..rng.below(80) {
            input.push_str(ALPHABET[rng.below(ALPHABET.len())]);
        }
        let mut t = Tokenizer::new(&input);
        t.set_mode(modes[i % modes.len()], "script");
        t.set_cdata_allowed(i % 3 == 0);
        let mut count = 0usize;
        loop {
            let token = t.next_token();
            match &token {
                Token::Eof => break,
                Token::Text(s) => assert!(!s.is_empty(), "empty text for {input:?}"),
                Token::StartTag { name, attrs, .. } => {
                    assert!(!name.is_empty());
                    assert!(name.bytes().all(|b| !b.is_ascii_uppercase()));
                    for (k, a) in attrs.iter().enumerate() {
                        assert!(a.name.bytes().all(|b| !b.is_ascii_uppercase()));
                        assert!(
                            attrs[..k].iter().all(|o| o.name != a.name),
                            "duplicate attribute for {input:?}"
                        );
                    }
                }
                _ => {}
            }
            count += 1;
            assert!(count <= input.len() + 1, "no progress on {input:?}");
        }
        // EOF is sticky.
        assert_eq!(t.next_token(), Token::Eof);
    }
}

#[test]
fn entity_decoding_properties() {
    let mut rng = Rng(0x0BAD_CAFE_0000_0001);
    let alphabet: Vec<char> = "&#;xX0123456789abcdefABCDEFamplgtnotinéé日 ="
        .chars()
        .collect();
    for _ in 0..5_000 {
        let s: String = (0..rng.below(40))
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        for in_attr in [false, true] {
            let decoded = entities::decode(&s, in_attr);
            if !s.contains('&') {
                assert_eq!(decoded, s);
            }
            // Decoding only ever shortens references: never longer than
            // the input by more than the longest expansion per '&'.
            let amps = s.matches('&').count();
            assert!(decoded.len() <= s.len() + amps * 4);
        }
    }
    // Every legacy name is decoded without a semicolon in text.
    for name in [
        "amp", "lt", "gt", "quot", "copy", "reg", "nbsp", "eacute", "yuml",
    ] {
        assert!(entities::is_legacy(name));
        assert_eq!(
            entities::decode(&format!("&{name}"), false),
            entities::lookup(name).unwrap()
        );
    }
}
