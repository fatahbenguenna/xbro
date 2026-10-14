//! Malformed, buggy or hostile font data: metrics must stay sane, nothing
//! may panic and nothing may take quadratic time.

mod common;

use std::time::{Duration, Instant};

use common::*;
use xbro_text::{FontId, FontSystem, Synthesis};

/// Offsets of the `hhea` fields.
const HHEA_ASCENDER: usize = 4;
const HHEA_DESCENDER: usize = 6;

fn read(path: &str) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {path} not installed");
            None
        }
    }
}

#[test]
fn positive_hhea_descender_still_gives_a_positive_descent() {
    // Some buggy fonts store the descender as a positive number.
    let Some(mut font) = read(SANS) else { return };
    patch_i16(&mut font, b"hhea", HHEA_DESCENDER, 483);
    let fonts = FontSystem::from_fonts(vec![font]);
    let m = fonts.metrics(FontId(0), 16.0);
    assert!(m.descent > 0.0, "descent must be positive: {m:?}");
    assert_close(m.descent, 16.0 * 483.0 / 2048.0, 0.01);
    assert_close(m.ascent, 16.0 * 1901.0 / 2048.0, 0.01);
    assert!(m.normal_line_height() > m.ascent);
}

#[test]
fn negative_hhea_ascender_still_gives_a_positive_ascent() {
    let Some(mut font) = read(SANS) else { return };
    patch_i16(&mut font, b"hhea", HHEA_ASCENDER, -1901);
    let fonts = FontSystem::from_fonts(vec![font]);
    let m = fonts.metrics(FontId(0), 16.0);
    assert!(m.ascent > 0.0, "ascent must be positive: {m:?}");
    assert_close(m.ascent, 16.0 * 1901.0 / 2048.0, 0.01);
    assert!(m.descent > 0.0);
}

#[test]
fn zero_vertical_metrics_use_the_em_box() {
    // ttf-parser falls back to OS/2 when hhea is zero: zero those too by
    // removing the OS/2 table.
    let Some(mut font) = read(SANS) else { return };
    patch_i16(&mut font, b"hhea", HHEA_ASCENDER, 0);
    patch_i16(&mut font, b"hhea", HHEA_DESCENDER, 0);
    rename_table(&mut font, b"OS/2", b"XS/2");
    let fonts = FontSystem::from_fonts(vec![font]);
    let m = fonts.metrics(FontId(0), 10.0);
    assert_close(m.ascent, 8.0, 0.001);
    assert_close(m.descent, 2.0, 0.001);
}

/// DejaVu Sans without its `head` table: fontdb indexes it (it has a name)
/// but ab_glyph cannot parse it.
fn unparsable_sans() -> Option<Vec<u8>> {
    let mut font = read(SANS)?;
    rename_table(&mut font, b"head", b"hexd");
    Some(font)
}

#[test]
fn a_database_of_unparsable_faces_degrades_like_an_empty_one() {
    let Some(broken) = unparsable_sans() else {
        return;
    };
    let fonts = FontSystem::from_fonts(vec![broken]);
    assert_eq!(fonts.face_count(), 1, "fontdb indexes the face");
    assert!(!fonts.has_fonts(), "but no face is usable");
    let id = select(&fonts, &["dejavu sans"], 400, false);
    assert_eq!(id, FontId(0));
    let m = fonts.metrics(id, 10.0);
    assert_eq!((m.ascent, m.descent), (8.0, 2.0));
    assert_eq!(fonts.measure(id, 10.0, "abcd", 0.0, 0.0), 20.0);
    let run = fonts.shape(id, 10.0, "ab", 0.0, 0.0);
    assert_eq!(run.glyphs.len(), 2);
    assert!(fonts.rasterize(id, 36, 10.0, 0.0).is_none());
}

#[test]
fn unparsable_faces_are_replaced_by_usable_ones() {
    let (Some(broken), Some(sans)) = (unparsable_sans(), read(SANS)) else {
        return;
    };
    let fonts = FontSystem::from_fonts(vec![broken, sans]);
    assert!(fonts.has_fonts());
    // Both faces claim the family: the usable one is selected.
    assert_eq!(select(&fonts, &["dejavu sans"], 400, false), FontId(1));
    assert_eq!(fonts.default_font(), FontId(1));
    // Asking for the broken face directly: metrics and glyphs come from a
    // usable face instead of the degraded 0.8em/0.5em guesses.
    assert_eq!(
        fonts.metrics(FontId(0), 16.0),
        fonts.metrics(FontId(1), 16.0)
    );
    let run = fonts.shape(FontId(0), 16.0, "abc", 0.0, 0.0);
    assert!(run.glyphs.iter().all(|g| g.font == FontId(1)));
    assert_eq!(
        run.width,
        fonts.measure(FontId(1), 16.0, "abc", 0.0, 0.0),
        "same advances as the usable face"
    );
}

#[test]
fn huge_collections_are_indexed_in_linear_time() {
    // A collection whose header announces 150,000 faces, all pointing to
    // the same face. Indexing must not be quadratic in the number of faces
    // (it took ~10 s with a linear duplicate check per face).
    let Some(sans) = read(SANS) else { return };
    let count = 150_000u32;
    let ttc = collection_of_copies(&sans, count);
    let start = Instant::now();
    let fonts = FontSystem::from_fonts(vec![ttc]);
    let built = start.elapsed();
    assert_eq!(fonts.face_count(), count as usize);
    let id = select(&fonts, &["dejavu sans"], 700, true);
    assert_eq!(id.face(), FontId(0), "ties keep the lowest id");
    // Every copy is a regular face: bold and italic are synthesised.
    assert_eq!(
        id.synthesis(),
        Synthesis {
            oblique: true,
            bold: true
        }
    );
    assert_eq!(fonts.family_name(FontId(count - 1)), "DejaVu Sans");
    assert!(fonts.measure(FontId(count - 1), 16.0, "abc", 0.0, 0.0) > 0.0);
    eprintln!("indexed {count} faces in {built:?}");
    assert!(
        built < Duration::from_secs(2),
        "indexing {count} faces took {built:?}"
    );
}

/// DejaVu Sans whose `cmap` is replaced by `cmap` (appended to the file).
fn with_cmap(font: &[u8], cmap: &[u8]) -> Vec<u8> {
    let mut font = font.to_vec();
    let (record, _, _) = table_record(&font, b"cmap").expect("cmap");
    let offset = font.len() as u32;
    font[record + 8..record + 12].copy_from_slice(&offset.to_be_bytes());
    font[record + 12..record + 16].copy_from_slice(&(cmap.len() as u32).to_be_bytes());
    font.extend_from_slice(cmap);
    font
}

#[test]
fn cmap_with_thousands_of_subtables_is_bounded() {
    // 65,535 Unicode subtable records at distinct offsets into a region of
    // 0x0006 words (each parses as a tiny format 6 subtable).
    let (Some(sans), Some(mono)) = (read(SANS), read(MONO)) else {
        return;
    };
    let records = 65_535usize;
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&(records as u16).to_be_bytes());
    let region = 4 + 8 * records;
    for i in 0..records {
        cmap.extend_from_slice(&3u16.to_be_bytes());
        cmap.extend_from_slice(&1u16.to_be_bytes());
        cmap.extend_from_slice(&((region + 2 * i) as u32).to_be_bytes());
    }
    for _ in 0..records + 64 {
        cmap.extend_from_slice(&6u16.to_be_bytes());
    }
    let fonts = FontSystem::from_fonts(vec![mono, with_cmap(&sans, &cmap)]);
    let start = Instant::now();
    // U+01C4 is missing from Mono: the fallback search reads the cmap of face 1.
    let run = fonts.shape(FontId(0), 16.0, "\u{1C4}", 0.0, 0.0);
    let elapsed = start.elapsed();
    assert_eq!(run.glyphs.len(), 1);
    assert!(
        elapsed < Duration::from_millis(250),
        "coverage of a hostile cmap took {elapsed:?}"
    );
}

#[test]
fn mutated_font_files_never_panic() {
    let Some(original) = read(MONO) else { return };
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let header_len = 12 + 16 * usize::from(u16::from_be_bytes([original[4], original[5]]));
    for round in 0..60 {
        let mut font = original.clone();
        // Corrupt the table directory, then random bytes anywhere.
        for _ in 0..(1 + rng.below(6)) {
            let at = rng.below(header_len as u64) as usize;
            font[at] = rng.next_u64() as u8;
        }
        for _ in 0..rng.below(200) {
            let at = rng.below(font.len() as u64) as usize;
            font[at] = rng.next_u64() as u8;
        }
        if round % 5 == 0 {
            let keep = rng.below(font.len() as u64) as usize;
            font.truncate(keep);
        }
        let fonts = FontSystem::from_fonts(vec![font]);
        let id = select(&fonts, &["monospace"], 700, true);
        let m = fonts.metrics(id, 16.0);
        assert!(m.ascent.is_finite() && m.ascent >= 0.0, "{m:?}");
        assert!(m.descent.is_finite() && m.descent >= 0.0, "{m:?}");
        assert!(m.line_gap.is_finite() && m.line_gap >= 0.0, "{m:?}");
        let text = "Hello, wörld! \u{1C4} 中 \t\u{A0}AV";
        let run = fonts.shape(id, 16.0, text, 0.5, 1.0);
        assert_eq!(fonts.measure(id, 16.0, text, 0.5, 1.0), run.width);
        assert!(run.width.is_finite());
        for glyph in &run.glyphs {
            if let Some(bitmap) = fonts.rasterize(glyph.font, glyph.glyph_id, 16.0, 0.25) {
                assert_eq!(
                    bitmap.coverage.len(),
                    (bitmap.width * bitmap.height) as usize
                );
            }
        }
    }
}

#[test]
fn truncated_collections_never_panic() {
    let Some(sans) = read(SANS) else { return };
    let ttc = collection_of_copies(&sans, 3);
    for len in (0..ttc.len()).step_by(ttc.len() / 97 + 1) {
        let fonts = FontSystem::from_fonts(vec![ttc[..len].to_vec()]);
        let run = fonts.shape(FontId(2), 12.0, "abc", 0.0, 0.0);
        assert_eq!(fonts.measure(FontId(2), 12.0, "abc", 0.0, 0.0), run.width);
        let _ = fonts.metrics(FontId(1), 12.0);
    }
}
