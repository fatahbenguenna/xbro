//! Per-character font fallback: finds a face covering a character missing
//! from the requested face.
//!
//! Faces are scanned in [`crate::collection::FontCollection::fallback_order`]
//! order (generic sans-serif/serif/monospace candidates first, then one face
//! per installed family). Coverage is read from each face's `cmap` in place
//! (no copy) and cached per face; at most [`MAX_NEW_COVERAGE_PER_LOOKUP`]
//! coverages are computed per lookup, so a single character never triggers a
//! scan of thousands of fonts. Results (including "no face covers it", once
//! the scan was complete) are cached per character. Once a scan has visited
//! every face, the union of all coverages is built, so characters that no
//! face covers are rejected in O(log n) even when they defeat the
//! per-character cache (pages using hundreds of thousands of distinct
//! characters).

use crate::hash::FastMap;
use crate::matching::best_unchecked;
use crate::sync::{read, write};
use crate::{FontId, FontSystem};

/// Maximum number of face coverages computed during one lookup.
pub(crate) const MAX_NEW_COVERAGE_PER_LOOKUP: usize = 200;

/// Maximum number of cached per-character results before a reset.
const MAX_FALLBACK_ENTRIES: usize = 1 << 16;

/// Cache: character → face covering it (`None` = no face covers it).
pub(crate) type FallbackMap = FastMap<char, Option<FontId>>;

impl FontSystem {
    /// A face covering `c`, adapted to the weight/slant of `primary` when
    /// the covering family has a better matching face that also covers it.
    /// The flag is false when the result must not be cached because the
    /// scan stopped early (budget exhausted) without finding an outline face.
    pub(crate) fn fallback_face(&self, c: char, primary: FontId) -> (Option<FontId>, bool) {
        let cached = read(&self.fallback_cache).get(&c).copied();
        let (base, cacheable) = match cached {
            Some(result) => (result, true),
            None => {
                let (result, complete) = self.scan_fallback(c);
                if complete {
                    let mut map = write(&self.fallback_cache);
                    if map.len() >= MAX_FALLBACK_ENTRIES {
                        map.clear();
                    }
                    map.insert(c, result);
                }
                (result, complete)
            }
        };
        (base.map(|id| self.adapt_style(id, primary, c)), cacheable)
    }

    /// Scans faces for one covering `c`. Returns the face (an outline face
    /// when possible, else a bitmap-only face) and whether the scan was
    /// conclusive.
    fn scan_fallback(&self, c: char) -> (Option<FontId>, bool) {
        let collection = &self.collection;
        if let Some(union) = collection.fallback_union() {
            if !union.contains(c as u32) {
                return (None, true);
            }
        }
        let mut budget = MAX_NEW_COVERAGE_PER_LOOKUP;
        let mut bitmap_only: Option<FontId> = None;
        for &id in collection.fallback_order() {
            let Some(face) = collection.face(id) else {
                continue;
            };
            if !face.coverage_ready() {
                if budget == 0 {
                    return (bitmap_only, false);
                }
                budget -= 1;
            }
            let Some(coverage) = collection.coverage(id) else {
                continue;
            };
            if !coverage.coverage.contains(c as u32) {
                continue;
            }
            if !coverage.has_outlines {
                if bitmap_only.is_none() && self.covers(id, c) {
                    bitmap_only = Some(id);
                }
                continue;
            }
            if self.covers(id, c) {
                return (Some(id), true);
            }
        }
        // Every face was visited, so every coverage is known.
        collection.build_fallback_union();
        (bitmap_only, true)
    }

    /// True when the loaded face `id` maps `c` to a glyph.
    pub(crate) fn covers(&self, id: FontId, c: char) -> bool {
        self.collection
            .loaded(id)
            .is_some_and(|face| face.glyph(c) != 0)
    }

    /// Picks, within the family of `base`, the face best matching the
    /// weight/slant of `primary`, provided it also covers `c`.
    fn adapt_style(&self, base: FontId, primary: FontId, c: char) -> FontId {
        let collection = &self.collection;
        let (Some(base_face), Some(primary_face)) =
            (collection.face(base), collection.face(primary))
        else {
            return base;
        };
        if base_face.weight == primary_face.weight
            && base_face.is_slanted() == primary_face.is_slanted()
        {
            return base;
        }
        let Some(ids) = base_face
            .families_lower
            .first()
            .and_then(|family| collection.family(family))
        else {
            return base;
        };
        match best_unchecked(
            collection.faces(),
            ids,
            primary_face.weight,
            primary_face.is_slanted(),
        ) {
            Some(best) if best != base && self.covers(best, c) => best,
            _ => base,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmap::Coverage;

    const MONO: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";
    const SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

    /// `count` faces of DejaVu Sans Mono, each in its own family.
    fn many_families(count: usize) -> Option<FontSystem> {
        let mut source = fontdb::Database::new();
        source.load_font_data(std::fs::read(MONO).ok()?);
        let template = source.faces().next()?.clone();
        let mut db = fontdb::Database::new();
        for i in 0..count {
            let mut info = template.clone();
            info.families = vec![(
                format!("Family {i}"),
                fontdb::Language::English_UnitedStates,
            )];
            db.push_face_info(info);
        }
        Some(FontSystem::from_database(db, false))
    }

    fn ready(fonts: &FontSystem) -> usize {
        fonts
            .collection
            .faces()
            .iter()
            .filter(|face| face.coverage_ready())
            .count()
    }

    #[test]
    fn scans_are_budgeted_and_resumed() {
        let Some(fonts) = many_families(300) else {
            return;
        };
        assert_eq!(fonts.collection.fallback_order().len(), 300);
        // U+01C4 is in no face: the first lookup stops after 200 coverages
        // and is not cached; the next one finishes the scan.
        let first = fonts.fallback_face('\u{1C4}', FontId(0));
        assert_eq!(first, (None, false));
        assert_eq!(ready(&fonts), MAX_NEW_COVERAGE_PER_LOOKUP);
        assert!(read(&fonts.fallback_cache).get(&'\u{1C4}').is_none());
        assert!(fonts.collection.fallback_union().is_none());
        let second = fonts.fallback_face('\u{1C4}', FontId(0));
        assert_eq!(second, (None, true));
        assert_eq!(ready(&fonts), 300);
        assert_eq!(read(&fonts.fallback_cache).get(&'\u{1C4}'), Some(&None));
        // The scan was complete: the union of coverages now exists.
        let union = fonts.collection.fallback_union().expect("union built");
        assert!(union.contains('A' as u32));
        assert!(!union.contains(0x1C4));
        // Shaping agrees: .notdef of the requested face.
        let run = fonts.shape(FontId(5), 16.0, "a\u{1C4}", 0.0, 0.0);
        assert_eq!(run.glyphs[1].font, FontId(5));
        assert_eq!(run.glyphs[1].glyph_id, 0);
    }

    #[test]
    fn union_rejects_uncovered_characters_without_scanning() {
        let (Ok(mono), Ok(sans)) = (std::fs::read(MONO), std::fs::read(SANS)) else {
            return;
        };
        let fonts = FontSystem::from_fonts(vec![mono, sans]);
        // A complete scan (nothing covers U+10FFFD) builds the union.
        assert_eq!(fonts.fallback_face('\u{10FFFD}', FontId(0)), (None, true));
        let union = fonts.collection.fallback_union().expect("union built");
        let expected = Coverage::union(
            fonts
                .collection
                .fallback_order()
                .iter()
                .filter_map(|&id| fonts.collection.coverage(id))
                .map(|face| &face.coverage),
        );
        assert_eq!(union, &expected);
        assert!(union.contains(0x1C4), "DejaVu Sans has U+01C4");
        // Covered and uncovered characters still resolve correctly.
        assert_eq!(fonts.scan_fallback('\u{1C4}'), (Some(FontId(1)), true));
        assert_eq!(fonts.scan_fallback('\u{F0000}'), (None, true));
        let run = fonts.shape(FontId(0), 16.0, "\u{1C4}\u{F0001}", 0.0, 0.0);
        assert_eq!(run.glyphs[0].font, FontId(1));
        assert_eq!((run.glyphs[1].font, run.glyphs[1].glyph_id), (FontId(0), 0));
    }
}
