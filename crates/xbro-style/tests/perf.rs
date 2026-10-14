//! Performance checks. The full-size benchmark (15,000 elements, ~3,000
//! rules, < 300 ms) only runs in release builds:
//! `cargo test -p xbro-style --release --test perf`.

use std::time::{Duration, Instant};

use xbro_style::{compute_styles, StyleContext};

/// A Wikipedia-like document with about `sections * 75` elements.
fn document(sections: usize) -> String {
    let mut html = String::from(
        "<!DOCTYPE html><html><head><title>t</title></head><body class=\"skin mediawiki\">\
         <div id=content class=\"mw-body\"><div class=\"mw-parser-output\">",
    );
    for s in 0..sections {
        html.push_str(&format!(
            "<h2 id=h{s}><span class=\"mw-headline c{}\">Section {s}</span></h2>",
            s % 97
        ));
        html.push_str("<div class=\"thumb tright\"><div class=thumbinner><a href=\"#\" class=image>\
                       <img src=x.png width=100 height=80></a><div class=thumbcaption>cap</div></div></div>");
        for p in 0..4 {
            html.push_str(&format!("<p class=\"para p{p}\">Text <b>bold</b> and <a href=\"/wiki/{s}_{p}\" class=\"link c{}\" title=t>link</a> \
                 <i>it</i> <span class=\"ref c{}\"><sup><a href=#r>[1]</a></sup></span> more text.</p>",
                (s + p) % 97,
                (s * 7 + p) % 89
            ));
        }
        html.push_str("<ul class=list>");
        for i in 0..5 {
            html.push_str(&format!(
                "<li class=\"item i{i}\"><a href=#x class=c{}>item</a> <code>c</code></li>",
                (s + i) % 50
            ));
        }
        html.push_str("</ul><table class=\"wikitable sortable\"><tbody>");
        for r in 0..4 {
            html.push_str(&format!(
                "<tr class=r{r}><th scope=row>h</th><td class=num>1</td><td><a href=#t>t</a></td><td style=\"color: red\">x</td></tr>"
            ));
        }
        html.push_str("</tbody></table>");
    }
    html.push_str("</div></div></body></html>");
    html
}

/// About `n` rules of the usual kinds (classes, descendants, ids, attributes, pseudo-classes).
fn stylesheet(n: usize) -> String {
    let mut css = String::from(
        ":root { --accent: #36c; --space: 8px } body { margin: 0; font: 14px/1.6 sans-serif }
         a { color: var(--accent) } a:hover { text-decoration: underline }
         .mw-parser-output table { border-collapse: collapse }
         .wikitable > tbody > tr > th, .wikitable > tbody > tr > td { border: 1px solid #a2a9b1; padding: 0.2em 0.4em }
         .thumb { margin: var(--space) } .tright { float: right; clear: right }
         p:first-child::first-line { color: red } li:nth-child(2n+1) { background: #f8f9fa }
         * { box-sizing: border-box }
         @media (max-width: 720px) { .thumb { float: none } }",
    );
    for i in 0..n {
        let rule = match i % 10 {
            0 => format!(".c{} {{ color: #{:06x} }}", i % 500, i * 977 % 0xffffff),
            1 => format!(".mw-parser-output .x{i} {{ margin: {}px }}", i % 20),
            2 => format!("div.k{i} > p {{ padding: 1px {}px }}", i % 9),
            3 => format!("#id{i} {{ width: {}% }}", i % 100),
            4 => format!("[data-n=\"{i}\"] {{ display: none }}"),
            5 => format!("ul.l{i} li a:hover {{ color: red }}"),
            6 => format!(".a{i} .b{i} .c{} {{ font-weight: bold }}", i % 97),
            7 => format!("table.t{i} td:nth-child(2) {{ text-align: right }}"),
            8 => format!(".s{i}::before {{ content: '{i}' }}"),
            _ => {
                format!("section:is(.q{i}, .r{i}) h2 {{ border-bottom: 1px solid var(--accent) }}")
            }
        };
        css.push_str(&rule);
        css.push('\n');
    }
    css
}

fn run(sections: usize, rules: usize) -> (usize, Duration) {
    let doc = xbro_html::parse_document(&document(sections));
    let sheet = xbro_css::parse_stylesheet(&stylesheet(rules));
    let elements = doc.ids().filter(|&i| doc.node(i).is_element()).count();
    // Warm up the user-agent rule set (built once per process).
    let _ = compute_styles(
        &xbro_html::parse_document("<p>x</p>"),
        &[],
        &StyleContext::default(),
    );
    let start = Instant::now();
    let map = compute_styles(&doc, std::slice::from_ref(&sheet), &StyleContext::default());
    let elapsed = start.elapsed();
    assert!(map.styles.iter().filter(|s| s.is_some()).count() > elements / 2);
    (elements, elapsed)
}

#[test]
fn small_document_debug_smoke() {
    let (elements, elapsed) = run(30, 500);
    assert!(elements > 2_000, "{elements}");
    eprintln!("{elements} elements styled in {elapsed:?}");
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
}

#[test]
#[cfg_attr(debug_assertions, ignore = "benchmark: run with --release")]
fn large_document_under_300ms() {
    let (elements, elapsed) = run(200, 3_000);
    assert!(elements >= 15_000, "{elements}");
    eprintln!("{elements} elements, 3000 rules styled in {elapsed:?}");
    assert!(elapsed < Duration::from_millis(300), "{elapsed:?}");
}

#[test]
fn has_in_ancestor_compounds_is_evaluated_once_per_anchor() {
    // `:has()` left of a descendant combinator is evaluated on the
    // ancestors of every element; the cascade shares its results between
    // elements (1.8 s for 21 000 elements when each paragraph re-walked
    // the whole page; 330 ms for a 256-deep chain found by fuzzing).
    let wide = format!(
        "<div class=page>{}</div>",
        "<section><p>para <span>x</span> <b>y</b></p><p>two</p></section>".repeat(3000)
    );
    let deep = "<div class=c1><span><b>".repeat(100) + "text";
    let css = ".page:has(.missing) p { color: red } .page:has(.sidebar ~ .x) span { color: blue } \
               :has(:link ~ *:hover > .c6) * { color: red } div:has(.c6) b { margin: 1px } \
               section:has(> b, span) p { color: green }";
    let sheet = xbro_css::parse_stylesheet(css);
    for html in [&wide, &deep] {
        let doc = xbro_html::parse_document(html);
        let start = Instant::now();
        let map = compute_styles(&doc, std::slice::from_ref(&sheet), &StyleContext::default());
        let elapsed = start.elapsed();
        assert!(map.styles.iter().any(Option::is_some));
        eprintln!("{} nodes styled in {elapsed:?}", doc.len());
        let budget = if cfg!(debug_assertions) { 3000 } else { 300 };
        assert!(elapsed < Duration::from_millis(budget), "{elapsed:?}");
    }
    // The shared results are right: `section:has(> b, span) p` matches.
    let doc = xbro_html::parse_document(
        "<section><p id=a>x</p><span></span></section><section><p id=b>y</p></section>",
    );
    let map = compute_styles(&doc, std::slice::from_ref(&sheet), &StyleContext::default());
    let color = |id: &str| map.get(doc.get_element_by_id(id).unwrap()).unwrap().color;
    assert_eq!(color("a"), xbro_style::Color::rgb(0, 128, 0));
    assert_ne!(color("b"), xbro_style::Color::rgb(0, 128, 0));
}
