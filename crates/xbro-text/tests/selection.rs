//! Font selection: generic families, aliases, weight and style matching.

mod common;

use common::*;
use xbro_text::{FontId, FontQuery, FontSystem};

#[test]
fn generic_families_resolve_to_installed_fonts() {
    let Some(fonts) = dejavu() else { return };
    for generic in [
        "sans-serif",
        "system-ui",
        "ui-sans-serif",
        "-apple-system",
        "blinkmacsystemfont",
        "cursive",
        "fantasy",
    ] {
        let id = select(&fonts, &[generic], 400, false);
        assert_eq!(id, FontId(0), "{generic}");
        assert_eq!(fonts.family_name(id), "DejaVu Sans", "{generic}");
    }
    for generic in ["serif", "ui-serif"] {
        let id = select(&fonts, &[generic], 400, false);
        assert_eq!(fonts.family_name(id), "DejaVu Serif", "{generic}");
    }
    for generic in ["monospace", "ui-monospace"] {
        let id = select(&fonts, &[generic], 400, false);
        assert_eq!(fonts.family_name(id), "DejaVu Sans Mono", "{generic}");
    }
}

#[test]
fn well_known_fonts_map_to_equivalents() {
    let Some(fonts) = dejavu() else { return };
    let cases = [
        ("arial", "DejaVu Sans"),
        ("helvetica", "DejaVu Sans"),
        ("helvetica neue", "DejaVu Sans"),
        ("segoe ui", "DejaVu Sans"),
        ("roboto", "DejaVu Sans"),
        ("verdana", "DejaVu Sans"),
        ("tahoma", "DejaVu Sans"),
        ("trebuchet ms", "DejaVu Sans"),
        ("open sans", "DejaVu Sans"),
        ("times new roman", "DejaVu Serif"),
        ("times", "DejaVu Serif"),
        ("georgia", "DejaVu Serif"),
        ("courier new", "DejaVu Sans Mono"),
        ("courier", "DejaVu Sans Mono"),
        ("consolas", "DejaVu Sans Mono"),
        ("menlo", "DejaVu Sans Mono"),
        ("monaco", "DejaVu Sans Mono"),
        ("sfmono-regular", "DejaVu Sans Mono"),
        ("sf mono", "DejaVu Sans Mono"),
    ];
    for (family, expected) in cases {
        let id = select(&fonts, &[family], 400, false);
        assert_eq!(fonts.family_name(id), expected, "{family}");
    }
}

#[test]
fn metric_compatible_fonts_are_preferred() {
    let Some(fonts) = system(&[SANS, LIBERATION_SANS, LIBERATION_SERIF, CARLITO]) else {
        return;
    };
    let name = |families: &[&str]| fonts.family_name(select(&fonts, families, 400, false));
    assert_eq!(name(&["arial"]), "Liberation Sans");
    assert_eq!(name(&["helvetica"]), "Liberation Sans");
    assert_eq!(name(&["times new roman"]), "Liberation Serif");
    assert_eq!(name(&["calibri"]), "Carlito");
    // The generic keeps its own priority order.
    assert_eq!(name(&["sans-serif"]), "DejaVu Sans");
    assert_eq!(name(&["serif"]), "Liberation Serif");
}

#[test]
fn installed_family_names_match_case_insensitively() {
    let Some(fonts) = dejavu() else { return };
    assert_eq!(select(&fonts, &["dejavu serif"], 400, false), FontId(2));
    assert_eq!(select(&fonts, &["DejaVu Serif"], 400, false), FontId(2));
    assert_eq!(
        select(&fonts, &["  'DejaVu Sans Mono' "], 400, false),
        FontId(3)
    );
    assert_eq!(
        select(&fonts, &["\"dejavu sans mono\""], 400, false),
        FontId(3)
    );
}

#[test]
fn first_available_family_wins() {
    let Some(fonts) = dejavu() else { return };
    assert_eq!(
        select(&fonts, &["no such font", "serif"], 400, false),
        FontId(2)
    );
    assert_eq!(
        select(&fonts, &["monospace", "serif"], 400, false),
        FontId(3)
    );
    assert_eq!(
        select(&fonts, &["dejavu serif", "monospace"], 400, false),
        FontId(2)
    );
    // A comma-separated list in one string is split.
    assert_eq!(select(&fonts, &["nope, monospace"], 400, false), FontId(3));
}

#[test]
fn unknown_families_fall_back_to_sans_serif() {
    let Some(fonts) = dejavu() else { return };
    assert_eq!(select(&fonts, &["no such font"], 400, false), FontId(0));
    assert_eq!(select(&fonts, &[], 400, false), FontId(0));
    assert_eq!(select(&fonts, &["", "  "], 400, false), FontId(0));
    assert_eq!(fonts.default_font(), FontId(0));
}

#[test]
fn weight_matching() {
    let Some(fonts) = dejavu() else { return };
    let weight = |w: u16| select(&fonts, &["sans-serif"], w, false);
    assert_eq!(weight(700), FontId(1), "700 -> bold");
    assert_eq!(weight(800), FontId(1), "800 -> bold");
    assert_eq!(weight(900), FontId(1), "900 -> bold");
    assert_eq!(weight(600), FontId(1), "600 -> heavier first");
    assert_eq!(weight(500), FontId(0), "500 -> 400 before heavier");
    assert_eq!(weight(400), FontId(0));
    assert_eq!(weight(300), FontId(0), "300 -> regular");
    assert_eq!(weight(100), FontId(0));
    // Out-of-range weights are clamped, never panic.
    assert_eq!(weight(0), FontId(0));
    assert_eq!(weight(u16::MAX), FontId(1));
    // Serif has no bold face in this set: the regular one is used.
    assert_eq!(select(&fonts, &["serif"], 700, false), FontId(2));
}

#[test]
fn style_matching() {
    let Some(fonts) = system(&[MONO, MONO_OBLIQUE, SANS]) else {
        return;
    };
    // Italic falls back to oblique, normal prefers the upright face.
    assert_eq!(select(&fonts, &["monospace"], 400, true), FontId(1));
    assert_eq!(select(&fonts, &["monospace"], 400, false), FontId(0));
    // No slanted sans face: italic falls back to normal.
    assert_eq!(select(&fonts, &["sans-serif"], 400, true), FontId(2));
}

#[test]
fn selection_is_cached_and_stable() {
    let Some(fonts) = dejavu() else { return };
    let families = vec!["arial".to_string(), "sans-serif".to_string()];
    let query = FontQuery {
        families: &families,
        weight: 700,
        italic: false,
    };
    let first = fonts.select(&query);
    for _ in 0..100 {
        assert_eq!(fonts.select(&query), first);
    }
    assert_eq!(first, FontId(1));
}

#[test]
fn family_names() {
    let Some(fonts) = dejavu() else { return };
    assert_eq!(fonts.family_name(FontId(0)), "DejaVu Sans");
    assert_eq!(fonts.family_name(FontId(1)), "DejaVu Sans");
    assert_eq!(fonts.family_name(FontId(2)), "DejaVu Serif");
    assert_eq!(fonts.family_name(FontId(3)), "DejaVu Sans Mono");
    assert_eq!(fonts.family_name(FontId(4)), "");
    assert_eq!(fonts.family_name(FontId(u32::MAX)), "");
    assert_eq!(fonts.face_count(), 4);
    assert!(fonts.has_fonts());
}

#[test]
fn pathological_queries_do_not_panic() {
    let Some(fonts) = dejavu() else { return };
    let long: Vec<String> = (0..10_000).map(|i| format!("font{i}")).collect();
    let id = fonts.select(&FontQuery {
        families: &long,
        weight: 400,
        italic: false,
    });
    assert_eq!(id, FontId(0));
    let commas = vec![",".repeat(100_000)];
    let id = fonts.select(&FontQuery {
        families: &commas,
        weight: 400,
        italic: true,
    });
    assert_eq!(id, FontId(0));
    let weird = vec!["\u{0}\u{FFFF}\"'".to_string(), "\u{1F600}".to_string()];
    let _ = fonts.select(&FontQuery {
        families: &weird,
        weight: 1,
        italic: false,
    });
}

#[test]
fn font_system_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<FontSystem>();
}

#[test]
fn bitmap_only_families_are_skipped_when_an_outline_family_follows() {
    // Noto Color Emoji has digits and '#' (for keycaps) but no outlines:
    // selecting it would make that text invisible.
    let Some(fonts) = system(&[COLOR_EMOJI, SANS]) else {
        return;
    };
    let id = select(&fonts, &["noto color emoji", "sans-serif"], 400, false);
    assert_eq!(id, FontId(1), "{}", fonts.family_name(id));
    let run = fonts.shape(id, 16.0, "1#", 0.0, 0.0);
    for glyph in &run.glyphs {
        assert!(
            fonts
                .rasterize(glyph.font, glyph.glyph_id, 16.0, 0.0)
                .is_some(),
            "{glyph:?} must be visible"
        );
    }
    // Alone in the database, the bitmap font is still used (never fails).
    let Some(only) = system(&[COLOR_EMOJI]) else {
        return;
    };
    assert_eq!(select(&only, &["noto color emoji"], 400, false), FontId(0));
    assert_eq!(select(&only, &["serif"], 700, true), FontId(0));
    assert!(only.has_fonts());
}

#[test]
fn quoted_generic_keywords_are_family_names() {
    // CSS: `"monospace"` (quoted) names a font family, not the generic.
    let Some(fonts) = dejavu() else { return };
    assert_eq!(
        select(&fonts, &["\"monospace\"", "serif"], 400, false),
        FontId(2)
    );
    assert_eq!(
        select(&fonts, &["'sans-serif'", "monospace"], 400, false),
        FontId(3)
    );
    assert_eq!(
        select(&fonts, &["monospace", "serif"], 400, false),
        FontId(3)
    );
    // Quoted well-known (non-generic) names keep their substitutions.
    assert_eq!(select(&fonts, &["\"courier new\""], 400, false), FontId(3));
    assert_eq!(
        select(&fonts, &["'times new roman'"], 400, false),
        FontId(2)
    );
}

#[test]
fn commas_inside_quotes_do_not_split_family_lists() {
    let Some(fonts) = dejavu() else { return };
    // The quoted name `"x, dejavu serif, y"` is a single (unknown) family:
    // a naive split would find "dejavu serif" inside it.
    assert_eq!(
        select(&fonts, &["\"x, dejavu serif, y\", monospace"], 400, false),
        FontId(3)
    );
    assert_eq!(
        select(
            &fonts,
            &["'a, serif', \"b, dejavu serif\"", "monospace"],
            400,
            false
        ),
        FontId(3)
    );
    // Unquoted commas still split.
    assert_eq!(select(&fonts, &["nope, serif"], 400, false), FontId(2));
    assert_eq!(
        select(&fonts, &["'nope', \"dejavu serif\""], 400, false),
        FontId(2)
    );
}

#[test]
fn huge_family_lists_give_stable_results() {
    let Some(fonts) = dejavu() else { return };
    for i in 0..64 {
        // 64 distinct 100 KB lists: results are right whether cached or not.
        let families = vec![format!("{}{i}", "x".repeat(100_000)), "serif".to_string()];
        let query = FontQuery {
            families: &families,
            weight: 400,
            italic: false,
        };
        assert_eq!(fonts.select(&query), FontId(2));
        assert_eq!(fonts.select(&query), FontId(2));
    }
}

/// The CSS Fonts 4 §5.2 weight algorithm, step by step: the first
/// available weight met while walking the candidates in the specified order.
fn css_weight_reference(desired: u16, available: &[u16]) -> u16 {
    let has = |w: u16| available.contains(&w);
    let mut order: Vec<u16> = Vec::new();
    let mut sorted: Vec<u16> = available.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if (400..=500).contains(&desired) {
        order.extend(sorted.iter().copied().filter(|&w| w >= desired && w <= 500));
        order.extend(sorted.iter().rev().copied().filter(|&w| w < desired));
        order.extend(sorted.iter().copied().filter(|&w| w > 500));
    } else if desired < 400 {
        order.extend(sorted.iter().rev().copied().filter(|&w| w <= desired));
        order.extend(sorted.iter().copied().filter(|&w| w > desired));
    } else {
        order.extend(sorted.iter().copied().filter(|&w| w >= desired));
        order.extend(sorted.iter().rev().copied().filter(|&w| w < desired));
    }
    order.into_iter().find(|&w| has(w)).expect("non-empty")
}

#[test]
fn weight_matching_follows_the_css_algorithm() {
    // Two faces (regular 400, bold 700) of DejaVu Sans: every desired
    // weight picks what the reference algorithm picks.
    let Some(fonts) = dejavu() else { return };
    for desired in (1..=1000u16).step_by(7).chain([400, 450, 500, 501, 399]) {
        let expected = css_weight_reference(desired, &[400, 700]);
        let id = select(&fonts, &["dejavu sans"], desired, false);
        let expected_id = if expected == 400 {
            FontId(0)
        } else {
            FontId(1)
        };
        assert_eq!(id, expected_id, "desired {desired}");
    }
}
