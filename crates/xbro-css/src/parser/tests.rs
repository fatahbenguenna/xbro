use super::*;
use crate::media::MediaContext;
use crate::selector::{parse_selector_list, Combinator, PseudoClass, PseudoElement, Specificity};
use crate::tokenizer::tokenize;

/// The style rules of a sheet, flattened (ignoring conditions).
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

fn value_css(decl: &Declaration) -> String {
    to_css(&decl.value)
}

/// `(name, value, important)` triples of a style rule.
fn decls(rule: &StyleRule) -> Vec<(String, String, bool)> {
    rule.declarations
        .iter()
        .map(|d| (d.name.clone(), value_css(d), d.important))
        .collect()
}

fn d(name: &str, value: &str) -> (String, String, bool) {
    (name.to_string(), value.to_string(), false)
}

fn sel(s: &str) -> Vec<crate::selector::Selector> {
    parse_selector_list(s).unwrap_or_else(|| panic!("selector {s:?} should parse"))
}

#[test]
fn simple_stylesheet() {
    let sheet = parse_stylesheet("a { color: red; margin: 0 auto } b{}");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].selectors, sel("a"));
    assert_eq!(
        decls(rules[0]),
        vec![d("color", "red"), d("margin", "0 auto")]
    );
    assert_eq!(rules[1].selectors, sel("b"));
    assert!(rules[1].declarations.is_empty());
}

#[test]
fn empty_and_whitespace_sheets() {
    assert!(parse_stylesheet("").rules.is_empty());
    assert!(parse_stylesheet("  \n\t /* only a comment */ ")
        .rules
        .is_empty());
}

#[test]
fn declaration_names_are_lowercased_except_custom_properties() {
    let sheet = parse_stylesheet("a { COLOR: Red; --MyVar: Foo }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(
        decls(rules[0]),
        vec![d("color", "Red"), d("--MyVar", "Foo")]
    );
}

#[test]
fn empty_declarations_are_skipped() {
    let sheet = parse_stylesheet("a{color:red;;;color:blue}");
    let rules = style_rules(&sheet.rules);
    assert_eq!(decls(rules[0]), vec![d("color", "red"), d("color", "blue")]);
}

#[test]
fn stray_closing_braces_drop_the_next_rule() {
    // At top level a `}` becomes part of the next rule's prelude, which
    // therefore fails to parse as a selector.
    let sheet = parse_stylesheet("a{color:red} }} b{color:blue} c{color:green}");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].selectors, sel("a"));
    assert_eq!(rules[1].selectors, sel("c"));
}

#[test]
fn unclosed_blocks_at_eof() {
    let sheet = parse_stylesheet("a { color: red");
    let rules = style_rules(&sheet.rules);
    assert_eq!(decls(rules[0]), vec![d("color", "red")]);

    let sheet = parse_stylesheet("@media screen { a { color: red");
    match &sheet.rules[0] {
        CssRule::Media { rules, .. } => assert_eq!(style_rules(rules).len(), 1),
        other => panic!("unexpected {other:?}"),
    }

    // A rule with only a prelude is dropped.
    assert!(parse_stylesheet("a, b").rules.is_empty());
    let sheet = parse_stylesheet("a{x:y} b");
    assert_eq!(style_rules(&sheet.rules).len(), 1);

    // Unclosed function in a value.
    let sheet = parse_stylesheet("a { background: url(x.png");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules[0].declarations[0].name, "background");
}

#[test]
fn bad_declarations_are_skipped_to_the_next_semicolon() {
    let sheet =
        parse_stylesheet("a { color red; *zoom: 1; width: 10px; 123: x; height: (1;2); top: 0 }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(
        decls(rules[0]),
        vec![d("width", "10px"), d("height", "(1;2)"), d("top", "0")]
    );
}

#[test]
fn semicolons_inside_blocks_do_not_end_declarations() {
    let sheet = parse_stylesheet("a { content: \"a;b\"; background: url(a;b.png); x: f(1;2) }");
    let rules = style_rules(&sheet.rules);
    let names: Vec<_> = rules[0]
        .declarations
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(names, vec!["content", "background", "x"]);
    assert_eq!(value_css(&rules[0].declarations[2]), "f(1;2)");
}

#[test]
fn important_flag() {
    let sheet = parse_stylesheet(
        "a { color: red !important; width: 1px ! IMPORTANT ; height: 2px!Important; \
         top: 1px !ie; --x: 1 !important }",
    );
    let rules = style_rules(&sheet.rules);
    let got = decls(rules[0]);
    assert_eq!(
        got,
        vec![
            ("color".into(), "red".into(), true),
            ("width".into(), "1px".into(), true),
            ("height".into(), "2px".into(), true),
            ("top".into(), "1px !ie".into(), false),
            ("--x".into(), "1".into(), true),
        ]
    );
}

#[test]
fn custom_properties_keep_their_value() {
    let sheet = parse_stylesheet("a { --empty:; --space: ; --x: { a: b } ; --y:[1, 2] foo(bar) }");
    let rules = style_rules(&sheet.rules);
    let decl = &rules[0].declarations;
    assert_eq!(decl.len(), 4);
    assert_eq!(decl[0].name, "--empty");
    assert!(decl[0].value.is_empty());
    assert_eq!(decl[1].name, "--space");
    assert!(decl[1].value.is_empty());
    assert_eq!(decl[2].name, "--x");
    assert!(matches!(
        decl[2].value[0],
        ComponentValue::Block { open: '{', .. }
    ));
    assert_eq!(to_css(&decl[3].value), "[1, 2] foo(bar)");
}

#[test]
fn curly_block_in_normal_value_is_invalid() {
    let sheet = parse_stylesheet("a { color: red { x } ; width: 1px; foo: {bar} }");
    let rules = style_rules(&sheet.rules);
    // `color: red { x }` is not a declaration (and `color: red` is not a valid
    // selector for a nested rule); `foo: {bar}` is (a block as the whole value).
    let flat: Vec<_> = rules
        .iter()
        .flat_map(|r| r.declarations.iter().map(|d| d.name.clone()))
        .collect();
    assert_eq!(flat, vec!["width", "foo"]);
}

#[test]
fn invalid_selectors_drop_the_rule() {
    let sheet = parse_stylesheet(
        "a:unknown { color: red } b { color: blue } c, d:bogus(1) { x: y } e::nope {} #1 {} f {}",
    );
    let rules = style_rules(&sheet.rules);
    let selectors: Vec<_> = rules.iter().map(|r| r.selectors.clone()).collect();
    assert_eq!(selectors, vec![sel("b"), sel("f")]);
}

#[test]
fn cdo_cdc_are_ignored_at_top_level() {
    let sheet = parse_stylesheet("<!-- a { color: red } --> <!-- b { x: y } -->");
    assert_eq!(style_rules(&sheet.rules).len(), 2);
}

#[test]
fn media_rules() {
    let sheet = parse_stylesheet("@media screen{a{b:c}} @media print { d { e: f } }");
    assert_eq!(sheet.rules.len(), 2);
    let ctx = MediaContext::default();
    match &sheet.rules[0] {
        CssRule::Media { query, rules } => {
            assert!(query.matches(&ctx));
            assert_eq!(style_rules(rules).len(), 1);
            assert_eq!(decls(style_rules(rules)[0]), vec![d("b", "c")]);
        }
        other => panic!("unexpected {other:?}"),
    }
    match &sheet.rules[1] {
        CssRule::Media { query, .. } => assert!(!query.matches(&ctx)),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn nested_conditional_rules() {
    let sheet = parse_stylesheet(
        "@media screen { @supports (display: flex) { @media (min-width: 10px) { a { x: y } } } }",
    );
    match &sheet.rules[0] {
        CssRule::Media { rules, .. } => match &rules[0] {
            CssRule::Supports { supported, rules } => {
                assert!(*supported);
                assert!(matches!(rules[0], CssRule::Media { .. }));
            }
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(style_rules(&sheet.rules).len(), 1);
}

#[test]
fn declarations_directly_in_top_level_media_are_dropped() {
    let sheet = parse_stylesheet("@media all { color: red; a { x: y } b; c { z: w } }");
    let rules = style_rules(&sheet.rules);
    let selectors: Vec<_> = rules.iter().map(|r| r.selectors.clone()).collect();
    assert_eq!(selectors, vec![sel("a"), sel("c")]);
}

#[test]
fn supports_rules() {
    let cases = [
        ("(display: flex)", true),
        ("(display: grid)", true),
        ("(DISPLAY: Flex)", true),
        ("(display: bogus)", false),
        ("(transform: rotate(1deg))", false),
        ("(color: rgb(1 2 3 / 50%))", true),
        ("(color: color-mix(in srgb, red, blue))", true),
        ("(color: notacolor)", false),
        ("(--custom: x)", true),
        ("not (display: flex)", false),
        ("not (transform: none)", true),
        ("(display: flex) and (transform: none)", false),
        ("(display: flex) or (transform: none)", true),
        (
            "(display: flex) and (gap: 1px) and (position: sticky)",
            true,
        ),
        (
            "((display: flex) and (gap: 1px)) or (transform: none)",
            true,
        ),
        ("(display: flex) and (gap: 1px) or (x: y)", false),
        ("selector(a > b)", true),
        ("selector(:has(> img))", true),
        ("selector(:bogus)", false),
        ("selector(a, b)", false),
        ("font-tech(color-COLRv1)", false),
        ("not font-tech(color-COLRv1)", true),
        ("(foo)", false),
        ("not (foo)", true),
        ("", false),
        ("display: flex", false),
        ("(display:)", false),
        ("not(display: flex)", false),
    ];
    for (condition, expected) in cases {
        let sheet = parse_stylesheet(&format!("@supports {condition} {{ a {{ x: y }} }}"));
        match &sheet.rules[..] {
            [CssRule::Supports { supported, rules }] => {
                assert_eq!(*supported, expected, "@supports {condition}");
                assert_eq!(style_rules(rules).len(), 1);
            }
            other => panic!("unexpected {other:?} for {condition}"),
        }
    }
    assert!(is_supported_property("margin-left"));
    assert!(is_supported_property("Z-INDEX"));
    assert!(!is_supported_property("transform"));
    assert!(!is_supported_property("animation"));
}

#[test]
fn import_rules() {
    let sheet = parse_stylesheet(
        "@charset \"utf-8\"; @layer base, theme; @import url(a.css); @import \"b.css\" screen; \
         @import url('c.css') layer(x) supports(display: grid) print, screen; \
         @import url(d.css) supports(transform: none); @import 42; \
         a {} @import url(late.css);",
    );
    let imports: Vec<_> = sheet
        .rules
        .iter()
        .filter_map(|r| match r {
            CssRule::Import { url, media } => Some((url.clone(), to_css(&media.values))),
            _ => None,
        })
        .collect();
    assert_eq!(
        imports,
        vec![
            ("a.css".to_string(), String::new()),
            ("b.css".to_string(), "screen".to_string()),
            ("c.css".to_string(), "print, screen".to_string()),
        ]
    );
    assert_eq!(style_rules(&sheet.rules).len(), 1);
}

#[test]
fn layer_container_scope_and_document_are_groups() {
    let sheet = parse_stylesheet(
        "@layer base { a { x: 1 } } @layer { b { x: 2 } } @layer x, y; \
         @container sidebar (min-width: 400px) { c { x: 3 } } @scope (.card) { d { x: 4 } } \
         @document url-prefix() { e { x: 5 } } @-moz-document url-prefix() { f { x: 6 } }",
    );
    assert_eq!(sheet.rules.len(), 6);
    assert!(sheet
        .rules
        .iter()
        .all(|r| matches!(r, CssRule::Group { .. })));
    assert_eq!(style_rules(&sheet.rules).len(), 6);
}

#[test]
fn unknown_and_unsupported_at_rules_are_dropped() {
    let sheet = parse_stylesheet(
        "@font-face { font-family: x; src: url(x.woff) } @keyframes spin { from { x: y } } \
         @-webkit-keyframes spin { to { x: y } } @page :first { margin: 1in } \
         @namespace svg url(http://www.w3.org/2000/svg); @property --x { syntax: '*' } \
         @counter-style foo { system: cyclic } @font-feature-values Font { @swash { x: 1 } } \
         @starting-style { a { opacity: 0 } } @unknown foo { a { x: y } } @bogus; \
         b { x: y }",
    );
    let rules = style_rules(&sheet.rules);
    assert_eq!(sheet.rules.len(), 1);
    assert_eq!(rules[0].selectors, sel("b"));
}

#[test]
fn at_rule_without_block_swallows_up_to_semicolon() {
    let sheet = parse_stylesheet("@media screen; a { x: y }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(sheet.rules.len(), 1);
    assert_eq!(rules[0].selectors, sel("a"));
}

#[test]
fn nesting_basic() {
    let sheet = parse_stylesheet(
        ".a { color: red; & .b { x: 1 } .c { x: 2 } &:hover { x: 3 } > d { x: 4 } + e { x: 5 } \
         ~ f { x: 6 } .g & { x: 7 } }",
    );
    let rules = style_rules(&sheet.rules);
    let got: Vec<_> = rules.iter().map(|r| r.selectors.clone()).collect();
    assert_eq!(
        got,
        vec![
            sel(".a"),
            sel(".a .b"),
            sel(".a .c"),
            sel(".a:hover"),
            sel(".a > d"),
            sel(".a + e"),
            sel(".a ~ f"),
            sel(".g .a"),
        ]
    );
    assert_eq!(decls(rules[0]), vec![d("color", "red")]);
    assert_eq!(decls(rules[3]), vec![d("x", "3")]);
}

#[test]
fn nesting_with_ident_starting_selectors() {
    // `div:hover {}` looks like a declaration until its `{}` block.
    let sheet = parse_stylesheet(".a { div:hover { x: 1 } span { x: 2 } color: red }");
    let rules = style_rules(&sheet.rules);
    let got: Vec<_> = rules.iter().map(|r| r.selectors.clone()).collect();
    assert_eq!(got, vec![sel(".a div:hover"), sel(".a span"), sel(".a")]);
    assert_eq!(decls(rules[2]), vec![d("color", "red")]);
}

#[test]
fn nesting_declarations_after_nested_rules_belong_to_parent() {
    let sheet = parse_stylesheet(".p { a: 1; .c { b: 2 } d: 3; e: 4 }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[0].selectors, sel(".p"));
    assert_eq!(decls(rules[0]), vec![d("a", "1")]);
    assert_eq!(rules[1].selectors, sel(".p .c"));
    assert_eq!(rules[2].selectors, sel(".p"));
    assert_eq!(decls(rules[2]), vec![d("d", "3"), d("e", "4")]);
}

#[test]
fn nesting_multiple_parent_selectors_use_is() {
    let sheet = parse_stylesheet(".a, #b { .c { x: 1 } &.d { x: 2 } }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 2);
    let nested = &rules[0].selectors[0];
    assert_eq!(nested.subject.classes, vec!["c".to_string()]);
    assert_eq!(nested.ancestors.len(), 1);
    assert_eq!(nested.ancestors[0].0, Combinator::Descendant);
    assert_eq!(
        nested.ancestors[0].1.pseudo_classes,
        vec![PseudoClass::Is(sel(".a, #b"))]
    );
    // Specificity of `&` is that of :is(.a, #b) = (1,0,0).
    assert_eq!(nested.specificity(), crate::selector::Specificity(1, 1, 0));
    let compound = &rules[1].selectors[0].subject;
    assert_eq!(compound.classes, vec!["d".to_string()]);
    assert_eq!(
        compound.pseudo_classes,
        vec![PseudoClass::Is(sel(".a, #b"))]
    );
}

#[test]
fn nesting_deep_and_nested_lists() {
    let sheet = parse_stylesheet(".a { .b { .c { x: 1 } } &.d, .e { x: 2 } }");
    let rules = style_rules(&sheet.rules);
    let got: Vec<_> = rules.iter().map(|r| r.selectors.clone()).collect();
    // Rules without declarations of their own produce no style rule.
    assert_eq!(got, vec![sel(".a .b .c"), sel(".a.d, .a .e")]);
}

#[test]
fn nesting_with_type_selectors_and_conflicts() {
    let sheet = parse_stylesheet("div { p& { x: 1 } &p { x: 2 } div& { x: 3 } }");
    let rules = style_rules(&sheet.rules);
    // `p&` cannot merge `p` and `div` into one compound: :is(div) is used.
    let first = &rules[0].selectors[0];
    assert_eq!(first.subject.tag.as_deref(), Some("p"));
    assert_eq!(
        first.subject.pseudo_classes,
        vec![PseudoClass::Is(sel("div"))]
    );
    // `&p` is invalid (type selector must come first): dropped.
    assert_eq!(rules.len(), 2);
    // `div&` is `div:is(div)`: same matching as `div`, but `&` keeps the
    // specificity of its parent, so it weighs (0,0,2).
    assert_eq!(rules[1].selectors, sel("div:is(div)"));
    assert_eq!(rules[1].selectors[0].specificity(), Specificity(0, 0, 2));
}

#[test]
fn nesting_parent_with_pseudo_element() {
    let sheet = parse_stylesheet("a::before { content: 'x'; &:hover { x: 1 } }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].selectors[0].pseudo_element,
        Some(PseudoElement::Before)
    );
    let sheet = parse_stylesheet("a::before, b { &:hover { x: 1 } }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].selectors, sel("b:hover"));
}

#[test]
fn nesting_media_inside_style_rule() {
    let sheet = parse_stylesheet(
        ".a { color: red; @media (max-width: 100px) { color: blue; .b { x: 1 } } top: 0 }",
    );
    assert_eq!(sheet.rules.len(), 3);
    match &sheet.rules[1] {
        CssRule::Media { query, rules } => {
            assert_eq!(to_css(&query.values), "(max-width: 100px)");
            let inner = style_rules(rules);
            assert_eq!(inner.len(), 2);
            assert_eq!(inner[0].selectors, sel(".a"));
            assert_eq!(decls(inner[0]), vec![d("color", "blue")]);
            assert_eq!(inner[1].selectors, sel(".a .b"));
        }
        other => panic!("unexpected {other:?}"),
    }
    match &sheet.rules[2] {
        CssRule::Style(rule) => assert_eq!(decls(rule), vec![d("top", "0")]),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn nesting_supports_layer_and_dropped_at_rules() {
    let sheet = parse_stylesheet(
        ".a { @supports (display: grid) { x: 1 } @layer l { y: 2 } \
         @starting-style { opacity: 0 } @font-face { z: 3 } w: 4 }",
    );
    // @supports, @layer, then the trailing `w: 4`.
    assert_eq!(sheet.rules.len(), 3);
    assert!(matches!(
        sheet.rules[0],
        CssRule::Supports {
            supported: true,
            ..
        }
    ));
    assert!(matches!(sheet.rules[1], CssRule::Group { .. }));
    match &sheet.rules[2] {
        CssRule::Style(rule) => assert_eq!(decls(rule), vec![d("w", "4")]),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn nested_invalid_rules_are_dropped_without_affecting_the_parent() {
    let sheet = parse_stylesheet(".a { x: 1; :bogus { y: 2 } z: 3 }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 1);
    assert_eq!(decls(rules[0]), vec![d("x", "1"), d("z", "3")]);
}

#[test]
fn top_level_nesting_selector_means_root() {
    let sheet = parse_stylesheet("& { x: 1 }");
    let rules = style_rules(&sheet.rules);
    assert_eq!(
        rules[0].selectors[0].subject.pseudo_classes,
        vec![PseudoClass::Root]
    );
}

#[test]
fn declaration_list_parsing() {
    let decls = parse_declaration_list(" color : red ; background:blue!important;; bad; x:y");
    let got: Vec<_> = decls
        .iter()
        .map(|d| (d.name.as_str(), to_css(&d.value), d.important))
        .collect();
    assert_eq!(
        got,
        vec![
            ("color", "red".to_string(), false),
            ("background", "blue".to_string(), true),
            ("x", "y".to_string(), false),
        ]
    );
    assert!(parse_declaration_list("").is_empty());
    // Nested rules are ignored in declaration lists.
    let decls = parse_declaration_list("a: 1; b { c: 2 } d: 3");
    let names: Vec<_> = decls.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, vec!["a", "d"]);
}

#[test]
fn component_values_parse() {
    let values = parse_component_values("calc(1px + 2%) [a] {b}");
    assert_eq!(values.len(), 5);
    assert!(values[0].is_function("CALC"));
    assert!(values[1].is_whitespace());
    assert!(matches!(values[2], ComponentValue::Block { open: '[', .. }));
}

#[test]
fn component_value_helpers() {
    let values = parse_component_values("foo , ! url(x)");
    assert_eq!(values[0].ident(), Some("foo"));
    assert!(values[0].is_ident("FOO"));
    assert!(values[2].is_comma());
    assert!(values[4].is_delim('!'));
    assert!(!values[4].is_delim('?'));
    assert!(values[1].is_whitespace());
}

fn round_trip(css: &str) {
    let values = parse_component_values(css);
    let serialized = to_css(&values);
    let reparsed = parse_component_values(&serialized);
    assert_eq!(values, reparsed, "{css:?} serialized as {serialized:?}");
}

#[test]
fn serialization_round_trips() {
    let cases = [
        "1px solid red",
        "calc(100% - (2 * 3.5em))",
        "\"a \\\"quoted\\\" string\" 'single'",
        "\"line\\a break\"",
        "url(a\\ b.png) url( c.png )",
        "url(\"quoted.png\")",
        "#fff #123 #-a",
        "rgb(255 0 0 / 50%)",
        "1.5 -2 +3 .5 1e3 1.0 10%",
        "a{b:c}[d](e)",
        "@media",
        "\\31 23 \\-",
        "-- --x -\\31 a",
        "a\\.b c\\:d",
        "1\\65 3",
        "1\\65 m",
        "U+0025-00FF",
        "! important",
        "<!-- -->",
        "a/b * c",
        "foo(bar)baz",
        "\u{1}x",
        "\"\u{1}\"",
        "\"unterminated\nnext",
        "url(a b) next",
        "\\\n",
    ];
    for css in cases {
        round_trip(css);
    }
}

#[test]
fn serialization_inserts_comments_between_merging_tokens() {
    use crate::tokenizer::Token;
    let tokens = |t: Vec<Token>| t.into_iter().map(ComponentValue::Token).collect::<Vec<_>>();
    let cases = vec![
        tokens(vec![
            Token::Number {
                value: 1.0,
                is_integer: true,
            },
            Token::Ident("px".into()),
        ]),
        tokens(vec![Token::Ident("a".into()), Token::Ident("b".into())]),
        tokens(vec![
            Token::Ident("a".into()),
            Token::Number {
                value: -1.0,
                is_integer: true,
            },
        ]),
        tokens(vec![Token::Delim('#'), Token::Ident("x".into())]),
        tokens(vec![
            Token::Delim('.'),
            Token::Number {
                value: 5.0,
                is_integer: true,
            },
        ]),
        tokens(vec![Token::Delim('/'), Token::Delim('*')]),
        tokens(vec![
            Token::Number {
                value: 1.0,
                is_integer: true,
            },
            Token::Delim('%'),
        ]),
        vec![
            ComponentValue::Token(Token::Ident("f".into())),
            ComponentValue::Block {
                open: '(',
                content: vec![],
            },
        ],
        tokens(vec![Token::Delim('@'), Token::Ident("m".into())]),
        tokens(vec![
            Token::Dimension {
                value: 1.0,
                unit: "e3".into(),
            },
            Token::Ident("x".into()),
        ]),
    ];
    for values in cases {
        let css = to_css(&values);
        assert_eq!(
            parse_component_values(&css),
            values,
            "serialized as {css:?}"
        );
    }
}

#[test]
fn serialization_examples() {
    assert_eq!(to_css(&parse_component_values("a   b\n\tc")), "a b c");
    assert_eq!(to_css(&parse_component_values("f( 1 ,2 )")), "f( 1 ,2 )");
    assert_eq!(to_css(&parse_component_values("'x'")), "\"x\"");
    assert_eq!(to_css(&parse_component_values("1.0")), "1.0");
    assert_eq!(to_css(&parse_component_values("2e1")), "20.0");
    assert_eq!(parse_component_values("url(x)")[0].to_css(), "url(x)");
    assert_eq!(
        ComponentValue::Token(Token::Ident("1a".into())).to_css(),
        "\\31 a"
    );
}

#[test]
fn rewrite_urls_everywhere() {
    let mut sheet = parse_stylesheet(
        "@import url(i.css); @import \"j.css\"; \
         a { background: url(a.png), url(\"b.png\") no-repeat; --x: url(c.png); \
         content: image-set(\"d.png\" 1x, url(e.png) 2x); mask: src('f.png'); empty: url() } \
         @media screen { b { background: linear-gradient(red, blue), url( g.png ) } } \
         .p { .q { background: url(h.png) } }",
    );
    let mut seen = Vec::new();
    sheet.rewrite_urls(&mut |u: &str| {
        seen.push(u.to_string());
        format!("http://x/{u}")
    });
    seen.sort();
    assert_eq!(
        seen,
        vec![
            "a.png", "b.png", "c.png", "d.png", "e.png", "f.png", "g.png", "h.png", "i.css",
            "j.css"
        ]
    );
    let css = format!("{sheet:?}");
    assert!(css.contains("http://x/a.png"));
    assert!(css.contains("http://x/i.css"));
    let rules = style_rules(&sheet.rules);
    assert_eq!(
        crate::values::url(&rules[0].declarations[0].value[0]).as_deref(),
        Some("http://x/a.png")
    );
}

#[test]
fn performance_large_stylesheet() {
    // ~500 KB of typical CSS.
    let chunk = "\
.nav > li a:hover, #main .content p.intro::before { color: #333; margin: 0 auto 1.5em; \
background: url(img/bg.png) no-repeat 50% 50% / cover, linear-gradient(to right, rgba(0,0,0,.5), transparent); }\n\
@media (max-width: 600px) { .nav li:nth-child(2n+1) { display: none !important; } }\n\
/* comment */ .btn[data-kind=\"primary\"]:not(.disabled) { padding: 4px 8px; border: 1px solid var(--accent, blue); }\n";
    let css = chunk.repeat(500_000 / chunk.len() + 1);
    let start = std::time::Instant::now();
    let sheet = parse_stylesheet(&css);
    let elapsed = start.elapsed();
    assert!(sheet.rules.len() > 3000);
    // Generous bound so the test is stable in debug builds; the release
    // target (< 300 ms) is checked with `cargo test --release`.
    let limit = if cfg!(debug_assertions) { 3000 } else { 300 };
    assert!(
        elapsed.as_millis() < limit,
        "parsing {} bytes took {elapsed:?}",
        css.len()
    );
}

#[test]
fn never_panics_on_garbage() {
    let inputs = [
        "}",
        "{",
        "@",
        "@media",
        "@media {",
        "@import",
        "@supports",
        "@supports (",
        "a{",
        "a{b",
        "a{b:",
        "a{b:c",
        "a{--x",
        "a{--x:",
        "a{!important}",
        "a{b:!important}",
        "::",
        ":",
        "&",
        "& {",
        "a { & { & { & { x: y } } } }",
        "a{@media{b{c:d}}}",
        "a { > { } }",
        ">{}",
        "[{]}",
        "(((",
        ")))",
        "\\",
        "url(",
        "a{b:url(}",
        "@media screen and (",
        "a,{}",
        ",{}",
        "a{;}",
        ";;;;",
        "<!--",
        "-->",
        "a{b:c}}}}{{{{",
        "@layer a.b, c;",
        "@scope{",
    ];
    for input in inputs {
        let _ = parse_stylesheet(input);
        let _ = parse_declaration_list(input);
        let _ = to_css(&parse_component_values(input));
    }
}

#[test]
fn deep_nesting_does_not_overflow() {
    let deep_rules = format!("{}x: y{}", "a{".repeat(10_000), "}".repeat(10_000));
    let sheet = parse_stylesheet(&deep_rules);
    assert!(!sheet.rules.is_empty());
    let deep_media = format!(
        "{}a{{x:y}}{}",
        "@media all{".repeat(5_000),
        "}".repeat(5_000)
    );
    let _ = parse_stylesheet(&deep_media);
    let deep_parens = format!("a{{x:{}1{}}}", "(".repeat(50_000), ")".repeat(50_000));
    let _ = parse_stylesheet(&deep_parens);
    let deep_is = format!("{}a{} {{x:y}}", ":is(".repeat(5_000), ")".repeat(5_000));
    let _ = parse_stylesheet(&deep_is);
    let multi = format!("{}x:y{}", "a, b, c, d {".repeat(200), "}".repeat(200));
    let _ = parse_stylesheet(&multi);
}

#[test]
fn nesting_expansion_is_bounded() {
    // Each level doubles the selector list; the expansion is capped.
    let css = format!("{}x:y{}", "x:y; & a, & b {".repeat(30), "}".repeat(30));
    let css = format!(".root {{ {css} }}");
    let start = std::time::Instant::now();
    let sheet = parse_stylesheet(&css);
    assert!(start.elapsed().as_secs() < 5);
    assert!(!sheet.rules.is_empty());
    // Many nested rules under a large parent list.
    let parent: Vec<String> = (0..500).map(|i| format!(".p{i}")).collect();
    let css = format!(
        "{} {{ {} }}",
        parent.join(","),
        "& x { y: z } ".repeat(5_000)
    );
    let start = std::time::Instant::now();
    let sheet = parse_stylesheet(&css);
    assert!(start.elapsed().as_secs() < 10);
    assert!(!sheet.rules.is_empty());
    // Many nested conditional rules each copying a large parent list; the
    // budget stops the copies, but the parent's own declarations and later
    // top-level rules are kept.
    let css = format!(
        "{} {{ a: b; {} c: d }} .after {{ e: f }}",
        parent.join(","),
        "@media all { x: y } ".repeat(5_000)
    );
    let sheet = parse_stylesheet(&css);
    let rules = style_rules(&sheet.rules);
    assert!(rules.len() < 1_000, "{} rules", rules.len());
    let first = rules.first().expect("parent rule");
    assert_eq!(decls(first), vec![d("a", "b")]);
    let last = rules.last().expect("last rule");
    assert_eq!(last.selectors, sel(".after"));
    let parent_tail = rules[rules.len() - 2];
    assert_eq!(decls(parent_tail), vec![d("c", "d")]);
}

#[test]
fn tokens_round_trip_through_tokenizer() {
    // to_css of a token list built by the tokenizer itself.
    let css = "a:hover>b~c+d e[f|=\"g\"]{h:i}";
    let values: Vec<ComponentValue> = tokenize(css)
        .into_iter()
        .map(ComponentValue::Token)
        .collect();
    let serialized = to_css(&values);
    assert_eq!(tokenize(&serialized), tokenize(css));
}

#[test]
fn real_world_tailwind_like_sheet() {
    let css = r#"
@layer theme, base, components, utilities;
@layer theme {
  :root, :host {
    --font-sans: ui-sans-serif, system-ui, sans-serif, "Apple Color Emoji";
    --color-red-500: oklch(63.7% 0.237 25.331);
    --tw-empty: ;
  }
}
@layer base {
  *, ::after, ::before, ::backdrop, ::file-selector-button { box-sizing: border-box; margin: 0; }
  html, :host { line-height: 1.5; -webkit-text-size-adjust: 100%; }
  abbr:where([title]) { text-decoration: underline dotted; }
  ::-webkit-date-and-time-value { min-height: 1lh; }
  @supports (not (-webkit-appearance: -apple-pay-button)) or (contain-intrinsic-size: 1px) {
    ::placeholder { color: color-mix(in oklab, currentcolor 50%, transparent); }
  }
}
@layer utilities {
  .hover\:bg-red-500 { &:hover { @media (hover: hover) { background-color: var(--color-red-500); } } }
  .md\:flex { @media (width >= 48rem) { display: flex; } }
  .w-1\/2 { width: calc(1 / 2 * 100%); }
}
@property --tw-rotate-x { syntax: "*"; inherits: false; }
@keyframes spin { to { transform: rotate(360deg); } }
"#;
    let sheet = parse_stylesheet(css);
    // Three @layer blocks; the statement, @property and @keyframes are dropped.
    assert_eq!(sheet.rules.len(), 3);
    let rules = style_rules(&sheet.rules);
    let root = rules[0];
    assert_eq!(root.selectors.len(), 2);
    assert_eq!(root.declarations.len(), 3);
    assert_eq!(root.declarations[0].name, "--font-sans");
    assert!(root.declarations[2].value.is_empty());
    let reset = rules[1];
    assert_eq!(reset.selectors.len(), 5);
    assert_eq!(
        reset.selectors[0].specificity(),
        crate::selector::Specificity(0, 0, 0)
    );
    assert_eq!(
        reset.selectors[1].pseudo_element,
        Some(PseudoElement::After)
    );
    // The @supports condition is true thanks to `or`.
    let CssRule::Group { rules: base } = &sheet.rules[1] else {
        panic!("expected a group");
    };
    assert!(base.iter().any(|r| matches!(
        r,
        CssRule::Supports {
            supported: true,
            ..
        }
    )));
    // Nested hover utility: `.hover\:bg-red-500:hover` inside @media (hover: hover).
    let CssRule::Group { rules: utilities } = &sheet.rules[2] else {
        panic!("expected a group");
    };
    match &utilities[0] {
        CssRule::Media { query, rules } => {
            assert!(query.matches(&MediaContext::default()));
            let inner = style_rules(rules);
            assert_eq!(inner[0].selectors, sel(".hover\\:bg-red-500:hover"));
            assert_eq!(inner[0].declarations[0].name, "background-color");
        }
        other => panic!("unexpected {other:?}"),
    }
    match &utilities[1] {
        CssRule::Media { query, rules } => {
            assert!(query.matches(&MediaContext::default()));
            assert_eq!(
                style_rules(rules)[0].selectors[0].subject.classes,
                vec!["md:flex"]
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    match &utilities[2] {
        CssRule::Style(rule) => assert_eq!(rule.selectors[0].subject.classes, vec!["w-1/2"]),
        other => panic!("unexpected {other:?}"),
    }
    // Colors used by the sheet parse.
    assert!(crate::Color::parse(&root.declarations[1].value).is_some());
}
