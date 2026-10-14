//! Faces of the font database: static info gathered from fontdb, plus the
//! lazily loaded ab_glyph font and lazily computed character coverage.

use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, FontVec};

use crate::cmap::Coverage;
use crate::sfnt::{self, FaceTables};

/// Style of a face, as reported by fontdb.
pub(crate) type FaceStyle = fontdb::Style;

/// One face of the database, addressed by its dense [`crate::FontId`].
pub(crate) struct FaceEntry {
    /// fontdb identifier, used to reach the font data.
    pub db_id: fontdb::ID,
    /// Family names as found in the font (English US first when present).
    pub families: Vec<String>,
    /// Lowercased family names (same order).
    pub families_lower: Vec<String>,
    /// OS/2 weight class.
    pub weight: u16,
    pub style: FaceStyle,
    /// Width class 1 (ultra-condensed) ..= 9 (ultra-expanded), 5 = normal.
    pub stretch: u16,
    pub monospaced: bool,
    /// The parsed font; `None` inside when the face failed to load.
    loaded: OnceLock<Option<LoadedFace>>,
    /// The face has vector outlines (read from the table directory).
    outlines: OnceLock<bool>,
    /// Character coverage; `None` inside when the data could not be read.
    coverage: OnceLock<Option<FaceCoverage>>,
}

impl FaceEntry {
    /// Builds an entry from fontdb's face info.
    pub(crate) fn new(info: &fontdb::FaceInfo) -> Self {
        let families: Vec<String> = info.families.iter().map(|(name, _)| name.clone()).collect();
        let families_lower = families
            .iter()
            .map(|name| crate::families::normalize(name))
            .collect();
        Self {
            db_id: info.id,
            families,
            families_lower,
            weight: info.weight.0,
            style: info.style,
            stretch: info.stretch.to_number(),
            monospaced: info.monospaced,
            loaded: OnceLock::new(),
            outlines: OnceLock::new(),
            coverage: OnceLock::new(),
        }
    }

    /// The loaded face, loading it on first use. `None` when the face data
    /// cannot be read or parsed by ab_glyph.
    pub(crate) fn loaded(&self, db: &fontdb::Database) -> Option<&LoadedFace> {
        self.loaded
            .get_or_init(|| db.with_face_data(self.db_id, LoadedFace::load).flatten())
            .as_ref()
    }

    /// True when the face has vector outlines (`glyf`, `CFF ` or `CFF2`);
    /// false for bitmap-only faces such as colour emoji fonts, whose glyphs
    /// cannot be rasterised. Reads the table directory in place (no copy);
    /// unreadable data counts as having outlines (loading will then fail).
    pub(crate) fn has_outlines(&self, db: &fontdb::Database) -> bool {
        *self.outlines.get_or_init(|| {
            db.with_face_data(self.db_id, |data, index| {
                FaceTables::parse(data, index).is_none_or(|tables| tables.has_outlines())
            })
            .unwrap_or(true)
        })
    }

    /// The loaded face if it was already loaded (never loads).
    #[inline]
    pub(crate) fn loaded_if_ready(&self) -> Option<&LoadedFace> {
        self.loaded.get().and_then(Option::as_ref)
    }

    /// True when the coverage was already computed (or failed).
    pub(crate) fn coverage_ready(&self) -> bool {
        self.coverage.get().is_some()
    }

    /// The face's character coverage, computed on first use from the raw
    /// font data (without copying it).
    pub(crate) fn coverage(&self, db: &fontdb::Database) -> Option<&FaceCoverage> {
        self.coverage
            .get_or_init(|| {
                db.with_face_data(self.db_id, |data, index| {
                    let tables = FaceTables::parse(data, index)?;
                    let coverage = tables.table(b"cmap").map(Coverage::from_cmap)?;
                    Some(FaceCoverage {
                        coverage,
                        has_outlines: tables.has_outlines(),
                    })
                })
                .flatten()
            })
            .as_ref()
    }

    /// True for italic or oblique faces.
    pub(crate) fn is_slanted(&self) -> bool {
        !matches!(self.style, FaceStyle::Normal)
    }
}

/// Coverage information used by the fallback search.
pub(crate) struct FaceCoverage {
    pub coverage: Coverage,
    /// False for bitmap-only fonts (e.g. colour emoji) that cannot be outlined.
    pub has_outlines: bool,
}

/// A face parsed by ab_glyph, with metrics pre-extracted in font units.
pub(crate) struct LoadedFace {
    pub font: FontArc,
    pub units_per_em: f32,
    /// Ascent above the baseline (font units, positive).
    pub ascent: f32,
    /// Descent below the baseline (font units, positive).
    pub descent: f32,
    pub line_gap: f32,
    /// Height of the `x` glyph outline (font units), when available.
    pub x_height: Option<f32>,
    /// Underline offset below the baseline (positive = below) and thickness,
    /// in font units, from the `post` table when usable.
    pub underline: Option<(f32, f32)>,
    /// Glyph of U+0020 (0 when missing).
    pub space_glyph: u16,
    /// The face has a `kern` table (otherwise kerning lookups are skipped).
    pub has_kern: bool,
}

impl LoadedFace {
    /// Copies `data` and parses face `index` of it.
    fn load(data: &[u8], index: u32) -> Option<Self> {
        let info = sfnt::inspect(data, index);
        let font = FontVec::try_from_vec_and_index(data.to_vec(), index).ok()?;
        Some(Self::from_font(FontArc::from(font), info))
    }

    /// Extracts the metrics of an already parsed font.
    pub(crate) fn from_font(font: FontArc, info: sfnt::SfntInfo) -> Self {
        let units_per_em = font
            .units_per_em()
            .filter(|u| u.is_finite() && *u > 0.0)
            .unwrap_or(1000.0);
        // Buggy fonts store a positive descender (or a negative ascender):
        // only the magnitudes are meaningful.
        let mut ascent = font.ascent_unscaled().abs();
        let mut descent = font.descent_unscaled().abs();
        let line_gap = font.line_gap_unscaled();
        let line_gap = if line_gap.is_finite() {
            line_gap.max(0.0)
        } else {
            0.0
        };
        if !(ascent.is_finite() && descent.is_finite() && ascent + descent > 0.0) {
            ascent = 0.8 * units_per_em;
            descent = 0.2 * units_per_em;
        }

        let x_glyph = font.glyph_id('x');
        let x_height = if x_glyph.0 == 0 {
            None
        } else {
            // ab_glyph outline bounds: `min.y` holds the top (y_max, y-up).
            font.outline(x_glyph)
                .map(|outline| outline.bounds.min.y)
                .filter(|h| h.is_finite() && *h > 0.0)
        };

        let underline = info.underline.and_then(|(position, thickness)| {
            let offset = -f32::from(position);
            let thickness = f32::from(thickness);
            (offset > 0.0 && thickness > 0.0).then_some((offset, thickness))
        });

        let space_glyph = font.glyph_id(' ').0;
        Self {
            font,
            units_per_em,
            ascent,
            descent,
            line_gap,
            x_height,
            underline,
            space_glyph,
            has_kern: info.has_kern,
        }
    }

    /// Horizontal advance of `glyph` as a fraction of the em.
    #[inline]
    pub(crate) fn advance_em(&self, glyph: u16) -> f32 {
        let advance = self.font.h_advance_unscaled(ab_glyph::GlyphId(glyph));
        if advance.is_finite() {
            advance / self.units_per_em
        } else {
            0.0
        }
    }

    /// Kerning between two glyphs as a fraction of the em (0 without a `kern` table).
    #[inline]
    pub(crate) fn kern_em(&self, first: u16, second: u16) -> f32 {
        if !self.has_kern {
            return 0.0;
        }
        let kern = self
            .font
            .kern_unscaled(ab_glyph::GlyphId(first), ab_glyph::GlyphId(second));
        if kern.is_finite() {
            kern / self.units_per_em
        } else {
            0.0
        }
    }

    /// Glyph id of `c` (0 when missing).
    #[inline]
    pub(crate) fn glyph(&self, c: char) -> u16 {
        self.font.glyph_id(c).0
    }
}
