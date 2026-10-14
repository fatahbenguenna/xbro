//! Find in page: case-insensitive search over the laid-out text.
//!
//! Text fragments are grouped per source node (a text node, or the element
//! for generated text) in document order; the fragments of one node are
//! searched as a single string, so a match may span line breaks. Each
//! match yields one rectangle per fragment it touches, computed from the
//! glyph positions (`ShapedGlyph::cluster` gives each glyph's byte offset in
//! the fragment text).

use xbro_layout::{LayoutTree, Rect, TextFragment};

/// Upper bound on the number of reported rectangles.
const MAX_RESULTS: usize = 10_000;

/// One character of the searchable text.
struct SourceChar {
    /// Index of the fragment in the group, `None` for a virtual separator.
    fragment: Option<usize>,
    /// Byte range of the character in the fragment text.
    start: usize,
    end: usize,
}

/// Horizontal extent (relative to the fragment's x) of the glyphs whose
/// cluster lies in `start..end`.
fn glyph_span(fragment: &TextFragment, start: usize, end: usize) -> Option<(f32, f32)> {
    let mut span: Option<(f32, f32)> = None;
    for g in &fragment.glyphs {
        if g.cluster >= start && g.cluster < end {
            let (a, b) = (g.x, g.x + g.advance.max(0.0));
            span = Some(match span {
                Some((x0, x1)) => (x0.min(a), x1.max(b)),
                None => (a, b),
            });
        }
    }
    span.filter(|(a, b)| a.is_finite() && b.is_finite())
}

/// A folded query with its Knuth–Morris–Pratt failure table, so searching
/// is linear in the text length whatever the query.
struct Needle {
    chars: Vec<char>,
    /// `failure[i]`: length of the longest proper border of `chars[..=i]`.
    failure: Vec<usize>,
}

impl Needle {
    fn new(chars: Vec<char>) -> Self {
        let mut failure = vec![0; chars.len()];
        let mut k = 0;
        for i in 1..chars.len() {
            while k > 0 && chars[i] != chars[k] {
                k = failure[k - 1];
            }
            if chars[i] == chars[k] {
                k += 1;
            }
            failure[i] = k;
        }
        Self { chars, failure }
    }

    /// Start indices of the non-overlapping occurrences in `text`.
    fn find_all(&self, text: &[char], mut on_match: impl FnMut(usize) -> bool) {
        let m = self.chars.len();
        if m == 0 {
            return;
        }
        let mut k = 0;
        for (i, &c) in text.iter().enumerate() {
            while k > 0 && self.chars.get(k) != Some(&c) {
                k = self.failure.get(k - 1).copied().unwrap_or(0);
            }
            if self.chars.get(k) == Some(&c) {
                k += 1;
            }
            if k == m {
                if !on_match(i + 1 - m) {
                    return;
                }
                k = 0; // Non-overlapping matches.
            }
        }
    }
}

/// Searches one group of fragments (same source node) and appends rects.
fn search_group(group: &[&TextFragment], needle: &Needle, out: &mut Vec<Rect>) {
    let mut folded: Vec<char> = Vec::new();
    let mut origin: Vec<usize> = Vec::new(); // folded index → source char index
    let mut chars: Vec<SourceChar> = Vec::new();
    for (fi, f) in group.iter().enumerate() {
        if fi > 0 {
            let prev = group[fi - 1];
            let joined =
                prev.text.ends_with(char::is_whitespace) || f.text.starts_with(char::is_whitespace);
            // A line break swallowed the space between two words.
            if !joined && (prev.baseline - f.baseline).abs() > 0.5 {
                chars.push(SourceChar {
                    fragment: None,
                    start: 0,
                    end: 0,
                });
                folded.push(' ');
                origin.push(chars.len() - 1);
            }
        }
        for (start, c) in f.text.char_indices() {
            chars.push(SourceChar {
                fragment: Some(fi),
                start,
                end: start + c.len_utf8(),
            });
            let idx = chars.len() - 1;
            if c.is_whitespace() {
                folded.push(' ');
                origin.push(idx);
            } else {
                for l in c.to_lowercase() {
                    folded.push(l);
                    origin.push(idx);
                }
            }
        }
    }
    let m = needle.chars.len();
    if m == 0 || folded.len() < m {
        return;
    }
    needle.find_all(&folded, |i| {
        if out.len() >= MAX_RESULTS {
            return false;
        }
        let (Some(&first), Some(&last)) = (origin.get(i), origin.get(i + m - 1)) else {
            return false;
        };
        // One rect per fragment touched by source chars first..=last.
        let mut current: Option<(usize, usize, usize)> = None; // (fragment, start, end)
        let flush = |cur: Option<(usize, usize, usize)>, out: &mut Vec<Rect>| {
            if let Some((fi, s, e)) = cur {
                if let Some(f) = group.get(fi) {
                    if let Some((x0, x1)) = glyph_span(f, s, e) {
                        out.push(Rect::new(f.rect.x + x0, f.rect.y, x1 - x0, f.rect.height));
                    }
                }
            }
        };
        for sc in chars.get(first..=last).unwrap_or(&[]) {
            let Some(fi) = sc.fragment else { continue };
            current = match current {
                Some((cf, s, _)) if cf == fi => Some((cf, s, sc.end)),
                other => {
                    flush(other, out);
                    Some((fi, sc.start, sc.end))
                }
            };
        }
        flush(current, out);
        true
    });
}

/// Case-insensitive search of `query` in the text of `tree` (see the module docs).
pub(crate) fn find(tree: &LayoutTree, query: &str) -> Vec<Rect> {
    let needle: Vec<char> = query
        .chars()
        .flat_map(|c| {
            let lower: Vec<char> = if c.is_whitespace() {
                vec![' ']
            } else {
                c.to_lowercase().collect()
            };
            lower
        })
        .collect();
    let mut out = Vec::new();
    if needle.iter().all(|c| *c == ' ') {
        return out;
    }
    let needle = Needle::new(needle);
    let mut fragments = tree.text_fragments();
    // Document order: node ids follow tree order for parsed documents; the
    // stable sort keeps the layout order of the fragments of one node.
    fragments.sort_by_key(|f| f.node);
    let mut start = 0;
    while start < fragments.len() {
        let node = fragments[start].node;
        let end = fragments[start..]
            .iter()
            .position(|f| f.node != node)
            .map_or(fragments.len(), |p| start + p);
        search_group(&fragments[start..end], &needle, &mut out);
        if out.len() >= MAX_RESULTS {
            out.truncate(MAX_RESULTS);
            break;
        }
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(needle: &str, text: &str) -> Vec<usize> {
        let n = Needle::new(needle.chars().collect());
        let t: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        n.find_all(&t, |i| {
            out.push(i);
            true
        });
        out
    }

    #[test]
    fn kmp_search() {
        assert_eq!(starts("ab", "xabyabab"), vec![1, 4, 6]);
        assert_eq!(starts("aa", "aaaaa"), vec![0, 2]);
        assert_eq!(starts("aab", "aaab"), vec![1]);
        assert_eq!(starts("abcabd", "abcabcabd"), vec![3]);
        assert!(starts("x", "").is_empty());
        assert!(starts("", "abc").is_empty());
        // Linear on pathological input.
        let text = "a".repeat(200_000);
        let needle = format!("{}b", "a".repeat(5_000));
        assert!(starts(&needle, &text).is_empty());
    }
}
