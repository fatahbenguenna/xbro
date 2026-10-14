//! System fonts (no assumption about which fonts are installed) and the
//! `XBRO_FONT_DIRS` environment variable. This file holds a single test
//! because it modifies the process environment.

use xbro_text::{FontId, FontQuery, FontSystem};

fn exercise(fonts: &FontSystem) {
    for family in ["sans-serif", "serif", "monospace", "arial", "unknown-font"] {
        for (weight, italic) in [(400, false), (700, false), (400, true), (300, true)] {
            let families = vec![family.to_string()];
            let id = fonts.select(&FontQuery {
                families: &families,
                weight,
                italic,
            });
            let text = "Hello, world! 中文 😀 \u{1C4} ﷺ  א ñ";
            let run = fonts.shape(id, 16.0, text, 0.0, 0.0);
            assert_eq!(fonts.measure(id, 16.0, text, 0.0, 0.0), run.width);
            let m = fonts.metrics(id, 16.0);
            assert!(m.ascent.is_finite() && m.descent.is_finite());
            if fonts.has_fonts() {
                assert!(!fonts.family_name(id).is_empty());
                assert!(run.width > 0.0);
                assert!(m.ascent > 0.0);
                for glyph in run.glyphs.iter().take(8) {
                    let _ = fonts.rasterize(glyph.font, glyph.glyph_id, 16.0, 0.3);
                }
            }
        }
    }
}

#[test]
fn system_fonts_and_extra_font_dirs() {
    std::env::remove_var("XBRO_FONT_DIRS");
    let fonts = FontSystem::new();
    let base = fonts.face_count();
    eprintln!(
        "{base} system faces; default: {:?}",
        fonts.family_name(fonts.default_font())
    );
    exercise(&fonts);
    if !fonts.has_fonts() {
        assert_eq!(fonts.default_font(), FontId(0));
    }

    // Extra directories (and missing ones) from XBRO_FONT_DIRS are scanned too.
    let dejavu = "/usr/share/fonts/truetype/dejavu";
    if std::path::Path::new(dejavu).is_dir() {
        std::env::set_var("XBRO_FONT_DIRS", format!("/nonexistent/dir::{dejavu}"));
        let more = FontSystem::new();
        std::env::remove_var("XBRO_FONT_DIRS");
        assert!(
            more.face_count() > base,
            "{} vs {}",
            more.face_count(),
            base
        );
        exercise(&more);
    }
}
