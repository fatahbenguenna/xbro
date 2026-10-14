//! Property / fuzz-style tests: invariants of shaping and measurement over
//! pseudo-random text, sizes and spacings.

mod common;

use common::*;
use xbro_text::{FontId, FontSystem, ShapedRun};

/// Characters drawn by the random text generator: ASCII, controls,
/// default ignorables, Latin, CJK, emoji, symbols, noncharacters, private
/// use and unassigned code points.
const POOL: &[char] = &[
    'a',
    'V',
    'A',
    'T',
    'o',
    ' ',
    ' ',
    '.',
    '\t',
    '\n',
    '\r',
    '\0',
    '\u{7F}',
    '\u{85}',
    '\u{A0}',
    '\u{AD}',
    '\u{200B}',
    '\u{200D}',
    '\u{2028}',
    '\u{FE0F}',
    '\u{FEFF}',
    '\u{E0041}',
    'é',
    'ß',
    '\u{1C4}',
    '\u{301}',
    '中',
    '文',
    '😀',
    '\u{1FAE0}',
    '€',
    '—',
    '\u{FFFD}',
    '\u{FFFF}',
    '\u{E000}',
    '\u{10FFFD}',
    '\u{3FFFF}',
    'ﷺ',
    'א',
    'ñ',
];

fn random_text(rng: &mut Rng, max_len: u64) -> String {
    (0..rng.below(max_len + 1))
        .map(|_| {
            if rng.below(8) == 0 {
                // Any scalar value.
                char::from_u32(rng.below(0x11_0000) as u32).unwrap_or('x')
            } else {
                *rng.pick(POOL)
            }
        })
        .collect()
}

fn ignorable(c: char) -> bool {
    let u = c as u32;
    (c.is_control() && c != '\t')
        || matches!(
            u,
            // Unicode Default_Ignorable_Code_Point, plus U+2028/U+2029.
            0xAD | 0x34F
                | 0x61C
                | 0x115F..=0x1160
                | 0x17B4..=0x17B5
                | 0x180B..=0x180F
                | 0x200B..=0x200F
                | 0x2028..=0x202E
                | 0x2060..=0x206F
                | 0x3164
                | 0xFE00..=0xFE0F
                | 0xFEFF
                | 0xFFA0
                | 0xFFF0..=0xFFF8
                | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A
                | 0xE0000..=0xE0FFF
        )
}

/// Checks every documented invariant of a shaped run.
#[track_caller]
fn check_run(fonts: &FontSystem, font: FontId, size: f32, text: &str, ls: f32, ws: f32) {
    let run: ShapedRun = fonts.shape(font, size, text, ls, ws);
    let measured = fonts.measure(font, size, text, ls, ws);
    assert_eq!(
        measured.to_bits(),
        run.width.to_bits(),
        "measure != shape for {text:?} (size {size}, ls {ls}, ws {ws})"
    );
    assert!(run.width.is_finite(), "width {} for {text:?}", run.width);
    let visible = text.chars().filter(|c| !ignorable(*c)).count();
    assert_eq!(run.glyphs.len(), visible, "one glyph per visible char");
    // Pen positions are accumulated in f64 and rounded to f32 per glyph.
    let mut pen = 0.0f64;
    let mut last_cluster = None;
    for glyph in &run.glyphs {
        assert!(
            glyph.x.is_finite() && glyph.advance.is_finite(),
            "{glyph:?}"
        );
        assert_eq!(
            glyph.x.to_bits(),
            (pen as f32).to_bits(),
            "x is the running sum"
        );
        assert!(text.is_char_boundary(glyph.cluster), "{glyph:?}");
        let c = text[glyph.cluster..]
            .chars()
            .next()
            .expect("char at cluster");
        assert!(!ignorable(c), "{c:?} must not produce a glyph");
        assert!(last_cluster < Some(glyph.cluster), "clusters increase");
        last_cluster = Some(glyph.cluster);
        assert!(fonts.face_count() == 0 || (glyph.font.0 as usize) < fonts.face_count());
        pen += f64::from(glyph.advance);
    }
    assert_eq!(
        (pen as f32).to_bits(),
        run.width.to_bits(),
        "width = sum of advances"
    );
}

#[test]
fn random_text_satisfies_the_shaping_invariants() {
    let Some(fonts) = dejavu() else { return };
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let sizes = [16.0, 13.3, 0.0, 1.0e-3, 72.0, f32::NAN, -4.0];
    let spacings = [0.0, 1.5, -2.0, 1.0e3, f32::NAN, f32::NEG_INFINITY];
    for _ in 0..400 {
        let text = random_text(&mut rng, 40);
        let font = FontId(rng.below(6) as u32);
        let size = *rng.pick(&sizes);
        let ls = *rng.pick(&spacings);
        let ws = *rng.pick(&spacings);
        check_run(&fonts, font, size, &text, ls, ws);
    }
}

#[test]
fn random_text_without_fonts_satisfies_the_invariants() {
    let fonts = FontSystem::from_fonts(Vec::new());
    let mut rng = Rng(42);
    for _ in 0..200 {
        let text = random_text(&mut rng, 30);
        check_run(&fonts, FontId(rng.below(3) as u32), 10.0, &text, 0.5, 2.0);
        let visible = text.chars().filter(|c| !ignorable(*c)).count();
        assert_eq!(
            fonts.measure(FontId(0), 10.0, &text, 0.0, 0.0),
            5.0 * visible as f32
        );
    }
}

#[test]
fn extreme_sizes_and_spacings_never_produce_infinities() {
    let Some(fonts) = dejavu() else { return };
    let extremes = [f32::MAX, -f32::MAX, 3.0e38, -3.0e38, 1.0e30, 1.0e7];
    // U+2E3B (three-em dash) has an advance of several em.
    for text in ["W  W\u{A0}W", "\u{2E3B} \u{2E3B}", "AV AV", " "] {
        for size in [f32::MAX, 3.0e38, 1.0e30, 1.0e9, 16.0] {
            for ls in extremes.iter().copied().chain([0.0]) {
                for ws in extremes.iter().copied().chain([0.0]) {
                    check_run(&fonts, FontId(0), size, text, ls, ws);
                }
            }
        }
    }
    let m = fonts.metrics(FontId(0), f32::MAX);
    for value in [
        m.ascent,
        m.descent,
        m.x_height,
        m.underline_offset,
        m.underline_thickness,
    ] {
        assert!(value.is_finite() && value > 0.0, "{m:?}");
    }
    let degraded = FontSystem::from_fonts(Vec::new());
    let width = degraded.measure(FontId(0), f32::MAX, "abc", f32::MAX, f32::MAX);
    assert!(width.is_finite());
    assert!(degraded.metrics(FontId(0), f32::MAX).ascent.is_finite());
}

#[test]
fn measuring_is_additive_across_spaces_without_kerning() {
    // With a font that has no kerning between a letter and a space, the
    // width of "a b" is the width of "a " plus the width of "b".
    let Some(fonts) = system(&[MONO]) else { return };
    let mut rng = Rng(7);
    for _ in 0..100 {
        let a: String = (0..rng.below(12))
            .map(|_| *rng.pick(&['x', 'y', 'M', 'i']))
            .collect();
        let b: String = (0..rng.below(12))
            .map(|_| *rng.pick(&['x', 'y', 'M', 'i']))
            .collect();
        let whole = fonts.measure(FontId(0), 15.0, &format!("{a} {b}"), 0.25, 3.0);
        let parts = fonts.measure(FontId(0), 15.0, &format!("{a} "), 0.25, 3.0)
            + fonts.measure(FontId(0), 15.0, &b, 0.25, 3.0);
        assert_close(whole, parts, 1.0e-3);
    }
}

#[test]
fn every_unicode_scalar_value_shapes_quickly() {
    // Distinct characters defeat the per-character caches; uncovered ones
    // must still be cheap (no full fallback scan per character).
    let Some(fonts) = dejavu() else { return };
    let text: String = (0u32..0x11_0000).filter_map(char::from_u32).collect();
    let start = std::time::Instant::now();
    let first = fonts.measure(FontId(0), 16.0, &text, 0.0, 0.0);
    let second = fonts.measure(FontId(0), 16.0, &text, 0.0, 0.0);
    let elapsed = start.elapsed();
    assert_eq!(first, second);
    assert!(first.is_finite() && first > 0.0);
    eprintln!("two passes over all scalar values: {elapsed:?}");
    let limit = if cfg!(debug_assertions) { 20 } else { 4 };
    assert!(elapsed.as_secs() < limit, "took {elapsed:?}");
}
