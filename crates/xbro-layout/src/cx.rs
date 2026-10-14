//! Per-pass layout state: inputs, font and text-measurement caches, and the
//! bookkeeping of absolutely positioned boxes waiting for their containing
//! block to be laid out.

use std::collections::HashMap;
use std::sync::Arc;

use xbro_dom::Document;
use xbro_style::{ComputedStyle, StyleMap};
use xbro_text::{FontId, FontMetrics, FontQuery, FontSystem};

use crate::box_tree::BoxNode;
use crate::geom::{Point, Size};
use crate::sizing::sane_size;
use crate::{LayoutBox, ReplacedSizes};

/// Largest font size used by layout, in px.
const MAX_FONT_SIZE: f32 = 10_000.0;

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

/// Layout state for one pass.
pub(crate) struct LayoutCx<'t> {
    pub doc: &'t Document,
    pub styles: &'t StyleMap,
    pub fonts: &'t FontSystem,
    pub replaced: &'t dyn ReplacedSizes,
    pub viewport: Size,
    font_ids: HashMap<FontKey, FontId>,
    /// Keeps the family lists used as `FontKey`s alive (their addresses are keys).
    font_family_owners: Vec<Arc<Vec<String>>>,
    metrics: HashMap<(FontId, u32), FontMetrics>,
    words: HashMap<WordKey, HashMap<Box<str>, f32>>,
    /// Absolutely positioned boxes waiting for their containing block.
    pub pending_abs: Vec<PendingAbs<'t>>,
    frames: Vec<u32>,
    next_frame: u32,
}

impl<'t> LayoutCx<'t> {
    pub fn new(
        doc: &'t Document,
        styles: &'t StyleMap,
        fonts: &'t FontSystem,
        replaced: &'t dyn ReplacedSizes,
        viewport: Size,
    ) -> Self {
        LayoutCx {
            doc,
            styles,
            fonts,
            replaced,
            viewport: Size::new(sane_size(viewport.width), sane_size(viewport.height)),
            font_ids: HashMap::new(),
            font_family_owners: Vec::new(),
            metrics: HashMap::new(),
            words: HashMap::new(),
            pending_abs: Vec::new(),
            frames: Vec::new(),
            next_frame: 0,
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
        let metrics = *self
            .metrics
            .entry((id, size.to_bits()))
            .or_insert_with(|| self.fonts.metrics(id, size));
        let ascent = sane_size(metrics.ascent);
        let descent = sane_size(metrics.descent);
        let line_height = sane_size(style.line_height_px(metrics.normal_line_height()));
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
        if text.len() <= 64 {
            map.insert(text.into(), w);
        }
        w
    }

    /// Starts the layout of a positioned box: absolutely positioned
    /// descendants registered from now on wait for it.
    pub fn push_frame(&mut self) -> u32 {
        let id = self.next_frame;
        self.next_frame = self.next_frame.wrapping_add(1);
        self.frames.push(id);
        id
    }

    /// Ends a positioned box's layout and returns the boxes waiting for it.
    pub fn pop_frame(&mut self, id: u32) -> Vec<PendingAbs<'t>> {
        if let Some(pos) = self.frames.iter().rposition(|&f| f == id) {
            self.frames.truncate(pos);
        }
        self.take_pending(|cb| cb == AbsCb::Frame(id))
    }

    /// Registers an absolutely positioned box at its static position.
    pub fn register_abs(&mut self, b: &'t BoxNode, static_pos: Point) {
        let cb = if b.style.position == xbro_style::Position::Fixed {
            AbsCb::Viewport
        } else {
            self.frames.last().map_or(AbsCb::Icb, |&f| AbsCb::Frame(f))
        };
        self.pending_abs.push(PendingAbs { b, static_pos, cb });
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

    /// Mark to pass to [`translate_pending`](Self::translate_pending).
    pub fn pending_mark(&self) -> usize {
        self.pending_abs.len()
    }

    /// Moves the static positions of the boxes registered since `mark`
    /// (used when an already laid-out subtree is moved).
    pub fn translate_pending(&mut self, mark: usize, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        for p in self.pending_abs.iter_mut().skip(mark) {
            p.static_pos.x += dx;
            p.static_pos.y += dy;
        }
    }

    /// Translates a laid-out box and the static positions registered since `mark`.
    pub fn move_box(&mut self, b: &mut LayoutBox, mark: usize, dx: f32, dy: f32) {
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
