//! CSS Fonts 4 §5.2 face matching within a family (stretch, then style,
//! then weight) and the cache of [`crate::FontSystem::select`] results.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

use crate::face::{FaceEntry, FaceStyle};
use crate::FontId;

/// Lexicographic match quality of a face (lower is better).
pub(crate) type MatchKey = ((u8, u16), u8, (u8, u16));

/// Rank of a face weight for a desired weight (lower is better), following
/// CSS Fonts 4 §5.2:
/// * desired in 400..=500: weights in desired..=500 ascending, then weights
///   below desired descending, then weights above 500 ascending;
/// * desired < 400: weights ≤ desired descending, then heavier ascending;
/// * desired > 500: weights ≥ desired ascending, then lighter descending.
pub(crate) fn weight_rank(desired: u16, actual: u16) -> (u8, u16) {
    if (400..=500).contains(&desired) {
        if actual >= desired && actual <= 500 {
            (0, actual - desired)
        } else if actual < desired {
            (1, desired - actual)
        } else {
            (2, actual - 500)
        }
    } else if desired < 400 {
        if actual <= desired {
            (0, desired - actual)
        } else {
            (1, actual - desired)
        }
    } else if actual >= desired {
        (0, actual - desired)
    } else {
        (1, desired - actual)
    }
}

/// Rank of a face style (lower is better): italic requests prefer italic,
/// then oblique, then normal; normal requests prefer normal, then oblique,
/// then italic.
pub(crate) fn style_rank(italic: bool, style: FaceStyle) -> u8 {
    match (italic, style) {
        (true, FaceStyle::Italic) | (false, FaceStyle::Normal) => 0,
        (_, FaceStyle::Oblique) => 1,
        _ => 2,
    }
}

/// Rank of a face width class for a `normal` stretch request: normal, then
/// narrower (closest first), then wider (closest first).
pub(crate) fn stretch_rank(stretch: u16) -> (u8, u16) {
    const NORMAL: u16 = 5;
    if stretch <= NORMAL {
        (0, NORMAL - stretch)
    } else {
        (1, stretch - NORMAL)
    }
}

/// Full match key of `face` for the requested weight and slant.
pub(crate) fn match_key(face: &FaceEntry, weight: u16, italic: bool) -> MatchKey {
    (
        stretch_rank(face.stretch),
        style_rank(italic, face.style),
        weight_rank(weight, face.weight),
    )
}

/// Orders `ids` (faces of one family, or any set) from best to worst match;
/// ties keep the lowest id first.
pub(crate) fn rank_faces(faces: &[FaceEntry], ids: &mut [FontId], weight: u16, italic: bool) {
    ids.sort_by_key(|id| {
        let key = faces
            .get(id.0 as usize)
            .map(|face| match_key(face, weight, italic))
            .unwrap_or(((u8::MAX, 0), u8::MAX, (u8::MAX, 0)));
        (key, *id)
    });
}

/// Best face among `ids` without loading anything.
pub(crate) fn best_unchecked(
    faces: &[FaceEntry],
    ids: &[FontId],
    weight: u16,
    italic: bool,
) -> Option<FontId> {
    ids.iter()
        .filter_map(|id| {
            let face = faces.get(id.0 as usize)?;
            Some((match_key(face, weight, italic), *id))
        })
        .min()
        .map(|(_, id)| id)
}

/// Maximum number of cached `select` results before the cache is reset.
const MAX_SELECT_ENTRIES: usize = 4096;

/// Family lists larger than this (approximate bytes) are not cached: they
/// would make the cache hold gigabytes, and hashing them on every lookup
/// costs about as much as resolving them (only the first 256 names are
/// examined).
const MAX_CACHED_KEY_BYTES: usize = 4096;

/// Approximate memory taken by a family list.
fn key_bytes(families: &[String]) -> usize {
    families.iter().fold(0usize, |total, family| {
        total
            .saturating_add(family.len())
            .saturating_add(std::mem::size_of::<String>())
    })
}

/// Cache of `select` results keyed by (families, weight, italic). Lookups
/// hash the borrowed key, so hits do not allocate.
pub(crate) struct SelectCache {
    hasher: RandomState,
    map: HashMap<u64, Vec<SelectEntry>>,
    len: usize,
}

struct SelectEntry {
    families: Vec<String>,
    weight: u16,
    italic: bool,
    font: FontId,
}

impl Default for SelectCache {
    fn default() -> Self {
        Self {
            hasher: RandomState::new(),
            map: HashMap::new(),
            len: 0,
        }
    }
}

impl SelectCache {
    fn key_hash(&self, families: &[String], weight: u16, italic: bool) -> u64 {
        let mut hasher = self.hasher.build_hasher();
        families.hash(&mut hasher);
        weight.hash(&mut hasher);
        italic.hash(&mut hasher);
        hasher.finish()
    }

    /// Cached result for the query, if any.
    pub(crate) fn get(&self, families: &[String], weight: u16, italic: bool) -> Option<FontId> {
        if families.len() > MAX_CACHED_KEY_BYTES || key_bytes(families) > MAX_CACHED_KEY_BYTES {
            return None;
        }
        let hash = self.key_hash(families, weight, italic);
        self.map.get(&hash)?.iter().find_map(|entry| {
            (entry.weight == weight && entry.italic == italic && entry.families == families)
                .then_some(entry.font)
        })
    }

    /// Stores a result (the cache is cleared when it grows too large).
    pub(crate) fn insert(&mut self, families: &[String], weight: u16, italic: bool, font: FontId) {
        if families.len() > MAX_CACHED_KEY_BYTES
            || key_bytes(families) > MAX_CACHED_KEY_BYTES
            || self.get(families, weight, italic).is_some()
        {
            return;
        }
        if self.len >= MAX_SELECT_ENTRIES {
            self.map.clear();
            self.len = 0;
        }
        let hash = self.key_hash(families, weight, italic);
        self.map.entry(hash).or_default().push(SelectEntry {
            families: families.to_vec(),
            weight,
            italic,
            font,
        });
        self.len += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Picks the best weight among `available` for `desired`.
    fn pick(desired: u16, available: &[u16]) -> u16 {
        *available
            .iter()
            .min_by_key(|w| weight_rank(desired, **w))
            .unwrap()
    }

    #[test]
    fn weight_400_prefers_400_then_500_then_lighter_then_heavier() {
        assert_eq!(pick(400, &[100, 300, 400, 500, 700]), 400);
        assert_eq!(pick(400, &[100, 300, 500, 700]), 500);
        assert_eq!(pick(400, &[100, 300, 700]), 300);
        assert_eq!(pick(400, &[700, 900]), 700);
    }

    #[test]
    fn weight_500_prefers_500_then_400_then_lighter_then_heavier() {
        assert_eq!(pick(500, &[400, 500, 600]), 500);
        assert_eq!(pick(500, &[300, 400, 600]), 400);
        assert_eq!(pick(500, &[300, 600]), 300);
        assert_eq!(pick(500, &[600, 900]), 600);
    }

    #[test]
    fn light_weights_prefer_lighter_descending() {
        assert_eq!(pick(300, &[100, 200, 400, 700]), 200);
        assert_eq!(pick(300, &[400, 700]), 400);
        assert_eq!(pick(300, &[300, 400]), 300);
        assert_eq!(pick(100, &[400, 700]), 400);
    }

    #[test]
    fn heavy_weights_prefer_heavier_ascending() {
        assert_eq!(pick(700, &[400, 700]), 700);
        assert_eq!(pick(600, &[400, 700, 900]), 700);
        assert_eq!(pick(800, &[400, 700]), 700);
        assert_eq!(pick(900, &[100, 400]), 400);
        assert_eq!(pick(600, &[500, 900]), 900);
    }

    #[test]
    fn style_order() {
        assert!(style_rank(true, FaceStyle::Italic) < style_rank(true, FaceStyle::Oblique));
        assert!(style_rank(true, FaceStyle::Oblique) < style_rank(true, FaceStyle::Normal));
        assert!(style_rank(false, FaceStyle::Normal) < style_rank(false, FaceStyle::Oblique));
        assert!(style_rank(false, FaceStyle::Oblique) < style_rank(false, FaceStyle::Italic));
    }

    #[test]
    fn stretch_order() {
        assert!(stretch_rank(5) < stretch_rank(4));
        assert!(stretch_rank(4) < stretch_rank(3));
        assert!(stretch_rank(1) < stretch_rank(6));
        assert!(stretch_rank(6) < stretch_rank(7));
    }

    #[test]
    fn select_cache_round_trip_and_bound() {
        let mut cache = SelectCache::default();
        let fams = vec!["arial".to_string(), "sans-serif".to_string()];
        assert_eq!(cache.get(&fams, 400, false), None);
        cache.insert(&fams, 400, false, FontId(3));
        assert_eq!(cache.get(&fams, 400, false), Some(FontId(3)));
        assert_eq!(cache.get(&fams, 700, false), None);
        assert_eq!(cache.get(&fams, 400, true), None);
        assert_eq!(cache.get(&fams[..1], 400, false), None);
        for i in 0..(MAX_SELECT_ENTRIES as u16 + 10) {
            cache.insert(&[format!("f{i}")], i, false, FontId(u32::from(i)));
        }
        assert!(cache.len <= MAX_SELECT_ENTRIES);
    }

    #[test]
    fn huge_family_lists_are_not_cached() {
        let mut cache = SelectCache::default();
        let long = vec!["x".repeat(MAX_CACHED_KEY_BYTES + 1)];
        cache.insert(&long, 400, false, FontId(1));
        assert_eq!(cache.get(&long, 400, false), None);
        let many: Vec<String> = (0..10_000).map(|_| String::new()).collect();
        cache.insert(&many, 400, false, FontId(1));
        assert_eq!(cache.get(&many, 400, false), None);
        assert_eq!(cache.len, 0);
        let normal = vec!["arial".to_string(), "sans-serif".to_string()];
        cache.insert(&normal, 400, false, FontId(2));
        assert_eq!(cache.get(&normal, 400, false), Some(FontId(2)));
        assert_eq!(cache.len, 1);
    }

    /// Reference CSS Fonts 4 §5.2 weight search: walk the candidate weights
    /// in the specified order and stop at the first available one.
    fn reference_pick(desired: u16, available: &[u16]) -> u16 {
        let mut sorted = available.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let up = |lo: u16, hi: u16| sorted.iter().copied().filter(move |&w| w >= lo && w <= hi);
        let down = |lo: u16, hi: u16| {
            sorted
                .iter()
                .rev()
                .copied()
                .filter(move |&w| w >= lo && w <= hi)
        };
        let order: Vec<u16> = if (400..=500).contains(&desired) {
            up(desired, 500)
                .chain(down(0, desired - 1))
                .chain(up(501, u16::MAX))
                .collect()
        } else if desired < 400 {
            down(0, desired).chain(up(desired + 1, u16::MAX)).collect()
        } else {
            up(desired, u16::MAX).chain(down(0, desired - 1)).collect()
        };
        order[0]
    }

    #[test]
    fn weight_rank_matches_the_css_algorithm_on_random_sets() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..5_000 {
            let count = 1 + (next() % 6) as usize;
            let available: Vec<u16> = (0..count)
                .map(|_| {
                    if next() % 3 == 0 {
                        (next() % 1001) as u16
                    } else {
                        (1 + next() % 9) as u16 * 100
                    }
                })
                .collect();
            let desired = if next() % 2 == 0 {
                (1 + next() % 1000) as u16
            } else {
                (1 + next() % 9) as u16 * 100
            };
            assert_eq!(
                pick(desired, &available),
                reference_pick(desired, &available),
                "desired {desired} among {available:?}"
            );
        }
    }
}
