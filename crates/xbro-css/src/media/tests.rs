use super::*;

fn eval(query: &str, ctx: &MediaContext) -> bool {
    MediaQueryList::parse_str(query).matches(ctx)
}

#[test]
fn evaluation_table() {
    // Default context: 1024x768, 1dppx, light.
    let ctx = MediaContext::default();
    let table = [
        ("", true),
        ("   ", true),
        ("all", true),
        ("screen", true),
        ("SCREEN", true),
        ("print", false),
        ("speech", false),
        ("tv", false),
        ("bogus", false),
        ("not print", true),
        ("not screen", false),
        ("only screen", true),
        ("only print", false),
        ("only", false),
        ("not", false),
        ("and", false),
        ("screen, print", true),
        ("print, screen", true),
        ("print, bogus", false),
        ("print, , screen", true),
        ("screen and (min-width: 1024px)", true),
        ("screen and (min-width: 1025px)", false),
        ("screen and (max-width: 1024px)", true),
        ("screen and (max-width: 1023.5px)", false),
        ("(width: 1024px)", true),
        ("(width: 1000px)", false),
        ("(min-width: 64em)", true),
        ("(min-width: 64.1em)", false),
        ("(min-width: 64rem)", true),
        ("(max-width: 768pt)", true),
        ("(max-width: 767pt)", false),
        ("(min-width: 10in)", true),
        ("(min-width: 0)", true),
        ("(min-width: 100)", false),
        ("(min-height: 768px)", true),
        ("(max-height: 767px)", false),
        ("(height: 768px)", true),
        ("(min-width: 50vw)", true),
        ("(max-width: 100vh)", false),
        (
            "only screen and (min-width: 600px) and (max-width: 1200px)",
            true,
        ),
        (
            "only screen and (min-width: 600px) and (max-width: 1000px)",
            false,
        ),
        (
            "screen and (min-width: 600px), print and (max-width: 100px)",
            true,
        ),
        ("not screen and (max-width: 600px)", true),
        ("not screen and (min-width: 600px)", false),
        ("not all and (monochrome)", true),
        ("print and (min-width: 0)", false),
        // Range syntax.
        ("(width >= 600px)", true),
        ("(width > 1024px)", false),
        ("(width >= 1024px)", true),
        ("(width < 1024px)", false),
        ("(width <= 1024px)", true),
        ("(width = 1024px)", true),
        ("(600px <= width)", true),
        ("(1024px < width)", false),
        ("(400px < width < 1100px)", true),
        ("(400px < width < 800px)", false),
        ("(400px <= width <= 1024px)", true),
        ("(1100px > width > 400px)", true),
        ("(1100px > width >= 1024px)", true),
        ("(400px < width > 300px)", false),
        ("(width > = 600px)", false),
        ("(min-width >= 600px)", false),
        ("(500px < height < 800px)", true),
        // Conditions.
        ("(min-width: 100px) and (max-width: 2000px)", true),
        ("(min-width: 100px) and (max-width: 200px)", false),
        ("(max-width: 100px) or (min-width: 200px)", true),
        ("(max-width: 100px) or (min-width: 2000px)", false),
        ("not (max-width: 100px)", true),
        ("not (min-width: 100px)", false),
        (
            "((min-width: 100px) and (max-width: 2000px)) or (color)",
            true,
        ),
        (
            "(not (max-width: 100px)) and (orientation: landscape)",
            true,
        ),
        (
            "(min-width: 100px) and (max-width: 2000px) or (color)",
            false,
        ),
        ("screen and (max-width: 100px) or (color)", false),
        ("screen and (not (max-width: 100px))", true),
        // Unknown features / general enclosed.
        ("(unknown-feature)", false),
        ("not (unknown-feature)", false),
        ("(unknown: 1)", false),
        ("screen and (unknown)", false),
        ("not screen and (unknown)", false),
        ("(unknown) or (min-width: 0)", true),
        ("(unknown) and (min-width: 0)", false),
        ("foo(bar)", false),
        ("(min-width: 100px), (unknown)", true),
        ("(min-width: bogus)", false),
        ("(min-orientation: portrait)", false),
        // Discrete features.
        ("(orientation: landscape)", true),
        ("(orientation: portrait)", false),
        ("(orientation: sideways)", false),
        ("(orientation)", true),
        ("(prefers-color-scheme: light)", true),
        ("(prefers-color-scheme: dark)", false),
        ("(prefers-reduced-motion: no-preference)", true),
        ("(prefers-reduced-motion: reduce)", false),
        ("(prefers-reduced-motion)", false),
        ("(prefers-contrast: more)", false),
        ("(forced-colors: active)", false),
        ("(forced-colors)", false),
        ("(hover: hover)", true),
        ("(hover: none)", false),
        ("(hover)", true),
        ("(any-hover: hover)", true),
        ("(pointer: fine)", true),
        ("(pointer: coarse)", false),
        ("(any-pointer: coarse)", false),
        ("(color-gamut: srgb)", true),
        ("(color-gamut: p3)", false),
        ("(scripting: none)", true),
        ("(scripting: enabled)", false),
        ("(update: fast)", true),
        ("(display-mode: browser)", true),
        ("(dynamic-range: high)", false),
        ("(-webkit-transform-3d)", false),
        // Numeric features.
        ("(color)", true),
        ("(min-color: 8)", true),
        ("(min-color: 9)", false),
        ("(color-index)", false),
        ("(monochrome)", false),
        ("(monochrome: 0)", true),
        ("(grid)", false),
        ("(grid: 0)", true),
        ("(aspect-ratio: 4/3)", true),
        ("(aspect-ratio: 4 / 3)", true),
        ("(aspect-ratio: 16/9)", false),
        ("(min-aspect-ratio: 1/1)", true),
        ("(min-aspect-ratio: 16/9)", false),
        ("(max-aspect-ratio: 2)", true),
        ("(aspect-ratio > 1)", true),
        ("(aspect-ratio: 0/0)", false),
        ("(resolution: 1dppx)", true),
        ("(resolution: 1x)", true),
        ("(resolution: 96dpi)", true),
        ("(min-resolution: 2dppx)", false),
        ("(min-resolution: 1.5x)", false),
        ("(max-resolution: 1dppx)", true),
        ("(min-resolution: 192dpi)", false),
        ("(resolution < infinite)", true),
        ("(-webkit-min-device-pixel-ratio: 2)", false),
        ("(-webkit-min-device-pixel-ratio: 1)", true),
        ("(-webkit-max-device-pixel-ratio: 1)", true),
        ("(-webkit-device-pixel-ratio: 1)", true),
        ("(min--moz-device-pixel-ratio: 1.5)", false),
        ("(min-device-width: 1000px)", true),
        ("(max-device-width: 480px)", false),
    ];
    for (query, expected) in table {
        assert_eq!(eval(query, &ctx), expected, "{query:?}");
    }
}

#[test]
fn context_changes() {
    let phone = MediaContext {
        width: 375.0,
        height: 812.0,
        resolution: 3.0,
        dark: true,
    };
    assert!(eval("(max-width: 600px)", &phone));
    assert!(eval("(orientation: portrait)", &phone));
    assert!(eval("(prefers-color-scheme: dark)", &phone));
    assert!(!eval("(prefers-color-scheme: light)", &phone));
    assert!(eval("(min-resolution: 2dppx)", &phone));
    assert!(eval("(-webkit-min-device-pixel-ratio: 2)", &phone));
    assert!(eval("(min-resolution: 288dpi)", &phone));
    assert!(!eval("(min-width: 37.5em)", &phone));
    let square = MediaContext {
        width: 500.0,
        height: 500.0,
        ..MediaContext::default()
    };
    // A square viewport is portrait.
    assert!(eval("(orientation: portrait)", &square));
    assert!(eval("(aspect-ratio: 1)", &square));
    let zero = MediaContext {
        width: 0.0,
        height: 0.0,
        ..MediaContext::default()
    };
    assert!(!eval("(width)", &zero));
    assert!(eval("(max-width: 0)", &zero));
}

#[test]
fn query_list_construction() {
    let list = MediaQueryList::new(Vec::new());
    assert!(list.matches(&MediaContext::default()));
    assert_eq!(MediaQueryList::default(), list);
    assert!(!MediaQueryList::parse_str("print").matches(&MediaContext::default()));
}

#[test]
fn garbage_does_not_panic() {
    let ctx = MediaContext::default();
    for query in [
        "(",
        ")",
        "((",
        "(()",
        "(:)",
        "(width:)",
        "(: 1px)",
        "(<)",
        "(< width)",
        "(width <)",
        "(1px < width <)",
        "(a < b < c < d)",
        "not not screen",
        "screen and",
        "screen and and",
        "(width: 1px 2px)",
        "(aspect-ratio: 1/)",
        "(aspect-ratio: /2)",
        "{}",
        "[]",
        "@media",
        "(width: calc(1px + 2px))",
        "screen,",
        ",",
    ] {
        let _ = eval(query, &ctx);
    }
    let deep = format!("{}(color){}", "(".repeat(10_000), ")".repeat(10_000));
    let _ = eval(&deep, &ctx);
    let deep_not = format!("{}(color){}", "(not ".repeat(100), ")".repeat(100));
    let _ = eval(&deep_not, &ctx);
}
