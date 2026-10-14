//! Font synthesis: synthetic oblique and bold for families without
//! slanted or bold faces, as browsers do.

mod common;

use std::sync::Arc;

use common::*;
use xbro_text::{FontId, FontSystem, GlyphBitmap, Synthesis};

const MONO_BOLD: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf";

const OBLIQUE: Synthesis = Synthesis {
    oblique: true,
    bold: false,
};
const BOLD: Synthesis = Synthesis {
    oblique: false,
    bold: true,
};
const BOLD_OBLIQUE: Synthesis = Synthesis {
    oblique: true,
    bold: true,
};

/// The bitmap of the first glyph of `text` shaped with `font`.
fn glyph(fonts: &FontSystem, font: FontId, size: f32, text: &str) -> Arc<GlyphBitmap> {
    let run = fonts.shape(font, size, text, 0.0, 0.0);
    let g = run.glyphs[0];
    fonts
        .rasterize(g.font, g.glyph_id, size, 0.0)
        .expect("glyph with ink")
}

/// Absolute x (relative to the pen) of the leftmost half-covered pixel of
/// each row, top to bottom (rows without such a pixel are skipped).
fn left_edges(bitmap: &GlyphBitmap) -> Vec<i32> {
    let w = bitmap.width as usize;
    bitmap
        .coverage
        .chunks_exact(w)
        .filter_map(|row| row.iter().position(|&c| c >= 128))
        .map(|x| bitmap.left + x as i32)
        .collect()
}

fn ink(bitmap: &GlyphBitmap) -> u64 {
    bitmap.coverage.iter().map(|&c| u64::from(c)).sum()
}

#[test]
fn italic_without_a_slanted_face_is_synthesised() {
    let Some(fonts) = system(&[SANS]) else { return };
    let regular = select(&fonts, &["dejavu sans"], 400, false);
    assert_eq!(regular, FontId(0));
    assert!(regular.synthesis().is_none());
    let italic = select(&fonts, &["dejavu sans"], 400, true);
    assert_ne!(italic, regular, "a distinct id");
    assert_eq!(italic.face(), FontId(0));
    assert_eq!(italic.synthesis(), OBLIQUE);
    // Same family, metrics and advances as the face.
    assert_eq!(fonts.family_name(italic), "DejaVu Sans");
    assert_eq!(fonts.metrics(italic, 16.0), fonts.metrics(regular, 16.0));
    let text = "Hello, world";
    let a = fonts.shape(regular, 16.0, text, 0.0, 0.0);
    let b = fonts.shape(italic, 16.0, text, 0.0, 0.0);
    assert_eq!(a.width, b.width);
    for (x, y) in a.glyphs.iter().zip(&b.glyphs) {
        assert_eq!((x.glyph_id, x.x, x.advance), (y.glyph_id, y.x, y.advance));
        assert_eq!(y.font, italic, "glyphs are drawn with the synthetic face");
    }
    assert_eq!(fonts.measure(italic, 16.0, text, 0.0, 0.0), b.width);
    // Results are cached and stable.
    assert_eq!(select(&fonts, &["dejavu sans"], 400, true), italic);
}

#[test]
fn synthetic_oblique_slants_the_outlines() {
    let Some(fonts) = system(&[SANS]) else { return };
    let regular = select(&fonts, &["sans-serif"], 400, false);
    let italic = select(&fonts, &["sans-serif"], 400, true);
    let size = 48.0;
    let upright = glyph(&fonts, regular, size, "l");
    let slanted = glyph(&fonts, italic, size, "l");

    let edges = left_edges(&upright);
    let (top, bottom) = (edges[0], *edges.last().unwrap());
    assert!(
        (top - bottom).abs() <= 1,
        "upright 'l' has a vertical left edge: {top} vs {bottom}"
    );

    let edges = left_edges(&slanted);
    let (top, bottom) = (edges[0], *edges.last().unwrap());
    // ≈14°: the top of the stem moves right by a quarter of its height.
    let expected = 0.25 * edges.len() as f32;
    let shift = (top - bottom) as f32;
    assert!(
        (shift - expected).abs() <= 2.0,
        "slanted 'l': left edge {top} at the top, {bottom} at the bottom (expected ≈{expected})"
    );
    // The edge moves right steadily going up.
    assert!(edges.windows(2).all(|w| w[0] >= w[1]), "{edges:?}");
    // Around the baseline: the bottom of the stem barely moves.
    let upright_bottom = *left_edges(&upright).last().unwrap();
    assert!((bottom - upright_bottom).abs() <= 1);
    // Same height, wider box, about the same ink.
    assert_eq!(slanted.height, upright.height);
    assert_eq!(slanted.top, upright.top);
    assert!(slanted.width > upright.width);
    let (a, b) = (ink(&upright) as f32, ink(&slanted) as f32);
    assert!((a - b).abs() / a < 0.1, "ink {a} vs {b}");
}

#[test]
fn synthetic_oblique_shears_descenders_left() {
    let Some(fonts) = system(&[SANS]) else { return };
    let regular = select(&fonts, &["sans-serif"], 400, false);
    let italic = select(&fonts, &["sans-serif"], 400, true);
    let upright = glyph(&fonts, regular, 48.0, "p");
    let slanted = glyph(&fonts, italic, 48.0, "p");
    // The descender (below the baseline) moves left of the upright one.
    let below = |b: &GlyphBitmap| *left_edges(b).last().unwrap();
    assert!(below(&slanted) < below(&upright));
}

#[test]
fn synthetic_bold_widens_advances_and_ink() {
    let Some(fonts) = system(&[SANS]) else { return };
    let regular = select(&fonts, &["dejavu sans"], 400, false);
    let bold = select(&fonts, &["dejavu sans"], 700, false);
    assert_eq!(bold.face(), FontId(0));
    assert_eq!(bold.synthesis(), BOLD);
    assert_eq!(fonts.family_name(bold), "DejaVu Sans");
    assert_eq!(fonts.metrics(bold, 16.0), fonts.metrics(regular, 16.0));

    let size = 16.0;
    let extra = size / 24.0;
    let text = "Hello world";
    let a = fonts.shape(regular, size, text, 0.0, 0.0);
    let b = fonts.shape(bold, size, text, 0.0, 0.0);
    assert_eq!(a.glyphs.len(), b.glyphs.len());
    for (x, y) in a.glyphs.iter().zip(&b.glyphs) {
        assert_close(y.advance, x.advance + extra, 1e-4);
        assert_eq!(y.font, bold);
    }
    assert_close(b.width, a.width + extra * text.len() as f32, 1e-3);
    // Measuring agrees with shaping, with spacing too.
    for (ls, ws) in [(0.0, 0.0), (1.5, 0.0), (0.0, 4.0), (-0.5, 2.0)] {
        let shaped = fonts.shape(bold, size, text, ls, ws).width;
        assert_eq!(fonts.measure(bold, size, text, ls, ws), shaped);
    }

    // More ink, spread to the right by about size/24 px.
    let size = 48.0;
    let thin = glyph(&fonts, regular, size, "l");
    let thick = glyph(&fonts, bold, size, "l");
    assert_eq!(thick.left, thin.left);
    assert_eq!(thick.top, thin.top);
    assert_eq!(thick.height, thin.height);
    assert_eq!(thick.width, thin.width + 2, "48/24 = 2 px wider");
    let (a, b) = (ink(&thin) as f32, ink(&thick) as f32);
    let rows = thin.height as f32;
    let expected = a + 2.0 * 255.0 * rows;
    assert!(
        b > a && (b - expected).abs() / expected < 0.1,
        "ink {a} → {b} (expected ≈{expected})"
    );
}

#[test]
fn zero_width_glyphs_stay_zero_width_in_synthetic_bold() {
    let Some(fonts) = system(&[SANS]) else { return };
    let bold = select(&fonts, &["dejavu sans"], 700, false);
    // U+0301 COMBINING ACUTE ACCENT has no advance in DejaVu Sans.
    let regular = fonts.shape(FontId(0), 16.0, "e\u{301}", 0.0, 0.0);
    let run = fonts.shape(bold, 16.0, "e\u{301}", 0.0, 0.0);
    assert_eq!(regular.glyphs[1].advance, 0.0);
    assert_eq!(run.glyphs[1].advance, 0.0);
    assert_close(
        run.glyphs[0].advance,
        regular.glyphs[0].advance + 16.0 / 24.0,
        1e-4,
    );
}

#[test]
fn bold_and_oblique_combine() {
    let Some(fonts) = system(&[SANS]) else { return };
    let both = select(&fonts, &["sans-serif"], 800, true);
    assert_eq!(both.face(), FontId(0));
    assert_eq!(both.synthesis(), BOLD_OBLIQUE);
    let oblique = glyph(&fonts, FontId(0).with_synthesis(OBLIQUE), 48.0, "l");
    let bold_oblique = glyph(&fonts, both, 48.0, "l");
    assert_eq!(left_edges(&oblique), left_edges(&bold_oblique));
    assert_eq!(bold_oblique.width, oblique.width + 2);
    assert!(ink(&bold_oblique) > ink(&oblique));
}

#[test]
fn real_italic_and_bold_faces_are_preferred() {
    let Some(fonts) = system(&[SANS, SANS_BOLD, MONO, MONO_OBLIQUE]) else {
        return;
    };
    let bold = select(&fonts, &["dejavu sans"], 700, false);
    assert_eq!(bold, FontId(1));
    assert!(bold.synthesis().is_none());
    assert_eq!(select(&fonts, &["dejavu sans"], 600, false), FontId(1));
    // 500 is not bold: the regular face, not emboldened.
    assert_eq!(select(&fonts, &["dejavu sans"], 500, false), FontId(0));
    let oblique = select(&fonts, &["monospace"], 400, true);
    assert_eq!(oblique, FontId(3));
    // The bold face has no slanted variant: it is sheared.
    let bold_italic = select(&fonts, &["dejavu sans"], 700, true);
    assert_eq!(bold_italic, FontId(1).with_synthesis(OBLIQUE));
    // The oblique mono face has no bold variant: it is emboldened.
    let mono = select(&fonts, &["monospace"], 700, true);
    assert_eq!(mono, FontId(3).with_synthesis(BOLD));
    // Light requests are never synthesised.
    assert!(select(&fonts, &["monospace"], 300, false)
        .synthesis()
        .is_none());
}

#[test]
fn bold_faces_heavier_than_500_are_not_emboldened() {
    // A 600 request in a family whose heaviest face is 500 is emboldened,
    // but not one whose face is 600 or more.
    let Some(fonts) = system(&[SANS]) else { return };
    assert_eq!(
        select(&fonts, &["sans-serif"], 600, false).synthesis(),
        BOLD
    );
    assert!(select(&fonts, &["sans-serif"], 599, false)
        .synthesis()
        .is_none());
}

#[test]
fn fallback_glyphs_get_the_requested_style() {
    // DejaVu Sans Mono lacks U+01C4, DejaVu Sans has it.
    let Some(fonts) = system(&[MONO, SANS]) else {
        return;
    };
    let italic = select(&fonts, &["monospace"], 400, true);
    assert_eq!(italic, FontId(0).with_synthesis(OBLIQUE));
    let run = fonts.shape(italic, 16.0, "a\u{1C4}", 0.0, 0.0);
    assert_eq!(run.glyphs[0].font, italic);
    assert_eq!(run.glyphs[1].font, FontId(1).with_synthesis(OBLIQUE));
    let bold = select(&fonts, &["monospace"], 700, false);
    let run = fonts.shape(bold, 16.0, "\u{1C4}", 0.0, 0.0);
    assert_eq!(run.glyphs[0].font, FontId(1).with_synthesis(BOLD));
    let plain = fonts.shape(FontId(1), 16.0, "\u{1C4}", 0.0, 0.0);
    assert_close(
        run.glyphs[0].advance,
        plain.glyphs[0].advance + 16.0 / 24.0,
        1e-4,
    );
    // Upright regular text keeps plain fallback faces.
    let run = fonts.shape(FontId(0), 16.0, "\u{1C4}", 0.0, 0.0);
    assert_eq!(run.glyphs[0].font, FontId(1));
}

#[test]
fn fallback_from_a_real_bold_face_is_emboldened_or_uses_real_bold() {
    let Some(fonts) = system(&[MONO_BOLD, SANS]) else {
        return;
    };
    // The fallback family has no bold face: synthesised.
    let run = fonts.shape(FontId(0), 16.0, "\u{1C4}", 0.0, 0.0);
    assert_eq!(run.glyphs[0].font, FontId(1).with_synthesis(BOLD));
    let Some(fonts) = system(&[MONO, SANS, SANS_BOLD]) else {
        return;
    };
    // The fallback family has a bold face: used as is, even for a
    // synthetic bold primary.
    let bold = select(&fonts, &["monospace"], 700, false);
    assert_eq!(bold, FontId(0).with_synthesis(BOLD));
    let run = fonts.shape(bold, 16.0, "\u{1C4}", 0.0, 0.0);
    assert_eq!(run.glyphs[0].font, FontId(2));
}

#[test]
fn caches_are_keyed_by_synthesis() {
    let Some(fonts) = system(&[SANS]) else { return };
    let ids = [
        FontId(0),
        FontId(0).with_synthesis(OBLIQUE),
        FontId(0).with_synthesis(BOLD),
        FontId(0).with_synthesis(BOLD_OBLIQUE),
    ];
    let g = fonts.shape(FontId(0), 20.0, "k", 0.0, 0.0).glyphs[0].glyph_id;
    for _ in 0..2 {
        let bitmaps: Vec<Arc<GlyphBitmap>> = ids
            .iter()
            .map(|&id| fonts.rasterize(id, g, 20.0, 0.25).expect("ink"))
            .collect();
        for i in 0..bitmaps.len() {
            for j in i + 1..bitmaps.len() {
                assert_ne!(bitmaps[i], bitmaps[j], "{:?} vs {:?}", ids[i], ids[j]);
            }
            // Cached: the same bitmap comes back.
            let again = fonts.rasterize(ids[i], g, 20.0, 0.25).expect("ink");
            assert!(Arc::ptr_eq(&again, &bitmaps[i]));
        }
        // Interleaved shaping keeps per-id advances.
        let widths: Vec<f32> = ids
            .iter()
            .map(|&id| fonts.measure(id, 20.0, "kk", 0.0, 0.0))
            .collect();
        assert_eq!(widths[0], widths[1]);
        assert_eq!(widths[2], widths[3]);
        assert_close(widths[2], widths[0] + 2.0 * 20.0 / 24.0, 1e-4);
    }
}

#[test]
fn synthetic_ids_never_panic() {
    let Some(fonts) = system(&[SANS]) else { return };
    let unknown = [
        FontId(u32::MAX),
        FontId(99).with_synthesis(BOLD_OBLIQUE),
        FontId((1 << 30) - 1),
    ];
    for id in unknown {
        assert_eq!(fonts.family_name(id), "");
        assert_eq!(fonts.metrics(id, 16.0), fonts.metrics(FontId(0), 16.0));
        assert!(fonts.rasterize(id, 36, 16.0, 0.0).is_none());
        let run = fonts.shape(id, 16.0, "abc", 0.0, 0.0);
        assert!(run.width > 0.0 && run.width.is_finite());
    }
    // The flag bits alone name face 0, synthesised.
    assert_eq!(FontId(1 << 30), FontId(0).with_synthesis(BOLD));
    assert_eq!(fonts.family_name(FontId(1 << 30)), "DejaVu Sans");
    let both = FontId(0).with_synthesis(BOLD_OBLIQUE);
    for size in [
        0.0,
        -3.0,
        f32::NAN,
        f32::INFINITY,
        1.0e-3,
        0.5,
        900.0,
        5000.0,
        1.0e9,
    ] {
        let run = fonts.shape(both, size, "Wg\u{301}", 1.0, 1.0);
        assert!(run.width.is_finite(), "{size}");
        for g in &run.glyphs {
            if let Some(b) = fonts.rasterize(g.font, g.glyph_id, size, 0.5) {
                assert_eq!(b.coverage.len(), (b.width * b.height) as usize);
                assert!(b.width <= 2048 && b.height <= 2048);
            }
        }
    }
    // Every glyph of the face, sheared and emboldened, at a small size.
    for glyph in 0..6000u16 {
        if let Some(b) = fonts.rasterize(both, glyph, 9.0, 0.75) {
            assert_eq!(b.coverage.len(), (b.width * b.height) as usize);
        }
    }
}

#[test]
fn degraded_systems_do_not_synthesise() {
    let fonts = FontSystem::from_fonts(Vec::new());
    let id = select(&fonts, &["serif"], 900, true);
    assert_eq!(id, FontId(0));
    assert!(fonts.measure(id, 10.0, "ab", 0.0, 0.0) > 0.0);
}
