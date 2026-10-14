//! Shaping and measurement.

mod common;

use std::sync::Arc;

use common::*;
use xbro_text::{FontId, FontSystem, ShapedRun};

fn assert_consistent(fonts: &FontSystem, font: FontId, size: f32, text: &str, ls: f32, ws: f32) {
    let run = fonts.shape(font, size, text, ls, ws);
    let measured = fonts.measure(font, size, text, ls, ws);
    assert_eq!(
        measured.to_bits(),
        run.width.to_bits(),
        "measure != shape for {text:?}"
    );
    // Pen positions are accumulated in f64 and rounded to f32 per glyph.
    let mut pen = 0.0f64;
    for glyph in &run.glyphs {
        assert_eq!(
            glyph.x.to_bits(),
            (pen as f32).to_bits(),
            "x of {glyph:?} in {text:?}"
        );
        pen += f64::from(glyph.advance);
    }
    assert_eq!(
        (pen as f32).to_bits(),
        run.width.to_bits(),
        "width != sum of advances"
    );
}

#[test]
fn long_runs_do_not_drift() {
    let Some(fonts) = dejavu() else { return };
    let one = fonts.measure(FontId(0), 13.0, "x", 0.0, 0.0);
    let text = "x".repeat(500_000);
    let run = fonts.shape(FontId(0), 13.0, &text, 0.0, 0.0);
    let expected = f64::from(one) * 500_000.0;
    assert!(
        (f64::from(run.width) - expected).abs() < 1.0,
        "width {} vs {expected}",
        run.width
    );
    let last = run.glyphs.last().unwrap();
    assert!((f64::from(last.x) - (expected - f64::from(one))).abs() < 1.0);
    assert_eq!(fonts.measure(FontId(0), 13.0, &text, 0.0, 0.0), run.width);
}

#[test]
fn advance_uses_em_size_not_font_height() {
    let Some(fonts) = dejavu() else { return };
    // 'M' advance: 1767 / 2048 em.
    let m = fonts.measure(FontId(0), 16.0, "M", 0.0, 0.0);
    assert_close(m, 16.0 * 1767.0 / 2048.0, 0.01);
    assert_close(m, 13.8, 0.1);
    assert_close(
        fonts.measure(FontId(0), 32.0, "M", 0.0, 0.0),
        2.0 * m,
        0.001,
    );
}

#[test]
fn measure_equals_shape_width() {
    let Some(fonts) = dejavu() else { return };
    let samples = [
        "",
        "Hello, world!",
        "AVAVAV To Ty",
        "naïve café — “quotes” 中文 😀 \u{1C4}",
        "tabs\tand\u{A0}nbsp and\u{200B}zero\u{AD}width\n",
        "\u{0}\u{1}\u{7F}\u{85}",
        "a b  c   d",
    ];
    for font in 0..5 {
        for text in samples {
            for (ls, ws) in [(0.0, 0.0), (1.5, 0.0), (0.0, 4.0), (-0.5, 2.25)] {
                assert_consistent(&fonts, FontId(font), 16.0, text, ls, ws);
                assert_consistent(&fonts, FontId(font), 13.3, text, ls, ws);
            }
        }
    }
}

#[test]
fn clusters_are_byte_offsets() {
    let Some(fonts) = dejavu() else { return };
    let text = "aé中b";
    let run = fonts.shape(FontId(0), 16.0, text, 0.0, 0.0);
    let clusters: Vec<usize> = run.glyphs.iter().map(|g| g.cluster).collect();
    assert_eq!(clusters, vec![0, 1, 3, 6]);
    assert!(run.glyphs.iter().all(|g| g.advance > 0.0));
    assert_eq!(run.glyphs[0].x, 0.0);
}

#[test]
fn kerning_is_applied_between_glyphs_of_the_same_face() {
    let Some(fonts) = dejavu() else { return };
    let pair = fonts.measure(FontId(0), 16.0, "AV", 0.0, 0.0);
    let separate = fonts.measure(FontId(0), 16.0, "A", 0.0, 0.0)
        + fonts.measure(FontId(0), 16.0, "V", 0.0, 0.0);
    assert!(
        pair < separate - 0.1,
        "AV {pair} should be kerned (A+V = {separate})"
    );
    let run = fonts.shape(FontId(0), 16.0, "AV", 0.0, 0.0);
    assert_eq!(run.glyphs.len(), 2);
    // Kerning is folded into the first glyph's advance.
    assert_close(run.glyphs[1].x, run.glyphs[0].advance, 0.0);
    assert!(run.glyphs[0].advance < fonts.measure(FontId(0), 16.0, "A", 0.0, 0.0));
    // A zero-width character does not prevent kerning, a space does.
    let with_zwsp = fonts.measure(FontId(0), 16.0, "A\u{200B}V", 0.0, 0.0);
    assert_close(with_zwsp, pair, 0.0001);
}

#[test]
fn cached_kerning_matches_fresh_lookups() {
    // Kerning pairs are cached per face: a warm cache, a cold one (a new
    // font system) and threads filling a shared one concurrently must
    // agree bit for bit.
    let Some(fonts) = dejavu() else { return };
    let text = "AVATAR To Wa Yo P. LT AV\u{200B}V fiflWAVE Ty r, v. 0123";
    let cold: Vec<u32> = {
        let Some(fresh) = dejavu() else { return };
        fresh
            .shape(FontId(0), 15.0, text, 0.0, 0.0)
            .glyphs
            .iter()
            .map(|g| g.advance.to_bits())
            .collect()
    };
    let advances = |fonts: &FontSystem| -> Vec<u32> {
        fonts
            .shape(FontId(0), 15.0, text, 0.0, 0.0)
            .glyphs
            .iter()
            .map(|g| g.advance.to_bits())
            .collect()
    };
    assert_eq!(advances(&fonts), cold);
    assert_eq!(advances(&fonts), cold);
    let fonts = Arc::new(fonts);
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let fonts = Arc::clone(&fonts);
            std::thread::spawn(move || {
                // Fill the cache with many distinct pairs.
                let mut s = String::new();
                for c in (0x20u32..0x250).filter_map(char::from_u32) {
                    s.push(c);
                    s.push(char::from_u32(0x41 + (c as u32 * 7 + t) % 58).unwrap_or('A'));
                }
                for _ in 0..20 {
                    let _ = fonts.measure(FontId(0), 12.0, &s, 0.0, 0.0);
                }
                advances(&fonts)
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), cold);
    }
    assert_eq!(advances(&fonts), cold);
}

#[test]
fn letter_and_word_spacing() {
    let Some(fonts) = dejavu() else { return };
    let font = FontId(0);
    let base = fonts.measure(font, 16.0, "a b\u{A0}c", 0.0, 0.0);
    // 5 glyphs (incl. both spaces) get letter spacing.
    assert_close(
        fonts.measure(font, 16.0, "a b\u{A0}c", 2.0, 0.0),
        base + 10.0,
        0.001,
    );
    // Word spacing after U+0020 and U+00A0 only.
    assert_close(
        fonts.measure(font, 16.0, "a b\u{A0}c", 0.0, 3.0),
        base + 6.0,
        0.001,
    );
    assert_close(
        fonts.measure(font, 16.0, "a\tb", 0.0, 3.0),
        fonts.measure(font, 16.0, "a\tb", 0.0, 0.0),
        0.0,
    );
    // Negative spacing is allowed.
    assert_close(
        fonts.measure(font, 16.0, "abc", -1.0, 0.0),
        fonts.measure(font, 16.0, "abc", 0.0, 0.0) - 3.0,
        0.001,
    );
    // The spacing is part of each glyph's advance.
    let run = fonts.shape(font, 16.0, "a b", 1.0, 5.0);
    let plain = fonts.shape(font, 16.0, "a b", 0.0, 0.0);
    assert_close(run.glyphs[1].advance, plain.glyphs[1].advance + 6.0, 0.001);
    assert_close(run.glyphs[0].advance, plain.glyphs[0].advance + 1.0, 0.001);
    // Non-finite spacing is ignored.
    assert_eq!(
        fonts.measure(font, 16.0, "a b", f32::NAN, f32::INFINITY),
        plain.width
    );
}

#[test]
fn zero_width_characters_produce_no_glyph() {
    let Some(fonts) = dejavu() else { return };
    let font = FontId(0);
    for c in [
        '\u{0}',
        '\n',
        '\r',
        '\u{1B}',
        '\u{7F}',
        '\u{AD}',
        '\u{200B}',
        '\u{200C}',
        '\u{200D}',
        '\u{200E}',
        '\u{200F}',
        '\u{2028}',
        '\u{2029}',
        '\u{FEFF}',
        '\u{FE00}',
        '\u{FE0F}',
        '\u{34F}',
        '\u{2060}',
        '\u{2064}',
        '\u{202E}',
        '\u{E0041}',
    ] {
        let text = c.to_string();
        let run = fonts.shape(font, 16.0, &text, 2.0, 2.0);
        assert!(run.glyphs.is_empty(), "{c:?} produced {:?}", run.glyphs);
        assert_eq!(run.width, 0.0);
        assert_eq!(
            fonts.measure(font, 16.0, &format!("x{c}y"), 0.0, 0.0),
            fonts.measure(font, 16.0, "xy", 0.0, 0.0)
        );
    }
    let run = fonts.shape(font, 16.0, "a\u{AD}b", 0.0, 0.0);
    assert_eq!(run.glyphs.len(), 2);
    assert_eq!(run.glyphs[1].cluster, 3);
}

#[test]
fn tab_and_no_break_space_render_as_spaces() {
    let Some(fonts) = dejavu() else { return };
    let font = FontId(0);
    let space = fonts.shape(font, 16.0, " ", 0.0, 0.0);
    let tab = fonts.shape(font, 16.0, "\t", 0.0, 0.0);
    assert_eq!(tab.glyphs.len(), 1);
    assert_eq!(tab.glyphs[0].glyph_id, space.glyphs[0].glyph_id);
    assert_eq!(tab.width, space.width);
    let nbsp = fonts.shape(font, 16.0, "\u{A0}", 0.0, 0.0);
    assert_eq!(nbsp.glyphs.len(), 1);
    assert_close(nbsp.width, space.width, 0.001);
}

#[test]
fn missing_characters_use_a_fallback_face() {
    // Mono lacks U+01C4 (DŽ), which DejaVu Sans has.
    let Some(fonts) = system(&[MONO, SANS]) else {
        return;
    };
    let run = fonts.shape(FontId(0), 16.0, "a\u{1C4}b", 0.0, 0.0);
    assert_eq!(run.glyphs.len(), 3);
    assert_eq!(run.glyphs[0].font, FontId(0));
    assert_eq!(run.glyphs[1].font, FontId(1));
    assert_ne!(run.glyphs[1].glyph_id, 0);
    assert_eq!(run.glyphs[2].font, FontId(0));
    assert_consistent(&fonts, FontId(0), 16.0, "a\u{1C4}b", 0.0, 0.0);
    // Cached result is stable.
    assert_eq!(fonts.shape(FontId(0), 16.0, "a\u{1C4}b", 0.0, 0.0), run);
    // The fallback glyph has the fallback face's advance.
    let direct = fonts.measure(FontId(1), 16.0, "\u{1C4}", 0.0, 0.0);
    assert_close(run.glyphs[1].advance, direct, 0.0001);
}

#[test]
fn fallback_prefers_the_matching_weight() {
    // Requested: Mono (lacks U+01C4); fallback family DejaVu Sans has a bold face.
    let Some(fonts) = system(&[
        MONO,
        SANS,
        SANS_BOLD,
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
    ]) else {
        return;
    };
    let regular = fonts.shape(FontId(0), 16.0, "\u{1C4}", 0.0, 0.0);
    assert_eq!(regular.glyphs[0].font, FontId(1));
    let bold = fonts.shape(FontId(3), 16.0, "\u{1C4}", 0.0, 0.0);
    assert_eq!(bold.glyphs[0].font, FontId(2));
}

#[test]
fn characters_missing_everywhere_use_notdef() {
    let Some(fonts) = dejavu() else { return };
    let run = fonts.shape(FontId(0), 16.0, "\u{10FFFD}", 0.0, 0.0);
    assert_eq!(run.glyphs.len(), 1);
    assert_eq!(run.glyphs[0].font, FontId(0));
    assert_eq!(run.glyphs[0].glyph_id, 0);
    assert!(run.width > 0.0);
}

#[test]
fn degenerate_inputs() {
    let Some(fonts) = dejavu() else { return };
    assert_eq!(
        fonts.shape(FontId(0), 16.0, "", 0.0, 0.0),
        ShapedRun::default()
    );
    for size in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(fonts.measure(FontId(0), size, "abc", 0.0, 0.0), 0.0);
    }
    // Unknown ids use the default face.
    assert_eq!(
        fonts.measure(FontId(1000), 16.0, "abc", 0.0, 0.0),
        fonts.measure(FontId(0), 16.0, "abc", 0.0, 0.0)
    );
    // Arbitrary text never panics.
    let text: String = (0u32..0x11_0000)
        .step_by(97)
        .filter_map(char::from_u32)
        .collect();
    assert_consistent(&fonts, FontId(2), 12.0, &text, 0.0, 0.0);
}

#[test]
fn concurrent_use_from_many_threads() {
    let Some(fonts) = dejavu() else { return };
    let fonts = Arc::new(fonts);
    let text = "The quick brown fox jumps over the lazy dog — 中文 \u{1C4} AV";
    let expected = fonts.shape(FontId(3), 15.0, text, 0.0, 0.0);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let fonts = Arc::clone(&fonts);
            std::thread::spawn(move || {
                for _ in 0..50 {
                    let run = fonts.shape(FontId(3), 15.0, text, 0.0, 0.0);
                    assert_eq!(fonts.measure(FontId(3), 15.0, text, 0.0, 0.0), run.width);
                    for g in &run.glyphs {
                        let _ = fonts.rasterize(g.font, g.glyph_id, 15.0, 0.25);
                    }
                }
                fonts.shape(FontId(3), 15.0, text, 0.0, 0.0)
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().expect("thread"), expected);
    }
}

#[test]
fn shaping_without_fonts_uses_half_em_advances() {
    let fonts = FontSystem::from_fonts(Vec::new());
    assert!(!fonts.has_fonts());
    assert_eq!(fonts.measure(FontId(0), 10.0, "abc", 0.0, 0.0), 15.0);
    assert_eq!(
        fonts.measure(FontId(9), 10.0, "a b", 1.0, 2.0),
        15.0 + 3.0 + 2.0
    );
    let run = fonts.shape(FontId(0), 10.0, "a\u{200B}\tb", 0.0, 0.0);
    assert_eq!(run.glyphs.len(), 3);
    assert!(run
        .glyphs
        .iter()
        .all(|g| g.font == FontId(0) && g.glyph_id == 0 && g.advance == 5.0));
    assert_eq!(run.width, 15.0);
    assert_eq!(
        fonts.measure(FontId(0), 10.0, "a\u{200B}\tb", 0.0, 0.0),
        15.0
    );
}

#[test]
fn bitmap_only_fonts_are_a_last_resort_for_fallback() {
    // U+1FAE0 (melting face): DejaVu Sans Mono lacks it; the colour font
    // (bitmap-only) and Unifont Upper (outlines) both have it.
    let melting = "\u{1FAE0}";
    if let Some(fonts) = system(&[MONO, COLOR_EMOJI, UNIFONT_UPPER]) {
        let run = fonts.shape(FontId(0), 16.0, melting, 0.0, 0.0);
        assert_eq!(run.glyphs.len(), 1);
        assert_eq!(run.glyphs[0].font, FontId(2), "outline face preferred");
        assert!(fonts
            .rasterize(FontId(2), run.glyphs[0].glyph_id, 16.0, 0.0)
            .is_some());
    }
    if let Some(fonts) = system(&[MONO, COLOR_EMOJI]) {
        let run = fonts.shape(FontId(0), 16.0, melting, 0.0, 0.0);
        assert_eq!(run.glyphs.len(), 1);
        assert_eq!(run.glyphs[0].font, FontId(1), "colour font as last resort");
        assert!(run.width > 0.0);
        // No outlines: nothing to rasterise, but no panic either.
        assert!(fonts
            .rasterize(FontId(1), run.glyphs[0].glyph_id, 16.0, 0.0)
            .is_none());
    }
}
