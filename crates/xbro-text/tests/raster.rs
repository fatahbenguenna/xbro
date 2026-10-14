//! Glyph rasterisation.

mod common;

use std::sync::Arc;

use common::*;
use xbro_text::{FontId, FontSystem};

fn glyph_of(fonts: &FontSystem, font: FontId, c: char) -> u16 {
    fonts.shape(font, 16.0, &c.to_string(), 0.0, 0.0).glyphs[0].glyph_id
}

#[test]
fn rasterizes_a_letter() {
    let Some(fonts) = dejavu() else { return };
    let glyph = glyph_of(&fonts, FontId(0), 'A');
    let bitmap = fonts
        .rasterize(FontId(0), glyph, 16.0, 0.0)
        .expect("'A' has ink");
    assert!(bitmap.width > 5 && bitmap.height > 5);
    assert_eq!(
        bitmap.coverage.len(),
        (bitmap.width * bitmap.height) as usize
    );
    assert!(
        bitmap.coverage.iter().any(|&c| c > 200),
        "solid pixels expected"
    );
    assert!(bitmap.coverage.contains(&0), "empty pixels expected");
    // Cap height of DejaVu Sans is 1493 units: ~11.7px above the baseline.
    assert!(bitmap.top <= -11 && bitmap.top >= -13, "top {}", bitmap.top);
    assert_eq!(
        bitmap.top + bitmap.height as i32,
        0,
        "'A' sits on the baseline"
    );
    assert!(
        bitmap.left >= -1 && bitmap.left <= 1,
        "left {}",
        bitmap.left
    );
    // Twice the size, roughly twice the bitmap.
    let big = fonts.rasterize(FontId(0), glyph, 32.0, 0.0).expect("ink");
    assert!(big.height >= 2 * bitmap.height - 2 && big.height <= 2 * bitmap.height + 2);
}

#[test]
fn descenders_extend_below_the_baseline() {
    let Some(fonts) = dejavu() else { return };
    let glyph = glyph_of(&fonts, FontId(0), 'p');
    let bitmap = fonts.rasterize(FontId(0), glyph, 20.0, 0.0).expect("ink");
    assert!(bitmap.top < 0);
    assert!(bitmap.top + bitmap.height as i32 > 2);
}

#[test]
fn empty_glyphs_give_none() {
    let Some(fonts) = dejavu() else { return };
    let space = glyph_of(&fonts, FontId(0), ' ');
    assert!(fonts.rasterize(FontId(0), space, 16.0, 0.0).is_none());
    assert!(fonts.rasterize(FontId(0), space, 16.0, 0.5).is_none());
}

#[test]
fn bitmaps_are_cached_per_subpixel_bucket() {
    let Some(fonts) = dejavu() else { return };
    let glyph = glyph_of(&fonts, FontId(0), 'o');
    let a = fonts.rasterize(FontId(0), glyph, 16.0, 0.0).expect("ink");
    let b = fonts.rasterize(FontId(0), glyph, 16.0, 0.05).expect("ink");
    assert!(Arc::ptr_eq(&a, &b), "same quarter-pixel bucket");
    let half = fonts.rasterize(FontId(0), glyph, 16.0, 0.5).expect("ink");
    assert!(!Arc::ptr_eq(&a, &half));
    assert_ne!(a.coverage, half.coverage, "shifted by half a pixel");
    // A fraction close to 1 rounds to the next pixel.
    let next = fonts.rasterize(FontId(0), glyph, 16.0, 0.95).expect("ink");
    assert_eq!(next.left, a.left + 1);
    assert_eq!(next.coverage, a.coverage);
}

#[test]
fn invalid_requests_give_none() {
    let Some(fonts) = dejavu() else { return };
    let glyph = glyph_of(&fonts, FontId(0), 'A');
    assert!(fonts.rasterize(FontId(99), glyph, 16.0, 0.0).is_none());
    assert!(fonts.rasterize(FontId(0), u16::MAX, 16.0, 0.0).is_none());
    for size in [0.0, -3.0, f32::NAN, f32::INFINITY, 1.0e7] {
        assert!(
            fonts.rasterize(FontId(0), glyph, size, 0.0).is_none(),
            "size {size}"
        );
    }
    for subpixel in [f32::NAN, f32::INFINITY, -7.3, 1.0e20] {
        let _ = fonts.rasterize(FontId(0), glyph, 16.0, subpixel);
    }
    // Tiny sizes do not panic.
    let _ = fonts.rasterize(FontId(0), glyph, 0.01, 0.0);
}

#[test]
fn every_glyph_of_a_font_rasterizes_without_panicking() {
    let Some(fonts) = system(&[MONO]) else { return };
    for glyph in 0..u16::MAX {
        let result = fonts.rasterize(FontId(0), glyph, 9.0, 0.0);
        if let Some(bitmap) = result {
            assert_eq!(
                bitmap.coverage.len(),
                (bitmap.width * bitmap.height) as usize
            );
        } else if glyph > 6000 {
            break;
        }
    }
}

#[test]
fn rasterizing_without_fonts_gives_none() {
    let fonts = FontSystem::from_fonts(Vec::new());
    assert!(fonts.rasterize(FontId(0), 0, 16.0, 0.0).is_none());
    assert!(fonts.rasterize(FontId(0), 36, 16.0, 0.0).is_none());
}
