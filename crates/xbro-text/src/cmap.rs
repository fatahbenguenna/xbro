//! Character coverage of a face, computed from its raw `cmap` table.
//!
//! Used by the per-character fallback search: computing the coverage reads
//! the font data in place (memory-mapped by fontdb) instead of copying and
//! fully loading every candidate face. The coverage mirrors the Unicode
//! subtables ab_glyph uses (platform 0; platform 3 encoding 1; platform 3
//! encoding 10 with formats 12/13). Candidates are still verified against the
//! loaded face before use, so a slightly inexact coverage on malformed fonts
//! only costs time, never correctness.

use crate::sfnt::{read_u16, read_u32};

/// Upper bound on the number of code points examined one by one in a
/// single `cmap` (protects against overlapping/malformed format 4 segments).
const ITERATION_BUDGET: usize = 1 << 18;

/// Upper bound on the number of groups read from the format 12/13
/// subtables of one `cmap` (all subtables together).
const MAX_GROUPS: usize = 1 << 17;

/// Upper bound on the number of Unicode subtables parsed in one `cmap`
/// (real fonts have a handful; a forged table may list 65,535).
const MAX_SUBTABLES: usize = 32;

/// Highest Unicode scalar value.
const MAX_CHAR: u32 = 0x10_FFFF;

/// Sorted, non-overlapping, non-adjacent inclusive code point ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Coverage {
    ranges: Vec<(u32, u32)>,
}

impl Coverage {
    /// Builds the coverage of a raw `cmap` table.
    pub(crate) fn from_cmap(cmap: &[u8]) -> Self {
        let mut builder = Builder::default();
        let num_tables = read_u16(cmap, 2).map_or(0, usize::from);
        // At most MAX_SUBTABLES entries: the linear `contains` stays cheap.
        let mut seen_offsets: Vec<usize> = Vec::new();
        for i in 0..num_tables {
            if seen_offsets.len() >= MAX_SUBTABLES {
                break;
            }
            let record = 4 + i * 8;
            let (Some(platform), Some(encoding), Some(offset)) = (
                read_u16(cmap, record),
                read_u16(cmap, record + 2),
                read_u32(cmap, record + 4),
            ) else {
                break;
            };
            let offset = offset as usize;
            if seen_offsets.contains(&offset) {
                continue;
            }
            let Some(subtable) = cmap.get(offset..) else {
                continue;
            };
            let Some(format) = read_u16(subtable, 0) else {
                continue;
            };
            let unicode = match (platform, encoding) {
                (0, _) => true,
                (3, 1) => true,
                (3, 10) => matches!(format, 12 | 13),
                _ => false,
            };
            if !unicode {
                continue;
            }
            seen_offsets.push(offset);
            match format {
                0 => parse_format0(subtable, &mut builder),
                4 => parse_format4(subtable, &mut builder),
                6 => parse_format6(subtable, &mut builder),
                10 => parse_format10(subtable, &mut builder),
                12 => parse_format12_13(subtable, false, &mut builder),
                13 => parse_format12_13(subtable, true, &mut builder),
                _ => {}
            }
        }
        builder.finish()
    }

    /// The union of several coverages.
    pub(crate) fn union<'a>(coverages: impl IntoIterator<Item = &'a Coverage>) -> Self {
        let mut builder = Builder::default();
        for coverage in coverages {
            builder.ranges.extend_from_slice(&coverage.ranges);
        }
        builder.finish()
    }

    /// Builds a coverage from arbitrary inclusive ranges (normalised).
    #[cfg(test)]
    pub(crate) fn from_ranges(ranges: &[(u32, u32)]) -> Self {
        let mut builder = Builder::default();
        for &(start, end) in ranges {
            builder.push_range(start, end);
        }
        builder.finish()
    }

    /// True when code point `c` is mapped to a non-zero glyph.
    #[inline]
    pub(crate) fn contains(&self, c: u32) -> bool {
        let idx = self.ranges.partition_point(|&(_, end)| end < c);
        self.ranges
            .get(idx)
            .is_some_and(|&(start, end)| start <= c && c <= end)
    }

    /// True when no code point is covered.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// The normalised ranges.
    #[cfg(test)]
    pub(crate) fn ranges(&self) -> &[(u32, u32)] {
        &self.ranges
    }
}

/// Accumulates ranges, coalescing consecutive pushes.
#[derive(Default)]
struct Builder {
    ranges: Vec<(u32, u32)>,
    budget_used: usize,
    groups_used: usize,
}

impl Builder {
    /// Consumes one unit of the per-cmap group budget; false when exhausted.
    #[inline]
    fn spend_group(&mut self) -> bool {
        if self.groups_used >= MAX_GROUPS {
            return false;
        }
        self.groups_used += 1;
        true
    }

    /// Consumes one unit of the per-cmap iteration budget; false when exhausted.
    #[inline]
    fn spend(&mut self) -> bool {
        if self.budget_used >= ITERATION_BUDGET {
            return false;
        }
        self.budget_used += 1;
        true
    }

    fn push_range(&mut self, start: u32, end: u32) {
        if start > end || start > MAX_CHAR {
            return;
        }
        let end = end.min(MAX_CHAR);
        if let Some(last) = self.ranges.last_mut() {
            if start >= last.0 && start <= last.1.saturating_add(1) {
                last.1 = last.1.max(end);
                return;
            }
        }
        self.ranges.push((start, end));
    }

    #[inline]
    fn push(&mut self, c: u32) {
        self.push_range(c, c);
    }

    fn finish(mut self) -> Coverage {
        self.ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(self.ranges.len());
        for (start, end) in self.ranges {
            match merged.last_mut() {
                Some(last) if start <= last.1.saturating_add(1) => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        merged.shrink_to_fit();
        Coverage { ranges: merged }
    }
}

/// Format 0: byte encoding table (256 glyph ids).
fn parse_format0(data: &[u8], builder: &mut Builder) {
    for c in 0..256usize {
        match data.get(6 + c) {
            Some(&glyph) if glyph != 0 => builder.push(c as u32),
            Some(_) => {}
            None => break,
        }
    }
}

/// Format 4: segment mapping to delta values (BMP).
fn parse_format4(data: &[u8], builder: &mut Builder) {
    let Some(seg_count_x2) = read_u16(data, 6) else {
        return;
    };
    let seg_count = usize::from(seg_count_x2 / 2);
    let end_codes = 14;
    let start_codes = end_codes + seg_count * 2 + 2;
    let id_deltas = start_codes + seg_count * 2;
    let id_range_offsets = id_deltas + seg_count * 2;
    for i in 0..seg_count {
        let (Some(end), Some(start), Some(delta), Some(range_offset)) = (
            read_u16(data, end_codes + i * 2),
            read_u16(data, start_codes + i * 2),
            read_u16(data, id_deltas + i * 2),
            read_u16(data, id_range_offsets + i * 2),
        ) else {
            return;
        };
        if start > end || (start == 0xFFFF && end == 0xFFFF) {
            continue;
        }
        let (start, end) = (u32::from(start), u32::from(end));
        if range_offset == 0 {
            // glyph = (c + delta) mod 65536: zero for exactly one code point.
            let zero_at = (0x1_0000 - u32::from(delta)) & 0xFFFF;
            if zero_at >= start && zero_at <= end {
                if zero_at > start {
                    builder.push_range(start, zero_at - 1);
                }
                if zero_at < end {
                    builder.push_range(zero_at + 1, end);
                }
            } else {
                builder.push_range(start, end);
            }
        } else if range_offset != 0xFFFF {
            let base = id_range_offsets + i * 2 + usize::from(range_offset);
            for c in start..=end {
                if !builder.spend() {
                    return;
                }
                let pos = base + 2 * (c - start) as usize;
                match read_u16(data, pos) {
                    Some(0) => {}
                    Some(glyph) => {
                        if glyph.wrapping_add(delta) != 0 {
                            builder.push(c);
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

/// Format 6: trimmed table mapping (BMP).
fn parse_format6(data: &[u8], builder: &mut Builder) {
    let (Some(first), Some(count)) = (read_u16(data, 6), read_u16(data, 8)) else {
        return;
    };
    for i in 0..u32::from(count) {
        if !builder.spend() {
            return;
        }
        match read_u16(data, 10 + 2 * i as usize) {
            Some(0) => {}
            Some(_) => builder.push(u32::from(first) + i),
            None => return,
        }
    }
}

/// Format 10: trimmed array (32-bit code points).
fn parse_format10(data: &[u8], builder: &mut Builder) {
    let (Some(first), Some(count)) = (read_u32(data, 12), read_u32(data, 16)) else {
        return;
    };
    for i in 0..count {
        if !builder.spend() {
            return;
        }
        match read_u16(data, 20usize.saturating_add(2 * i as usize)) {
            Some(0) => {}
            Some(_) => match first.checked_add(i) {
                Some(c) => builder.push(c),
                None => return,
            },
            None => return,
        }
    }
}

/// Formats 12 (segmented coverage) and 13 (many-to-one range mappings).
fn parse_format12_13(data: &[u8], many_to_one: bool, builder: &mut Builder) {
    let Some(num_groups) = read_u32(data, 12) else {
        return;
    };
    let num_groups = (num_groups as usize).min(MAX_GROUPS);
    for g in 0..num_groups {
        if !builder.spend_group() {
            return;
        }
        let base = 16 + g * 12;
        let (Some(start), Some(end), Some(glyph)) = (
            read_u32(data, base),
            read_u32(data, base + 4),
            read_u32(data, base + 8),
        ) else {
            return;
        };
        if start > end || start > MAX_CHAR {
            continue;
        }
        if many_to_one {
            if glyph != 0 && glyph <= 0xFFFF {
                builder.push_range(start, end);
            }
            continue;
        }
        // glyph id for c = glyph + (c - start); it must be in 1..=65535.
        if glyph > 0xFFFF {
            continue;
        }
        let lo = if glyph == 0 {
            start.saturating_add(1)
        } else {
            start
        };
        let hi = end.min(start.saturating_add(0xFFFF - glyph));
        builder.push_range(lo, hi);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmap_with(records: &[(u16, u16)], subtable: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&(records.len() as u16).to_be_bytes());
        let offset = 4 + 8 * records.len() as u32;
        for &(platform, encoding) in records {
            out.extend_from_slice(&platform.to_be_bytes());
            out.extend_from_slice(&encoding.to_be_bytes());
            out.extend_from_slice(&offset.to_be_bytes());
        }
        out.extend_from_slice(subtable);
        out
    }

    /// Format 4 subtable from (start, end, delta, glyph array or none).
    fn format4(segments: &[(u16, u16, i16, Option<Vec<u16>>)]) -> Vec<u8> {
        let seg_count = segments.len() + 1;
        let mut ends = Vec::new();
        let mut starts = Vec::new();
        let mut deltas = Vec::new();
        let mut offsets = Vec::new();
        let mut glyph_array: Vec<u16> = Vec::new();
        for (i, (start, end, delta, glyphs)) in segments.iter().enumerate() {
            ends.push(*end);
            starts.push(*start);
            deltas.push(*delta as u16);
            match glyphs {
                None => offsets.push(0u16),
                Some(g) => {
                    // Offset from this idRangeOffset entry to the glyph array slot.
                    let remaining = (seg_count - i) as u16;
                    offsets.push((remaining + glyph_array.len() as u16) * 2);
                    glyph_array.extend_from_slice(g);
                }
            }
        }
        ends.push(0xFFFF);
        starts.push(0xFFFF);
        deltas.push(1);
        offsets.push(0);
        let mut out = Vec::new();
        for v in [4u16, 0, 0, (seg_count * 2) as u16, 0, 0, 0] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        let push = |out: &mut Vec<u8>, values: &[u16]| {
            for v in values {
                out.extend_from_slice(&v.to_be_bytes());
            }
        };
        push(&mut out, &ends);
        push(&mut out, &[0]);
        push(&mut out, &starts);
        push(&mut out, &deltas);
        push(&mut out, &offsets);
        push(&mut out, &glyph_array);
        out
    }

    fn format12(groups: &[(u32, u32, u32)], format: u16) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&format.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&((16 + 12 * groups.len()) as u32).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(groups.len() as u32).to_be_bytes());
        for &(s, e, g) in groups {
            out.extend_from_slice(&s.to_be_bytes());
            out.extend_from_slice(&e.to_be_bytes());
            out.extend_from_slice(&g.to_be_bytes());
        }
        out
    }

    #[test]
    fn format4_delta_segments() {
        // 'A'..='Z' -> glyphs 1..=26; delta = 1 - 65.
        let sub = format4(&[(0x41, 0x5A, 1 - 0x41, None)]);
        let cov = Coverage::from_cmap(&cmap_with(&[(3, 1)], &sub));
        assert!(cov.contains('A' as u32));
        assert!(cov.contains('Z' as u32));
        assert!(!cov.contains('a' as u32));
        assert!(!cov.contains(0xFFFF));
        assert_eq!(cov.ranges(), &[(0x41, 0x5A)]);
    }

    #[test]
    fn format4_delta_zero_point_is_excluded() {
        // delta = -0x42 maps 'B' to glyph 0.
        let sub = format4(&[(0x41, 0x43, -0x42, None)]);
        let cov = Coverage::from_cmap(&cmap_with(&[(0, 3)], &sub));
        assert!(cov.contains(0x41));
        assert!(!cov.contains(0x42));
        assert!(cov.contains(0x43));
    }

    #[test]
    fn format4_glyph_array() {
        let sub = format4(&[(0x30, 0x33, 0, Some(vec![5, 0, 7, 8]))]);
        let cov = Coverage::from_cmap(&cmap_with(&[(3, 1)], &sub));
        assert!(cov.contains(0x30));
        assert!(!cov.contains(0x31));
        assert!(cov.contains(0x32));
        assert!(cov.contains(0x33));
        assert!(!cov.contains(0x34));
    }

    #[test]
    fn format12_and_13_groups() {
        let sub = format12(&[(0x1F600, 0x1F64F, 10), (0x20, 0x22, 0)], 12);
        let cov = Coverage::from_cmap(&cmap_with(&[(3, 10)], &sub));
        assert!(cov.contains(0x1F600));
        assert!(cov.contains(0x1F64F));
        assert!(!cov.contains(0x1F650));
        assert!(!cov.contains(0x20), "maps to glyph 0");
        assert!(cov.contains(0x21));

        let sub = format12(&[(0x100, 0x1FF, 3), (0x300, 0x3FF, 0)], 13);
        let cov = Coverage::from_cmap(&cmap_with(&[(0, 6)], &sub));
        assert!(cov.contains(0x150));
        assert!(!cov.contains(0x350));
    }

    #[test]
    fn non_unicode_subtables_are_ignored() {
        let sub = format4(&[(0x41, 0x5A, 1 - 0x41, None)]);
        // Windows symbol encoding and Macintosh platform.
        let cov = Coverage::from_cmap(&cmap_with(&[(3, 0), (1, 0)], &sub));
        assert!(cov.is_empty());
        // Windows full repertoire requires format 12/13.
        let cov = Coverage::from_cmap(&cmap_with(&[(3, 10)], &sub));
        assert!(cov.is_empty());
    }

    #[test]
    fn ranges_are_merged() {
        let cov = Coverage::from_ranges(&[(10, 20), (5, 9), (30, 40), (15, 35), (100, 100)]);
        assert_eq!(cov.ranges(), &[(5, 40), (100, 100)]);
        assert!(cov.contains(5) && cov.contains(40) && cov.contains(100));
        assert!(!cov.contains(4) && !cov.contains(41) && !cov.contains(101));
        assert!(Coverage::default().is_empty());
        assert!(!Coverage::default().contains(0));
    }

    /// xorshift64 for property tests.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Reference format 4 lookup (as ttf-parser does it): glyph id of `c`.
    fn format4_reference(segments: &[(u16, u16, i16, Option<Vec<u16>>)], c: u32) -> u16 {
        for (start, end, delta, glyphs) in segments {
            let (start, end) = (u32::from(*start), u32::from(*end));
            if c < start || c > end {
                continue;
            }
            let delta = *delta as u16;
            return match glyphs {
                None => (c as u16).wrapping_add(delta),
                Some(array) => match array[(c - start) as usize] {
                    0 => 0,
                    glyph => glyph.wrapping_add(delta),
                },
            };
        }
        0
    }

    #[test]
    fn format4_matches_the_reference_on_random_tables() {
        let mut rng = Rng(0x0123_4567_89AB_CDEF);
        for _ in 0..40 {
            // Sorted, non-overlapping segments below U+FFFF.
            let mut segments = Vec::new();
            let mut next_start = rng.below(300) as u32;
            while segments.len() < 12 && next_start < 0xF000 {
                let start = next_start;
                let end = start + rng.below(200) as u32;
                let delta = rng.next() as i16;
                let glyphs = (rng.below(2) == 0).then(|| {
                    (start..=end)
                        .map(|_| {
                            if rng.below(4) == 0 {
                                0
                            } else {
                                rng.next() as u16
                            }
                        })
                        .collect::<Vec<u16>>()
                });
                segments.push((start as u16, end as u16, delta, glyphs));
                next_start = end + 1 + rng.below(500) as u32;
            }
            let coverage = Coverage::from_cmap(&cmap_with(&[(3, 1)], &format4(&segments)));
            for c in 0..0xFFFFu32 {
                assert_eq!(
                    coverage.contains(c),
                    format4_reference(&segments, c) != 0,
                    "U+{c:04X} in {segments:?}"
                );
            }
        }
    }

    #[test]
    fn format12_matches_the_reference_on_random_tables() {
        let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
        for format in [12u16, 13] {
            for _ in 0..40 {
                let mut groups = Vec::new();
                let mut next_start = rng.below(0x1000) as u32;
                while groups.len() < 10 && next_start < 0x10_0000 {
                    let start = next_start;
                    let end = start + rng.below(0x3000) as u32;
                    let glyph = match rng.below(4) {
                        0 => 0,
                        1 => 0xFFFF - rng.below(0x100) as u32,
                        2 => 0x1_0000 + rng.below(10) as u32,
                        _ => rng.below(0x8000) as u32,
                    };
                    groups.push((start, end, glyph));
                    next_start = end + 1 + rng.below(0x4000) as u32;
                }
                let coverage =
                    Coverage::from_cmap(&cmap_with(&[(3, 10)], &format12(&groups, format)));
                let reference = |c: u32| {
                    groups.iter().any(|&(start, end, glyph)| {
                        let id = if format == 13 {
                            u64::from(glyph)
                        } else {
                            u64::from(glyph) + u64::from(c.wrapping_sub(start))
                        };
                        (start..=end).contains(&c) && (1..=0xFFFF).contains(&id)
                    })
                };
                for _ in 0..4000 {
                    let c = rng.below(0x11_0000) as u32;
                    assert_eq!(
                        coverage.contains(c),
                        reference(c),
                        "U+{c:04X} in {groups:?}"
                    );
                }
                for &(start, end, _) in &groups {
                    for c in [start, end, start.saturating_sub(1), end + 1] {
                        assert_eq!(
                            coverage.contains(c),
                            reference(c),
                            "U+{c:04X} in {groups:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn thousands_of_subtables_are_bounded() {
        // 65,535 Unicode records at distinct offsets into a region of 0x0006
        // words: each parses as a format 6 subtable mapping U+0006..=U+000B.
        let records = 65_535usize;
        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&(records as u16).to_be_bytes());
        let region = 4 + 8 * records;
        for i in 0..records {
            cmap.extend_from_slice(&3u16.to_be_bytes());
            cmap.extend_from_slice(&1u16.to_be_bytes());
            cmap.extend_from_slice(&((region + 2 * i) as u32).to_be_bytes());
        }
        for _ in 0..records + 64 {
            cmap.extend_from_slice(&6u16.to_be_bytes());
        }
        let start = std::time::Instant::now();
        let coverage = Coverage::from_cmap(&cmap);
        let elapsed = start.elapsed();
        assert_eq!(coverage.ranges(), &[(6, 11)]);
        assert!(elapsed.as_millis() < 100, "took {elapsed:?}");
    }

    #[test]
    fn group_budget_is_shared_by_all_subtables() {
        // Two format 12 subtables of MAX_GROUPS groups each: only the first
        // one's groups fit in the budget.
        let groups: Vec<(u32, u32, u32)> =
            (0..MAX_GROUPS as u32).map(|i| (i * 2, i * 2, 1)).collect();
        let first = format12(&groups, 12);
        let shifted: Vec<(u32, u32, u32)> =
            groups.iter().map(|&(s, e, g)| (s + 1, e + 1, g)).collect();
        let second = format12(&shifted, 12);
        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&2u16.to_be_bytes());
        let first_at = 4 + 16u32;
        let second_at = first_at + first.len() as u32;
        for (encoding, offset) in [(10u16, first_at), (4u16, second_at)] {
            cmap.extend_from_slice(&(if encoding == 4 { 0u16 } else { 3u16 }).to_be_bytes());
            cmap.extend_from_slice(&encoding.to_be_bytes());
            cmap.extend_from_slice(&offset.to_be_bytes());
        }
        cmap.extend_from_slice(&first);
        cmap.extend_from_slice(&second);
        let coverage = Coverage::from_cmap(&cmap);
        assert!(coverage.contains(0) && coverage.contains(2));
        assert!(!coverage.contains(1), "second subtable is past the budget");
    }

    #[test]
    fn union_merges_coverages() {
        let a = Coverage::from_ranges(&[(10, 20), (40, 50)]);
        let b = Coverage::from_ranges(&[(21, 30), (45, 60), (100, 100)]);
        let union = Coverage::union([&a, &b, &Coverage::default()]);
        assert_eq!(union.ranges(), &[(10, 30), (40, 60), (100, 100)]);
        assert!(Coverage::union(std::iter::empty()).is_empty());
    }

    #[test]
    fn malformed_cmaps_never_panic() {
        // Truncations of valid tables.
        let sub = format4(&[(0x30, 0x39, 0, Some(vec![1; 10])), (0x41, 0x5A, 3, None)]);
        let full = cmap_with(&[(3, 1)], &sub);
        for len in 0..full.len() {
            let _ = Coverage::from_cmap(&full[..len]);
        }
        let sub = format12(&[(0, u32::MAX, 5), (u32::MAX, 0, 1)], 12);
        let full = cmap_with(&[(3, 10)], &sub);
        for len in 0..full.len() {
            let _ = Coverage::from_cmap(&full[..len]);
        }
        let cov = Coverage::from_cmap(&full);
        assert!(cov.contains(0x41));
        // Huge, overlapping glyph-array segments are bounded by the budget.
        let mut segments = Vec::new();
        for _ in 0..200 {
            segments.push((0u16, 0xFFFEu16, 0i16, Some(vec![1u16; 4])));
        }
        let cov = Coverage::from_cmap(&cmap_with(&[(0, 3)], &format4(&segments)));
        assert!(cov.contains(0));
        // Garbage.
        let _ = Coverage::from_cmap(&[0xFF; 300]);
        let _ = Coverage::from_cmap(&[]);
    }
}
