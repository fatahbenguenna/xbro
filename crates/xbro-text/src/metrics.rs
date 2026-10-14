//! Vertical font metrics at a given size.

use crate::face::LoadedFace;
use crate::FontMetrics;

/// Largest font size in px; larger sizes are clamped to it.
///
/// Together with [`MAX_SPACING`] this keeps every advance, position and
/// width finite: a glyph advance is at most 4096 em (65,535 units for 16
/// units per em), so even billions of glyphs stay far below `f32::MAX`.
pub(crate) const MAX_SIZE: f32 = 1.0e6;

/// Largest letter/word spacing magnitude in px; larger values are clamped.
pub(crate) const MAX_SPACING: f32 = 1.0e6;

/// Sanitises a size in px: non-finite or negative sizes become 0, huge
/// sizes are clamped to [`MAX_SIZE`].
#[inline]
pub(crate) fn sanitize_size(size: f32) -> f32 {
    if size.is_finite() && size > 0.0 {
        size.min(MAX_SIZE)
    } else {
        0.0
    }
}

/// Sanitises a spacing value: non-finite values become 0, huge values are
/// clamped to ±[`MAX_SPACING`].
#[inline]
pub(crate) fn sanitize_spacing(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-MAX_SPACING, MAX_SPACING)
    } else {
        0.0
    }
}

/// Underline thickness used when the font gives none: max(1, em/14).
fn fallback_underline_thickness(size: f32) -> f32 {
    (size / 14.0).max(1.0)
}

/// Metrics used when no font is available: ascent 0.8em, descent 0.2em,
/// x-height 0.5em, underline 0.1em below the baseline.
pub(crate) fn degraded_metrics(size: f32) -> FontMetrics {
    FontMetrics {
        ascent: 0.8 * size,
        descent: 0.2 * size,
        line_gap: 0.0,
        x_height: 0.5 * size,
        underline_offset: 0.1 * size,
        underline_thickness: fallback_underline_thickness(size),
    }
}

/// Metrics of a loaded face at `size` px (font units scaled by size / em).
pub(crate) fn face_metrics(face: &LoadedFace, size: f32) -> FontMetrics {
    let scale = size / face.units_per_em;
    let (underline_offset, underline_thickness) = match face.underline {
        Some((offset, thickness)) => (offset * scale, thickness * scale),
        None => (0.1 * size, fallback_underline_thickness(size)),
    };
    FontMetrics {
        ascent: face.ascent * scale,
        descent: face.descent * scale,
        line_gap: face.line_gap * scale,
        x_height: face.x_height.map_or(0.5 * size, |h| h * scale),
        underline_offset,
        underline_thickness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_values() {
        let m = degraded_metrics(20.0);
        assert_eq!(m.ascent, 16.0);
        assert_eq!(m.descent, 4.0);
        assert_eq!(m.line_gap, 0.0);
        assert_eq!(m.x_height, 10.0);
        assert_eq!(m.underline_offset, 2.0);
        assert_eq!(m.underline_thickness, 20.0 / 14.0);
        assert_eq!(m.normal_line_height(), 20.0);
        assert_eq!(degraded_metrics(7.0).underline_thickness, 1.0);
    }

    #[test]
    fn sanitizing() {
        assert_eq!(sanitize_size(f32::NAN), 0.0);
        assert_eq!(sanitize_size(f32::INFINITY), 0.0);
        assert_eq!(sanitize_size(-3.0), 0.0);
        assert_eq!(sanitize_size(12.5), 12.5);
        assert_eq!(sanitize_spacing(f32::NAN), 0.0);
        assert_eq!(sanitize_spacing(f32::NEG_INFINITY), 0.0);
        assert_eq!(sanitize_spacing(-2.0), -2.0);
        assert_eq!(sanitize_size(f32::MAX), MAX_SIZE);
        assert_eq!(sanitize_size(MAX_SIZE * 2.0), MAX_SIZE);
        assert_eq!(sanitize_size(f32::MIN_POSITIVE), f32::MIN_POSITIVE);
        assert_eq!(sanitize_spacing(f32::MAX), MAX_SPACING);
        assert_eq!(sanitize_spacing(-f32::MAX), -MAX_SPACING);
    }

    #[test]
    fn degraded_metrics_are_finite_at_the_largest_size() {
        let m = degraded_metrics(sanitize_size(f32::MAX));
        for value in [
            m.ascent,
            m.descent,
            m.x_height,
            m.underline_offset,
            m.underline_thickness,
        ] {
            assert!(value.is_finite() && value > 0.0, "{m:?}");
        }
    }
}
