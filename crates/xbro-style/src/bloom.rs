//! Counting Bloom filter of the tags, ids and classes of the ancestors of
//! the element being styled. Selectors whose ancestor compounds require a
//! tag/id/class absent from the filter cannot match and are skipped before
//! running the matcher (a standard browser-engine optimization).

use xbro_dom::ElementData;

const BITS: usize = 12;
const SIZE: usize = 1 << BITS;
const MASK: u32 = (SIZE as u32) - 1;

/// Hash kinds, mixed into the hash so `#a`, `.a` and `a` differ.
pub(crate) const KIND_TAG: u8 = 1;
pub(crate) const KIND_ID: u8 = 2;
pub(crate) const KIND_CLASS: u8 = 3;

/// FNV-1a of `s` (ASCII-lowercased for tags) salted with `kind`.
pub(crate) fn hash(kind: u8, s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5 ^ u32::from(kind);
    h = h.wrapping_mul(0x0100_0193);
    for b in s.bytes() {
        let b = if kind == KIND_TAG {
            b.to_ascii_lowercase()
        } else {
            b
        };
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    // Final avalanche so both halves used below are well mixed.
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h
}

pub(crate) struct AncestorFilter {
    counts: Vec<u16>,
}

impl AncestorFilter {
    pub fn new() -> Self {
        AncestorFilter {
            counts: vec![0; SIZE],
        }
    }

    fn slots(h: u32) -> (usize, usize) {
        ((h & MASK) as usize, ((h >> BITS) & MASK) as usize)
    }

    /// Increments or decrements both slots of `h`. A saturated counter is
    /// sticky: it is never decremented again (its true count is unknown),
    /// so saturation can only cause false positives, never false negatives
    /// (an element with tens of thousands of repeated class names would
    /// otherwise wrap the count back to zero while ancestors still hold it).
    fn add(&mut self, h: u32, delta: i32) {
        let (a, b) = Self::slots(h);
        for i in [a, b] {
            if let Some(c) = self.counts.get_mut(i) {
                if *c == u16::MAX {
                    continue;
                }
                *c = if delta > 0 {
                    c.saturating_add(1)
                } else {
                    c.saturating_sub(1)
                };
            }
        }
    }

    fn for_each_hash(e: &ElementData, mut f: impl FnMut(u32)) {
        f(hash(KIND_TAG, &e.name));
        if let Some(id) = e.id() {
            f(hash(KIND_ID, id));
        }
        for c in e.classes() {
            f(hash(KIND_CLASS, c));
        }
    }

    /// Adds an element entering the ancestor chain.
    pub fn push(&mut self, e: &ElementData) {
        Self::for_each_hash(e, |h| self.add(h, 1));
    }

    /// Removes an element previously pushed.
    pub fn pop(&mut self, e: &ElementData) {
        Self::for_each_hash(e, |h| self.add(h, -1));
    }

    /// False only when `h` is certainly not in the filter.
    pub fn may_contain(&self, h: u32) -> bool {
        let (a, b) = Self::slots(h);
        self.counts.get(a).is_some_and(|c| *c > 0) && self.counts.get(b).is_some_and(|c| *c > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_dom::Attribute;

    #[test]
    fn push_pop() {
        let mut f = AncestorFilter::new();
        let e = ElementData::new(
            "div",
            vec![Attribute::new("id", "main"), Attribute::new("class", "a b")],
        );
        assert!(!f.may_contain(hash(KIND_CLASS, "a")));
        f.push(&e);
        f.push(&e);
        assert!(f.may_contain(hash(KIND_TAG, "DIV")));
        assert!(f.may_contain(hash(KIND_ID, "main")));
        assert!(f.may_contain(hash(KIND_CLASS, "b")));
        f.pop(&e);
        assert!(f.may_contain(hash(KIND_CLASS, "b")));
        f.pop(&e);
        assert!(!f.may_contain(hash(KIND_CLASS, "b")));
        assert_ne!(hash(KIND_ID, "a"), hash(KIND_CLASS, "a"));
    }

    #[test]
    fn saturation_never_causes_false_negatives() {
        let mut f = AncestorFilter::new();
        let outer = ElementData::new("div", vec![Attribute::new("class", "x")]);
        let repeated = "x ".repeat(70_000);
        let inner = ElementData::new("div", vec![Attribute::new("class", repeated)]);
        f.push(&outer);
        f.push(&inner);
        f.pop(&inner);
        // `outer` is still an ancestor: its class must still be reported.
        assert!(f.may_contain(hash(KIND_CLASS, "x")));
        assert!(f.may_contain(hash(KIND_TAG, "div")));
    }
}
