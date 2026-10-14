//! Arbitrary input must never panic (or take unbounded time).

use std::time::{Duration, Instant};

use xbro_style::{compute_styles, StyleContext};

/// Deterministic pseudo-random generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[(self.next() % items.len() as u64) as usize]
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const PROPERTIES: &[&str] = &[
    "display",
    "position",
    "top",
    "inset",
    "inset-inline",
    "z-index",
    "float",
    "clear",
    "width",
    "min-width",
    "max-width",
    "height",
    "inline-size",
    "margin",
    "margin-block",
    "padding",
    "padding-inline-start",
    "border",
    "border-top",
    "border-inline",
    "border-width",
    "border-style",
    "border-color",
    "border-radius",
    "border-top-left-radius",
    "overflow",
    "overflow-x",
    "aspect-ratio",
    "visibility",
    "opacity",
    "background",
    "background-color",
    "background-image",
    "background-position",
    "background-position-x",
    "background-size",
    "background-repeat",
    "outline",
    "cursor",
    "color",
    "font",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "text-align",
    "text-indent",
    "text-transform",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "text-wrap",
    "word-break",
    "overflow-wrap",
    "text-decoration",
    "list-style",
    "list-style-type",
    "border-collapse",
    "border-spacing",
    "vertical-align",
    "flex",
    "flex-flow",
    "flex-basis",
    "order",
    "justify-content",
    "align-items",
    "align-self",
    "place-content",
    "place-items",
    "gap",
    "grid-template-columns",
    "grid-template-rows",
    "grid-template",
    "grid",
    "grid-column",
    "grid-row",
    "grid-row-start",
    "grid-column-end",
    "grid-area",
    "table-layout",
    "caption-side",
    "content",
    "all",
    "--a",
    "--b",
    "--c",
    "box-sizing",
    "text-decoration-color",
];

const VALUES: &[&str] = &[
    "0",
    "1",
    "-1",
    "1.5",
    "1e40",
    "-1e40",
    "10px",
    "-3em",
    "50%",
    "2rem",
    "3vw",
    "1ch",
    "auto",
    "none",
    "inherit",
    "initial",
    "unset",
    "revert",
    "red",
    "#abc",
    "rgb(1 2 3 / 50%)",
    "currentcolor",
    "solid",
    "dashed",
    "thin",
    "thick",
    "bold",
    "italic",
    "serif",
    "\"A B\"",
    "url(x.png)",
    "url(",
    "calc(",
    "calc(1px + 2%)",
    "calc(100% / 0)",
    "calc(1px * 2px)",
    "min(1px, 2%)",
    "max(1em, 3px)",
    "clamp(1px, 2vw, 3em)",
    "var(--a)",
    "var(--b, 3px)",
    "var(--c, var(--a))",
    "var(",
    "env(safe-area-inset-top)",
    "linear-gradient(to right, red, blue)",
    "linear-gradient(45deg, red 10%, 20px, blue)",
    "radial-gradient(red, blue)",
    "repeat(3, 1fr)",
    "repeat(auto-fill, minmax(1px, 1fr))",
    "repeat(2147483647, 1em 2px)",
    "repeat(auto-fit, 1em)",
    "fit-content(1em)",
    "minmax(auto, 1fr)",
    "subgrid",
    "auto-flow",
    "dense",
    "\"a b\"",
    "[a b]",
    "span",
    "span 2",
    "-1",
    "1fr",
    "fixed",
    "break-word",
    "anywhere",
    "break-all",
    "/",
    ",",
    "!",
    "attr(title)",
    "open-quote",
    "counter(x)",
    "flex",
    "inline grid",
    "block flow list-item",
    "contents",
    "table-cell",
    "center",
    "left",
    "bottom",
    "space-between",
    "first baseline",
    "safe end",
    "nowrap",
    "pre",
    "break-spaces",
    "underline",
    "line-through red",
    "square inside",
    "1 / 2",
    "16/9",
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    ";",
    "@media",
    "\\",
    "'",
    "--x",
];

fn random_css(rng: &mut Rng, rules: usize) -> String {
    let selectors = [
        "*",
        "div",
        ".a",
        "#b",
        "p > span",
        "div .a ~ p",
        ":root",
        "a:hover",
        "li:nth-child(2n+1)",
        ":is(.a, .b) p",
        ":not(.x)",
        "p::before",
        "p::after",
        "[data-x]",
        "[title=\"y\" i]",
        "td:first-child",
        ":has(> .a)",
        "& .z",
        "x\\",
        ":hover:focus-within",
    ];
    let mut css = String::new();
    for _ in 0..rules {
        if rng.below(10) == 0 {
            css.push_str(rng.pick(&[
                "@media (min-width: 500px) {",
                "@supports (display: grid) {",
                "@layer x {",
            ]));
        }
        css.push_str(rng.pick(&selectors));
        css.push_str(" { ");
        for _ in 0..rng.below(6) {
            css.push_str(rng.pick(PROPERTIES));
            css.push(':');
            for _ in 0..=rng.below(4) {
                css.push(' ');
                css.push_str(rng.pick(VALUES));
            }
            if rng.below(8) == 0 {
                css.push_str(" !important");
            }
            css.push(';');
        }
        css.push_str("}\n");
        if rng.below(10) == 0 {
            css.push('}');
        }
    }
    css
}

fn random_html(rng: &mut Rng, elements: usize) -> String {
    let tags = [
        "div", "p", "span", "a", "ul", "li", "table", "tr", "td", "th", "img", "font", "hr", "pre",
        "input", "button", "b", "i", "h1", "q", "center", "select", "option", "body", "br", "col",
    ];
    let attrs = [
        "class=a",
        "class='a b'",
        "id=b",
        "style='color: red; width: var(--a)'",
        "hidden",
        "bgcolor=#ff0",
        "width=50%",
        "height=0",
        "align=center",
        "valign=top",
        "border",
        "cellpadding=3",
        "size=+9",
        "color=nonsense",
        "face=''",
        "type=A",
        "data-x=1",
        "title=y",
        "href=#",
        "style='--a: var(--a)'",
        "style='font: 1e30px serif'",
        "nowrap",
        "hspace=-5",
        "cellspacing=999999999999",
    ];
    let mut html = String::from("<!DOCTYPE html>");
    for _ in 0..elements {
        let tag = rng.pick(&tags);
        html.push('<');
        html.push_str(tag);
        for _ in 0..rng.below(3) {
            html.push(' ');
            html.push_str(rng.pick(&attrs));
        }
        html.push('>');
        if rng.below(3) == 0 {
            html.push_str("text ");
        }
        if rng.below(4) == 0 {
            html.push_str(&format!("</{tag}>"));
        }
    }
    html
}

#[test]
fn random_documents_and_sheets() {
    // `XBRO_FUZZ_ROUNDS=5000 cargo test --release --test robustness` for a longer run.
    let rounds: u64 = std::env::var("XBRO_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let seed = std::env::var("XBRO_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x9e37_79b9_7f4a_7c15u64);
    let mut rng = Rng(seed | 1);
    for round in 0..rounds {
        let html = random_html(&mut rng, 150);
        let css = random_css(&mut rng, 60);
        let doc = xbro_html::parse_document(&html);
        let sheet = xbro_css::parse_stylesheet(&css);
        let hover = xbro_dom::NodeId(rng.below(doc.len() as u64 + 10) as usize);
        let ctx = StyleContext {
            hover: Some(hover),
            focus: Some(hover),
            active: Some(xbro_dom::NodeId(usize::MAX)),
            ..StyleContext::default()
        };
        let map = compute_styles(&doc, &[sheet], &ctx);
        assert_eq!(map.styles.len(), doc.len(), "round {round}");
        for style in map.styles.iter().flatten() {
            assert!(style.font_size.is_finite() && style.font_size >= 0.0);
            assert!(style.opacity >= 0.0 && style.opacity <= 1.0);
            assert!(style.letter_spacing.is_finite());
            for w in [style.border.top.width, style.outline_width] {
                assert!(w.is_finite() && w >= 0.0);
            }
        }
    }
}

#[test]
fn pathological_inputs_are_bounded() {
    let start = Instant::now();
    // Deeply nested calc, huge var() fan-out, very long declarations.
    let mut css = String::from("p { width: ");
    css.push_str(&"calc(".repeat(500));
    css.push_str("1px");
    css.push_str(&")".repeat(500));
    css.push_str("; }\n:root {");
    css.push_str("--v0: x x x x x x x x x x x x x x x x;");
    for i in 1..30 {
        css.push_str(&format!(
            "--v{i}: {};",
            format!("var(--v{}) ", i - 1).repeat(16)
        ));
    }
    css.push_str("} p { content: var(--v29); margin: var(--v29); }\n");
    css.push_str("div { font-family: ");
    css.push_str(&"a, ".repeat(10_000));
    css.push_str("serif }\n");
    let mut many_classes = String::from("<p class='");
    for i in 0..5_000 {
        many_classes.push_str(&format!("c{i} "));
    }
    many_classes.push_str("'>x</p>");
    let mut sheet_css = css.clone();
    for i in 0..2_000 {
        sheet_css.push_str(&format!(".c{i} {{ --x{i}: {i}px }}\n"));
    }
    let html = format!(
        "<!DOCTYPE html><div>{many_classes}{}</div>",
        "<p>y</p>".repeat(500)
    );
    let doc = xbro_html::parse_document(&html);
    let map = compute_styles(
        &doc,
        &[xbro_css::parse_stylesheet(&sheet_css)],
        &StyleContext::default(),
    );
    assert!(map.styles.iter().any(|s| s.is_some()));
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn wide_lists_with_sibling_selectors() {
    let mut html = String::from("<!DOCTYPE html><ul>");
    for _ in 0..5_000 {
        html.push_str("<li class=i>x</li>");
    }
    html.push_str("</ul>");
    let doc = xbro_html::parse_document(&html);
    let sheet = xbro_css::parse_stylesheet(
        "li:nth-child(odd) { color: red } li + li { margin: 1px } .i ~ .i { padding: 1px }
         li:nth-last-of-type(3n) { color: blue } li:last-child { color: green }",
    );
    let start = Instant::now();
    let map = compute_styles(&doc, &[sheet], &StyleContext::default());
    assert!(map.styles.iter().filter(|s| s.is_some()).count() > 5_000);
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
}
