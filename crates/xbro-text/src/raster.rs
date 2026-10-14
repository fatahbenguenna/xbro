//! Glyph rasterisation into 8-bit coverage masks, with a bounded cache.

use std::sync::Arc;

use ab_glyph::{point, Font, Glyph, GlyphId, OutlinedGlyph, PxScale, PxScaleFactor};

use crate::face::LoadedFace;
use crate::hash::FastMap;
use crate::metrics::sanitize_size;
use crate::sync::lock;
use crate::{FontId, FontSystem, GlyphBitmap};

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

/// Cache key: face, glyph, size bits and subpixel bucket.
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
            .and_then(|face| render(face, glyph_id, size, f32::from(bucket) / SUBPIXEL_STEPS))
            .map(Arc::new);
        lock(&self.raster_cache).insert(key, bitmap.clone());
        bitmap
    }
}

/// Rasterises `glyph` of `face` at `size` px (CSS font size, i.e. px per
/// em) with the pen at (`offset_x`, 0) on the baseline. `None` for glyphs
/// without outline or ink, and for absurdly large bitmaps.
pub(crate) fn render(
    face: &LoadedFace,
    glyph: u16,
    size: f32,
    offset_x: f32,
) -> Option<GlyphBitmap> {
    let id = GlyphId(glyph);
    let outline = face.font.outline(id)?;
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
    inked.then_some(GlyphBitmap {
        width: width as u32,
        height: height as u32,
        left: bounds.min.x as i32,
        top: bounds.min.y as i32,
        coverage,
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
