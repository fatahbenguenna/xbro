use super::*;

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Option<Color> {
    Some(Color::rgba(r, g, b, a))
}

fn rgb(r: u8, g: u8, b: u8) -> Option<Color> {
    Some(Color::rgb(r, g, b))
}

/// Asserts each channel is within `tolerance` of the expected value.
fn assert_close(input: &str, expected: (u8, u8, u8, u8), tolerance: u8) {
    let c = Color::parse_str(input).unwrap_or_else(|| panic!("{input:?} should parse"));
    let close = |a: u8, b: u8| a.abs_diff(b) <= tolerance;
    assert!(
        close(c.r, expected.0)
            && close(c.g, expected.1)
            && close(c.b, expected.2)
            && close(c.a, expected.3),
        "{input:?} gave {c:?}, expected {expected:?}"
    );
}

#[test]
fn named_colors() {
    assert_eq!(named::named_count(), 148);
    assert!(named::tables_sorted());
    assert_eq!(Color::named("red"), rgb(255, 0, 0));
    assert_eq!(Color::named("RED"), rgb(255, 0, 0));
    assert_eq!(Color::named("RebeccaPurple"), rgb(0x66, 0x33, 0x99));
    assert_eq!(Color::named("lightgoldenrodyellow"), rgb(0xfa, 0xfa, 0xd2));
    assert_eq!(Color::named("grey"), Color::named("gray"));
    assert_eq!(Color::named("transparent"), Some(Color::TRANSPARENT));
    assert_eq!(Color::named("currentcolor"), None);
    assert_eq!(Color::named("canvas"), None);
    assert_eq!(Color::named(""), None);
    assert_eq!(Color::named(&"a".repeat(10_000)), None);
}

#[test]
fn system_colors() {
    assert_eq!(Color::parse_str("Canvas"), rgb(255, 255, 255));
    assert_eq!(Color::parse_str("CanvasText"), rgb(0, 0, 0));
    assert_eq!(Color::parse_str("LinkText"), rgb(0, 0, 0xee));
    assert_eq!(Color::parse_str("visitedtext"), rgb(0x55, 0x1a, 0x8b));
    assert_eq!(Color::parse_str("ActiveText"), rgb(255, 0, 0));
    assert_eq!(Color::parse_str("ButtonFace"), rgb(0xef, 0xef, 0xef));
    assert_eq!(Color::parse_str("ButtonText"), rgb(0, 0, 0));
    assert!(Color::parse_str("ButtonBorder").is_some());
    assert_eq!(Color::parse_str("Field"), rgb(255, 255, 255));
    assert_eq!(Color::parse_str("FieldText"), rgb(0, 0, 0));
    assert!(Color::parse_str("Highlight").is_some());
    assert!(Color::parse_str("HighlightText").is_some());
    assert!(Color::parse_str("GrayText").is_some());
    assert_eq!(Color::parse_str("Mark"), rgb(255, 255, 0));
    assert_eq!(Color::parse_str("MarkText"), rgb(0, 0, 0));
    assert!(Color::parse_str("AccentColor").is_some());
    assert!(Color::parse_str("AccentColorText").is_some());
    assert_eq!(Color::parse_str("WindowText"), rgb(0, 0, 0));
    assert_eq!(Color::parse_str("-webkit-link"), rgb(0, 0, 0xee));
}

#[test]
fn hex_colors() {
    let table = [
        ("#f00", rgb(255, 0, 0)),
        ("#F00", rgb(255, 0, 0)),
        ("#0f08", rgba(0, 255, 0, 0x88)),
        ("#ff0000", rgb(255, 0, 0)),
        ("#12345678", rgba(0x12, 0x34, 0x56, 0x78)),
        ("#123", rgb(0x11, 0x22, 0x33)),
        ("#abcdef", rgb(0xab, 0xcd, 0xef)),
        ("#fff", rgb(255, 255, 255)),
        ("#ff", None),
        ("#fffff", None),
        ("#fffffff", None),
        ("#ggg", None),
        ("#", None),
        ("#fff0000000", None),
    ];
    for (input, expected) in table {
        assert_eq!(Color::parse_str(input), expected, "{input:?}");
    }
    assert_eq!(Color::from_hex("abc"), rgb(0xaa, 0xbb, 0xcc));
    assert_eq!(Color::from_hex("é12"), None);
}

#[test]
fn rgb_functions() {
    let table = [
        ("rgb(255, 0, 0)", rgb(255, 0, 0)),
        ("rgb(255,0,0)", rgb(255, 0, 0)),
        ("RGB(255, 0, 0)", rgb(255, 0, 0)),
        ("rgba(255, 0, 0, 0.5)", rgba(255, 0, 0, 128)),
        ("rgba(255, 0, 0, 50%)", rgba(255, 0, 0, 128)),
        ("rgb(255, 0, 0, 0.5)", rgba(255, 0, 0, 128)),
        ("rgba(255, 0, 0)", rgb(255, 0, 0)),
        ("rgb(100%, 0%, 50%)", rgb(255, 0, 128)),
        ("rgb(255 0 0)", rgb(255, 0, 0)),
        ("rgb(255 0 0 / 50%)", rgba(255, 0, 0, 128)),
        ("rgb(255 0 0/0.25)", rgba(255, 0, 0, 64)),
        ("rgb(100% 50% 0)", rgb(255, 128, 0)),
        ("rgb(none 255 none)", rgb(0, 255, 0)),
        ("rgb(255 0 0 / none)", rgba(255, 0, 0, 0)),
        ("rgb(300 -10 127.5)", rgb(255, 0, 128)),
        ("rgb(127.4 0 0)", rgb(127, 0, 0)),
        ("rgb(255 0 0 / 2)", rgb(255, 0, 0)),
        ("rgb(255 0 0 / -1)", rgba(255, 0, 0, 0)),
        ("rgb(150%, -5%, 0%)", rgb(255, 0, 0)),
        ("rgb( 1 2 3 )", rgb(1, 2, 3)),
        // Invalid.
        ("rgb(255, 0%, 0)", None),
        ("rgb(none, 0, 0)", None),
        ("rgb(255, 0, 0, none)", None),
        ("rgb(255, 0)", None),
        ("rgb(255 0)", None),
        ("rgb(255 0 0 0)", None),
        ("rgb(255, 0, 0, 0, 0)", None),
        ("rgb(255 0 0 /)", None),
        ("rgb(255 0 0 / 1 2)", None),
        ("rgb(255deg 0 0)", None),
        ("rgb(red)", None),
        ("rgb()", None),
        ("rgb(255, 0 0)", None),
    ];
    for (input, expected) in table {
        assert_eq!(Color::parse_str(input), expected, "{input:?}");
    }
}

#[test]
fn hsl_and_hwb() {
    let table = [
        ("hsl(120 100% 25%)", rgb(0, 128, 0)),
        ("hsl(120, 100%, 25%)", rgb(0, 128, 0)),
        ("hsl(0, 100%, 50%)", rgb(255, 0, 0)),
        ("hsl(0 100 50)", rgb(255, 0, 0)),
        ("hsl(360 100% 50%)", rgb(255, 0, 0)),
        ("hsl(-120 100% 50%)", rgb(0, 0, 255)),
        ("hsl(240deg 100% 50%)", rgb(0, 0, 255)),
        ("hsl(0.5turn 100% 50%)", rgb(0, 255, 255)),
        ("hsl(200grad 100% 50%)", rgb(0, 255, 255)),
        ("hsl(3.14159rad 100% 50%)", rgb(0, 255, 255)),
        ("hsla(120, 100%, 50%, 0.5)", rgba(0, 255, 0, 128)),
        ("hsl(120 100% 50% / 25%)", rgba(0, 255, 0, 64)),
        ("hsl(0 0% 50%)", rgb(128, 128, 128)),
        ("hsl(none 0% 100%)", rgb(255, 255, 255)),
        ("hsl(30 200% 50%)", rgb(255, 128, 0)),
        ("hsl(0, 100, 50)", None),
        ("hsl(0, 100%)", None),
        ("hsl(10% 100% 50%)", None),
        ("hwb(0 0% 0%)", rgb(255, 0, 0)),
        ("hwb(120 0% 50%)", rgb(0, 128, 0)),
        ("hwb(0 100% 0%)", rgb(255, 255, 255)),
        ("hwb(0 0% 100%)", rgb(0, 0, 0)),
        ("hwb(0 60% 60%)", rgb(128, 128, 128)),
        ("hwb(240 20% 20% / 0.5)", rgba(51, 51, 204, 128)),
        ("hwb(0, 0%, 0%)", None),
    ];
    for (input, expected) in table {
        assert_eq!(Color::parse_str(input), expected, "{input:?}");
    }
}

#[test]
fn lab_lch_oklab_oklch() {
    assert_close("lab(50 0 0)", (119, 119, 119, 255), 1);
    assert_close("lab(100 0 0)", (255, 255, 255, 255), 1);
    assert_close("lab(0 0 0)", (0, 0, 0, 255), 0);
    assert_close("lab(54.29 80.8 69.89)", (255, 0, 0, 255), 2);
    assert_close("lab(50% 0 0)", (119, 119, 119, 255), 1);
    assert_close("lab(54.29 80.8 69.89 / 0.5)", (255, 0, 0, 128), 2);
    assert_close("lch(54.29 106.8 40.86)", (255, 0, 0, 255), 2);
    assert_close("lch(50 0 0)", (119, 119, 119, 255), 1);
    assert_close("oklab(0.628 0.2249 0.1258)", (255, 0, 0, 255), 2);
    assert_close("oklab(62.8% 0.2249 0.1258)", (255, 0, 0, 255), 2);
    assert_close("oklab(1 0 0)", (255, 255, 255, 255), 1);
    assert_close("oklch(0.628 0.2577 29.23)", (255, 0, 0, 255), 2);
    assert_close("oklch(0.866 0.2948 142.5)", (0, 255, 0, 255), 2);
    assert_close("oklch(0.452 0.313 264.05)", (0, 0, 255, 255), 2);
    assert_close("oklch(0.5 0 0 / 50%)", (99, 99, 99, 128), 2);
    // Out of gamut values are clamped.
    assert_close("oklch(0.7 0.4 150)", (0, 194, 0, 255), 255);
    assert!(Color::parse_str("lab(50, 0, 0)").is_none());
    assert!(Color::parse_str("lab(50 0)").is_none());
}

#[test]
fn color_function() {
    assert_eq!(Color::parse_str("color(srgb 1 0 0)"), rgb(255, 0, 0));
    assert_eq!(
        Color::parse_str("color(srgb 100% 50% 0%)"),
        rgb(255, 128, 0)
    );
    assert_eq!(
        Color::parse_str("color(srgb 1 0 0 / 0.5)"),
        rgba(255, 0, 0, 128)
    );
    assert_eq!(Color::parse_str("color(srgb 2 -1 0)"), rgb(255, 0, 0));
    assert_eq!(Color::parse_str("color(srgb none 1 none)"), rgb(0, 255, 0));
    assert_close(
        "color(srgb-linear 0.2140 0.2140 0.2140)",
        (128, 128, 128, 255),
        1,
    );
    assert_close("color(display-p3 1 0 0)", (255, 0, 0, 255), 0);
    assert_close("color(display-p3 0.5 0.5 0.5)", (128, 128, 128, 255), 1);
    // Grays keep their linear light: 0.5 encoded with each space's transfer
    // function, re-encoded with sRGB's.
    assert_close("color(rec2020 0.5 0.5 0.5)", (139, 139, 139, 255), 1);
    assert_close("color(a98-rgb 0.5 0.5 0.5)", (129, 129, 129, 255), 1);
    assert_close("color(prophoto-rgb 0.5 0.5 0.5)", (146, 146, 146, 255), 1);
    assert_close("color(rec2020 1 1 1)", (255, 255, 255, 255), 1);
    assert_close("color(prophoto-rgb 1 1 1)", (255, 255, 255, 255), 1);
    assert_close("color(xyz 0.9505 1 1.089)", (255, 255, 255, 255), 1);
    assert_close("color(xyz-d65 0.9505 1 1.089)", (255, 255, 255, 255), 1);
    assert_close("color(xyz-d50 0.9642 1 0.8251)", (255, 255, 255, 255), 1);
    assert!(Color::parse_str("color(unknown 1 0 0)").is_none());
    assert!(Color::parse_str("color(srgb 1 0)").is_none());
    assert!(Color::parse_str("color(srgb, 1, 0, 0)").is_none());
    assert!(Color::parse_str("color()").is_none());
}

#[test]
fn color_mix() {
    let table = [
        ("color-mix(in srgb, red, blue)", rgb(128, 0, 128)),
        ("color-mix(in srgb, red 25%, blue)", rgb(64, 0, 191)),
        ("color-mix(in srgb, red, blue 75%)", rgb(64, 0, 191)),
        ("color-mix(in srgb, 25% red, blue)", rgb(64, 0, 191)),
        ("color-mix(in srgb, red 50%, blue 50%)", rgb(128, 0, 128)),
        (
            "color-mix(in srgb, red 20%, blue 20%)",
            rgba(128, 0, 128, 102),
        ),
        ("color-mix(in srgb, red 100%, blue 100%)", rgb(128, 0, 128)),
        ("color-mix(in oklab, white, black)", rgb(128, 128, 128)),
        ("color-mix(in srgb, red, transparent)", rgba(255, 0, 0, 128)),
        ("color-mix(in hsl longer hue, red, blue)", rgb(128, 0, 128)),
        // The inner mix is rounded to 8 bits (128) before the outer one.
        (
            "color-mix(in srgb, color-mix(in srgb, red, blue), white)",
            rgb(192, 128, 192),
        ),
        ("color-mix(in srgb, red 0%, blue 0%)", None),
        ("color-mix(in srgb, red 150%, blue)", None),
        ("color-mix(in bogus, red, blue)", None),
        ("color-mix(srgb, red, blue)", None),
        ("color-mix(in srgb, red)", None),
        ("color-mix(in srgb, red, blue, green)", None),
        ("color-mix(in srgb, currentcolor, blue)", None),
        ("color-mix(in srgb bogus hue, red, blue)", None),
    ];
    for (input, expected) in table {
        assert_eq!(Color::parse_str(input), expected, "{input:?}");
    }
}

#[test]
fn light_dark() {
    assert_eq!(
        Color::parse_str("light-dark(white, black)"),
        rgb(255, 255, 255)
    );
    assert_eq!(
        Color::parse_str("light-dark(#123, rgb(0 0 0))"),
        rgb(0x11, 0x22, 0x33)
    );
    assert!(Color::parse_str("light-dark(white)").is_none());
    assert!(Color::parse_str("light-dark(white, nope)").is_none());
}

#[test]
fn parse_from_component_values() {
    let values = crate::parser::parse_component_values("  red  ");
    assert_eq!(Color::parse(&values), rgb(255, 0, 0));
    assert_eq!(Color::parse(&[]), None);
    assert_eq!(Color::parse_str("currentcolor"), None);
    assert_eq!(Color::parse_str("currentColor"), None);
    assert_eq!(Color::parse_str("red blue"), None);
    assert_eq!(Color::parse_str("nope"), None);
    assert_eq!(Color::parse_str("1px"), None);
    assert_eq!(Color::parse_str("calc(1)"), None);
    assert_eq!(Color::parse_str("transparent"), Some(Color::TRANSPARENT));
}

#[test]
fn legacy_colors() {
    let table = [
        ("red", rgb(255, 0, 0)),
        (" Red ", rgb(255, 0, 0)),
        ("#fff", rgb(255, 255, 255)),
        ("#FfF", rgb(255, 255, 255)),
        ("fff", rgb(0x0f, 0x0f, 0x0f)),
        ("ff0000", rgb(255, 0, 0)),
        ("#ff0000", rgb(255, 0, 0)),
        ("#f00", rgb(255, 0, 0)),
        ("chucknorris", rgb(0xc0, 0, 0)),
        ("ninjaturtle", rgb(0, 0xa0, 0)),
        ("crap", rgb(0xc0, 0xa0, 0)),
        ("grass", rgb(0, 0xa0, 0)),
        ("sick", rgb(0, 0xc0, 0)),
        ("mrt", rgb(0, 0, 0)),
        ("#1234567890", rgb(0x12, 0x56, 0x90)),
        ("#abcdefghijkl", rgb(0xab, 0xef, 0)),
        ("0", rgb(0, 0, 0)),
        ("#ff00", rgb(0xff, 0, 0)),
        ("#123456789", rgb(0x12, 0x45, 0x78)),
        (
            "#0000000000000000000000000000000000000000000000ff",
            rgb(0, 0, 0xff),
        ),
        ("rgb(1, 2, 3)", rgb(0, 0x10, 0)),
        ("transparent", None),
        ("", None),
        // Only the empty string fails: whitespace is stripped first, then the
        // empty value is padded to "000" (HTML §2.3.6 steps 1, 2 and 10).
        ("   ", rgb(0, 0, 0)),
        ("\t\n", rgb(0, 0, 0)),
        (" transparent ", None),
        ("\u{1F600}", rgb(0, 0, 0)),
    ];
    for (input, expected) in table {
        assert_eq!(Color::parse_legacy(input), expected, "{input:?}");
    }
    // Very long input is truncated to 128 characters.
    let long = format!("#{}", "f".repeat(10_000));
    assert_eq!(Color::parse_legacy(&long), rgb(255, 255, 255));
}

#[test]
fn opacity_helpers() {
    let c = Color::rgb(10, 20, 30);
    assert_eq!(c.with_opacity(0.5).a, 128);
    assert_eq!(c.with_opacity(2.0).a, 255);
    assert_eq!(c.with_opacity(-1.0).a, 0);
    assert!(Color::TRANSPARENT.is_transparent());
    assert!(!Color::BLACK.is_transparent());
    assert_eq!(Color::WHITE, Color::rgb(255, 255, 255));
}

#[test]
fn garbage_does_not_panic() {
    for input in [
        "rgb(",
        "rgb(,,,)",
        "hsl(/)",
        "color(",
        "color-mix(",
        "color-mix(in",
        "lab(nan 0 0)",
        "oklch(1e30 1e30 1e30)",
        "lch(-1e30 -1e30 -1e30)",
        "hwb(1e38 1e38 1e38)",
        "color(srgb 1e38 -1e38 1e38)",
        "rgb(1e38 1e38 1e38 / 1e38)",
        "#\\",
        "light-dark(,)",
    ] {
        let _ = Color::parse_str(input);
        let _ = Color::parse_legacy(input);
    }
    let deep = format!(
        "{}red{}",
        "color-mix(in srgb, ".repeat(100),
        ", blue)".repeat(100)
    );
    assert_eq!(Color::parse_str(&deep), None);
}
