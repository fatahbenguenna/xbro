//! Per-pass layout state: inputs, font and text-measurement caches, and the
//! bookkeeping of absolutely positioned boxes waiting for their containing
//! block to be laid out.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use xbro_dom::Document;
use xbro_style::ComputedStyle;
use xbro_text::{FontId, FontMetrics, FontQuery, FontSystem};

use crate::box_tree::BoxNode;
use crate::geom::{Point, Size};
use crate::sizing::sane_size;
use crate::{LayoutBox, ReplacedSizes};

/// Largest font size used by layout, in px.
const MAX_FONT_SIZE: f32 = 10_000.0;

/// Words longer than this (in bytes) are measured but not cached.
const MAX_CACHED_WORD: usize = 64;

/// A small, fast, non-cryptographic hasher (FxHash) for the per-pass caches,
/// whose keys are not attacker-controlled in a way that matters for a single
/// layout pass (the caches are dropped at the end of the pass).
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher {
    hash: u64,
}

const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            let mut b = [0u8; 8];
            b.copy_from_slice(c);
            self.add(u64::from_le_bytes(b));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut b = [0u8; 8];
            b[..rest.len()].copy_from_slice(rest);
            // Tag the tail with its length so zero padding cannot collide.
            self.add(u64::from_le_bytes(b) ^ ((rest.len() as u64) << 61));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// `HashMap` using [`FxHasher`].
pub(crate) type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// Fonts and vertical metrics resolved for a style.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct FontInfo {
    pub id: FontId,
    /// Sanitised font size in px.
    pub size: f32,
    pub metrics: FontMetrics,
    /// Used line height.
    pub line_height: f32,
    /// Ascent plus half-leading (`A'` of CSS 2.1 §10.8.1).
    pub above: f32,
    /// Descent plus half-leading (`D'`).
    pub below: f32,
}

impl FontInfo {
    /// Sanitised ascent.
    pub fn ascent(&self) -> f32 {
        sane_size(self.metrics.ascent)
    }

    /// Sanitised descent.
    pub fn descent(&self) -> f32 {
        sane_size(self.metrics.descent)
    }
}

/// Which containing block an absolutely positioned box waits for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AbsCb {
    /// A positioned ancestor currently being laid out.
    Frame(u32),
    /// The initial containing block (no positioned ancestor).
    Icb,
    /// `position: fixed`: the viewport.
    Viewport,
}

/// An absolutely positioned box whose layout is deferred until the size of
/// its containing block is known.
#[derive(Debug)]
pub(crate) struct PendingAbs<'t> {
    pub b: &'t BoxNode,
    /// Static position: top-left of the margin box had the box been in flow.
    pub static_pos: Point,
    pub cb: AbsCb,
    /// Registration sequence number (see [`LayoutCx::pending_mark`]).
    pub seq: u64,
}

/// Moves the static positions of absolutely positioned boxes taken out of
/// the pending list (see [`LayoutCx::layout_independent_deferring`]).
pub(crate) fn translate_deferred(boxes: &mut [PendingAbs<'_>], dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    for p in boxes {
        p.static_pos.x += dx;
        p.static_pos.y += dy;
    }
}

/// Font lookup key: family list identity + weight + italic.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct FontKey {
    families: usize,
    weight: u16,
    italic: bool,
}

/// Word-width cache key: font, size, letter-spacing and word-spacing bits.
type WordKey = (FontId, u32, u32, u32);

/// Measurement cache key: box identity (address of its [`BoxNode`], stable
/// for the pass), content width bits and containing block height bits
/// ([`NO_HEIGHT`] when the box does not depend on it or it is indefinite).
pub(crate) type MeasureKey = (usize, u32, u32);

/// [`MeasureKey`] height component meaning "irrelevant or indefinite".
pub(crate) const NO_HEIGHT: u32 = u32::MAX;

/// Result of laying out a flex/grid item with an auto height.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Measured {
    /// Content-box height.
    pub height: f32,
    /// First baseline, relative to the top of the border box as it was laid
    /// out (a table's captions push its border box further down).
    pub baseline: Option<f32>,
}

/// Layout state for one pass.
pub(crate) struct LayoutCx<'t> {
    pub doc: &'t Document,
    pub fonts: &'t FontSystem,
    pub replaced: &'t dyn ReplacedSizes,
    pub viewport: Size,
    font_ids: FxHashMap<FontKey, FontId>,
    /// Keeps the family lists used as `FontKey`s alive (their addresses are keys).
    font_family_owners: Vec<Arc<Vec<String>>>,
    metrics: FxHashMap<(FontId, u32), FontMetrics>,
    words: FxHashMap<WordKey, FxHashMap<Box<str>, f32>>,
    /// Absolutely positioned boxes waiting for their containing block.
    pub pending_abs: Vec<PendingAbs<'t>>,
    next_seq: u64,
    /// Positioned boxes being laid out: frame id and the sequence number
    /// of the first box that may register with it.
    frames: Vec<(u32, u64)>,
    next_frame: u32,
    /// Auto-height measurements of flex/grid items (see `flex::item`), so
    /// that nested containers measure each item once per width.
    pub measure_cache: FxHashMap<MeasureKey, Measured>,
}

impl<'t> LayoutCx<'t> {
    pub fn new(
        doc: &'t Document,
        fonts: &'t FontSystem,
        replaced: &'t dyn ReplacedSizes,
        viewport: Size,
    ) -> Self {
        LayoutCx {
            doc,
            fonts,
            replaced,
            viewport: Size::new(sane_size(viewport.width), sane_size(viewport.height)),
            font_ids: FxHashMap::default(),
            font_family_owners: Vec::new(),
            metrics: FxHashMap::default(),
            words: FxHashMap::default(),
            pending_abs: Vec::new(),
            next_seq: 0,
            frames: Vec::new(),
            next_frame: 0,
            measure_cache: FxHashMap::default(),
        }
    }

    /// Font id, size, metrics and line-height values for `style`.
    pub fn font(&mut self, style: &ComputedStyle) -> FontInfo {
        let key = FontKey {
            families: Arc::as_ptr(&style.font_family) as usize,
            weight: style.font_weight,
            italic: style.is_italic(),
        };
        let id = match self.font_ids.get(&key) {
            Some(&id) => id,
            None => {
                let id = self.fonts.select(&FontQuery {
                    families: &style.font_family,
                    weight: style.font_weight,
                    italic: style.is_italic(),
                });
                self.font_family_owners.push(style.font_family.clone());
                self.font_ids.insert(key, id);
                id
            }
        };
        let size = font_size(style);
        let fonts = self.fonts;
        let metrics = *self
            .metrics
            .entry((id, size.to_bits()))
            .or_insert_with(|| fonts.metrics(id, size));
        let ascent = sane_size(metrics.ascent);
        let descent = sane_size(metrics.descent);
        let line_height = sane_size(style.line_height_px(sane_size(metrics.normal_line_height())));
        let half_leading = (line_height - (ascent + descent)) / 2.0;
        FontInfo {
            id,
            size,
            metrics,
            line_height,
            above: ascent + half_leading,
            below: descent + half_leading,
        }
    }

    /// Advance width of `text` (cached per font, size and spacing).
    pub fn measure(
        &mut self,
        font: FontId,
        size: f32,
        text: &str,
        letter_spacing: f32,
        word_spacing: f32,
    ) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let key = (
            font,
            size.to_bits(),
            letter_spacing.to_bits(),
            word_spacing.to_bits(),
        );
        let map = self.words.entry(key).or_default();
        if let Some(&w) = map.get(text) {
            return w;
        }
        let w = sane_size(
            self.fonts
                .measure(font, size, text, letter_spacing, word_spacing),
        );
        // Very long strings are rarely repeated: don't keep them.
        if text.len() <= MAX_CACHED_WORD {
            map.insert(text.into(), w);
        }
        w
    }

    /// Starts the layout of a positioned box: absolutely positioned
    /// descendants registered from now on wait for it.
    pub fn push_frame(&mut self) -> u32 {
        let id = self.next_frame;
        self.next_frame = self.next_frame.wrapping_add(1);
        self.frames.push((id, self.next_seq));
        id
    }

    /// Ends a positioned box's layout and returns the boxes waiting for it.
    pub fn pop_frame(&mut self, id: u32) -> Vec<PendingAbs<'t>> {
        let Some(pos) = self.frames.iter().rposition(|&(f, _)| f == id) else {
            return self.take_pending(|cb| cb == AbsCb::Frame(id));
        };
        let first_seq = self.frames.get(pos).map_or(0, |&(_, s)| s);
        self.frames.truncate(pos);
        // Boxes waiting for this frame registered after it was pushed:
        // only the tail of the (seq-sorted) pending list is examined, so
        // many positioned boxes stay linear however many boxes wait for
        // outer containing blocks.
        let lo = self.pending_abs.partition_point(|p| p.seq < first_seq);
        let target = AbsCb::Frame(id);
        let tail = self.pending_abs.get(lo..).unwrap_or(&[]);
        if !tail.iter().any(|p| p.cb == target) {
            return Vec::new();
        }
        let rest: Vec<PendingAbs<'t>> = self.pending_abs.drain(lo..).collect();
        let (taken, kept): (Vec<_>, Vec<_>) = rest.into_iter().partition(|p| p.cb == target);
        self.pending_abs.extend(kept);
        taken
    }

    /// Registers an absolutely positioned box at its static position.
    pub fn register_abs(&mut self, b: &'t BoxNode, static_pos: Point) {
        let cb = if b.style.position == xbro_style::Position::Fixed {
            AbsCb::Viewport
        } else {
            self.frames
                .last()
                .map_or(AbsCb::Icb, |&(f, _)| AbsCb::Frame(f))
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        self.pending_abs.push(PendingAbs {
            b,
            static_pos,
            cb,
            seq,
        });
    }

    /// Removes and returns the pending boxes whose containing block matches.
    pub fn take_pending(&mut self, matches: impl Fn(AbsCb) -> bool) -> Vec<PendingAbs<'t>> {
        if !self.pending_abs.iter().any(|p| matches(p.cb)) {
            return Vec::new();
        }
        let (taken, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_abs)
            .into_iter()
            .partition(|p| matches(p.cb));
        self.pending_abs = kept;
        taken
    }

    /// Mark identifying the boxes registered from now on, to pass to
    /// [`translate_pending`](Self::translate_pending) /
    /// [`translate_pending_range`](Self::translate_pending_range) when an
    /// already laid-out subtree is moved.
    pub fn pending_mark(&self) -> u64 {
        self.next_seq
    }

    /// Moves the static positions of the boxes registered since `mark`.
    pub fn translate_pending(&mut self, mark: u64, dx: f32, dy: f32) {
        self.translate_pending_range(mark, u64::MAX, dx, dy);
    }

    /// Moves the static positions of the boxes registered between the marks
    /// `start` (inclusive) and `end` (exclusive).
    pub fn translate_pending_range(&mut self, start: u64, end: u64, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        // `pending_abs` is sorted by `seq` (appended in order, removals keep
        // the order): binary-search the range.
        let lo = self.pending_abs.partition_point(|p| p.seq < start);
        let hi = self.pending_abs.partition_point(|p| p.seq < end);
        if let Some(range) = self.pending_abs.get_mut(lo..hi.max(lo)) {
            for p in range {
                p.static_pos.x += dx;
                p.static_pos.y += dy;
            }
        }
    }

    /// Forgets the boxes registered between the marks `start` (inclusive)
    /// and `end` (exclusive): used when a subtree laid out for measurement
    /// only is thrown away.
    pub fn discard_pending_range(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let lo = self.pending_abs.partition_point(|p| p.seq < start);
        let hi = self.pending_abs.partition_point(|p| p.seq < end);
        if lo < hi {
            self.pending_abs.drain(lo..hi);
        }
    }

    /// Translates a laid-out box and the static positions registered since `mark`.
    pub fn move_box(&mut self, b: &mut LayoutBox, mark: u64, dx: f32, dy: f32) {
        crate::tree::translate_box(b, dx, dy);
        self.translate_pending(mark, dx, dy);
    }
}

/// Sanitised font size of a style.
pub(crate) fn font_size(style: &ComputedStyle) -> f32 {
    let s = style.font_size;
    if s.is_finite() {
        s.clamp(0.0, MAX_FONT_SIZE)
    } else {
        16.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    #[test]
    fn fx_hasher_distinguishes_strings() {
        let h = |s: &str| {
            let mut h = FxHasher::default();
            s.hash(&mut h);
            h.finish()
        };
        assert_ne!(h("abc"), h("abd"));
        assert_ne!(h("a"), h("a\0"));
        assert_eq!(h("hello world, long string"), h("hello world, long string"));
    }
}
