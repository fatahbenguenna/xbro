//! Parser tests: linear-time block parsing, declaration/rule disambiguation
//! per CSS Syntax §5, CSS Nesting limits and specificity, `@import`
//! placement, and fuzzing of every public entry point.

mod support;

use std::time::{Duration, Instant};

use support::Rng;
use xbro_css::parser::{parse_component_values, parse_declaration_list, to_css, CssRule};
use xbro_css::*;

fn style_rules(rules: &[CssRule]) -> Vec<&StyleRule> {
    let mut out = Vec::new();
    for rule in rules {
        match rule {
            CssRule::Style(s) => out.push(s),
            CssRule::Media { rules, .. }
            | CssRule::Supports { rules, .. }
            | CssRule::Group { rules } => out.extend(style_rules(rules)),
            CssRule::Import { .. } => {}
        }
    }
    out
}

/// `(name, value as CSS, important)` of each declaration.
type Decls = Vec<(String, String, bool)>;

fn decls(rule: &StyleRule) -> Decls {
    rule.declarations
        .iter()
        .map(|d| (d.name.clone(), to_css(&d.value), d.important))
        .collect()
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed())
}

/// Parsing time must grow linearly: 4x the input may take at most ~8x the
/// time (generous for timer noise), whereas the old quadratic scan took 16x.
fn assert_linear(make: impl Fn(usize) -> String, expected_rules: impl Fn(usize) -> usize) {
    let small = 5_000;
    let large = 4 * small;
    // Warm up allocations, then keep the best of a few runs.
    let best = |n: usize| {
        let css = make(n);
        (0..3)
            .map(|_| {
                let (sheet, elapsed) = timed(|| parse_stylesheet(&css));
                assert_eq!(style_rules(&sheet.rules).len(), expected_rules(n));
                elapsed
            })
            .min()
            .unwrap_or_default()
    };
    let t_small = best(small).max(Duration::from_micros(500));
    let t_large = best(large);
    assert!(
        t_large < t_small * 8,
        "{small} rules: {t_small:?}, {large} rules: {t_large:?}"
    );
}

#[test]
fn rules_inside_conditional_blocks_parse_in_linear_time() {
    // A whole framework wrapped in one `@layer` / `@media` block is common
    // (Tailwind); the block's contents used to be rescanned for each rule.
    assert_linear(
        |n| format!("@layer utilities {{ {} }}", ".a{color:red}\n".repeat(n)),
        |n| n,
    );
    assert_linear(
        |n| format!("@media screen {{ {} }}", ".b:hover > i{x:y;z:w}".repeat(n)),
        |n| n,
    );
}

#[test]
fn nested_rules_parse_in_linear_time() {
    // `a:hover {}` starts like a declaration (`ident :`), which used to be
    // scanned up to the next `;`, i.e. to the end of the parent block.
    assert_linear(
        |n| format!(".p {{ {} }}", "a:hover{color:red}\n".repeat(n)),
        |n| n,
    );
    assert_linear(
        |n| format!(".p {{ {} }}", "a:not(.x) b{c:d} e:f;".repeat(n)),
        |n| 2 * n,
    );
}

#[test]
fn curly_block_values_follow_the_spec_order() {
    // §5.4.6: `!important` is removed before checking that a `{}` block is
    // the whole value.
    let sheet = parse_stylesheet("a { b: {c} !important; d: e }");
    assert_eq!(
        decls(style_rules(&sheet.rules)[0]),
        vec![
            ("b".into(), "{c}".into(), true),
            ("d".into(), "e".into(), false)
        ]
    );
    let sheet = parse_stylesheet("a { b: { c } ! IMPORTANT ; d: e }");
    assert_eq!(decls(style_rules(&sheet.rules)[0]).len(), 2);
    // Anything else next to the block makes it a (here invalid) nested rule.
    for css in [
        "a { b: x {c} ; d: e }",
        "a { b: {c} x; d: e }",
        "a { b: {c} {d}; d: e }",
        "a { b: {c} !important x; d: e }",
        "a { b: ! {c}; d: e }",
    ] {
        let sheet = parse_stylesheet(css);
        let rules = style_rules(&sheet.rules);
        let all: Vec<_> = rules.iter().flat_map(|r| decls(r)).collect();
        assert_eq!(all, vec![("d".into(), "e".into(), false)], "{css}");
    }
    // Custom properties accept any block placement.
    let sheet = parse_stylesheet("a { --x: {a} b {c}; d: e }");
    assert_eq!(
        decls(style_rules(&sheet.rules)[0])[0],
        ("--x".into(), "{a} b {c}".into(), false)
    );
}

#[test]
fn declaration_like_preludes_become_nested_rules() {
    let sheet = parse_stylesheet(
        ".p { color: red; a:hover { x: 1 } b:is(.c) { y: 2 } font: bold; div:not(.d):focus{z:3} }",
    );
    let rules = style_rules(&sheet.rules);
    let shapes: Vec<(usize, Decls)> = rules
        .iter()
        .map(|r| (r.selectors.len(), decls(r)))
        .collect();
    assert_eq!(
        shapes,
        vec![
            (1, vec![("color".into(), "red".into(), false)]),
            (1, vec![("x".into(), "1".into(), false)]),
            (1, vec![("y".into(), "2".into(), false)]),
            (1, vec![("font".into(), "bold".into(), false)]),
            (1, vec![("z".into(), "3".into(), false)]),
        ]
    );
    assert_eq!(
        rules[1].selectors,
        parse_selector_list(".p a:hover").expect("valid")
    );
    // The style attribute ignores nested rules but keeps what follows them.
    let list = parse_declaration_list("a:hover{x:y} color: red; b {c:d} margin: 0");
    let names: Vec<&str> = list.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, vec!["color", "margin"]);
}

#[test]
fn deep_nesting_with_complex_parents_reaches_the_documented_limit() {
    // Each level is `.b .b ... &` (63 compounds); the parent is complex and
    // `&` is not leftmost, so `:is(parent)` is used. Each `&` used to be
    // charged twice, halving the documented 2048-compound limit.
    for levels in [8, 18, 24, 31] {
        let mut css = String::from(".r {");
        for _ in 0..levels {
            css.push_str(&format!(" {} & {{", ".b ".repeat(62)));
        }
        css.push_str(" x: y");
        css.push_str(&"}".repeat(levels + 1));
        let sheet = parse_stylesheet(&css);
        let rules = style_rules(&sheet.rules);
        assert_eq!(rules.len(), 1, "{levels} levels");
        let weight = selector::list_weight(&rules[0].selectors);
        assert_eq!(weight, 63 * levels, "{levels} levels");
    }
}

#[test]
fn many_nested_rules_under_a_huge_parent_are_linear() {
    // Every nested rule used to filter, clone and weigh the whole parent
    // list before failing: 20,000 parent selectors with 40,000 children took
    // minutes. The parent is now prepared once per rule.
    for (parents, children) in [(2_000, 40_000), (20_000, 40_000)] {
        for child in ["!{}", "b{}", "@media screen{c:d}", "&:hover{e:f}"] {
            let css = format!(
                "{}.z::before {{ g: h; {} }}",
                ".a,".repeat(parents),
                child.repeat(children)
            );
            let (sheet, elapsed) = timed(|| parse_stylesheet(&css));
            assert!(
                elapsed < Duration::from_secs(2),
                "{parents} parents x {children} {child:?}: {elapsed:?}"
            );
            // The parent's own declarations are always kept; nested copies
            // stop at the nesting budget.
            let rules = style_rules(&sheet.rules);
            assert_eq!(decls(rules[0]), vec![("g".into(), "h".into(), false)]);
            assert!(rules.len() < 250, "{} rules", rules.len());
        }
    }
    // Below the limits, nested rules under a big parent still resolve.
    let css = format!("{} .last {{ i {{ j: k }} }}", ".a,".repeat(1_000));
    let rules = style_rules(&parse_stylesheet(&css).rules).len();
    assert_eq!(rules, 1);
}

#[test]
fn nesting_selector_keeps_its_parent_specificity() {
    let cases = [
        // `div&` is `div:is(div)`, not `div`.
        ("div { div& { x: y } }", Specificity(0, 0, 2)),
        ("div { &.x { x: y } }", Specificity(0, 1, 1)),
        ("div.a { p& { x: y } }", Specificity(0, 1, 2)),
        (".a { & & { x: y } }", Specificity(0, 2, 0)),
        ("#i .a { &:hover .b { x: y } }", Specificity(1, 3, 0)),
        (".a, #b { .c { x: y } }", Specificity(1, 1, 0)),
    ];
    for (css, expected) in cases {
        let sheet = parse_stylesheet(css);
        let rules = style_rules(&sheet.rules);
        assert_eq!(rules.len(), 1, "{css}");
        assert_eq!(rules[0].selectors[0].specificity(), expected, "{css}");
    }
}

#[test]
fn import_placement() {
    let imports = |css: &str| -> Vec<String> {
        parse_stylesheet(css)
            .rules
            .iter()
            .filter_map(|r| match r {
                CssRule::Import { url, .. } => Some(url.clone()),
                _ => None,
            })
            .collect()
    };
    // Unknown at-rules and invalid style rules are dropped by the parser and
    // do not count as preceding rules.
    assert_eq!(
        imports("@-ms-viewport { width: device-width } @foo; ]] {} @import 'a.css';"),
        vec!["a.css"]
    );
    assert_eq!(
        imports("@charset 'utf-8'; @layer x, y; @import url(b.css) screen;"),
        vec!["b.css"]
    );
    // Valid rules, even ones xbro does not apply, close the import section.
    for css in [
        "@namespace svg url(http://www.w3.org/2000/svg); @import 'c.css';",
        "@font-face { font-family: x } @import 'c.css';",
        "@-webkit-keyframes k { } @import 'c.css';",
        "@media print {} @import 'c.css';",
        "@layer l {} @import 'c.css';",
        "a {} @import 'c.css';",
    ] {
        assert!(imports(css).is_empty(), "{css}");
    }
}

// ---------------------------------------------------------------------------
// Fuzzing.
// ---------------------------------------------------------------------------

const FRAGMENTS: &[&str] = &[
    "a",
    "div",
    ".x",
    "#y",
    "[z=\"1\" i]",
    ":hover",
    "::before",
    ":is(",
    ":not(",
    ":has(> ",
    ":nth-child(2n+1",
    "&",
    " ",
    " > ",
    " + ",
    " ~ ",
    ",",
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    ";",
    ":",
    "!important",
    "color",
    "--v",
    "red",
    "1px",
    "50%",
    "url(x.png)",
    "url(",
    "\"s",
    "'t'",
    "\\",
    "\\31 ",
    "/*",
    "*/",
    "<!--",
    "-->",
    "@media",
    "@supports",
    "@import",
    "@layer",
    "@font-face",
    "@container",
    "screen",
    "and",
    "(min-width: 100px)",
    "not",
    "or",
    "only",
    "var(--v)",
    "calc(1px + 2%)",
    "rgb(1 2 3 / 50%)",
    "\n",
    "\0",
    "\u{e9}",
    "\u{1F600}",
    "e3",
    "-",
    "+",
    ".",
    "#",
    "@",
    "*",
    "|",
    "=",
    "^=",
    "$=",
    "*=",
    "~=",
    "|=",
];

fn random_css(rng: &mut Rng, len: usize) -> String {
    let mut css = String::new();
    for _ in 0..len {
        css.push_str(rng.pick(FRAGMENTS));
        if rng.chance(30) {
            css.push(' ');
        }
    }
    css
}

/// Checks the structural invariants of a parsed sheet.
fn check_sheet(sheet: &Stylesheet, css: &str) {
    for rule in style_rules(&sheet.rules) {
        assert!(
            !rule.selectors.is_empty(),
            "empty selector list for {css:?}"
        );
        for selector in &rule.selectors {
            assert!(selector.ancestors.len() < selector::MAX_COMPOUNDS);
            let _ = selector.specificity();
        }
        for decl in &rule.declarations {
            assert!(!decl.name.is_empty());
            if !decl.name.starts_with("--") {
                assert_eq!(decl.name, decl.name.to_ascii_lowercase());
            }
            // Declaration values survive a serialization round trip.
            let text = to_css(&decl.value);
            assert_eq!(
                to_css(&parse_component_values(&text)),
                text,
                "value of {:?} in {css:?}",
                decl.name
            );
        }
    }
}

#[test]
fn random_stylesheets_never_panic_and_keep_invariants() {
    let mut rng = Rng::new(0xf022);
    for _ in 0..3000 {
        let len = 1 + rng.below(60);
        let css = random_css(&mut rng, len);
        let mut sheet = parse_stylesheet(&css);
        check_sheet(&sheet, &css);
        sheet.rewrite_urls(&mut |u| format!("http://h/{u}"));
        let _ = parse_declaration_list(&css);
        let _ = parse_selector_list(&css);
        let _ = MediaQueryList::parse_str(&css).matches(&MediaContext::default());
        let _ = Color::parse_str(&css);
        let _ = Color::parse_legacy(&css);
    }
}

#[test]
fn random_unicode_never_panics() {
    // Arbitrary code points, including ones that are multi-byte in UTF-8, to
    // catch slicing at non-char boundaries.
    let mut rng = Rng::new(0xb17e);
    let pool: Vec<char> = "a-_0\\\"'()[]{};:,.#@!%*+/<>=~|^$&? \n\t\r\u{c}\0\u{7f}\u{e9}\u{2028}\u{ffff}\u{10ffff}\u{1F600}"
        .chars()
        .collect();
    for _ in 0..3000 {
        let len = rng.below(80);
        let s: String = (0..len).map(|_| *rng.pick(&pool)).collect();
        let sheet = parse_stylesheet(&s);
        check_sheet(&sheet, &s);
        let _ = parse_declaration_list(&s);
        let _ = parse_selector_list(&s);
        let _ = Color::parse_str(&s);
        let _ = Color::parse_legacy(&s);
        let _ = MediaQueryList::parse_str(&s).matches(&MediaContext::default());
        let _ = to_css(&parse_component_values(&s));
        for value in parse_component_values(&s) {
            let _ = Length::parse(&value);
            let _ = values::url(&value);
        }
    }
}

#[test]
fn huge_inputs_stay_fast() {
    let cases = [
        // Very long tokens.
        format!("a{{b:{}}}", "9".repeat(200_000)),
        format!(".{}{{}}", "x".repeat(200_000)),
        format!("a{{b:\"{}\"}}", "s".repeat(200_000)),
        format!("a{{b:url({})}}", "u".repeat(200_000)),
        // Many tiny rules and declarations.
        "a{b:c}".repeat(50_000),
        format!("a{{{}}}", "b:c;".repeat(50_000)),
        // Long selector lists and attribute values.
        format!("{}{{x:y}}", ".a,".repeat(20_000) + ".b"),
        format!("[a=\"{}\"]{{x:y}}", "v".repeat(100_000)),
        // Pathological nesting and unclosed blocks.
        "{(".repeat(50_000),
        "a{".repeat(20_000),
        "@media screen{".repeat(20_000),
        ":is(".repeat(20_000),
    ];
    for css in &cases {
        let (sheet, elapsed) = timed(|| parse_stylesheet(css));
        check_sheet(&sheet, "<huge>");
        assert!(
            elapsed < Duration::from_secs(2),
            "{} bytes starting with {:?} took {elapsed:?}",
            css.len(),
            css.chars().take(20).collect::<String>()
        );
    }
}
