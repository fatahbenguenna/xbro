//! The set of faces of a [`crate::FontSystem`]: the fontdb database, the
//! dense face table, the family index and family resolution (CSS matching).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::cmap::Coverage;
use crate::face::{FaceEntry, LoadedFace};
use crate::families::{
    is_generic_keyword, normalize, normalize_quoted, split_family_list, substitution, Generic,
    Substitution,
};
use crate::matching::{best_unchecked, rank_faces};
use crate::FontId;

/// Maximum number of family names examined per query (pathological lists).
const MAX_FAMILY_NAMES: usize = 256;

/// Database + faces indexed by [`FontId`].
pub(crate) struct FontCollection {
    db: fontdb::Database,
    faces: Vec<FaceEntry>,
    /// Lowercased family name (every name of every face) → faces.
    family_index: HashMap<String, Vec<FontId>>,
    /// Family preferred by the system configuration (fontconfig) for each
    /// generic family, lowercased; tried after the built-in candidates.
    configured: [String; 3],
    /// Faces scanned, in order, by the per-character fallback search.
    fallback_order: Vec<FontId>,
    /// Union of the coverages of every face of `fallback_order`, built once
    /// all of them are known (lets the fallback search reject characters no
    /// face covers without scanning).
    fallback_union: OnceLock<Coverage>,
}

impl FontCollection {
    /// Indexes the faces of `db`. With `sort_by_path`, faces backed by files
    /// are ordered by (path, face index) so ids are deterministic regardless
    /// of directory iteration order; in-memory fonts keep insertion order.
    pub(crate) fn new(db: fontdb::Database, sort_by_path: bool) -> Self {
        let mut infos: Vec<&fontdb::FaceInfo> = db.faces().collect();
        if sort_by_path {
            infos.sort_by(|a, b| {
                source_path(&a.source)
                    .cmp(&source_path(&b.source))
                    .then(a.index.cmp(&b.index))
            });
        }
        // FontId keeps 30 bits for the face index (the others select
        // synthetic styles): ignore faces beyond that (never happens in
        // practice).
        let faces: Vec<FaceEntry> = infos
            .into_iter()
            .take(crate::synthesis::MAX_FACES)
            .map(FaceEntry::new)
            .collect();

        let mut family_index: HashMap<String, Vec<FontId>> = HashMap::new();
        for (i, face) in faces.iter().enumerate() {
            let id = FontId(i as u32);
            for name in &face.families_lower {
                let ids = family_index.entry(name.clone()).or_default();
                // Faces are visited in id order, so a face listing the same
                // name twice (several languages) can only repeat the last
                // entry: checking it keeps indexing linear.
                if ids.last() != Some(&id) {
                    ids.push(id);
                }
            }
        }

        let configured = [
            normalize(db.family_name(&fontdb::Family::SansSerif)),
            normalize(db.family_name(&fontdb::Family::Serif)),
            normalize(db.family_name(&fontdb::Family::Monospace)),
        ];

        let mut collection = Self {
            db,
            faces,
            family_index,
            configured,
            fallback_order: Vec::new(),
            fallback_union: OnceLock::new(),
        };
        collection.fallback_order = collection.build_fallback_order();
        collection
    }

    /// True when the collection has no face at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// Number of faces.
    pub(crate) fn len(&self) -> usize {
        self.faces.len()
    }

    /// The face table.
    pub(crate) fn faces(&self) -> &[FaceEntry] {
        &self.faces
    }

    /// The face `id`, if it exists (the face of a synthesised id).
    #[inline]
    pub(crate) fn face(&self, id: FontId) -> Option<&FaceEntry> {
        self.faces.get(id.face().0 as usize)
    }

    /// The loaded face `id` (loading it on first use; the face of a
    /// synthesised id).
    #[inline]
    pub(crate) fn loaded(&self, id: FontId) -> Option<&LoadedFace> {
        self.face(id)?.loaded(&self.db)
    }

    /// The face's coverage (computed on first use).
    pub(crate) fn coverage(&self, id: FontId) -> Option<&crate::face::FaceCoverage> {
        self.face(id)?.coverage(&self.db)
    }

    /// Faces of a family (normalised name), if installed.
    pub(crate) fn family(&self, name: &str) -> Option<&[FontId]> {
        self.family_index.get(name).map(Vec::as_slice)
    }

    /// Faces scanned by the fallback search, in order.
    pub(crate) fn fallback_order(&self) -> &[FontId] {
        &self.fallback_order
    }

    /// The union of the fallback faces' coverages, if already built.
    pub(crate) fn fallback_union(&self) -> Option<&Coverage> {
        self.fallback_union.get()
    }

    /// Builds (once) the union of the fallback faces' coverages. Meant to be
    /// called once every coverage is known (after a complete fallback
    /// scan); faces whose coverage cannot be read contribute nothing, as in
    /// the scan itself.
    pub(crate) fn build_fallback_union(&self) -> &Coverage {
        self.fallback_union.get_or_init(|| {
            Coverage::union(
                self.fallback_order
                    .iter()
                    .filter_map(|&id| self.coverage(id))
                    .map(|face| &face.coverage),
            )
        })
    }

    /// True when face `id` can be used: it loads and, unless `allow_bitmap`,
    /// it has vector outlines (bitmap-only faces such as colour emoji fonts
    /// would render nothing).
    fn usable(&self, id: FontId, allow_bitmap: bool) -> bool {
        let Some(face) = self.face(id) else {
            return false;
        };
        (allow_bitmap || face.has_outlines(&self.db)) && face.loaded(&self.db).is_some()
    }

    /// The best usable face among `ids` for the requested style (see
    /// [`Self::usable`]).
    fn best_usable(
        &self,
        ids: &[FontId],
        weight: u16,
        italic: bool,
        allow_bitmap: bool,
    ) -> Option<FontId> {
        let mut ranked = ids.to_vec();
        rank_faces(&self.faces, &mut ranked, weight, italic);
        ranked.into_iter().find(|&id| self.usable(id, allow_bitmap))
    }

    /// The best loadable face with outlines among `ids` for the requested style.
    pub(crate) fn best_loadable(
        &self,
        ids: &[FontId],
        weight: u16,
        italic: bool,
    ) -> Option<FontId> {
        self.best_usable(ids, weight, italic, false)
    }

    /// The best loadable face (with outlines) of the family `name`
    /// (normalised), if installed.
    pub(crate) fn best_in_family(&self, name: &str, weight: u16, italic: bool) -> Option<FontId> {
        let ids = self.family_index.get(name)?;
        self.best_loadable(ids, weight, italic)
    }

    /// CSS font matching for a family list (see [`crate::FontSystem::select`]).
    /// Never fails when the collection is not empty.
    ///
    /// Each entry may itself be a comma-separated list; commas inside
    /// quotes do not split. Quoted generic keywords (`"serif"`) name
    /// ordinary families. Families whose faces are all bitmap-only are
    /// skipped; such faces are only used as the very last resort.
    pub(crate) fn resolve(&self, families: &[String], weight: u16, italic: bool) -> FontId {
        let names = families
            .iter()
            .flat_map(|family| split_family_list(family))
            .take(MAX_FAMILY_NAMES);
        for raw in names {
            let (name, quoted) = normalize_quoted(raw);
            if name.is_empty() {
                continue;
            }
            if let Some(id) = self.best_in_family(&name, weight, italic) {
                return id;
            }
            if quoted && is_generic_keyword(&name) {
                continue;
            }
            if let Some(sub) = substitution(&name) {
                if let Some(id) = self.resolve_substitution(sub, weight, italic) {
                    return id;
                }
            }
        }
        if let Some(id) = self.resolve_generic(Generic::SansSerif, weight, italic) {
            return id;
        }
        let all: Vec<FontId> = (0..self.faces.len()).map(|i| FontId(i as u32)).collect();
        self.best_usable(&all, weight, italic, false)
            .or_else(|| self.best_usable(&all, weight, italic, true))
            .unwrap_or(FontId(0))
    }

    fn resolve_substitution(&self, sub: Substitution, weight: u16, italic: bool) -> Option<FontId> {
        sub.preferred
            .iter()
            .find_map(|name| self.best_in_family(name, weight, italic))
            .or_else(|| self.resolve_generic(sub.generic, weight, italic))
    }

    /// First installed candidate of a generic family.
    pub(crate) fn resolve_generic(
        &self,
        generic: Generic,
        weight: u16,
        italic: bool,
    ) -> Option<FontId> {
        if let Some(id) = generic
            .candidates()
            .iter()
            .find_map(|name| self.best_in_family(name, weight, italic))
        {
            return Some(id);
        }
        if let Some(id) = self.best_in_family(self.configured_name(generic), weight, italic) {
            return Some(id);
        }
        if generic == Generic::Monospace {
            let mono: Vec<FontId> = self
                .faces
                .iter()
                .enumerate()
                .filter(|(_, face)| face.monospaced)
                .map(|(i, _)| FontId(i as u32))
                .collect();
            return self.best_loadable(&mono, weight, italic);
        }
        None
    }

    fn configured_name(&self, generic: Generic) -> &str {
        match generic {
            Generic::SansSerif => &self.configured[0],
            Generic::Serif => &self.configured[1],
            Generic::Monospace => &self.configured[2],
        }
    }

    /// Fallback scan order: the regular face of every installed generic
    /// candidate family (sans-serif, serif, monospace), then one
    /// representative (regular) face per remaining family, in id order,
    /// last-resort families (GNU Unifont, Last Resort) at the very end.
    ///
    /// Families are grouped by each face's *first* name, so a face whose
    /// family merely shares a localised name with another family is still
    /// represented.
    fn build_fallback_order(&self) -> Vec<FontId> {
        let mut order = Vec::new();
        let mut seen: HashSet<FontId> = HashSet::new();
        let mut push = |id: FontId, order: &mut Vec<FontId>| {
            if seen.insert(id) {
                order.push(id);
            }
        };
        for generic in Generic::ALL {
            let configured = self.configured_name(generic);
            for name in generic
                .candidates()
                .iter()
                .copied()
                .chain(std::iter::once(configured))
            {
                if let Some(ids) = self.family_index.get(name) {
                    if let Some(best) = best_unchecked(&self.faces, ids, 400, false) {
                        push(best, &mut order);
                    }
                }
            }
        }
        // Faces grouped by first family name, groups in order of first appearance.
        let mut groups: HashMap<&str, Vec<FontId>> = HashMap::new();
        let mut group_order: Vec<&str> = Vec::new();
        for (i, face) in self.faces.iter().enumerate() {
            let Some(family) = face.families_lower.first() else {
                continue;
            };
            groups
                .entry(family.as_str())
                .or_insert_with(|| {
                    group_order.push(family.as_str());
                    Vec::new()
                })
                .push(FontId(i as u32));
        }
        // One representative per family; last-resort fonts (huge coverage,
        // poor quality) go after every other family.
        let mut last_resort = Vec::new();
        for family in group_order {
            let Some(best) = groups
                .get(family)
                .and_then(|ids| best_unchecked(&self.faces, ids, 400, false))
            else {
                continue;
            };
            if is_last_resort_family(family) {
                last_resort.push(best);
            } else {
                push(best, &mut order);
            }
        }
        for id in last_resort {
            push(id, &mut order);
        }
        order
    }
}

/// Families meant as a last resort (pixel or placeholder glyphs for most of
/// Unicode), scanned after all other fonts by the fallback search.
fn is_last_resort_family(family: &str) -> bool {
    family.contains("unifont") || family.contains("last resort")
}

/// Path of a font source (`None` for in-memory data).
fn source_path(source: &fontdb::Source) -> Option<&std::path::Path> {
    match source {
        fontdb::Source::Binary(_) => None,
        fontdb::Source::File(path) => Some(path),
        fontdb::Source::SharedFile(path, _) => Some(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
    const MONO: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";

    /// A database whose only face is DejaVu Sans with an extra, localised
    /// family name.
    fn aliased_db() -> Option<fontdb::Database> {
        let data = std::fs::read(SANS).ok()?;
        let mut source = fontdb::Database::new();
        source.load_font_data(data);
        let mut info = source.faces().next()?.clone();
        info.families.push((
            "Sans Alias Française".to_string(),
            fontdb::Language::French_France,
        ));
        info.id = fontdb::ID::dummy();
        let mut db = fontdb::Database::new();
        db.push_face_info(info);
        Some(db)
    }

    #[test]
    fn every_family_name_is_matched() {
        let Some(db) = aliased_db() else { return };
        let collection = FontCollection::new(db, false);
        assert_eq!(collection.len(), 1);
        let query = |name: &str| collection.resolve(&[name.to_string()], 400, false);
        assert_eq!(query("dejavu sans"), FontId(0));
        assert_eq!(query("sans alias française"), FontId(0));
        assert_eq!(query("SANS ALIAS FRANÇAISE"), FontId(0));
        assert_eq!(
            collection.family("sans alias française"),
            Some(&[FontId(0)][..])
        );
        assert!(collection.loaded(FontId(0)).is_some());
    }

    #[test]
    fn empty_collection() {
        let collection = FontCollection::new(fontdb::Database::new(), true);
        assert!(collection.is_empty());
        assert!(collection.fallback_order().is_empty());
        assert_eq!(
            collection.resolve(&["serif".to_string()], 700, true),
            FontId(0)
        );
        assert!(collection.loaded(FontId(0)).is_none());
        assert!(collection.coverage(FontId(0)).is_none());
    }

    #[test]
    fn fallback_order_starts_with_generic_candidates() {
        let (Ok(mono), Ok(sans)) = (std::fs::read(MONO), std::fs::read(SANS)) else {
            return;
        };
        let mut db = fontdb::Database::new();
        db.load_font_data(mono);
        db.load_font_data(sans);
        let collection = FontCollection::new(db, false);
        // DejaVu Sans (sans-serif candidate) comes before DejaVu Sans Mono.
        assert_eq!(collection.fallback_order(), &[FontId(1), FontId(0)]);
        let coverage = collection.coverage(FontId(1)).expect("coverage");
        assert!(coverage.has_outlines);
        assert!(coverage.coverage.contains('A' as u32));
        assert!(coverage.coverage.contains(0x1C4));
        let mono_coverage = collection.coverage(FontId(0)).expect("coverage");
        assert!(!mono_coverage.coverage.contains(0x1C4));
    }

    /// The first face of `path` with its family names replaced.
    fn renamed(path: &str, names: &[&str]) -> Option<fontdb::FaceInfo> {
        let mut source = fontdb::Database::new();
        source.load_font_data(std::fs::read(path).ok()?);
        let mut info = source.faces().next()?.clone();
        info.families = names
            .iter()
            .map(|name| (name.to_string(), fontdb::Language::English_UnitedStates))
            .collect();
        Some(info)
    }

    #[test]
    fn fallback_order_represents_every_first_family() {
        // Face 0 (Mono) also carries the name "DejaVu Sans", which is the
        // only name of face 1 (Sans). Face 1 must still be scanned: it is
        // the only one covering U+01C4.
        let (Some(mono), Some(sans)) = (
            renamed(MONO, &["DejaVu Sans Mono", "DejaVu Sans"]),
            renamed(SANS, &["DejaVu Sans"]),
        ) else {
            return;
        };
        let mut db = fontdb::Database::new();
        db.push_face_info(mono);
        db.push_face_info(sans);
        let fonts = crate::FontSystem::from_database(db, false);
        let order = fonts.collection.fallback_order();
        assert!(order.contains(&FontId(0)) && order.contains(&FontId(1)));
        let run = fonts.shape(FontId(0), 16.0, "\u{1C4}", 0.0, 0.0);
        assert_eq!(run.glyphs[0].font, FontId(1));
        assert_ne!(run.glyphs[0].glyph_id, 0);
    }

    #[test]
    fn repeated_names_are_indexed_once() {
        let Some(info) = renamed(SANS, &["Dup", "DUP", "dup ", "Other"]) else {
            return;
        };
        let mut db = fontdb::Database::new();
        db.push_face_info(info.clone());
        db.push_face_info(info);
        let collection = FontCollection::new(db, false);
        assert_eq!(collection.family("dup"), Some(&[FontId(0), FontId(1)][..]));
        assert_eq!(
            collection.family("other"),
            Some(&[FontId(0), FontId(1)][..])
        );
    }

    #[test]
    fn bitmap_only_faces_are_a_last_resort() {
        const EMOJI: &str = "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf";
        let (Ok(emoji), Ok(sans)) = (std::fs::read(EMOJI), std::fs::read(SANS)) else {
            return;
        };
        let mut db = fontdb::Database::new();
        db.load_font_data(emoji.clone());
        db.load_font_data(sans);
        let collection = FontCollection::new(db, false);
        assert!(!collection.faces()[0].has_outlines(&collection.db));
        assert!(collection.faces()[1].has_outlines(&collection.db));
        let query = |names: &[&str]| {
            let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
            collection.resolve(&names, 400, false)
        };
        assert_eq!(query(&["noto color emoji"]), FontId(1));
        assert_eq!(query(&["noto color emoji", "serif"]), FontId(1));
        // Not even loaded (no copy of the 10 MB file) to be rejected.
        assert!(collection.faces()[0].loaded_if_ready().is_none());

        let mut db = fontdb::Database::new();
        db.load_font_data(emoji);
        let only = FontCollection::new(db, false);
        assert_eq!(
            only.resolve(&["monospace".to_string()], 700, true),
            FontId(0)
        );
    }

    #[test]
    fn last_resort_families() {
        assert!(is_last_resort_family("unifont"));
        assert!(is_last_resort_family("unifont upper"));
        assert!(is_last_resort_family("last resort"));
        assert!(!is_last_resort_family("dejavu sans"));
    }
}
