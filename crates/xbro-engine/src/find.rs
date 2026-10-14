//! Find in page: case-insensitive search over the laid-out text.
//!
//! Text fragments are grouped per source node (a text node, or the element
//! for generated text) in document order; the fragments of one node are
//! searched as a single string, so a match may span line breaks. Each
//! match yields one rectangle per fragment it touches, computed from the
//! glyph positions (`ShapedGlyph::cluster` gives each glyph's byte offset in
//! the fragment text).
//!
//! Generated text is attributed to its element, whose index precedes its
//! children's: a run of an element's fragments that follows text of one of
//! its descendants is its `::after` content and is ordered after the
//! element's subtree (a `::before` run keeps the element's position), so
//! results stay in document order and `::before`/`::after` texts are not
//! searched as one string.
//!
//! Text in a `position: fixed` subtree is laid out in viewport coordinates;
//! its matches are flagged ([`FindMatch::fixed`]) so that the UI can place
//! them on screen whatever the scroll offset.

use xbro_dom::{Document, NodeId};
use xbro_layout::{LayoutTree, Point, Rect, TextFragment};

use crate::boxes;

/// One find-in-page match (a match spanning several fragments gives one
/// [`FindMatch`] per fragment it touches).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FindMatch {
    /// Rect of the matched text: in document coordinates, or in viewport
    /// coordinates (relative to the top left of the visible area) when
    /// [`FindMatch::fixed`] is set.
    pub rect: Rect,
    /// The text is in a `position: fixed` subtree: it stays at the same
    /// place on screen whatever the scroll offset (and is always in view
    /// when it is inside the viewport).
    pub fixed: bool,
}

impl FindMatch {
    /// The rect in document coordinates while the page is scrolled to `scroll`.
    pub fn document_rect(&self, scroll: Point) -> Rect {
        if self.fixed {
            self.rect.translate(scroll.x, scroll.y)
        } else {
            self.rect
        }
    }

    /// The rect relative to the top left of the viewport while the page is
    /// scrolled to `scroll`.
    pub fn viewport_rect(&self, scroll: Point) -> Rect {
        if self.fixed {
            self.rect
        } else {
            self.rect.translate(-scroll.x, -scroll.y)
        }
    }
}

/// A text fragment with whether it is in a `position: fixed` subtree.
type Frag<'a> = (&'a TextFragment, bool);

/// Upper bound on the number of reported rectangles.
const MAX_RESULTS: usize = 10_000;

/// U+00AD, ignored by the search.
const SOFT_HYPHEN: char = '\u{ad}';

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

/// Searches one group of fragments (same source node) and appends matches.
fn search_group(group: &[Frag<'_>], needle: &Needle, out: &mut Vec<FindMatch>) {
    let mut folded: Vec<char> = Vec::new();
    let mut origin: Vec<usize> = Vec::new(); // folded index → source char index
    let mut chars: Vec<SourceChar> = Vec::new();
    for (fi, &(f, _)) in group.iter().enumerate() {
        if fi > 0 {
            let prev = group[fi - 1].0;
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
            if c == SOFT_HYPHEN {
                // Invisible unless the line breaks there: "hyphen\u{ad}ation"
                // matches "hyphenation" (the match rect still covers it).
                continue;
            }
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
        let flush = |cur: Option<(usize, usize, usize)>, out: &mut Vec<FindMatch>| {
            if let Some((fi, s, e)) = cur {
                if let Some(&(f, fixed)) = group.get(fi) {
                    if let Some((x0, x1)) = glyph_span(f, s, e) {
                        out.push(FindMatch {
                            rect: Rect::new(f.rect.x + x0, f.rect.y, x1 - x0, f.rect.height),
                            fixed,
                        });
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

/// A run of consecutive fragments (in layout order) from one source node.
struct Run<'a> {
    node: NodeId,
    /// Document-order sort key: `(position, rank)`, where text nodes and
    /// `::before` runs sit at their node index (rank 1 and 0) and `::after`
    /// runs just before the first node following the element's subtree.
    key: (usize, u8),
    fragments: Vec<Frag<'a>>,
}

/// Index of the first node after the subtree of `node` in tree order
/// (`usize::MAX` when the subtree ends the document). Node indices follow
/// tree order for parsed documents.
fn subtree_end(doc: &Document, node: NodeId) -> usize {
    std::iter::once(node)
        .chain(doc.ancestors(node))
        .find_map(|n| doc.next_sibling(n))
        .map_or(usize::MAX, NodeId::index)
}

/// Groups the fragments of `tree` per source node, in document order.
fn runs<'a>(tree: &'a LayoutTree, doc: &Document) -> Vec<Run<'a>> {
    let mut runs: Vec<Run<'a>> = Vec::new();
    for (f, fixed) in boxes::text_fragments_fixed(tree) {
        if let Some(last) = runs.last_mut().filter(|r| r.node == f.node) {
            last.fragments.push((f, fixed));
            continue;
        }
        let in_document = f.node.index() < doc.len();
        let key = if !in_document || doc.node(f.node).is_text() {
            (f.node.index(), 1)
        } else {
            // Generated text (or a control's value, alt text…) of an element:
            // `::after` content when it follows text of a descendant.
            let after = runs.last().is_some_and(|prev| {
                prev.node.index() < doc.len() && doc.ancestors(prev.node).any(|a| a == f.node)
            });
            if after {
                (subtree_end(doc, f.node), 0)
            } else {
                (f.node.index(), 0)
            }
        };
        runs.push(Run {
            node: f.node,
            key,
            fragments: vec![(f, fixed)],
        });
    }
    // Stable: runs with equal keys keep their layout order.
    runs.sort_by_key(|r| r.key);
    // Fragments of one text node separated in layout order become adjacent.
    let mut merged: Vec<Run<'a>> = Vec::with_capacity(runs.len());
    for run in runs {
        match merged.last_mut() {
            Some(last) if last.node == run.node && last.key == run.key => {
                last.fragments.extend(run.fragments)
            }
            _ => merged.push(run),
        }
    }
    merged
}

/// Case-insensitive search of `query` in the text of `tree` (see the module docs).
pub(crate) fn find(tree: &LayoutTree, doc: &Document, query: &str) -> Vec<FindMatch> {
    let needle: Vec<char> = query
        .chars()
        .filter(|&c| c != SOFT_HYPHEN)
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
    for run in runs(tree, doc) {
        search_group(&run.fragments, &needle, &mut out);
        if out.len() >= MAX_RESULTS {
            out.truncate(MAX_RESULTS);
            break;
        }
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
