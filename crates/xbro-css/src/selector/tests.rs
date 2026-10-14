use super::*;

// ---------------------------------------------------------------------------
// A tiny DOM for matching tests.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Node {
    name: String,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, String)>,
    parent: Option<usize>,
    children: Vec<usize>,
    has_text: bool,
    hovered: bool,
    focused: bool,
    checked: bool,
}

#[derive(Default)]
struct Dom {
    nodes: Vec<Node>,
}

#[derive(Clone, Copy)]
struct El<'a> {
    dom: &'a Dom,
    index: usize,
}

impl Dom {
    /// Adds an element described like `tag#id.class[attr=value][flag]`.
    fn add(&mut self, parent: Option<usize>, spec: &str) -> usize {
        let mut node = Node {
            parent,
            ..Node::default()
        };
        let mut rest = spec;
        let end = rest.find(['#', '.', '[']).unwrap_or(rest.len());
        node.name = rest[..end].to_string();
        rest = &rest[end..];
        while !rest.is_empty() {
            let kind = rest.as_bytes()[0];
            rest = &rest[1..];
            if kind == b'[' {
                let close = rest.find(']').expect("unclosed attribute");
                let attr = &rest[..close];
                rest = &rest[close + 1..];
                let (name, value) = attr.split_once('=').unwrap_or((attr, ""));
                node.attrs.push((name.to_string(), value.to_string()));
                continue;
            }
            let end = rest.find(['#', '.', '[']).unwrap_or(rest.len());
            let value = rest[..end].to_string();
            rest = &rest[end..];
            match kind {
                b'#' => node.id = Some(value),
                _ => node.classes.push(value),
            }
        }
        if let Some(id) = &node.id {
            node.attrs.push(("id".into(), id.clone()));
        }
        if !node.classes.is_empty() {
            node.attrs.push(("class".into(), node.classes.join(" ")));
        }
        let index = self.nodes.len();
        self.nodes.push(node);
        if let Some(p) = parent {
            self.nodes[p].children.push(index);
        }
        index
    }

    fn el(&self, index: usize) -> El<'_> {
        El { dom: self, index }
    }

    fn node(&self, index: usize) -> &Node {
        &self.nodes[index]
    }
}

impl El<'_> {
    fn node(&self) -> &Node {
        self.dom.node(self.index)
    }

    fn sibling(&self, delta: isize) -> Option<Self> {
        let parent = self.node().parent?;
        let siblings = &self.dom.node(parent).children;
        let pos = siblings.iter().position(|&c| c == self.index)?;
        let target = pos.checked_add_signed(delta)?;
        siblings.get(target).map(|&index| El {
            dom: self.dom,
            index,
        })
    }
}

impl Element for El<'_> {
    fn local_name(&self) -> &str {
        &self.node().name
    }
    fn id(&self) -> Option<&str> {
        self.node().id.as_deref()
    }
    fn has_class(&self, class: &str) -> bool {
        self.node().classes.iter().any(|c| c == class)
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.node()
            .attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
    fn parent_element(&self) -> Option<Self> {
        self.node().parent.map(|index| El {
            dom: self.dom,
            index,
        })
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        self.sibling(-1)
    }
    fn next_sibling_element(&self) -> Option<Self> {
        self.sibling(1)
    }
    fn first_child_element(&self) -> Option<Self> {
        self.node().children.first().map(|&index| El {
            dom: self.dom,
            index,
        })
    }
    fn is_root(&self) -> bool {
        self.node().parent.is_none()
    }
    fn is_empty(&self) -> bool {
        self.node().children.is_empty() && !self.node().has_text
    }
    fn is_hovered(&self) -> bool {
        self.node().hovered
    }
    fn is_focused(&self) -> bool {
        self.node().focused
    }
    fn is_checked(&self) -> bool {
        self.node().checked
    }
    fn is_disabled(&self) -> bool {
        self.attr("disabled").is_some()
    }
}

fn parse(s: &str) -> Vec<Selector> {
    parse_selector_list(s).unwrap_or_else(|| panic!("{s:?} should parse"))
}

fn matches(dom: &Dom, selector: &str, index: usize) -> bool {
    parse(selector).iter().any(|s| s.matches(&dom.el(index)))
}

/// Indices of the elements matching `selector`.
fn select(dom: &Dom, selector: &str) -> Vec<usize> {
    let list = parse(selector);
    (0..dom.nodes.len())
        .filter(|&i| list.iter().any(|s| s.matches(&dom.el(i))))
        .collect()
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[test]
fn parses_structure() {
    let list = parse("ul > li.item a:hover::before");
    assert_eq!(list.len(), 1);
    let s = &list[0];
    assert_eq!(s.subject.tag.as_deref(), Some("a"));
    assert_eq!(s.subject.pseudo_classes, vec![PseudoClass::Hover]);
    assert_eq!(s.pseudo_element, Some(PseudoElement::Before));
    assert_eq!(s.ancestors.len(), 2);
    assert_eq!(s.ancestors[0].0, Combinator::Descendant);
    assert_eq!(s.ancestors[0].1.tag.as_deref(), Some("li"));
    assert_eq!(s.ancestors[0].1.classes, vec!["item".to_string()]);
    assert_eq!(s.ancestors[1].0, Combinator::Child);
    assert_eq!(s.ancestors[1].1.tag.as_deref(), Some("ul"));
}

#[test]
fn combinators_with_and_without_whitespace() {
    for (input, combinator) in [
        ("a>b", Combinator::Child),
        ("a > b", Combinator::Child),
        ("a >b", Combinator::Child),
        ("a+b", Combinator::NextSibling),
        ("a + b", Combinator::NextSibling),
        ("a~b", Combinator::SubsequentSibling),
        ("a ~ b", Combinator::SubsequentSibling),
        ("a b", Combinator::Descendant),
        ("a \n\t b", Combinator::Descendant),
        ("a /* c */ b", Combinator::Descendant),
    ] {
        let list = parse(input);
        assert_eq!(list[0].ancestors[0].0, combinator, "{input}");
        assert_eq!(list[0].subject.tag.as_deref(), Some("b"));
    }
}

#[test]
fn type_selectors_are_lowercased_and_namespaces_ignored() {
    assert_eq!(parse("DIV")[0].subject.tag.as_deref(), Some("div"));
    assert_eq!(parse("svg|RECT")[0].subject.tag.as_deref(), Some("rect"));
    assert_eq!(parse("*|p")[0].subject.tag.as_deref(), Some("p"));
    assert_eq!(parse("|p")[0].subject.tag.as_deref(), Some("p"));
    assert_eq!(parse("*|*")[0].subject.tag, None);
    assert_eq!(parse("ns|*")[0].subject.tag, None);
    assert_eq!(parse("*")[0].subject, Compound::default());
    assert_eq!(parse("*.a")[0].subject.classes, vec!["a".to_string()]);
}

#[test]
fn attribute_selectors() {
    let attr = |s: &str| parse(s)[0].subject.attrs[0].clone();
    let a = attr("[href]");
    assert_eq!((a.name.as_str(), a.op), ("href", AttrOperator::Exists));
    let a = attr("[DATA-X=foo]");
    assert_eq!(a.name, "data-x");
    assert_eq!(a.op, AttrOperator::Equals);
    assert_eq!(a.value, "foo");
    assert!(!a.case_insensitive);
    assert_eq!(attr("[a~=b]").op, AttrOperator::Includes);
    assert_eq!(attr("[a|=b]").op, AttrOperator::DashMatch);
    assert_eq!(attr("[a^=b]").op, AttrOperator::Prefix);
    assert_eq!(attr("[a$=b]").op, AttrOperator::Suffix);
    assert_eq!(attr("[a*=b]").op, AttrOperator::Substring);
    assert_eq!(attr("[ a = \"x y\" ]").value, "x y");
    assert!(attr("[a=b i]").case_insensitive);
    assert!(attr("[a='b' I]").case_insensitive);
    assert!(attr("[a=\"b\"i]").case_insensitive);
    assert!(!attr("[a=b s]").case_insensitive);
    // HTML attributes with case-insensitive values.
    assert!(attr("[type=checkbox]").case_insensitive);
    assert!(attr("[lang=en]").case_insensitive);
    assert!(!attr("[type=checkbox s]").case_insensitive);
    // Namespace prefixes.
    assert_eq!(attr("[xlink|href]").name, "href");
    assert_eq!(attr("[*|href]").name, "href");
    assert_eq!(attr("[|href]").name, "href");
    let a = attr("[ns|a|=b]");
    assert_eq!((a.name.as_str(), a.op), ("a", AttrOperator::DashMatch));
}

#[test]
fn invalid_selectors() {
    let invalid = [
        "",
        " ",
        ",",
        "a,",
        ",a",
        "a,,b",
        "a >",
        "> a",
        "a > > b",
        "a + ~ b",
        "#1",
        "#",
        ".",
        ".1",
        "a.",
        "[",
        "[]",
        "[1]",
        "[a=1]",
        "[a=b c]",
        "[a b]",
        "[a==b]",
        "[a=b x]",
        "[a i]",
        "a[b]c",
        ":unknown",
        "::unknown",
        "::before::after",
        "::before .a",
        "::before.a",
        "::before:first-child",
        ":not()",
        ":not(::before)",
        ":not(:bogus)",
        ":has()",
        ":has(:has(a))",
        ":has(::before)",
        ":nth-child(foo)",
        ":nth-child(2n+1 of .a)",
        ":nth-child()",
        ":hover()",
        "a||b",
        "a|",
        "a!",
        "a{",
        "foo(bar)",
        ":dir()",
        ":lang()",
        "::-ms-clear",
        ":-moz-placeholder",
    ];
    for s in invalid {
        assert!(parse_selector_list(s).is_none(), "{s:?} should be invalid");
    }
}

#[test]
fn valid_selectors() {
    let valid = [
        "a, b , c",
        "*",
        "html",
        "#main",
        ".a.b.c",
        "a#b.c[d]:hover",
        "a::before",
        "a::after",
        "a:before",
        "a:AFTER",
        "p::first-line",
        "p:first-letter",
        "li::marker",
        "input::placeholder",
        "::selection",
        "dialog::backdrop",
        "input::file-selector-button",
        "::-webkit-scrollbar",
        "::-webkit-scrollbar-thumb:hover",
        "::-webkit-scrollbar:horizontal",
        "::-moz-selection",
        "::-moz-focus-inner",
        "::part(label)",
        "::slotted(span)",
        "a::before:hover",
        ":root",
        ":empty",
        ":first-child:last-child:only-child",
        ":first-of-type:last-of-type:only-of-type",
        ":nth-child(odd)",
        ":nth-child( 2n + 1 )",
        ":nth-last-child(-n+3)",
        ":nth-of-type(even)",
        ":nth-last-of-type(3)",
        ":link:visited:any-link:-webkit-any-link",
        ":hover:active:focus:focus-within:focus-visible",
        ":checked:disabled:enabled",
        ":not(.a, #b > c)",
        ":is(h1, h2) + p",
        ":matches(a, b)",
        ":-webkit-any(a, b)",
        ":where(.x)",
        ":has(> img)",
        ":has(+ .x, ~ .y, .z)",
        ":is()",
        ":where(:bogus)",
        ":is(:bogus, a)",
        ":target",
        ":lang(en)",
        ":lang(\"fr\", de)",
        ":dir(rtl)",
        ":dir(foo)",
        ":placeholder-shown",
        ":indeterminate",
        ":default",
        ":required:optional",
        ":valid:invalid",
        ":read-only:read-write",
        ":in-range:out-of-range",
        ":defined",
        ":scope",
        ":fullscreen",
        ":modal",
        ":popover-open",
        ":autofill:-webkit-autofill",
        ":-moz-focusring",
        ":user-invalid:user-valid",
        ":host",
        ":host(.x)",
        ":host-context(.dark)",
        ":state(checked)",
        ":open:closed",
        ":playing:paused",
        ":first:left:right:blank",
        ":local-link:target-within",
        ":not(:hover)",
        "a:not([href])",
        "A:HOVER",
        "&",
        ".a&",
    ];
    for s in valid {
        assert!(parse_selector_list(s).is_some(), "{s:?} should be valid");
    }
}

#[test]
fn forgiving_lists_drop_invalid_items() {
    match &parse(":is(:bogus, .a, ::before)")[0].subject.pseudo_classes[0] {
        PseudoClass::Is(list) => assert_eq!(list, &parse(".a")),
        other => panic!("unexpected {other:?}"),
    }
    match &parse(":where(:bogus)")[0].subject.pseudo_classes[0] {
        PseudoClass::Where(list) => assert!(list.is_empty()),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn pseudo_class_variants() {
    let pc = |s: &str| parse(s)[0].subject.pseudo_classes[0].clone();
    assert_eq!(pc(":nth-child(2n+1)"), PseudoClass::NthChild { a: 2, b: 1 });
    assert_eq!(
        pc(":nth-last-child(-n+3)"),
        PseudoClass::NthLastChild { a: -1, b: 3 }
    );
    assert_eq!(
        pc(":nth-of-type(odd)"),
        PseudoClass::NthOfType { a: 2, b: 1 }
    );
    assert_eq!(
        pc(":nth-last-of-type(5)"),
        PseudoClass::NthLastOfType { a: 0, b: 5 }
    );
    assert_eq!(pc(":scope"), PseudoClass::Root);
    assert_eq!(pc(":visited"), PseudoClass::Visited);
    assert_eq!(pc(":valid"), PseudoClass::NeverMatches);
    assert_eq!(pc(":host(.x)"), PseudoClass::NeverMatches);
    assert_eq!(
        pc(":lang(en, \"fr\")"),
        PseudoClass::Lang(vec!["en".into(), "fr".into()])
    );
    assert_eq!(pc(":dir(RTL)"), PseudoClass::Dir { rtl: true });
    assert_eq!(pc(":dir(weird)"), PseudoClass::NeverMatches);
    assert_eq!(pc(":target"), PseudoClass::Target);
    match pc(":has(> a, + b, ~ c, d)") {
        PseudoClass::Has(list) => {
            let combinators: Vec<_> = list.iter().map(|(c, _)| *c).collect();
            assert_eq!(
                combinators,
                vec![
                    Combinator::Child,
                    Combinator::NextSibling,
                    Combinator::SubsequentSibling,
                    Combinator::Descendant
                ]
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    let s = &parse("a:before")[0];
    assert_eq!(s.pseudo_element, Some(PseudoElement::Before));
    assert_eq!(
        parse("p:first-line")[0].pseudo_element,
        Some(PseudoElement::Unsupported)
    );
    assert_eq!(
        parse("li::marker")[0].pseudo_element,
        Some(PseudoElement::Unsupported)
    );
}

#[test]
fn limits() {
    let long = vec!["a"; MAX_COMPOUNDS].join(" ");
    assert!(parse_selector_list(&long).is_some());
    let too_long = vec!["a"; MAX_COMPOUNDS + 1].join(" ");
    assert!(parse_selector_list(&too_long).is_none());
    let nested = format!(
        "{}a{}",
        ":is(".repeat(MAX_SELECTOR_NESTING),
        ")".repeat(MAX_SELECTOR_NESTING)
    );
    assert!(parse_selector_list(&nested).is_some());
    let deep = format!("{}a{}", ":not(".repeat(1000), ")".repeat(1000));
    assert!(parse_selector_list(&deep).is_none());
    let huge_list = vec!["a"; 20_000].join(",");
    assert_eq!(
        parse_selector_list(&huge_list).map(|l| l.len()),
        Some(20_000)
    );
}

// ---------------------------------------------------------------------------
// Specificity
// ---------------------------------------------------------------------------

#[test]
fn specificity_table() {
    let table = [
        ("*", (0, 0, 0)),
        ("li", (0, 0, 1)),
        ("ul li", (0, 0, 2)),
        ("ul ol+li", (0, 0, 3)),
        ("h1 + *[rel=up]", (0, 1, 1)),
        ("ul ol li.red", (0, 1, 3)),
        ("li.red.level", (0, 2, 1)),
        ("#x34y", (1, 0, 0)),
        ("#s12:not(FOO)", (1, 0, 1)),
        ("#x:not(.a.b)", (1, 2, 0)),
        (":where(#a) p", (0, 0, 1)),
        (":is(#a, p) .b", (1, 1, 0)),
        (":not(#a, .b)", (1, 0, 0)),
        (":has(> #a, .b)", (1, 0, 0)),
        ("a:hover", (0, 1, 1)),
        ("a::before", (0, 0, 2)),
        ("a:before", (0, 0, 2)),
        ("li::marker", (0, 0, 2)),
        ("a:hover::before", (0, 1, 2)),
        ("[a][b]", (0, 2, 0)),
        (":nth-child(2n+1)", (0, 1, 0)),
        (":lang(en)", (0, 1, 0)),
        (":valid", (0, 1, 0)),
        ("#a #b #c", (3, 0, 0)),
        ("*|*", (0, 0, 0)),
        ("ns|div", (0, 0, 1)),
        (":is()", (0, 0, 0)),
        (":root", (0, 1, 0)),
    ];
    for (selector, (a, b, c)) in table {
        assert_eq!(
            parse(selector)[0].specificity(),
            Specificity(a, b, c),
            "{selector}"
        );
    }
    assert!(Specificity(1, 0, 0) > Specificity(0, 99, 99));
    assert!(Specificity(0, 1, 0) > Specificity(0, 0, 99));
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// html > body > (div#main.content > (p.intro, p, ul > li*4), footer)
fn sample() -> (Dom, [usize; 10]) {
    let mut dom = Dom::default();
    let html = dom.add(None, "html[lang=en-US]");
    let body = dom.add(Some(html), "body");
    let main = dom.add(Some(body), "div#main.content.wide[data-x=Hello World]");
    let p1 = dom.add(Some(main), "p.intro");
    let p2 = dom.add(Some(main), "p");
    let ul = dom.add(Some(main), "ul");
    let li1 = dom.add(Some(ul), "li.a");
    let li2 = dom.add(Some(ul), "li.b");
    let li3 = dom.add(Some(ul), "li.a.c");
    let footer = dom.add(Some(body), "footer");
    dom.nodes[p1].has_text = true;
    (dom, [html, body, main, p1, p2, ul, li1, li2, li3, footer])
}

#[test]
fn simple_matching() {
    let (dom, [html, body, main, p1, p2, ul, li1, li2, li3, footer]) = sample();
    assert_eq!(select(&dom, "*").len(), dom.nodes.len());
    assert_eq!(select(&dom, "p"), vec![p1, p2]);
    assert_eq!(select(&dom, "P"), vec![p1, p2]);
    assert_eq!(select(&dom, "#main"), vec![main]);
    assert!(select(&dom, "#MAIN").is_empty());
    assert_eq!(select(&dom, ".a"), vec![li1, li3]);
    assert!(select(&dom, ".A").is_empty());
    assert_eq!(select(&dom, "li.a.c"), vec![li3]);
    assert_eq!(select(&dom, "div#main.content"), vec![main]);
    assert!(select(&dom, "#main#other").is_empty());
    assert_eq!(select(&dom, ":root"), vec![html]);
    assert_eq!(select(&dom, "body > *"), vec![main, footer]);
    assert_eq!(select(&dom, "html li"), vec![li1, li2, li3]);
    assert_eq!(select(&dom, "body li"), vec![li1, li2, li3]);
    assert!(select(&dom, "body > li").is_empty());
    assert_eq!(select(&dom, "ul > li + li"), vec![li2, li3]);
    assert_eq!(select(&dom, "li ~ .a"), vec![li3]);
    assert_eq!(select(&dom, "p + p"), vec![p2]);
    assert_eq!(select(&dom, "p ~ ul"), vec![ul]);
    assert_eq!(select(&dom, ".intro ~ *"), vec![p2, ul]);
    assert_eq!(select(&dom, "div p + ul li.a + li"), vec![li2]);
    assert_eq!(select(&dom, "p, footer"), vec![p1, p2, footer]);
    let _ = body;
}

#[test]
fn attribute_matching() {
    let mut dom = Dom::default();
    let root = dom.add(None, "div");
    let a = dom.add(Some(root), "a[href=https://Example.com/page.html][lang=en-GB][rel=Nofollow Noopener][type=Text/CSS][data-v=]");
    let check = |s: &str| matches(&dom, s, a);
    assert!(check("[href]"));
    assert!(check("[HREF]"));
    assert!(!check("[title]"));
    assert!(check("[href='https://Example.com/page.html']"));
    assert!(!check("[href='https://example.com/page.html']"));
    assert!(check("[href='https://example.com/page.html' i]"));
    assert!(check("[href^=https]"));
    assert!(check("[href^='HTTPS' i]"));
    assert!(!check("[href^=HTTPS]"));
    assert!(check("[href$='.html']"));
    assert!(check("[href$='.HTML' i]"));
    assert!(!check("[href$='.HTML']"));
    assert!(check("[href*=Example]"));
    assert!(!check("[href*=example]"));
    assert!(check("[href*=example i]"));
    assert!(check("[lang|=en]"));
    assert!(
        check("[lang|=EN]"),
        "lang values are case-insensitive in HTML"
    );
    assert!(check("[lang|=en s]"));
    assert!(!check("[lang|=EN s]"));
    assert!(!check("[lang|=e]"));
    assert!(check("[rel~=noopener]"));
    assert!(check("[rel~=Nofollow]"));
    assert!(!check("[rel~='Nofollow Noopener']"));
    assert!(check("[type='text/css']"));
    assert!(!check("[type='text/css' s]"));
    // Empty values never match for the substring operators.
    assert!(!check("[href^='']"));
    assert!(!check("[href$='']"));
    assert!(!check("[href*='']"));
    assert!(!check("[rel~='']"));
    assert!(check("[data-v='']"));
    assert!(check("[data-v]"));
}

#[test]
fn structural_pseudo_classes() {
    let mut dom = Dom::default();
    let root = dom.add(None, "ul");
    let items: Vec<usize> = (0..7)
        .map(|i| dom.add(Some(root), if i % 3 == 0 { "p" } else { "li" }))
        .collect();
    // children: p li li p li li p
    let empty = dom.add(None, "div");
    let pick = |v: &[usize]| v.iter().map(|&i| items[i]).collect::<Vec<_>>();
    assert_eq!(select(&dom, "ul > :first-child"), pick(&[0]));
    assert_eq!(select(&dom, "ul > :last-child"), pick(&[6]));
    assert!(select(&dom, "ul > :only-child").is_empty());
    assert_eq!(select(&dom, "li:first-of-type"), pick(&[1]));
    assert_eq!(select(&dom, "li:last-of-type"), pick(&[5]));
    assert_eq!(select(&dom, "ul > :nth-child(odd)"), pick(&[0, 2, 4, 6]));
    assert_eq!(select(&dom, "ul > :nth-child(even)"), pick(&[1, 3, 5]));
    assert_eq!(select(&dom, "ul > :nth-child(-n+3)"), pick(&[0, 1, 2]));
    assert_eq!(select(&dom, "ul > :nth-child(n+5)"), pick(&[4, 5, 6]));
    assert_eq!(select(&dom, "ul > :nth-child(3n)"), pick(&[2, 5]));
    assert_eq!(select(&dom, "ul > :nth-child(2)"), pick(&[1]));
    assert_eq!(select(&dom, "ul > :nth-last-child(2)"), pick(&[5]));
    assert_eq!(select(&dom, "ul > :nth-last-child(-n+2)"), pick(&[5, 6]));
    assert_eq!(select(&dom, "li:nth-of-type(2n+1)"), pick(&[1, 4]));
    assert_eq!(select(&dom, "p:nth-of-type(2)"), pick(&[3]));
    assert_eq!(select(&dom, "p:nth-last-of-type(1)"), pick(&[6]));
    assert_eq!(select(&dom, ":empty"), {
        let mut v = items.clone();
        v.push(empty);
        v
    });
    // Two roots in this test DOM: both are :root.
    assert_eq!(select(&dom, ":root"), vec![root, empty]);
    let mut dom2 = Dom::default();
    let r = dom2.add(None, "div");
    let only = dom2.add(Some(r), "span");
    assert!(matches(&dom2, ":only-child", only));
    assert!(matches(&dom2, ":only-of-type", only));
    assert!(!matches(&dom2, ":empty", r));
}

#[test]
fn logical_pseudo_classes() {
    let (dom, [html, _body, main, p1, p2, ul, li1, li2, li3, footer]) = sample();
    assert_eq!(select(&dom, "li:not(.a)"), vec![li2]);
    assert_eq!(select(&dom, "li:not(.a, .b)"), Vec::<usize>::new());
    assert_eq!(select(&dom, "li:not(ul > .b)"), vec![li1, li3]);
    assert_eq!(select(&dom, ":is(p, ul) > li.a"), vec![li1, li3]);
    assert_eq!(select(&dom, ":is(.intro, footer)"), vec![p1, footer]);
    assert_eq!(select(&dom, ":where(#main) > p"), vec![p1, p2]);
    assert_eq!(select(&dom, ":is(div p)"), vec![p1, p2]);
    assert_eq!(select(&dom, "li:is(:bogus, .c)"), vec![li3]);
    assert!(select(&dom, ":is()").is_empty());
    // :has
    assert_eq!(select(&dom, "div:has(li)"), vec![main]);
    assert_eq!(select(&dom, ":has(> li.c)"), vec![ul]);
    assert_eq!(select(&dom, "ul:has(> .b + .c)"), vec![ul]);
    assert!(select(&dom, "ul:has(> .c + .b)").is_empty());
    assert_eq!(select(&dom, "p:has(+ p)"), vec![p1]);
    assert_eq!(select(&dom, "p:has(~ ul)"), vec![p1, p2]);
    assert_eq!(select(&dom, "p:has(+ ul)"), vec![p2]);
    assert_eq!(select(&dom, "p:has(~ ul > .c)"), vec![p1, p2]);
    assert_eq!(select(&dom, "p:has(+ ul .c)"), vec![p2]);
    assert!(select(&dom, "p:has(+ p .c)").is_empty());
    assert_eq!(select(&dom, "body:has(> div ul > li)").len(), 1);
    assert!(select(&dom, "body:has(> ul)").is_empty());
    assert_eq!(select(&dom, "div:has(> ul li + li)"), vec![main]);
    // `> ul li`: li must be under a ul that is a child of the anchor.
    assert!(select(&dom, "body:has(> ul li)").is_empty());
    assert_eq!(select(&dom, "li:has(~ .c)"), vec![li1, li2]);
    assert!(select(&dom, "li:has(+ .a)").contains(&li2));
    assert_eq!(select(&dom, ":has(.intro):not(body)"), vec![html, main]);
    assert!(select(&dom, "li:has(*)").is_empty());
    let _ = (li1, li2, li3);
}

#[test]
fn dynamic_and_form_pseudo_classes() {
    let mut dom = Dom::default();
    let form = dom.add(None, "form");
    let text = dom.add(Some(form), "input[type=text][placeholder=Name][required]");
    let filled = dom.add(Some(form), "input[placeholder=x][value=v][readonly]");
    let check = dom.add(Some(form), "input[type=checkbox][checked]");
    let disabled = dom.add(Some(form), "button[disabled]");
    let area = dom.add(Some(form), "textarea[placeholder=x]");
    let link = dom.add(Some(form), "a[href=#x]");
    let anchor = dom.add(Some(form), "a");
    let editable = dom.add(Some(form), "div[contenteditable]");
    let inner = dom.add(Some(editable), "span");
    let progress = dom.add(Some(form), "progress");
    let option = dom.add(Some(form), "option[selected]");
    let details = dom.add(Some(form), "details[open]");
    let dialog = dom.add(Some(form), "dialog");
    dom.nodes[check].checked = true;
    dom.nodes[text].focused = true;
    dom.nodes[link].hovered = true;

    assert_eq!(select(&dom, ":checked"), vec![check]);
    assert_eq!(select(&dom, ":disabled"), vec![disabled]);
    assert_eq!(
        select(&dom, ":enabled"),
        vec![text, filled, check, area, option]
    );
    assert_eq!(select(&dom, ":focus"), vec![text]);
    assert_eq!(select(&dom, ":focus-visible"), vec![text]);
    assert_eq!(select(&dom, ":focus-within"), vec![form, text]);
    assert_eq!(select(&dom, ":hover"), vec![link]);
    assert!(select(&dom, ":active").is_empty());
    assert_eq!(select(&dom, ":link"), vec![link]);
    assert_eq!(select(&dom, ":any-link"), vec![link]);
    assert!(select(&dom, ":visited").is_empty());
    assert!(!matches(&dom, ":link", anchor));
    assert_eq!(select(&dom, ":required"), vec![text]);
    assert_eq!(select(&dom, ":optional"), vec![filled, check, area]);
    assert_eq!(select(&dom, ":placeholder-shown"), vec![text, area]);
    assert_eq!(
        select(&dom, ":read-write"),
        vec![text, area, editable, inner]
    );
    assert!(matches(&dom, ":read-only", filled));
    assert!(matches(&dom, ":read-only", check));
    assert_eq!(select(&dom, ":indeterminate"), vec![progress]);
    assert_eq!(select(&dom, ":default"), vec![check, option]);
    assert_eq!(select(&dom, ":open"), vec![details]);
    assert_eq!(select(&dom, ":closed"), vec![dialog]);
    assert_eq!(select(&dom, ":defined").len(), dom.nodes.len());
    for never in [
        ":target",
        ":valid",
        ":fullscreen",
        ":-webkit-autofill",
        ":host",
    ] {
        assert!(select(&dom, never).is_empty(), "{never}");
    }
}

#[test]
fn lang_and_dir() {
    let mut dom = Dom::default();
    let html = dom.add(None, "html[lang=en-US]");
    let fr = dom.add(Some(html), "div[lang=fr][dir=rtl]");
    let inner = dom.add(Some(fr), "p");
    let ltr = dom.add(Some(fr), "p[dir=ltr]");
    let plain = dom.add(Some(html), "p");
    assert_eq!(select(&dom, ":lang(en)"), vec![html, plain]);
    assert_eq!(select(&dom, ":lang(EN-us)"), vec![html, plain]);
    assert!(select(&dom, ":lang(en-GB)").is_empty());
    assert_eq!(select(&dom, ":lang(fr)"), vec![fr, inner, ltr]);
    assert_eq!(select(&dom, ":lang(de, fr)"), vec![fr, inner, ltr]);
    assert!(select(&dom, ":lang(f)").is_empty());
    assert_eq!(select(&dom, ":lang('*-US')"), vec![html, plain]);
    assert_eq!(select(&dom, ":dir(rtl)"), vec![fr, inner]);
    assert_eq!(select(&dom, ":dir(ltr)"), vec![html, ltr, plain]);
}

#[test]
fn pseudo_elements_are_ignored_by_matching() {
    let (dom, [_html, _body, _main, p1, p2, ..]) = sample();
    assert_eq!(select(&dom, "p::before"), vec![p1, p2]);
    assert_eq!(select(&dom, "p:hover::after"), Vec::<usize>::new());
}

#[test]
fn nested_selectors_match() {
    let (dom, [_html, _body, main, _p1, _p2, _ul, li1, _li2, li3, _footer]) = sample();
    let parent = parse("#main, footer");
    let nested =
        parse_nested_selector_list(&crate::parser::parse_component_values("& li.a"), &parent)
            .expect("valid nested selector");
    let found: Vec<usize> = (0..dom.nodes.len())
        .filter(|&i| nested.iter().any(|s| s.matches(&dom.el(i))))
        .collect();
    assert_eq!(found, vec![li1, li3]);
    let nested = parse_nested_selector_list(
        &crate::parser::parse_component_values("> ul > .c"),
        &parse("div.content"),
    )
    .expect("valid nested selector");
    assert!(nested[0].matches(&dom.el(li3)));
    assert!(!nested[0].matches(&dom.el(li1)));
    let _ = main;
}

#[test]
fn backtracking_is_correct() {
    // a > b c must try every `b` ancestor.
    let mut dom = Dom::default();
    let a = dom.add(None, "a");
    let b1 = dom.add(Some(a), "b");
    let x = dom.add(Some(b1), "x");
    let b2 = dom.add(Some(x), "b");
    let c = dom.add(Some(b2), "c");
    assert!(matches(&dom, "a > b c", c));
    assert!(matches(&dom, "a > b > x > b > c", c));
    assert!(!matches(&dom, "a > b > c", c));
    assert!(matches(&dom, "a b b c", c));
    assert!(!matches(&dom, "a b b b c", c));
    // Siblings with backtracking: p ~ q + r
    let mut dom = Dom::default();
    let root = dom.add(None, "div");
    let _p = dom.add(Some(root), "p");
    let _q1 = dom.add(Some(root), "q");
    let _s = dom.add(Some(root), "s");
    let _q2 = dom.add(Some(root), "q");
    let r = dom.add(Some(root), "r");
    assert!(matches(&dom, "p ~ q + r", r));
    assert!(matches(&dom, "p + q ~ r", r));
    assert!(!matches(&dom, "s + q + q + r", r));
    assert!(matches(&dom, "div > p ~ s ~ r", r));
}

#[test]
fn no_exponential_blowup_on_deep_trees() {
    let mut dom = Dom::default();
    let mut parent = dom.add(None, "a");
    for _ in 0..250 {
        parent = dom.add(Some(parent), "a");
    }
    let leaf = dom.add(Some(parent), "b");
    let start = std::time::Instant::now();
    // Never matches: there is no `c` ancestor, which is only discovered at the top.
    assert!(!matches(&dom, "c a a a a a a a a a b", leaf));
    assert!(!matches(&dom, "c > a a a a a a a a b", leaf));
    assert!(matches(&dom, "a a a a a a a a a a b", leaf));
    for i in 0..dom.nodes.len() {
        let _ = matches(&dom, "c a a a a a a a", i);
        let _ = matches(&dom, ":has(c) a a", i);
    }
    assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
}

#[test]
fn many_siblings_and_has_budget() {
    let mut dom = Dom::default();
    let root = dom.add(None, "ul");
    for _ in 0..2_000 {
        dom.add(Some(root), "li");
    }
    let start = std::time::Instant::now();
    let last = dom.nodes.len() - 1;
    assert!(matches(&dom, "li:nth-child(2000)", last));
    assert!(matches(&dom, "li ~ li ~ li", last));
    assert!(!matches(&dom, "p ~ li ~ li ~ li", last));
    assert!(matches(&dom, "ul:has(li + li + li)", root));
    assert!(!matches(&dom, "ul:has(p ~ li)", root));
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn default_focus_within_is_bounded() {
    let mut dom = Dom::default();
    let root = dom.add(None, "div");
    let mut parent = root;
    for _ in 0..10_000 {
        parent = dom.add(Some(parent), "div");
    }
    dom.nodes[parent].focused = true;
    // The focused element is deeper than the default walk's budget.
    assert!(!dom.el(root).is_focus_within());
    let near = dom.nodes[parent].parent.expect("has parent");
    assert!(dom.el(near).is_focus_within());
}
