//! Regression tests for bugs found in review: matching false negatives,
//! quadratic behaviours on hostile pages, and spec deviations.

mod common;

use std::time::{Duration, Instant};

use common::{style, Styled};
use xbro_style::*;

const RED: Color = Color::rgb(255, 0, 0);
const BLUE: Color = Color::rgb(0, 0, 255);

fn px(v: f32) -> LengthPercentageAuto {
    LengthPercentageAuto::px(v)
}

/// Generous bound for debug builds on a loaded machine; the inputs below
/// took tens of seconds before the fixes and take well under a second now.
fn assert_fast(start: Instant, what: &str) {
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(8), "{what}: {elapsed:?}");
}

#[test]
fn ancestor_filter_saturation_does_not_hide_matching_rules() {
    // A sibling subtree with tens of thousands of repeated class names used
    // to wrap the ancestor filter's counters back to zero, so `.x #t` was
    // skipped although its ancestor still has the class.
    let repeated = "x ".repeat(70_000);
    let html = format!("<div class=x><div class='{repeated}'></div><p id=t>t</p></div>");
    let s = style(&html, ".x #t { color: red } .x > p { margin-left: 3px }");
    assert_eq!(s.get("t").color, RED);
    assert_eq!(s.get("t").margin.left, px(3.0));
}

#[test]
fn universal_rule_referencing_a_huge_variable_is_linear() {
    let big = "a ".repeat(50_000);
    let css = format!(
        ":root {{ --big: {big}; --w: 5px }} * {{ --a: var(--big) x; --b: var(--big) }}
         p {{ content: var(--big); width: var(--w) }}"
    );
    let html = "<div><p id=p>x</p></div>".repeat(2_000);
    let start = Instant::now();
    let s = style(&html, &css);
    assert_fast(start, "huge variable on every element");
    assert_eq!(s.get("p").width, px(5.0));
}

#[test]
fn distinct_maps_with_huge_variables_are_memoized() {
    // Every element has its own custom property map (via `style`), so the
    // per-map caches miss; substitutions are memoized by the identity of
    // the referenced values instead.
    let big = "a ".repeat(50_000);
    let css = format!(
        ":root {{ --big: {big} }} * {{ --a: var(--big) x; --b: var(--big) y }}
         p {{ width: var(--w) }}"
    );
    let mut html = String::new();
    for i in 0..3_000 {
        html.push_str(&format!("<p id=p{i} style='--w: {i}px'>x</p>"));
    }
    let start = Instant::now();
    let s = style(&html, &css);
    assert_fast(start, "distinct maps");
    for i in [0, 1, 1_500, 2_999] {
        assert_eq!(s.get(&format!("p{i}")).width, px(i as f32));
    }
}

#[test]
fn many_universal_custom_properties_with_distinct_maps() {
    let mut css = String::from("* {");
    for i in 0..20_000 {
        css.push_str(&format!("--a{i}: {i}px;"));
    }
    css.push_str(
        "} p { width: var(--w) } span { margin-left: var(--a77); padding-left: var(--a5) }",
    );
    let mut html = String::new();
    for i in 0..3_000 {
        html.push_str(&format!(
            "<p id=p{i} style='--w: {i}px; --a5: 9px'><span id=s{i}>x</span></p>"
        ));
    }
    let start = Instant::now();
    let s = style(&html, &css);
    assert_fast(start, "20k universal custom properties");
    for i in [0, 1_234, 2_999] {
        assert_eq!(s.get(&format!("p{i}")).width, px(i as f32));
        let span = s.get(&format!("s{i}"));
        assert_eq!(span.margin.left, px(77.0));
        // `*` re-declares `--a5: 5px` on the span, over the parent's 9px.
        assert_eq!(span.padding.left, LengthPercentage::px(5.0));
    }
}

#[test]
fn repeated_declarations_are_deduplicated() {
    let css = format!(
        "* {{ {} color: blue; margin: 1px 2px }}",
        "color: red; margin: 7px; --v: 1; ".repeat(50_000)
    );
    let html = "<div><p id=p>x</p></div>".repeat(1_500);
    let start = Instant::now();
    let s = style(&html, &css);
    assert_fast(start, "repeated declarations");
    let p = s.get("p");
    assert_eq!(p.color, BLUE);
    assert_eq!(p.margin.top, px(1.0));
    assert_eq!(p.margin.left, px(2.0));
}

#[test]
fn disabled_fieldset_with_many_controls() {
    let html = format!(
        "<fieldset disabled><legend><input id=first></legend>{}<input id=last>\
         <select><option id=opt>o</option></select></fieldset>",
        "<input>".repeat(20_000)
    );
    let start = Instant::now();
    let s = style(
        &html,
        "input:disabled { color: red } input:enabled { color: blue } option:enabled { color: blue }",
    );
    assert_fast(start, "disabled fieldset");
    assert_eq!(s.get("last").color, RED);
    // Controls in the first legend are not disabled by the fieldset.
    assert_eq!(s.get("first").color, BLUE);
}

#[test]
fn hidden_inputs_stay_hidden() {
    // `input[type=hidden i] { display: none !important }` in the UA sheet.
    let s = style(
        "<input id=a type=hidden style='display: block'><input id=b type=HIDDEN>\
         <input id=c type=text>",
        "input { display: block !important }",
    );
    assert!(s.try_get("a").is_none());
    assert!(s.try_get("b").is_none());
    assert_eq!(s.get("c").display, Display::Block);
}

#[test]
fn hidden_embed_is_an_empty_inline_box() {
    let s = style(
        "<embed id=e hidden src=x><div id=d hidden>x</div><p id=u hidden=until-found>y</p>",
        "",
    );
    let e = s.get("e");
    assert_eq!(e.display, Display::Inline);
    assert_eq!(e.width, px(0.0));
    assert_eq!(e.height, px(0.0));
    assert!(s.try_get("d").is_none());
    assert!(s.try_get("u").is_some());
}

#[test]
fn escaped_percent_is_not_a_percentage() {
    let s = style(
        "<p id=p>x</p>",
        "#p { border: 2px solid; border-width: 1\\%; width: 10px; width: 5\\%;
              margin-left: 3px; margin-left: calc(1\\% + 1px) }",
    );
    let p = s.get("p");
    assert_eq!(p.border.top.width, 2.0);
    assert_eq!(p.width, px(10.0));
    assert_eq!(p.margin.left, px(3.0));
}

#[test]
fn font_shorthand_rejects_relative_weights() {
    let s = style(
        "<p id=p>x</p>",
        "#p { font: bold 10px serif; font: bolder 20px serif }",
    );
    let p = s.get("p");
    assert_eq!(p.font_weight, 700);
    assert_eq!(p.font_size, 10.0);
}

#[test]
fn cycles_through_fallbacks_are_invalid() {
    // `--a` depends on `--b` through its fallback even though `--x` is
    // defined: the dependency graph includes fallbacks, so both are invalid.
    let s = style(
        "<p id=p>x</p>",
        "p { --x: 1px; --a: var(--x, var(--b)); --b: var(--a);
             width: var(--a, 7px); height: var(--b, 9px); margin-left: var(--x) }",
    );
    let p = s.get("p");
    assert_eq!(p.width, px(7.0));
    assert_eq!(p.height, px(9.0));
    assert_eq!(p.margin.left, px(1.0));
}

#[test]
fn cycle_detection_is_independent_of_resolution_order() {
    // Found by the metamorphic fuzz below: `--v2 → --v10 → --v11 → --v5 →
    // --v2` is a cycle, but a depth-first search that memoizes finished
    // properties missed it when it started from `--v11` (which depends on
    // `--v10` and `--v5` and finishes `--v10` first), leaving `--v2` and
    // `--v5` valid depending on hash map iteration order.
    let css = "p { --v10: calc(var(--v11, 1px) + 2px) }
               p { --v5: calc(var(--v2, 1px) + 5px); --v11: var(--v10, var(--v5, 3px)) !important }
               .a { --v2: var(--v10, 1px) }
               .a { padding-left: var(--v2); padding-right: var(--v5, 8px); margin-left: var(--v9, 4px) }
               .a { --v9: calc(var(--v5, 2px) + var(--v12, 1px)); --v12: 6px }";
    for _ in 0..50 {
        let s = style("<p id=p class=a>x</p>", css);
        let p = s.get("p");
        assert_eq!(p.padding.left, LengthPercentage::ZERO, "--v2 is in a cycle");
        assert_eq!(
            p.padding.right,
            LengthPercentage::px(8.0),
            "--v5 is in a cycle"
        );
        // `--v9` only depends on the cycle: fallback for `--v5`, valid itself.
        assert_eq!(p.margin.left, px(8.0));
    }
}

#[test]
fn memoized_substitutions_follow_each_element() {
    let mut html = String::from("<div>");
    for i in 0..60 {
        html.push_str(&format!(
            "<p id=p{i} style='--w: {}px'><span id=s{i}>x</span></p>",
            i % 7
        ));
    }
    html.push_str("</div>");
    let s = style(
        &html,
        "p { --d: var(--w) var(--w); margin: var(--d); width: calc(var(--w) * 2) }
         span { --w: 100px; padding: var(--d, 1px) }",
    );
    for i in 0..60 {
        let w = (i % 7) as f32;
        let p = s.get(&format!("p{i}"));
        assert_eq!(p.width, px(2.0 * w), "p{i}");
        assert_eq!(p.margin.top, px(w), "p{i}");
        assert_eq!(p.margin.right, px(w), "p{i}");
        // `--d` was computed on the parent and is inherited as is.
        let span = s.get(&format!("s{i}"));
        assert_eq!(span.padding.top, LengthPercentage::px(w), "s{i}");
    }
}

#[test]
fn invalid_decoration_lines_and_match_parent_alias() {
    let s = style(
        "<div id=d style='text-align: center'><p id=p>x</p></div>",
        "#p { text-decoration: underline; text-decoration: underline underline;
              text-align: right; text-align: -webkit-match-parent }",
    );
    let p = s.get("p");
    assert!(p.text_decoration_line.underline);
    assert_eq!(p.text_align, TextAlign::Center);
}

#[test]
fn huge_values_copied_per_element_are_rejected() {
    let term = "min(1em, 2px)";
    let huge_calc = format!("calc({})", vec![term; 2_000].join(" + "));
    let huge_content = format!("\"{}\"", "y".repeat(70_000));
    let css = format!(
        "p {{ width: 10px; width: {huge_calc} }}
         p::before {{ content: 'a'; content: {huge_content} }}"
    );
    let s = style("<p id=p>x</p>", &css);
    assert_eq!(s.get("p").width, px(10.0));
    let before = s.before("p").expect("::before");
    assert_eq!(
        before.content,
        Content::Items(vec![ContentItem::Text("a".into())])
    );
}

/// Deterministic xorshift generator.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

fn custom_value(rng: &mut Rng) -> String {
    let v = rng.below(8);
    let m = rng.below(12);
    let k = rng.below(12);
    match rng.below(10) {
        0 => format!("{v}px"),
        1 => format!("var(--v{m})"),
        2 => format!("var(--v{m}, {v}px)"),
        3 => format!("calc(var(--v{m}, 1px) + {v}px)"),
        4 => "inherit".into(),
        5 => "initial".into(),
        6 => "red".into(),
        7 => format!("var(--v{m}) var(--v{k})"),
        8 => format!("var(--v{m}, var(--v{k}, 3px))"),
        _ => format!("{v}em"),
    }
}

fn longhand(rng: &mut Rng) -> String {
    let m = rng.below(12);
    let prop = rng.pick(&[
        "width",
        "margin",
        "padding-left",
        "color",
        "border",
        "font-size",
        "background-color",
        "border-top-width",
        "line-height",
    ]);
    let value = match rng.below(4) {
        0 => format!("var(--v{m})"),
        1 => format!("var(--v{m}, 4px)"),
        2 => format!("calc(var(--v{m}, 2px) * 2)"),
        _ => format!("var(--v{m}) solid"),
    };
    format!("{prop}: {value}")
}

fn random_sheet(rng: &mut Rng) -> String {
    let mut css = String::new();
    if rng.below(2) == 0 {
        // A large var-free universal list (incremental application).
        css.push_str("* {");
        for i in 0..(20 + rng.below(20)) {
            css.push_str(&format!("--v{}: {}px;", i % 12, rng.below(9)));
        }
        css.push_str("}\n");
    }
    for _ in 0..25 {
        let selector = rng.pick(&[
            "*", "div", "p", ".a", "#b", "div > p", "span", ":root", ".a span",
        ]);
        css.push_str(selector);
        css.push_str(" {");
        for _ in 0..(1 + rng.below(4)) {
            if rng.below(2) == 0 {
                css.push_str(&format!("--v{}: {}", rng.below(12), custom_value(rng)));
            } else {
                css.push_str(&longhand(rng));
            }
            if rng.below(6) == 0 {
                css.push_str(" !important");
            }
            css.push(';');
        }
        css.push_str("}\n");
    }
    css
}

fn random_document(rng: &mut Rng) -> String {
    let mut html = String::from("<!DOCTYPE html><body>");
    for _ in 0..120 {
        let tag = rng.pick(&["div", "p", "span"]);
        html.push('<');
        html.push_str(tag);
        match rng.below(4) {
            0 => html.push_str(" class=a"),
            1 => html.push_str(" id=b"),
            _ => {}
        }
        if rng.below(3) == 0 {
            html.push_str(&format!(
                " style='--v{}: {}; {}'",
                rng.below(12),
                custom_value(rng),
                longhand(rng)
            ));
        }
        html.push_str(">x");
        if rng.below(3) == 0 {
            html.push_str(&format!("</{tag}>"));
        }
    }
    html
}

fn snapshot(map: &StyleMap) -> Vec<Option<ComputedStyle>> {
    map.styles.iter().map(|s| s.as_deref().cloned()).collect()
}

#[test]
fn duplicated_sheets_give_identical_styles() {
    // Metamorphic fuzz: repeating the author sheet changes nothing (the
    // second copy overrides the first with identical declarations), but
    // every memoization key (block addresses) differs, so a cache returning
    // another element's result would show up as a difference.
    let rounds: u64 = std::env::var("XBRO_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for round in 0..rounds {
        let html = random_document(&mut rng);
        let css = random_sheet(&mut rng);
        let doc = xbro_html::parse_document(&html);
        let sheet = xbro_css::parse_stylesheet(&css);
        let ctx = StyleContext::default();
        let once = compute_styles(&doc, std::slice::from_ref(&sheet), &ctx);
        let twice = compute_styles(&doc, &[sheet.clone(), sheet], &ctx);
        let (a, b) = (snapshot(&once), snapshot(&twice));
        if a != b {
            if let Ok(dir) = std::env::var("XBRO_DUMP_DIR") {
                let _ = std::fs::write(format!("{dir}/fail.css"), &css);
                let _ = std::fs::write(format!("{dir}/fail.html"), &html);
            }
        }
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(x, y, "round {round}, node {i}\nCSS:\n{css}\nHTML:\n{html}");
        }
        for s in a.iter().flatten() {
            assert!(s.font_size.is_finite() && s.font_size >= 0.0);
            assert!(s.border.top.width.is_finite() && s.border.top.width >= 0.0);
        }
    }
}

#[test]
fn shared_styles_helper_smoke() {
    // Keeps the `Styled` helper type in use in this test crate.
    let s: Styled = style("<p id=p>x</p>", "p { color: red }");
    assert_eq!(s.get("p").color, RED);
}

#[test]
fn deep_documents_with_large_variable_maps() {
    // Large maps are never flattened, so their chains grow with the depth
    // of the document; lookups and drops must stay iterative / shallow.
    let mut css = String::from("* {");
    for i in 0..1_100 {
        css.push_str(&format!("--a{i}: {i}px;"));
    }
    css.push_str("} div { margin-left: var(--a1099); padding-left: var(--d) }");
    let mut html = String::new();
    for i in 0..300 {
        html.push_str(&format!(
            "<div id=d{i} style='--d: {i}px; --a{}: 0px'>",
            i % 1_100
        ));
    }
    let handle = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let s = style(&html, &css);
            let deepest = (0..300)
                .rev()
                .find_map(|i| s.try_get(&format!("d{i}")))
                .expect("styled");
            (deepest.margin.left, deepest.padding.left)
        })
        .expect("thread");
    let (margin, padding) = handle.join().expect("no stack overflow");
    assert_eq!(margin, px(1_099.0));
    assert!(padding.px > 200.0);
}
