//! Matching tests: a brute-force reference matcher compared with the real
//! one on random trees and selectors, complexity regressions (CSS Nesting
//! `:is()` chains, `:nth-*()` through [`Element::nth_index`]) and stack use.

mod support;

use support::{Dom, El, Rng};
use xbro_css::parser::CssRule;
use xbro_css::*;

// ---------------------------------------------------------------------------
// Reference matcher: exhaustive search, straight from the definitions.
// ---------------------------------------------------------------------------

fn all_elements(dom: &Dom) -> Vec<El<'_>> {
    (0..dom.nodes.len()).map(|i| dom.el(i)).collect()
}

fn ancestors<'a>(e: &El<'a>) -> Vec<El<'a>> {
    let mut out = Vec::new();
    let mut cur = e.parent_element();
    while let Some(p) = cur {
        cur = p.parent_element();
        out.push(p);
    }
    out
}

fn prev_siblings<'a>(e: &El<'a>) -> Vec<El<'a>> {
    let mut out = Vec::new();
    let mut cur = e.prev_sibling_element();
    while let Some(p) = cur {
        cur = p.prev_sibling_element();
        out.push(p);
    }
    out
}

/// Elements related to `e` by `combinator` when `e` is on its right side.
fn left_candidates<'a>(e: &El<'a>, combinator: Combinator) -> Vec<El<'a>> {
    match combinator {
        Combinator::Child => e.parent_element().into_iter().collect(),
        Combinator::Descendant => ancestors(e),
        Combinator::NextSibling => e.prev_sibling_element().into_iter().collect(),
        Combinator::SubsequentSibling => prev_siblings(e),
    }
}

/// Is `x` related to `anchor` by `combinator` (`anchor <combinator> x`)?
fn related(anchor: &El<'_>, combinator: Combinator, x: &El<'_>) -> bool {
    left_candidates(x, combinator)
        .iter()
        .any(|c| c.index == anchor.index)
}

fn ref_index(e: &El<'_>, of_type: bool, from_end: bool) -> i64 {
    let parent = e.dom.nodes[e.index].parent;
    let siblings: Vec<usize> = match parent {
        Some(p) => e.dom.nodes[p].children.clone(),
        None => vec![e.index],
    };
    let name = &e.dom.nodes[e.index].name;
    let filtered: Vec<usize> = siblings
        .into_iter()
        .filter(|&s| !of_type || &e.dom.nodes[s].name == name)
        .collect();
    let pos = filtered.iter().position(|&s| s == e.index).unwrap() as i64;
    if from_end {
        filtered.len() as i64 - pos
    } else {
        pos + 1
    }
}

fn ref_nth(a: i32, b: i32, index: i64) -> bool {
    (0..=index).any(|n| i64::from(a) * n + i64::from(b) == index)
}

fn ref_pseudo(pc: &PseudoClass, e: &El<'_>) -> bool {
    use PseudoClass as P;
    match pc {
        P::Root => e.is_root(),
        P::Empty => e.is_empty(),
        P::FirstChild => ref_index(e, false, false) == 1,
        P::LastChild => ref_index(e, false, true) == 1,
        P::OnlyChild => ref_index(e, false, false) == 1 && ref_index(e, false, true) == 1,
        P::FirstOfType => ref_index(e, true, false) == 1,
        P::LastOfType => ref_index(e, true, true) == 1,
        P::NthChild { a, b } => ref_nth(*a, *b, ref_index(e, false, false)),
        P::NthLastChild { a, b } => ref_nth(*a, *b, ref_index(e, false, true)),
        P::NthOfType { a, b } => ref_nth(*a, *b, ref_index(e, true, false)),
        P::Not(list) => !list.iter().any(|s| ref_matches(s, e)),
        P::Is(list) | P::Where(list) => list.iter().any(|s| ref_matches(s, e)),
        P::Has(list) => list.iter().any(|(leading, s)| {
            all_elements(e.dom)
                .iter()
                .any(|x| ref_chain(s, x, &|leftmost: &El<'_>| related(e, *leading, leftmost)))
        }),
        other => panic!("reference matcher does not support {other:?}"),
    }
}

fn ref_compound(c: &Compound, e: &El<'_>) -> bool {
    c.tag.as_ref().is_none_or(|t| t == e.local_name())
        && c.ids.iter().all(|id| e.id() == Some(id.as_str()))
        && c.classes.iter().all(|cl| e.has_class(cl))
        && c.attrs.is_empty()
        && c.pseudo_classes.iter().all(|pc| ref_pseudo(pc, e))
}

/// Does some assignment of the selector's compounds exist with `e` as the
/// subject and the leftmost element accepted by `leftmost_ok`?
fn ref_chain(s: &Selector, e: &El<'_>, leftmost_ok: &dyn Fn(&El<'_>) -> bool) -> bool {
    fn go(s: &Selector, i: usize, e: &El<'_>, leftmost_ok: &dyn Fn(&El<'_>) -> bool) -> bool {
        let compound = if i == 0 {
            &s.subject
        } else {
            &s.ancestors[i - 1].1
        };
        if !ref_compound(compound, e) {
            return false;
        }
        match s.ancestors.get(i) {
            None => leftmost_ok(e),
            Some((combinator, _)) => left_candidates(e, *combinator)
                .iter()
                .any(|c| go(s, i + 1, c, leftmost_ok)),
        }
    }
    go(s, 0, e, leftmost_ok)
}

fn ref_matches(s: &Selector, e: &El<'_>) -> bool {
    ref_chain(s, e, &|_: &El<'_>| true)
}

// ---------------------------------------------------------------------------
// Random trees and selectors.
// ---------------------------------------------------------------------------

fn random_dom(rng: &mut Rng) -> Dom {
    let mut dom = Dom::default();
    let size = 2 + rng.below(24);
    dom.add(None, "a.x");
    for _ in 1..size {
        // Prefer recent nodes as parents to get some depth.
        let len = dom.nodes.len();
        let parent = if rng.chance(60) {
            len - 1 - rng.below(len.min(3))
        } else {
            rng.below(len)
        };
        let tag = *rng.pick(&["a", "b", "c"]);
        let class = *rng.pick(&["", ".x", ".y", ".x.y"]);
        dom.add(Some(parent), &format!("{tag}{class}"));
    }
    dom
}

fn random_compound(rng: &mut Rng, depth: usize) -> String {
    let mut s = String::new();
    s.push_str(rng.pick(&["", "", "a", "b", "c", "*"]));
    for _ in 0..rng.below(3) {
        let simple = match rng.below(if depth < 2 { 12 } else { 8 }) {
            0 | 1 => ".x".to_string(),
            2 => ".y".to_string(),
            3 => ":first-child".to_string(),
            4 => ":last-of-type".to_string(),
            5 => format!(":nth-child({})", rng.pick(&["odd", "2n", "-n+2", "3"])),
            6 => ":empty".to_string(),
            7 => ":nth-last-child(2n+1)".to_string(),
            8 => format!(":not({})", random_list(rng, depth + 1)),
            9 => format!(":is({})", random_list(rng, depth + 1)),
            10 => format!(":where({})", random_list(rng, depth + 1)),
            _ if depth == 0 => {
                let lead = *rng.pick(&["", "> ", "+ ", "~ "]);
                format!(":has({lead}{})", random_complex(rng, depth + 1))
            }
            _ => ".x".to_string(),
        };
        s.push_str(&simple);
    }
    if s.is_empty() {
        s.push('*');
    }
    s
}

fn random_complex(rng: &mut Rng, depth: usize) -> String {
    let mut s = random_compound(rng, depth);
    for _ in 0..rng.below(4) {
        s.push_str(rng.pick(&[" ", " > ", " + ", " ~ ", " "]));
        s.push_str(&random_compound(rng, depth));
    }
    s
}

fn random_list(rng: &mut Rng, depth: usize) -> String {
    let n = 1 + rng.below(2);
    (0..n)
        .map(|_| random_complex(rng, depth))
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn matcher_agrees_with_reference_on_random_inputs() {
    let mut rng = Rng::new(0xc55);
    let mut compared = 0;
    for case in 0..3000 {
        let dom = random_dom(&mut rng);
        let text = random_complex(&mut rng, 0);
        let Some(list) = parse_selector_list(&text) else {
            continue;
        };
        for selector in &list {
            for e in all_elements(&dom) {
                let expected = ref_matches(selector, &e);
                assert_eq!(
                    selector.matches(&e),
                    expected,
                    "case {case}: {text:?} on node {} of {:?}",
                    e.index,
                    dom.nodes
                        .iter()
                        .map(|n| (n.name.clone(), n.classes.clone(), n.parent))
                        .collect::<Vec<_>>()
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 10_000, "only {compared} comparisons");
}

/// A compound that is a nested list with combinators, the shape whose
/// results are memoized by position.
fn random_list_compound(rng: &mut Rng) -> String {
    let inner = random_complex(rng, 2);
    let other = random_complex(rng, 2);
    match rng.below(6) {
        0 => format!(":is({inner})"),
        1 => format!(":not({inner})"),
        2 => format!(":where({inner}, {other})"),
        3 => {
            let lead = *rng.pick(&["", "> ", "+ ", "~ "]);
            format!("{}:has({lead}{inner})", rng.pick(&["", "a", "b", "c"]))
        }
        _ => random_compound(rng, 2),
    }
}

#[test]
fn memoized_lists_agree_with_reference() {
    // Lists evaluated at many positions: ancestors, their previous
    // siblings, and siblings of those ancestors' ancestors.
    let mut rng = Rng::new(0x3e30);
    let mut compared = 0;
    for case in 0..4000 {
        let dom = random_dom(&mut rng);
        let mut text = random_list_compound(&mut rng);
        for _ in 0..1 + rng.below(4) {
            text.push_str(rng.pick(&[" ", " > ", " ~ ", " + ", " ~ "]));
            text.push_str(&random_list_compound(&mut rng));
        }
        let Some(list) = parse_selector_list(&text) else {
            continue;
        };
        for selector in &list {
            for e in all_elements(&dom) {
                assert_eq!(
                    selector.matches(&e),
                    ref_matches(selector, &e),
                    "case {case}: {text:?} on node {} of {:?}",
                    e.index,
                    dom.nodes
                        .iter()
                        .map(|n| (n.name.clone(), n.classes.clone(), n.parent))
                        .collect::<Vec<_>>()
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 10_000, "only {compared} comparisons");
}

#[test]
fn shared_match_cache_agrees_with_reference() {
    // One cache for all the elements and selectors of a document, as in a
    // style pass: `:has()` results computed for an anchor while matching
    // one subject are reused for the others.
    let mut rng = Rng::new(0xcac4e);
    let mut compared = 0;
    for case in 0..3000 {
        let dom = random_dom(&mut rng);
        let mut cache = MatchCache::new();
        let texts: Vec<String> = (0..3)
            .map(|_| {
                let mut text = random_list_compound(&mut rng);
                text.push_str(rng.pick(&[" ", " > ", " ~ ", " "]));
                text.push_str(&random_complex(&mut rng, 0));
                text
            })
            .collect();
        let lists: Vec<_> = texts
            .iter()
            .filter_map(|t| parse_selector_list(t))
            .collect();
        for e in all_elements(&dom) {
            for (list, text) in lists.iter().zip(&texts) {
                for selector in list {
                    assert_eq!(
                        selector.matches_with_cache(&e, &mut cache),
                        ref_matches(selector, &e),
                        "case {case}: {text:?} on node {} of {:?}",
                        e.index,
                        dom.nodes
                            .iter()
                            .map(|n| (n.name.clone(), n.classes.clone(), n.parent))
                            .collect::<Vec<_>>()
                    );
                    compared += 1;
                }
            }
        }
    }
    assert!(compared > 10_000, "only {compared} comparisons");
}

#[test]
fn has_left_of_a_descendant_combinator_is_linear_with_a_cache() {
    // `:has()` in a compound left of the subject is evaluated on every
    // ancestor of every element: without a shared cache, each evaluation
    // walks the ancestor's subtree again (cubic on a deep chain, quadratic
    // on `.page:has(.missing) p`).
    let mut deep = Dom::default();
    deep.add(None, "a");
    for i in 1..300 {
        deep.add(Some(i - 1), "b.x");
    }
    let mut wide = Dom::default();
    let page = wide.add(None, "a.page");
    for _ in 0..3000 {
        let section = wide.add(Some(page), "b");
        wide.add(Some(section), "c");
    }
    for (dom, text) in [
        (&deep, ":has(.missing) *"),
        (&deep, "b:has(> .x .missing, + .y) .x"),
        (&wide, ".page:has(.missing) c"),
        (&wide, ".page:has(b + .missing) b > c"),
    ] {
        let list = parse_selector_list(text).unwrap();
        let mut cache = MatchCache::new();
        dom.steps.set(0);
        for e in all_elements(dom) {
            for selector in &list {
                assert!(!selector.matches_with_cache(&e, &mut cache));
            }
        }
        let cached = dom.steps.get();
        // Each anchor's subtree is walked once: linear in the sum of the
        // subtree sizes (the size of the document times its depth).
        let n = dom.nodes.len() as u64;
        let depth = if std::ptr::eq(dom, &deep) { n } else { 2 };
        assert!(cached < 4 * n * depth, "{text}: {cached} steps");
        // Same results without the cache, with far more work.
        dom.steps.set(0);
        for e in all_elements(dom).into_iter().take(400) {
            for selector in &list {
                assert!(!selector.matches(&e));
            }
        }
        let uncached = dom.steps.get();
        assert!(
            uncached > 10 * cached,
            "{text}: {uncached} vs {cached} steps"
        );
    }
}

#[test]
fn nested_rules_match_like_reference() {
    // Selectors produced by CSS Nesting (`:is()` chains with several parent
    // selectors) through the memoized path.
    let mut rng = Rng::new(0x7e57);
    for case in 0..400 {
        let dom = random_dom(&mut rng);
        let css = format!(
            "{} {{ {} {{ {} {{ color: red }} }} }}",
            random_list(&mut rng, 1),
            random_list(&mut rng, 1),
            random_list(&mut rng, 1)
        );
        for selector in style_selectors(&css) {
            for e in all_elements(&dom) {
                assert_eq!(
                    selector.matches(&e),
                    ref_matches(&selector, &e),
                    "case {case}: {css:?} on node {}",
                    e.index
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Complexity.
// ---------------------------------------------------------------------------

fn style_selectors(css: &str) -> Vec<Selector> {
    fn collect(rules: &[CssRule], out: &mut Vec<Selector>) {
        for rule in rules {
            match rule {
                CssRule::Style(s) => out.extend(s.selectors.iter().cloned()),
                CssRule::Media { rules, .. }
                | CssRule::Supports { rules, .. }
                | CssRule::Group { rules } => collect(rules, out),
                CssRule::Import { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    collect(&parse_stylesheet(css).rules, &mut out);
    out
}

/// A chain of `depth` nested `div`s ending with a `span`; returns the span.
fn deep_chain(dom: &mut Dom, depth: usize) -> usize {
    let mut cur = dom.add(None, "html");
    for _ in 0..depth {
        cur = dom.add(Some(cur), "div");
    }
    dom.add(Some(cur), "span")
}

#[test]
fn css_nesting_with_several_parents_is_not_exponential() {
    // Each nesting level with a selector list wraps the previous level in
    // `:is()`; without memoization every level multiplied the work by the
    // tree depth (6 levels on a 60-deep tree never finished).
    let mut dom = Dom::default();
    let span = deep_chain(&mut dom, 60);
    for levels in 1..=6 {
        let mut css = String::from(".foo, .bar {");
        for _ in 0..levels {
            css.push_str(" div, p {");
        }
        css.push_str(" span { color: red }");
        for _ in 0..=levels {
            css.push('}');
        }
        let selectors = style_selectors(&css);
        assert_eq!(selectors.len(), 1, "{css}");
        dom.steps.set(0);
        assert!(!selectors[0].matches(&dom.el(span)));
        let steps = dom.steps.get();
        assert!(
            steps < 200_000,
            "{levels} levels: {steps} DOM steps for one element"
        );
    }
    // And the positive case still matches.
    let mut dom = Dom::default();
    let root = dom.add(None, "html.foo");
    let mut cur = root;
    for _ in 0..40 {
        cur = dom.add(Some(cur), "div");
    }
    let span = dom.add(Some(cur), "span");
    let css = ".foo, .bar { div, p { div, p { div, p { span { x: y } } } } }";
    assert!(style_selectors(css)[0].matches(&dom.el(span)));
}

#[test]
fn nested_is_selectors_are_not_exponential() {
    // The same shape written by hand: `:is(:is(:is(.z b) b) b) b`.
    let mut dom = Dom::default();
    let mut cur = dom.add(None, "b");
    for _ in 0..80 {
        cur = dom.add(Some(cur), "b");
    }
    let mut selector = String::from(".z b");
    for _ in 0..8 {
        selector = format!(":is({selector}) b");
    }
    let list = parse_selector_list(&selector).expect("valid selector");
    dom.steps.set(0);
    assert!(!list[0].matches(&dom.el(cur)));
    assert!(dom.steps.get() < 500_000, "{} steps", dom.steps.get());
}

#[test]
fn pathological_sibling_chains_hit_the_budget_instead_of_hanging() {
    // `:is(:is(a ~ b) ~ b) ~ b` over thousands of siblings is polynomial even
    // with memoization; the global step budget keeps one match bounded.
    let mut dom = Dom::default();
    let root = dom.add(None, "r");
    let mut last = root;
    for _ in 0..3000 {
        last = dom.add(Some(root), "b");
    }
    let mut selector = String::from(".none ~ b");
    for _ in 0..6 {
        selector = format!(":is({selector}) ~ b");
    }
    let list = parse_selector_list(&selector).expect("valid selector");
    let start = std::time::Instant::now();
    assert!(!list[0].matches(&dom.el(last)));
    assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
}

/// A DOM wrapper whose sibling accessors are expensive (like a DOM that
/// searches the parent's child list) but which overrides `nth_index`.
#[derive(Clone, Copy)]
struct Indexed<'a>(El<'a>);

impl Element for Indexed<'_> {
    fn local_name(&self) -> &str {
        self.0.local_name()
    }
    fn id(&self) -> Option<&str> {
        self.0.id()
    }
    fn has_class(&self, class: &str) -> bool {
        self.0.has_class(class)
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.0.attr(name)
    }
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(Indexed)
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(Indexed)
    }
    fn next_sibling_element(&self) -> Option<Self> {
        self.0.next_sibling_element().map(Indexed)
    }
    fn first_child_element(&self) -> Option<Self> {
        self.0.first_child_element().map(Indexed)
    }
    fn is_root(&self) -> bool {
        self.0.is_root()
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn nth_index(&self, of_type: bool, from_end: bool) -> usize {
        ref_index(&self.0, of_type, from_end) as usize
    }
}

#[test]
fn nth_pseudo_classes_use_the_nth_index_hook() {
    let mut dom = Dom::default();
    let root = dom.add(None, "ul");
    let items: Vec<usize> = (0..2000)
        .map(|i| dom.add(Some(root), if i % 3 == 0 { "li" } else { "p" }))
        .collect();
    let selectors = [
        ":nth-child(odd)",
        ":nth-last-child(3n+1)",
        "li:nth-of-type(2n)",
        "p:nth-last-of-type(-n+5)",
    ];
    for text in selectors {
        let list = parse_selector_list(text).expect("valid selector");
        // Same answers through the default walk and through the hook.
        dom.steps.set(0);
        let walked: Vec<bool> = items.iter().map(|&i| list[0].matches(&dom.el(i))).collect();
        let walk_steps = dom.steps.get();
        dom.steps.set(0);
        let hooked: Vec<bool> = items
            .iter()
            .map(|&i| list[0].matches(&Indexed(dom.el(i))))
            .collect();
        assert_eq!(walked, hooked, "{text}");
        assert!(walked.iter().any(|&m| m) && !walked.iter().all(|&m| m));
        // The default walk is quadratic over the siblings; the hook is not.
        assert!(walk_steps > 500_000, "{text}: {walk_steps}");
        assert!(dom.steps.get() < 10_000, "{text}: {}", dom.steps.get());
    }
}

#[test]
fn worst_case_selectors_fit_in_a_default_thread_stack() {
    // 16 levels of `:is()` with 64 compounds each, the `:is()` in the
    // leftmost compound so every level is active at once, matched on a tree
    // deep enough for all of them; and the CSS Nesting equivalent (32
    // levels, 2048 compounds). Spawned threads get 2 MiB by default.
    let mut top = String::from("b");
    for _ in 0..15 {
        top = format!(":is({top} {})", "b ".repeat(62).trim_end());
    }
    let mut css = String::from(".r {");
    for _ in 0..31 {
        css.push_str(&format!(" {} & {{", ".b ".repeat(62)));
    }
    css.push_str(" x: y");
    css.push_str(&"}".repeat(32));
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let mut dom = Dom::default();
            let mut cur = dom.add(None, "b.r");
            for _ in 0..2100 {
                cur = dom.add(Some(cur), "b.b");
            }
            let list = parse_selector_list(&top).expect("valid selector");
            assert_eq!(list.len(), 1);
            assert!(list[0].matches(&dom.el(cur)));
            let nested = style_selectors(&css);
            assert!(!nested.is_empty());
            for selector in &nested {
                let _ = selector.matches(&dom.el(cur));
            }
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");
}
