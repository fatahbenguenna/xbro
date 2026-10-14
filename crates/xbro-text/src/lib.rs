//! Fonts and text for xbro: system font discovery (fontdb), font selection
//! with CSS-like matching and per-character fallback, simple shaping
//! (cmap lookup + advances + kerning, no complex scripts), text measurement
//! and cached glyph rasterisation (ab_glyph).
//!
//! When a family has no slanted or bold face, [`FontSystem::select`]
//! returns a [`FontId`] denoting the face plus synthetic oblique and/or
//! bold (see [`Synthesis`]), like browsers do.
//!
//! [`FontSystem`] is `Send + Sync`; caches use interior mutability so a single
//! instance can be shared (`Arc<FontSystem>`) between the UI thread and page
//! loading threads.
//!
//! # Sizes
//!
//! Every size is a CSS font size in px, i.e. pixels per em. (ab_glyph's own
//! `PxScale` is the font *height* — ascent minus descent — so this crate
//! converts explicitly: an `M` of DejaVu Sans at 16px advances ≈ 13.8px.)
//!
//! # Robustness
//!
//! No method panics, whatever the input: invalid font ids, NaN or negative
//! sizes, control characters, huge strings or missing fonts all degrade
//! gracefully. With no font at all, text is measured at 0.5em per character
//! and rasterises to nothing. Sizes are clamped to 1,000,000 px and
//! letter/word spacings to ±1,000,000 px, so every metric, position and
//! width is finite.

mod chars;
mod cmap;
mod collection;
mod face;
mod fallback;
mod families;
mod hash;
mod matching;
mod metrics;
mod raster;
mod sfnt;
mod shaping;
mod sync;
mod synthesis;

use std::sync::{Mutex, OnceLock, RwLock};

use collection::FontCollection;
use fallback::FallbackMap;
use matching::SelectCache;
use raster::RasterCache;
use shaping::GlyphMap;
use sync::{read, write};

/// Handle to a loaded font face inside a [`FontSystem`], possibly with
/// synthetic styles: the low 30 bits index the face, the two high bits
/// select synthetic oblique and bold (see [`FontId::face`] and
/// [`FontId::synthesis`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

/// Synthetic styles applied to a face when its family lacks the requested
/// slant or weight (CSS `font-synthesis`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Synthesis {
    /// Outlines sheared by ≈14° around the baseline (`x += 0.25·y`);
    /// advances are unchanged.
    pub oblique: bool,
    /// Ink dilated horizontally by `size / 24` px, and every glyph advance
    /// widened by the same amount.
    pub bold: bool,
}

/// Vertical metrics of a font at a given size, in px.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct FontMetrics {
    /// Distance from the baseline to the top of the em box area (positive).
    pub ascent: f32,
    /// Distance from the baseline to the bottom (positive).
    pub descent: f32,
    /// Extra spacing recommended between lines (hhea line gap, ≥ 0).
    pub line_gap: f32,
    /// Height of lowercase `x` (used for `ex` and `vertical-align: middle`).
    pub x_height: f32,
    /// Underline offset below the baseline (positive = below) and thickness.
    ///
    /// The offset locates the top edge of the underline, as the OpenType
    /// `post` table does.
    pub underline_offset: f32,
    /// Underline thickness in px.
    pub underline_thickness: f32,
}

impl FontMetrics {
    /// The `line-height: normal` value: ascent + descent + line gap.
    pub fn normal_line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }
}

/// One glyph of a shaped run.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ShapedGlyph {
    /// Face this glyph comes from (may differ from the requested font after fallback).
    pub font: FontId,
    /// Glyph index in `font` (0 = `.notdef`).
    pub glyph_id: u16,
    /// Pen x position of this glyph relative to the start of the run, in px.
    pub x: f32,
    /// Advance width in px (including letter-spacing / word-spacing contributions).
    pub advance: f32,
    /// Byte offset of the source character in the run's text.
    pub cluster: usize,
}

/// Result of shaping a string with one font at one size.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ShapedRun {
    /// Glyphs in logical (= visual, left-to-right) order.
    pub glyphs: Vec<ShapedGlyph>,
    /// Total advance width in px.
    pub width: f32,
}

/// An 8-bit coverage mask for one glyph at one size.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GlyphBitmap {
    /// Width of the mask in px.
    pub width: u32,
    /// Height of the mask in px.
    pub height: u32,
    /// Offset in px from the pen position to the bitmap's left edge.
    pub left: i32,
    /// Offset in px from the baseline to the bitmap's top edge (negative = above the baseline).
    pub top: i32,
    /// `width * height` coverage values, row-major, 0 = transparent, 255 = opaque.
    pub coverage: Vec<u8>,
}

/// Font request in CSS terms.
#[derive(Clone, Debug, PartialEq)]
pub struct FontQuery<'a> {
    /// Lowercased family names in priority order; may include generic families
    /// (`serif`, `sans-serif`, `monospace`, `cursive`, `fantasy`, `system-ui`, `ui-monospace`, …).
    pub families: &'a [String],
    /// 100..=900.
    pub weight: u16,
    /// Italic (or oblique) requested.
    pub italic: bool,
}

/// Font database + caches. See the crate documentation.
///
/// Faces are discovered up front (fontdb) but parsed lazily, the first time
/// they are used. Every cache is behind a lock, so the system can be shared
/// between threads.
pub struct FontSystem {
    /// Faces and family index.
    collection: FontCollection,
    /// Face used for an empty or unmatched family list.
    default_font: OnceLock<FontId>,
    /// `select` results.
    select_cache: RwLock<SelectCache>,
    /// (requested font, char) → glyph, face and advance.
    glyph_cache: RwLock<GlyphMap>,
    /// (face, first glyph, second glyph) → kerning in em.
    kern_cache: RwLock<shaping::KernMap>,
    /// char → fallback face.
    fallback_cache: RwLock<FallbackMap>,
    /// Rasterised glyphs.
    raster_cache: Mutex<RasterCache>,
}

// `FontSystem` is shared between the UI and loading threads.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<FontSystem>();
};

impl FontSystem {
    /// Loads the system fonts. Never fails: with no usable fonts at all, text
    /// is measured with a fixed advance of 0.5em and rasterises to nothing.
    ///
    /// Fonts are also loaded from the directories (or font files) listed in
    /// the `XBRO_FONT_DIRS` environment variable (`:`-separated on Unix).
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        if let Some(dirs) = std::env::var_os("XBRO_FONT_DIRS") {
            for dir in std::env::split_paths(&dirs) {
                if dir.as_os_str().is_empty() {
                    continue;
                }
                if dir.is_file() {
                    // Unreadable files are simply skipped.
                    let _ = db.load_font_file(&dir);
                } else {
                    db.load_fonts_dir(&dir);
                }
            }
        }
        Self::from_database(db, true)
    }

    /// Creates a font system containing only the given font files' data
    /// (deterministic, used by tests). Each element is the bytes of a
    /// TTF/OTF/TTC file.
    ///
    /// Face ids follow the order of `fonts` (and of faces inside a collection);
    /// unparsable data is skipped.
    pub fn from_fonts(fonts: Vec<Vec<u8>>) -> Self {
        let mut db = fontdb::Database::new();
        for data in fonts {
            if sfnt::plausible_font_data(&data) {
                db.load_font_data(data);
            }
        }
        Self::from_database(db, false)
    }

    fn from_database(db: fontdb::Database, sort_by_path: bool) -> Self {
        Self {
            collection: FontCollection::new(db, sort_by_path),
            default_font: OnceLock::new(),
            select_cache: RwLock::new(SelectCache::default()),
            glyph_cache: RwLock::new(GlyphMap::default()),
            kern_cache: RwLock::new(shaping::KernMap::default()),
            fallback_cache: RwLock::new(FallbackMap::default()),
            raster_cache: Mutex::new(RasterCache::default()),
        }
    }

    /// True when at least one font face is available.
    ///
    /// Faces that fontdb indexes but ab_glyph cannot parse do not count:
    /// with only such faces the system behaves as if it had no font.
    pub fn has_fonts(&self) -> bool {
        !self.collection.is_empty() && self.collection.loaded(self.default_font()).is_some()
    }

    /// Number of font faces known to the system (loaded or not).
    pub fn face_count(&self) -> usize {
        self.collection.len()
    }

    /// The face used when no requested family matches: the first installed
    /// sans-serif candidate at weight 400 (any face as a last resort;
    /// `FontId(0)` when there are no fonts).
    pub fn default_font(&self) -> FontId {
        if self.collection.is_empty() {
            return FontId(0);
        }
        *self
            .default_font
            .get_or_init(|| self.collection.resolve(&[], 400, false))
    }

    /// Picks the best face for `query` (CSS font matching: first available
    /// family; generic families map to installed fonts, e.g. `sans-serif` →
    /// DejaVu Sans / Liberation Sans / Arial / Helvetica / Noto Sans…; nearest
    /// weight; italic, else oblique, else normal). Results are cached.
    /// Always returns a usable id (falls back to any sans-serif face).
    ///
    /// Family names are compared case-insensitively with every name of every
    /// installed face. Well-known fonts that are not installed (Arial,
    /// Helvetica, Times New Roman, Courier New, Segoe UI, Roboto…) map to
    /// their metric-compatible or generic equivalents; other unknown
    /// families are skipped. Weights follow CSS Fonts 4 §5.2.
    ///
    /// An entry may hold a comma-separated list (commas inside quotes do not
    /// split); a quoted generic keyword (`"serif"`) names an ordinary family,
    /// as in CSS. Faces that cannot be parsed, and bitmap-only faces (colour
    /// emoji fonts, whose glyphs cannot be rasterised), are skipped; a
    /// bitmap-only face is chosen only when nothing else is installed.
    ///
    /// Font synthesis: when the matched face is upright although a slant
    /// was requested, or weighs 500 or less although 600 or more was
    /// requested, the result denotes that face with synthetic oblique
    /// and/or bold ([`FontId::synthesis`]); real italic, oblique and bold
    /// faces are preferred whenever the family has them. Characters taken
    /// from fallback faces get the same treatment.
    pub fn select(&self, query: &FontQuery<'_>) -> FontId {
        if self.collection.is_empty() {
            return FontId(0);
        }
        let weight = query.weight.clamp(1, 1000);
        if let Some(id) = read(&self.select_cache).get(query.families, weight, query.italic) {
            return id;
        }
        let id = if query.families.is_empty() && weight == 400 && !query.italic {
            self.default_font()
        } else {
            let face = self
                .collection
                .resolve(query.families, weight, query.italic);
            match self.collection.face(face) {
                Some(entry) => face.with_synthesis(synthesis::needed(entry, weight, query.italic)),
                None => face,
            }
        };
        write(&self.select_cache).insert(query.families, weight, query.italic, id);
        id
    }

    /// Vertical metrics of `font` at `size` px.
    ///
    /// Ascent, descent and line gap come from the font's horizontal header,
    /// the x-height from the `x` glyph (0.5em when missing) and the
    /// underline from the `post` table (else 0.1em below the baseline,
    /// max(1, em/14) thick). Without a usable font: ascent 0.8em, descent
    /// 0.2em.
    ///
    /// Every value is finite and non-negative: buggy fonts storing a positive
    /// descender (or a negative ascender) are read by magnitude, and fonts
    /// without any vertical extent get the 0.8em / 0.2em split.
    ///
    /// An id that names no usable face (unknown, or unparsable) gives the
    /// metrics of the default face, the one its text is shaped with.
    /// Synthetic styles do not change the metrics.
    pub fn metrics(&self, font: FontId, size: f32) -> FontMetrics {
        let size = metrics::sanitize_size(size);
        if self.collection.is_empty() {
            return metrics::degraded_metrics(size);
        }
        match self.collection.loaded(self.valid_font(font)) {
            Some(face) => metrics::face_metrics(face, size),
            None => metrics::degraded_metrics(size),
        }
    }

    /// Shapes `text` (left-to-right, one glyph per char, with kerning).
    /// Characters missing from `font` use a fallback face that has them (cached
    /// per char); control characters produce no glyph. `letter_spacing` is
    /// added after every glyph, `word_spacing` after every space (U+0020, U+00A0).
    ///
    /// Default-ignorable characters (soft hyphen, zero-width spaces and
    /// joiners, bidi controls, variation selectors, BOM…) and U+2028/U+2029
    /// produce no glyph either; a tab gets the glyph and advance of a space.
    /// Kerning between two glyphs of the same face is added to the first
    /// glyph's advance, so `width` is always the sum of the advances.
    ///
    /// With synthetic bold ([`FontId::synthesis`]), every glyph with a
    /// non-zero advance is `size / 24` px wider (zero-width marks stay
    /// zero-width); glyphs name the (possibly synthesised) face to
    /// rasterise them with.
    pub fn shape(
        &self,
        font: FontId,
        size: f32,
        text: &str,
        letter_spacing: f32,
        word_spacing: f32,
    ) -> ShapedRun {
        // One glyph per char at most; the byte length over-estimates it for
        // non-ASCII text, so cap the up-front reservation.
        let mut glyphs = Vec::with_capacity(text.len().min(4096));
        let width = self.layout_run(font, size, text, letter_spacing, word_spacing, |glyph| {
            glyphs.push(glyph)
        });
        ShapedRun { glyphs, width }
    }

    /// Advance width of `text`; equivalent to `shape(..).width` but cheaper (no allocation).
    pub fn measure(
        &self,
        font: FontId,
        size: f32,
        text: &str,
        letter_spacing: f32,
        word_spacing: f32,
    ) -> f32 {
        self.layout_run(font, size, text, letter_spacing, word_spacing, |_| {})
    }

    /// Rasterises one glyph. `subpixel_x` (0..1) is the fractional pen
    /// position, quantised internally (e.g. to quarters) for caching. Returns
    /// `None` for empty glyphs (spaces). Bitmaps are cached.
    ///
    /// Also `None` for unknown fonts or glyphs, non-positive sizes and
    /// glyphs larger than 2048 px.
    ///
    /// Synthetic styles of `font` are applied: oblique shears the outline
    /// (x += 0.25·y around the baseline), bold widens the ink to the right
    /// by `size / 24` px.
    pub fn rasterize(
        &self,
        font: FontId,
        glyph_id: u16,
        size: f32,
        subpixel_x: f32,
    ) -> Option<std::sync::Arc<GlyphBitmap>> {
        self.rasterize_cached(font, glyph_id, size, subpixel_x)
    }

    /// Family name of a loaded face (for debugging / `--dump-layout`).
    /// Synthesised faces have the name of their face.
    ///
    /// Empty for an unknown id.
    pub fn family_name(&self, font: FontId) -> String {
        self.collection
            .face(font)
            .and_then(|face| face.families.first().cloned())
            .unwrap_or_default()
    }

    /// `font` when it names a usable (parsable) face, else the default face.
    fn valid_font(&self, font: FontId) -> FontId {
        if self.collection.loaded(font).is_some() {
            font
        } else {
            self.default_font()
        }
    }
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for FontSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontSystem")
            .field("faces", &self.collection.len())
            .finish_non_exhaustive()
    }
}
