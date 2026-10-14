//! Vertical metrics.

mod common;

use common::*;
use xbro_text::{FontId, FontSystem};

#[test]
fn dejavu_sans_metrics_at_16px() {
    let Some(fonts) = dejavu() else { return };
    let m = fonts.metrics(FontId(0), 16.0);
    // hhea: ascender 1901, descender -483, line gap 0, 2048 units per em.
    assert_close(m.ascent, 16.0 * 1901.0 / 2048.0, 0.01);
    assert_close(m.ascent, 14.85, 0.01);
    assert_close(m.descent, 16.0 * 483.0 / 2048.0, 0.01);
    assert_close(m.line_gap, 0.0, 0.001);
    assert_close(m.normal_line_height(), 16.0 * 2384.0 / 2048.0, 0.01);
    // 'x' outline top: 1120 units.
    assert_close(m.x_height, 16.0 * 1120.0 / 2048.0, 0.01);
    // post table: underlinePosition -40, underlineThickness 90.
    assert_close(m.underline_offset, 16.0 * 40.0 / 2048.0, 0.01);
    assert_close(m.underline_thickness, 16.0 * 90.0 / 2048.0, 0.01);
}

#[test]
fn metrics_scale_linearly() {
    let Some(fonts) = dejavu() else { return };
    for id in 0..4 {
        let font = FontId(id);
        let a = fonts.metrics(font, 10.0);
        let b = fonts.metrics(font, 30.0);
        assert_close(b.ascent, a.ascent * 3.0, 0.001);
        assert_close(b.descent, a.descent * 3.0, 0.001);
        assert_close(b.x_height, a.x_height * 3.0, 0.001);
        assert!(a.ascent > 0.0 && a.descent > 0.0 && a.x_height > 0.0);
        assert!(a.x_height < a.ascent);
        assert!(a.underline_thickness > 0.0 && a.underline_offset > 0.0);
    }
}

#[test]
fn bad_sizes_and_ids_do_not_panic() {
    let Some(fonts) = dejavu() else { return };
    for size in [0.0, -5.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let m = fonts.metrics(FontId(0), size);
        assert_eq!(m.ascent, 0.0);
        assert_eq!(m.descent, 0.0);
        assert!(m.underline_thickness.is_finite());
    }
    let huge = fonts.metrics(FontId(0), 1.0e30);
    assert!(huge.ascent.is_finite() && huge.ascent > 0.0);
    // Unknown ids use the default face.
    assert_eq!(
        fonts.metrics(FontId(77), 16.0),
        fonts.metrics(FontId(0), 16.0)
    );
}

#[test]
fn degraded_metrics_without_fonts() {
    let fonts = FontSystem::from_fonts(Vec::new());
    let m = fonts.metrics(FontId(0), 20.0);
    assert_eq!(m.ascent, 16.0);
    assert_eq!(m.descent, 4.0);
    assert_eq!(m.line_gap, 0.0);
    assert_eq!(m.x_height, 10.0);
    assert_eq!(m.underline_offset, 2.0);
    assert_close(m.underline_thickness, 20.0 / 14.0, 1e-6);
    assert_eq!(fonts.metrics(FontId(5), 7.0).underline_thickness, 1.0);
}
