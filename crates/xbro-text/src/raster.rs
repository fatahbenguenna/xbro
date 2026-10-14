//! Glyph rasterisation into 8-bit coverage masks, with a bounded cache.

use std::collections::VecDeque;
use std::sync::Arc;

use ab_glyph::{
    point, Font, Glyph, GlyphId, Outline, OutlineCurve, OutlinedGlyph, Point, PxScale,
    PxScaleFactor,
};

use crate::face::LoadedFace;
use crate::hash::FastMap;
use crate::metrics::sanitize_size;
use crate::sync::lock;
use crate::synthesis::{BOLD_EM, OBLIQUE_SKEW};
use crate::{FontId, FontSystem, GlyphBitmap, Synthesis};

/// Maximum number of cached bitmaps before the cache is reset.
const MAX_RASTER_ENTRIES: usize = 20_000;

/// Maximum number of bytes held by the cache (coverage plus a fixed
/// per-entry overhead) before it is reset: large font sizes would otherwise
/// let 20,000 entries of up to 4 MiB each pile up.
pub(crate) const MAX_RASTER_BYTES: usize = 64 << 20;

/// Approximate bookkeeping cost of one cache entry, in bytes.
const ENTRY_OVERHEAD: usize = 96;

/// Largest bitmap side in pixels (larger glyphs are not rasterised).
const MAX_BITMAP_SIDE: f32 = 2048.0;

/// Largest bitmap area in pixels.
const MAX_BITMAP_AREA: f32 = 2048.0 * 2048.0;

/// Subpixel positions per pixel (pen x is quantised to quarters).
const SUBPIXEL_STEPS: f32 = 4.0;

/// Cache key: face (with its synthetic styles), glyph, size bits and
/// subpixel bucket.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RasterKey {
    font: FontId,
    glyph: u16,
    size_bits: u32,
    bucket: u8,
}

/// Cache: key → bitmap (`None` = empty glyph), bounded in entries and bytes.
#[derive(Default)]
pub(crate) struct RasterCache {
    map: FastMap<RasterKey, Option<Arc<GlyphBitmap>>>,
    /// Approximate memory held by `map`.
    bytes: usize,
}

/// Memory accounted for one cached value.
fn entry_cost(value: &Option<Arc<GlyphBitmap>>) -> usize {
    ENTRY_OVERHEAD + value.as_ref().map_or(0, |bitmap| bitmap.coverage.len())
}

impl RasterCache {
    /// The cached value for `key`, if any.
    fn get(&self, key: &RasterKey) -> Option<&Option<Arc<GlyphBitmap>>> {
        self.map.get(key)
    }

    /// Stores a value, resetting the cache first when it is full (in
    /// entries or bytes).
    fn insert(&mut self, key: RasterKey, value: Option<Arc<GlyphBitmap>>) {
        let cost = entry_cost(&value);
        if self.map.len() >= MAX_RASTER_ENTRIES
            || self.bytes.saturating_add(cost) > MAX_RASTER_BYTES
        {
            self.map.clear();
            self.bytes = 0;
        }
        if let Some(previous) = self.map.insert(key, value) {
            // Another thread rendered the same glyph meanwhile.
            self.bytes = self.bytes.saturating_sub(entry_cost(&previous));
        }
        self.bytes = self.bytes.saturating_add(cost);
    }

    /// Number of cached entries.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// Approximate memory held, in bytes.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Quantises a fractional pen position to a bucket in `0..=4` (in quarter
/// pixels; 4 means "one whole pixel to the right").
pub(crate) fn subpixel_bucket(subpixel_x: f32) -> u8 {
    if !subpixel_x.is_finite() {
        return 0;
    }
    let fract = subpixel_x - subpixel_x.floor();
    (fract * SUBPIXEL_STEPS).round().clamp(0.0, SUBPIXEL_STEPS) as u8
}

impl FontSystem {
    /// Cached rasterisation (see [`FontSystem::rasterize`]).
    pub(crate) fn rasterize_cached(
        &self,
        font: FontId,
        glyph_id: u16,
        size: f32,
        subpixel_x: f32,
    ) -> Option<Arc<GlyphBitmap>> {
        let size = sanitize_size(size);
        if size == 0.0 {
            return None;
        }
        self.collection.face(font)?;
        let bucket = subpixel_bucket(subpixel_x);
        let key = RasterKey {
            font,
            glyph: glyph_id,
            size_bits: size.to_bits(),
            bucket,
        };
        if let Some(hit) = lock(&self.raster_cache).get(&key) {
            return hit.clone();
        }
        let bitmap = self
            .collection
            .loaded(font)
            .and_then(|face| {
                let offset = f32::from(bucket) / SUBPIXEL_STEPS;
                render(face, glyph_id, size, offset, font.synthesis())
            })
            .map(Arc::new);
        lock(&self.raster_cache).insert(key, bitmap.clone());
        bitmap
    }
}

/// Rasterises `glyph` of `face` at `size` px (CSS font size, i.e. px per
/// em) with the pen at (`offset_x`, 0) on the baseline, applying the
/// synthetic styles of `synthesis`. `None` for glyphs without outline or
/// ink, and for absurdly large bitmaps.
pub(crate) fn render(
    face: &LoadedFace,
    glyph: u16,
    size: f32,
    offset_x: f32,
    synthesis: Synthesis,
) -> Option<GlyphBitmap> {
    let id = GlyphId(glyph);
    let mut outline = face.font.outline(id)?;
    if synthesis.oblique {
        shear(&mut outline, OBLIQUE_SKEW);
    }
    // ab_glyph's PxScale is the font *height* (ascent - descent) in px, not
    // the em size: scale by size / units_per_em explicitly.
    let factor = size / face.units_per_em;
    let height = face.font.height_unscaled();
    let px_scale = if height.is_finite() && height > 0.0 {
        size * height / face.units_per_em
    } else {
        size
    };
    let positioned = Glyph {
        id,
        scale: PxScale::from(px_scale),
        position: point(offset_x, 0.0),
    };
    let outlined = OutlinedGlyph::new(
        positioned,
        outline,
        PxScaleFactor {
            horizontal: factor,
            vertical: factor,
        },
    );
    let bounds = outlined.px_bounds();
    let (w, h) = (bounds.width(), bounds.height());
    let sane = |v: f32| v.is_finite() && v.abs() <= 1.0e6;
    if !(sane(w) && sane(h) && sane(bounds.min.x) && sane(bounds.min.y))
        || w < 1.0
        || h < 1.0
        || w > MAX_BITMAP_SIDE
        || h > MAX_BITMAP_SIDE
        || w * h > MAX_BITMAP_AREA
    {
        return None;
    }
    let (width, height) = (w as usize, h as usize);
    let mut coverage = vec![0u8; width * height];
    let mut inked = false;
    outlined.draw(|x, y, c| {
        let index = (y as usize)
            .checked_mul(width)
            .and_then(|row| row.checked_add(x as usize));
        if let Some(pixel) = index.and_then(|i| coverage.get_mut(i)) {
            let value = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
            *pixel = value;
            inked |= value != 0;
        }
    });
    if !inked {
        return None;
    }
    let bitmap = GlyphBitmap {
        width: width as u32,
        height: height as u32,
        left: bounds.min.x as i32,
        top: bounds.min.y as i32,
        coverage,
    };
    if synthesis.bold {
        embolden(bitmap, size * BOLD_EM)
    } else {
        Some(bitmap)
    }
}

/// Shears `outline` (font units, y up) horizontally around the baseline:
/// `x += y × skew`. The bounds become those of the sheared control points
/// (which contain the sheared curves).
fn shear(outline: &mut Outline, skew: f32) {
    let (mut x_min, mut x_max) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut slant = |p: &mut Point| {
        p.x += p.y * skew;
        x_min = x_min.min(p.x);
        x_max = x_max.max(p.x);
    };
    for curve in &mut outline.curves {
        match curve {
            OutlineCurve::Line(a, b) => [a, b].into_iter().for_each(&mut slant),
            OutlineCurve::Quad(a, b, c) => [a, b, c].into_iter().for_each(&mut slant),
            OutlineCurve::Cubic(a, b, c, d) => [a, b, c, d].into_iter().for_each(&mut slant),
        }
    }
    if x_min <= x_max {
        // Outline bounds hold (x_min, y_max) .. (x_max, y_min); a shear
        // leaves the vertical extent unchanged.
        outline.bounds.min.x = x_min;
        outline.bounds.max.x = x_max;
    }
}

/// Synthetic bold: dilates `bitmap` horizontally by `strength` px (the
/// union of the mask shifted right by every offset in `0..=strength`,
/// fractional offsets interpolated), widening it by `ceil(strength)`
/// columns on the right. `None` when the result would exceed the bitmap
/// size limits.
pub(crate) fn embolden(bitmap: GlyphBitmap, strength: f32) -> Option<GlyphBitmap> {
    if !(strength.is_finite() && strength > 0.0) {
        return Some(bitmap);
    }
    let whole = strength.floor();
    // Weight of the fractional copy, in 1/256.
    let frac = ((strength - whole) * 256.0).round() as u32;
    let reach = whole as usize;
    let extra = strength.ceil() as usize;
    let (w, h) = (bitmap.width as usize, bitmap.height as usize);
    let out_w = w.checked_add(extra)?;
    if out_w as f32 > MAX_BITMAP_SIDE || out_w as f32 * h as f32 > MAX_BITMAP_AREA {
        return None;
    }
    let mut coverage = vec![0u8; out_w * h];
    // Indices of the running maximum over the window `x - reach ..= x`.
    let mut window: VecDeque<usize> = VecDeque::with_capacity(reach + 1);
    for (src, dst) in bitmap
        .coverage
        .chunks_exact(w.max(1))
        .zip(coverage.chunks_exact_mut(out_w))
    {
        let at = |i: Option<usize>| i.and_then(|i| src.get(i)).map_or(0, |&c| u32::from(c));
        window.clear();
        for (x, out) in dst.iter_mut().enumerate() {
            if let Some(&value) = src.get(x) {
                while window.back().is_some_and(|&i| src[i] <= value) {
                    window.pop_back();
                }
                window.push_back(x);
            }
            while window.front().is_some_and(|&i| i + reach < x) {
                window.pop_front();
            }
            let max = window.front().map_or(0, |&i| u32::from(src[i]));
            // The copy shifted by the whole strength, between two columns.
            let near = at(x.checked_sub(reach));
            let far = at(x.checked_sub(reach + 1));
            let shifted = ((256 - frac) * near + frac * far + 128) >> 8;
            *out = max.max(shifted).min(255) as u8;
        }
    }
    Some(GlyphBitmap {
        width: out_w as u32,
        coverage,
        ..bitmap
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets() {
        assert_eq!(subpixel_bucket(0.0), 0);
        assert_eq!(subpixel_bucket(0.1), 0);
        assert_eq!(subpixel_bucket(0.2), 1);
        assert_eq!(subpixel_bucket(0.5), 2);
        assert_eq!(subpixel_bucket(0.74), 3);
        assert_eq!(subpixel_bucket(0.9), 4);
        assert_eq!(subpixel_bucket(3.25), 1);
        assert_eq!(subpixel_bucket(-0.25), 3);
        assert_eq!(subpixel_bucket(f32::NAN), 0);
        assert_eq!(subpixel_bucket(f32::INFINITY), 0);
        assert_eq!(subpixel_bucket(1.0e30), 0);
    }

    const SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

    #[test]
    fn cache_memory_is_bounded_for_large_glyphs() {
        let Ok(data) = std::fs::read(SANS) else {
            return;
        };
        let fonts = FontSystem::from_fonts(vec![data]);
        let mut rendered = 0usize;
        let mut drawn_bytes = 0usize;
        // Glyphs at 1000px take up to ~1 MB each: far more than the budget
        // in total, while 20,000 entries would allow gigabytes.
        for glyph in 1..400u16 {
            if let Some(bitmap) = fonts.rasterize(FontId(0), glyph, 1000.0, 0.0) {
                rendered += 1;
                drawn_bytes += bitmap.coverage.len();
            }
            let cache = lock(&fonts.raster_cache);
            assert!(
                cache.bytes() <= MAX_RASTER_BYTES,
                "{} bytes cached",
                cache.bytes()
            );
        }
        assert!(rendered > 100);
        assert!(
            drawn_bytes > 2 * MAX_RASTER_BYTES,
            "test must exceed the budget"
        );
        // Small glyphs are still cached normally.
        let a = fonts.rasterize(FontId(0), 36, 16.0, 0.0).expect("ink");
        let b = fonts.rasterize(FontId(0), 36, 16.0, 0.0).expect("ink");
        assert!(Arc::ptr_eq(&a, &b));
        assert!(lock(&fonts.raster_cache).len() >= 1);
    }

    fn row(coverage: &[u8]) -> GlyphBitmap {
        GlyphBitmap {
            width: coverage.len() as u32,
            height: 1,
            left: -1,
            top: -3,
            coverage: coverage.to_vec(),
        }
    }

    #[test]
    fn emboldening_dilates_to_the_right() {
        // Whole pixels: the union of the mask shifted by 0, 1 and 2 px.
        let out = embolden(row(&[0, 255, 100, 0]), 2.0).expect("bitmap");
        assert_eq!(out.coverage, vec![0, 255, 255, 255, 100, 0]);
        assert_eq!((out.width, out.height, out.left, out.top), (6, 1, -1, -3));
        // Fractional strength: the last copy is interpolated between two
        // columns, so the ink grows by exactly the strength.
        let out = embolden(row(&[255, 255]), 0.5).expect("bitmap");
        assert_eq!(out.coverage, vec![255, 255, 128]);
        let out = embolden(row(&[128, 255, 128]), 1.25).expect("bitmap");
        assert_eq!(out.width, 5);
        assert_eq!(out.coverage[..3], [128, 255, 255]);
        assert_eq!(out.coverage[3], 160, "max(128, 0.75×128 + 0.25×255)");
        assert_eq!(out.coverage[4], 32, "0.25×128");
        // Several rows are dilated independently.
        let two = GlyphBitmap {
            width: 2,
            height: 2,
            left: 0,
            top: 0,
            coverage: vec![255, 0, 0, 255],
        };
        let out = embolden(two, 1.0).expect("bitmap");
        assert_eq!(out.coverage, vec![255, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn emboldening_degenerate_inputs() {
        let base = row(&[10, 20]);
        for strength in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(embolden(base.clone(), strength), Some(base.clone()));
        }
        // Too wide once emboldened: not rasterised, like other huge glyphs.
        assert_eq!(embolden(row(&[1; 2047]), 1.5), None);
        assert!(embolden(row(&[1; 2046]), 1.5).is_some());
        let empty = GlyphBitmap::default();
        let out = embolden(empty, 3.0).expect("bitmap");
        assert!(out.coverage.is_empty());
    }

    #[test]
    fn shearing_moves_points_with_their_height() {
        let mut outline = Outline {
            bounds: ab_glyph::Rect {
                min: point(0.0, 100.0),
                max: point(10.0, -20.0),
            },
            curves: vec![
                OutlineCurve::Line(point(0.0, 0.0), point(0.0, 100.0)),
                OutlineCurve::Quad(point(10.0, 100.0), point(10.0, 40.0), point(10.0, -20.0)),
            ],
        };
        shear(&mut outline, 0.25);
        let OutlineCurve::Line(a, b) = &outline.curves[0] else {
            unreachable!()
        };
        assert_eq!((a.x, b.x, b.y), (0.0, 25.0, 100.0));
        assert_eq!((outline.bounds.min.x, outline.bounds.max.x), (0.0, 35.0));
        assert_eq!((outline.bounds.min.y, outline.bounds.max.y), (100.0, -20.0));
        // No curves: bounds untouched.
        let mut empty = Outline {
            bounds: outline.bounds,
            curves: Vec::new(),
        };
        shear(&mut empty, 0.25);
        assert_eq!(empty.bounds, outline.bounds);
    }

    #[test]
    fn cache_accounting_survives_replacement() {
        let key = RasterKey {
            font: FontId(0),
            glyph: 1,
            size_bits: 16.0f32.to_bits(),
            bucket: 0,
        };
        let bitmap = Arc::new(GlyphBitmap {
            width: 10,
            height: 10,
            left: 0,
            top: -10,
            coverage: vec![1; 100],
        });
        let mut cache = RasterCache::default();
        cache.insert(key, Some(Arc::clone(&bitmap)));
        cache.insert(key, Some(bitmap));
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.bytes(),
            ENTRY_OVERHEAD + 100,
            "replacement is not double counted"
        );
        cache.insert(key, None);
        assert_eq!(cache.bytes(), ENTRY_OVERHEAD);
    }
}
