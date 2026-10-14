//! Inline formatting contexts (CSS 2.1 §9.4.2, §10.8, CSS Text 3).
//!
//! Layout of an inline formatting context happens in three steps:
//!
//! 1. **Collection** ([`collect_items`]): the inline-level content of the
//!    container is flattened into [`Item`]s: words, spaces, tabs, forced
//!    breaks, inline box start/end markers (carrying their horizontal
//!    margin + border + padding), atomic inlines, floats and absolutely
//!    positioned boxes. White space is processed per `white-space`
//!    (collapsing across inline boundaries), `text-transform` is applied and
//!    every word is measured (with a per-pass cache).
//! 2. **Line breaking** (greedy): items are added to the current line until
//!    one overflows; the line then ends at the last soft wrap opportunity
//!    (spaces, after hyphens, `<wbr>`, U+200B, U+00AD, around CJK
//!    ideographs and atomic inlines). Words are only broken inside when
//!    `break_anywhere` is set. Lines next to floats are shortened, and moved
//!    below floats when not even their first unit fits.
//! 3. **Line construction**: collapsible spaces at line ends are removed,
//!    `text-align`/`text-indent`/`justify` position the items, the line box
//!    height comes from the CSS inline box model with half-leading and
//!    `vertical-align`, and [`Fragment`]s are produced.

use std::sync::Arc;

use xbro_style::{Clear, ComputedStyle, TextAlign, VerticalAlign, WhiteSpace};
use xbro_text::FontId;

use crate::block::Cb;
use crate::box_tree::{BoxNode, Kind};
use crate::cx::{FontInfo, LayoutCx};
use crate::float::FloatContext;
use crate::geom::{Edges, Point, Rect};
use crate::intrinsic::Intrinsic;
use crate::sizing::{box_edges, lp, sane, sane_size, EPSILON};
use crate::text::{
    continues_word, is_break_after_dash, is_cjk, is_cjk_closing, no_break_after, no_break_before,
    push_transformed,
};
use crate::{Fragment, InlineBoxFragment, LayoutBox, LineBox, TextFragment};

/// Parent index meaning "the root inline box" (the IFC container).
pub(crate) const NO_BOX: u32 = u32::MAX;

/// A text run: the processed text of one text box.
#[derive(Debug)]
pub(crate) struct Run {
    pub node: Option<xbro_dom::NodeId>,
    pub style: Arc<ComputedStyle>,
    /// Text after white-space processing and `text-transform`.
    pub text: String,
    pub font: FontInfo,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    /// Width of one space (with letter/word spacing).
    pub space_width: f32,
    pub break_anywhere: bool,
}

/// A non-atomic inline box (an inline element).
#[derive(Debug)]
pub(crate) struct InlineBoxInfo<'t> {
    pub b: &'t BoxNode,
    pub parent: u32,
    pub font: FontInfo,
    pub padding: Edges,
    pub border: Edges,
    pub margin: Edges,
    /// Cumulative `position: relative` offset of the box (its own plus its
    /// inline ancestors'), applied to its fragments and content.
    pub rel: (f32, f32),
    /// Nearest positioned inline box among this box and its inline
    /// ancestors (or [`NO_BOX`]): the containing block of absolutely
    /// positioned boxes in its content (CSS 2.1 §10.1, item 4).
    pub cb_box: u32,
}

impl InlineBoxInfo<'_> {
    /// Whether the box has non-zero margins, borders or padding (which makes
    /// a line containing it non-empty).
    fn has_mbp(&self) -> bool {
        self.padding != Edges::ZERO
            || self.border != Edges::ZERO
            || self.margin.left != 0.0
            || self.margin.right != 0.0
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum ItemKind {
    /// A word (no spaces) of `run`, bytes `start..end` of its text.
    Word {
        run: u32,
        start: u32,
        end: u32,
    },
    /// One collapsible space, or a sequence of preserved spaces.
    Space {
        run: u32,
        start: u32,
        end: u32,
        collapsible: bool,
    },
    /// A preserved tab (width computed from tab stops at line time).
    Tab {
        run: u32,
        start: u32,
        end: u32,
    },
    /// Forced line break (`<br>` or a preserved newline).
    Break {
        clear: Clear,
    },
    /// Start / end of inline box `i` (width = left / right margin+border+padding).
    Open(u32),
    Close(u32),
    Atomic(u32),
    Float(u32),
    Abspos(u32),
    /// `<wbr>`: a soft wrap opportunity.
    Opportunity,
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct Item {
    pub kind: ItemKind,
    pub width: f32,
    /// A soft wrap opportunity follows this item.
    pub break_after: bool,
    /// A soft wrap opportunity precedes this item (atomic inlines).
    pub break_before: bool,
    /// The opportunity after this word is a soft hyphen (draw a hyphen if broken there).
    pub shy: bool,
    /// Enclosing inline box (or [`NO_BOX`]).
    pub parent: u32,
}

/// Flattened inline content of one inline formatting context.
#[derive(Debug, Default)]
pub(crate) struct Items<'t> {
    pub runs: Vec<Run>,
    pub items: Vec<Item>,
    pub boxes: Vec<InlineBoxInfo<'t>>,
    pub atomics: Vec<&'t BoxNode>,
    pub floats: Vec<&'t BoxNode>,
    pub abspos: Vec<&'t BoxNode>,
}

/// Collects the inline content of `root` (a block container whose children
/// are inline-level). `cb_width` resolves percentage margins/padding of
/// inline boxes.
pub(crate) fn collect_items<'t>(
    cx: &mut LayoutCx<'t>,
    root: &'t BoxNode,
    cb_width: f32,
) -> Items<'t> {
    let mut c = Collector {
        cx,
        out: Items::default(),
        prev_space: true,
        prev_word: false,
        cb_width,
    };
    c.children(&root.children, NO_BOX, root.style.white_space, 0);
    c.out
}

struct Collector<'c, 't> {
    cx: &'c mut LayoutCx<'t>,
    out: Items<'t>,
    /// The last character emitted was a collapsible space (or a line start).
    prev_space: bool,
    /// The last character emitted continues a word (for `capitalize`).
    prev_word: bool,
    cb_width: f32,
}

impl<'t> Collector<'_, 't> {
    /// The last item, looking through inline box starts and ends (which
    /// do not separate content for line breaking).
    fn last_through_boxes(&mut self) -> Option<&mut Item> {
        self.out
            .items
            .iter_mut()
            .rev()
            .take(256)
            .find(|it| !matches!(it.kind, ItemKind::Open(_) | ItemKind::Close(_)))
    }

    /// Last character of the word right before the next item (looking
    /// through inline box starts and ends), if the previous item is a word.
    fn last_word_char(&self) -> Option<char> {
        let it = self
            .out
            .items
            .iter()
            .rev()
            .take(256)
            .find(|it| !matches!(it.kind, ItemKind::Open(_) | ItemKind::Close(_)))?;
        let ItemKind::Word { run, start, end } = it.kind else {
            return None;
        };
        let text = self
            .out
            .runs
            .get(run as usize)?
            .text
            .get(start as usize..end as usize)?;
        text.chars().next_back()
    }

    /// Appends an item; the returned handle sets its break flags.
    fn push(&mut self, kind: ItemKind, width: f32, parent: u32) -> ItemFlags<'_> {
        self.out.items.push(Item {
            kind,
            width,
            break_after: false,
            break_before: false,
            shy: false,
            parent,
        });
        ItemFlags(self.out.items.last_mut())
    }

    fn children(&mut self, kids: &'t [BoxNode], parent: u32, ws: WhiteSpace, depth: usize) {
        if depth > 1024 {
            return;
        }
        for k in kids {
            if k.is_float() {
                let i = self.out.floats.len() as u32;
                self.out.floats.push(k);
                self.push(ItemKind::Float(i), 0.0, parent);
                continue;
            }
            if k.is_absolute() {
                let i = self.out.abspos.len() as u32;
                self.out.abspos.push(k);
                self.push(ItemKind::Abspos(i), 0.0, parent);
                continue;
            }
            match &k.kind {
                Kind::Text(t) => self.text(k, t, parent),
                Kind::LineBreak => {
                    let clear = k.style.clear;
                    self.push(ItemKind::Break { clear }, 0.0, parent);
                    self.prev_space = true;
                    self.prev_word = false;
                }
                Kind::WordBreak => {
                    if ws.wraps() {
                        self.push(ItemKind::Opportunity, 0.0, parent);
                    }
                }
                Kind::Inline => {
                    let idx = self.out.boxes.len() as u32;
                    let edges = box_edges(&k.style, self.cb_width);
                    let font = self.cx.font(&k.style);
                    let left = edges.margin.left + edges.border.left + edges.padding.left;
                    let right = edges.margin.right + edges.border.right + edges.padding.right;
                    let own = crate::position::relative_offset(
                        &k.style,
                        Cb {
                            x: 0.0,
                            width: self.cb_width,
                            height: None,
                        },
                    );
                    let (up, up_cb) = self
                        .out
                        .boxes
                        .get(parent as usize)
                        .map_or(((0.0, 0.0), NO_BOX), |p| (p.rel, p.cb_box));
                    let cb_box = if k.style.position.is_positioned() {
                        idx
                    } else {
                        up_cb
                    };
                    self.out.boxes.push(InlineBoxInfo {
                        b: k,
                        parent,
                        font,
                        padding: edges.padding,
                        border: edges.border,
                        margin: edges.margin,
                        rel: (own.0 + up.0, own.1 + up.1),
                        cb_box,
                    });
                    self.push(ItemKind::Open(idx), left, parent);
                    self.children(&k.children, idx, k.style.white_space, depth + 1);
                    self.push(ItemKind::Close(idx), right, parent);
                }
                _ => {
                    let idx = self.out.atomics.len() as u32;
                    self.out.atomics.push(k);
                    // `(<img>`: no break between opening punctuation and
                    // the atomic inline right after it.
                    let after_opening = self.last_word_char().is_some_and(no_break_after);
                    let wraps = ws.wraps();
                    self.push(ItemKind::Atomic(idx), 0.0, parent)
                        .before(wraps && !after_opening)
                        .after(wraps);
                    self.prev_space = false;
                    self.prev_word = false;
                }
            }
        }
    }

    /// White-space processing and segmentation of one text box.
    fn text(&mut self, b: &'t BoxNode, raw: &str, parent: u32) {
        let style = &b.style;
        let font = self.cx.font(style);
        let ls = sane(style.letter_spacing);
        let wsp = sane(style.word_spacing);
        let space_width = self.cx.measure(font.id, font.size, " ", ls, wsp);
        let run = self.out.runs.len() as u32;
        let ws = style.white_space;
        let collapse = !ws.preserves_spaces();
        let newlines = ws.preserves_newlines();
        let wraps = ws.wraps();
        let break_spaces = ws == WhiteSpace::BreakSpaces;
        let tt = style.text_transform;

        let mut seg = Segmenter {
            text: String::with_capacity(raw.len()),
            word_start: None,
            run,
            parent,
            font,
            ls,
            wsp,
        };
        let mut chars = raw.chars().peekable();
        let mut prev_char = '\0';
        while let Some(c) = chars.next() {
            match c {
                '\n' if newlines => {
                    seg.flush_word(self, false, false);
                    self.push(ItemKind::Break { clear: Clear::None }, 0.0, parent);
                    self.prev_space = true;
                    self.prev_word = false;
                }
                ' ' | '\t' | '\n' | '\r' | '\x0C' if collapse => {
                    seg.flush_word(self, false, false);
                    if !self.prev_space {
                        let start = seg.text.len() as u32;
                        seg.text.push(' ');
                        self.push(
                            ItemKind::Space {
                                run,
                                start,
                                end: start + 1,
                                collapsible: true,
                            },
                            space_width,
                            parent,
                        )
                        .after(wraps);
                        self.prev_space = true;
                    }
                    self.prev_word = false;
                }
                '\r' => {}
                '\t' => {
                    seg.flush_word(self, false, false);
                    let start = seg.text.len() as u32;
                    seg.text.push('\t');
                    self.push(
                        ItemKind::Tab {
                            run,
                            start,
                            end: start + 1,
                        },
                        0.0,
                        parent,
                    )
                    .after(wraps);
                    self.prev_space = false;
                    self.prev_word = false;
                }
                ' ' | '\n' | '\x0C' => {
                    // Preserved space (a newline here means `pre`-like
                    // handling without newline preservation: never happens
                    // for valid `white-space` values, treated as a space).
                    seg.flush_word(self, false, false);
                    let start = seg.text.len() as u32;
                    seg.text.push(' ');
                    let merged = !break_spaces
                        && match self.out.items.last_mut() {
                            Some(Item {
                                kind:
                                    ItemKind::Space {
                                        run: r,
                                        end,
                                        collapsible: false,
                                        ..
                                    },
                                width,
                                ..
                            }) if *r == run && *end == start => {
                                *end = start + 1;
                                *width += space_width;
                                true
                            }
                            _ => false,
                        };
                    if !merged {
                        self.push(
                            ItemKind::Space {
                                run,
                                start,
                                end: start + 1,
                                collapsible: false,
                            },
                            space_width,
                            parent,
                        )
                        .after(wraps);
                    }
                    self.prev_space = false;
                    self.prev_word = false;
                }
                '\u{200B}' => {
                    seg.flush_word(self, wraps, false);
                }
                '\u{AD}' => {
                    seg.flush_word(self, wraps, wraps);
                }
                _ => {
                    let cjk = wraps && is_cjk(c);
                    if cjk && !is_cjk_closing(c) {
                        seg.flush_word(self, true, false);
                    }
                    if seg.word_start.is_none() {
                        seg.word_start = Some(seg.text.len());
                    }
                    push_transformed(&mut seg.text, c, tt, !self.prev_word);
                    let next = chars.peek().copied();
                    let cjk_break = cjk && !next.is_some_and(is_cjk_closing);
                    let dash_break = wraps
                        && is_break_after_dash(c)
                        && (c != '-'
                            || (prev_char.is_alphanumeric()
                                && next.is_some_and(|n| n.is_alphabetic())));
                    if cjk_break || dash_break {
                        seg.flush_word(self, true, false);
                    }
                    self.prev_word = continues_word(c);
                    self.prev_space = false;
                }
            }
            prev_char = c;
        }
        seg.flush_word(self, false, false);
        self.out.runs.push(Run {
            node: b.node,
            style: style.clone(),
            text: seg.text,
            font,
            letter_spacing: ls,
            word_spacing: wsp,
            space_width,
            break_anywhere: style.break_anywhere && wraps,
        });
    }
}

/// Handle on a just-pushed [`Item`] to set its flags.
struct ItemFlags<'a>(Option<&'a mut Item>);

impl ItemFlags<'_> {
    fn after(mut self, v: bool) -> Self {
        if let Some(it) = self.0.as_deref_mut() {
            it.break_after = v;
        }
        self
    }

    fn before(mut self, v: bool) -> Self {
        if let Some(it) = self.0.as_deref_mut() {
            it.break_before = v;
        }
        self
    }

    fn shy(mut self, v: bool) -> Self {
        if let Some(it) = self.0.as_deref_mut() {
            it.shy = v;
        }
        self
    }
}

/// Word accumulation state for one run.
struct Segmenter {
    text: String,
    word_start: Option<usize>,
    run: u32,
    parent: u32,
    font: FontInfo,
    ls: f32,
    wsp: f32,
}

impl Segmenter {
    /// Emits the pending word (if any). `break_after` marks a soft wrap
    /// opportunity after it (applied to the previous item when there is no
    /// pending word, e.g. U+200B after a space).
    fn flush_word(&mut self, c: &mut Collector<'_, '_>, break_after: bool, shy: bool) {
        match self.word_start.take() {
            Some(start) if start < self.text.len() => {
                let word = self.text.get(start..).unwrap_or("");
                if word.chars().next().is_some_and(no_break_before) {
                    // `<img>,`: no break between an atomic inline and the
                    // closing punctuation right after it.
                    if let Some(it) = c.last_through_boxes() {
                        if matches!(it.kind, ItemKind::Atomic(_)) {
                            it.break_after = false;
                        }
                    }
                }
                let w =
                    c.cx.measure(self.font.id, self.font.size, word, self.ls, self.wsp);
                c.push(
                    ItemKind::Word {
                        run: self.run,
                        start: start as u32,
                        end: self.text.len() as u32,
                    },
                    w,
                    self.parent,
                )
                .after(break_after)
                .shy(shy);
            }
            _ => {
                if break_after {
                    if let Some(last) = c.out.items.last_mut() {
                        last.break_after = true;
                    }
                }
            }
        }
    }
}

/// An atomic inline laid out with its margin box at the origin.
#[derive(Debug)]
pub(crate) struct AtomicBox {
    pub lb: LayoutBox,
    /// Margin box size.
    pub width: f32,
    pub height: f32,
    /// Baseline offset from the margin box top.
    pub baseline: f32,
    /// Range of pending absolutely positioned boxes registered during its layout.
    pub pending: (u64, u64),
}

/// Result of laying out an inline formatting context.
#[derive(Debug, Default)]
pub(crate) struct IfcOutput {
    pub lines: Vec<LineBox>,
    /// Floats placed by this context (children of the container box).
    pub floats: Vec<LayoutBox>,
    /// Height from the top of the area to the bottom of the last line box.
    pub height: f32,
}

/// One piece of a line: an item, or a byte sub-range of a word.
#[derive(Copy, Clone, Debug)]
struct Piece {
    item: usize,
    start: u32,
    end: u32,
    width: f32,
}

/// Position in the item list: item index and bytes consumed in a word.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Cursor {
    item: usize,
    offset: u32,
}

enum Scan {
    Line {
        pieces: Vec<Piece>,
        next: Cursor,
        forced: Option<Clear>,
        /// The line ends at a soft hyphen opportunity.
        shy_end: bool,
    },
    /// The first unit does not fit next to floats: move down and retry.
    Retry,
}

/// Per-line vertical alignment node (root, inline box or atomic).
#[derive(Copy, Clone, Debug)]
struct VNode {
    /// Index of the top/bottom-aligned node this one moves with (0 = root group).
    group: usize,
    /// Baseline relative to the group's baseline (y down).
    rel: f32,
    above: f32,
    below: f32,
    align: VerticalAlign,
    font: FontInfo,
}

/// Lays out the inline content of `root` in the area starting at `y`
/// inside containing block `cb`, placing floats into `floats`.
pub(crate) fn layout_ifc<'t>(
    cx: &mut LayoutCx<'t>,
    root: &'t BoxNode,
    cb: Cb,
    y: f32,
    floats: &mut FloatContext,
) -> IfcOutput {
    let items = collect_items(cx, root, cb.width);
    let mut atomics = Vec::with_capacity(items.atomics.len());
    for &a in &items.atomics {
        atomics.push(Some(cx.layout_atomic(a, cb)));
    }
    let mut items = items;
    for it in &mut items.items {
        if let ItemKind::Atomic(a) = it.kind {
            if let Some(Some(ab)) = atomics.get(a as usize) {
                it.width = ab.width;
            }
        }
    }
    let root_font = cx.font(&root.style);
    let float_count = items.floats.len();
    let box_count = items.boxes.len();
    let mut ifc = Ifc {
        cx,
        items: &items,
        floats,
        cb,
        root_style: &root.style,
        root_font,
        indent: lp(&root.style.text_indent, cb.width),
        atomics,
        float_done: vec![false; float_count],
        deferred: Vec::new(),
        out_floats: Vec::new(),
        vindex: vec![u32::MAX; box_count],
        open: Vec::new(),
        tail_start: tail_start(&items),
        inline_cbs: vec![None; box_count],
        inline_abs: Vec::new(),
    };
    ifc.run(y)
}

/// Index from which only "trailing junk" items remain (collapsible spaces,
/// inline box ends, out-of-flow boxes, opportunities): a line ending there is the last.
fn tail_start(items: &Items<'_>) -> usize {
    let mut i = items.items.len();
    while i > 0 {
        match items.items.get(i - 1).map(|it| it.kind) {
            Some(
                ItemKind::Space {
                    collapsible: true, ..
                }
                | ItemKind::Close(_)
                | ItemKind::Float(_)
                | ItemKind::Abspos(_)
                | ItemKind::Opportunity,
            ) => i -= 1,
            _ => break,
        }
    }
    i
}

struct Ifc<'c, 't> {
    cx: &'c mut LayoutCx<'t>,
    items: &'c Items<'t>,
    floats: &'c mut FloatContext,
    cb: Cb,
    root_style: &'t ComputedStyle,
    root_font: FontInfo,
    indent: f32,
    atomics: Vec<Option<AtomicBox>>,
    float_done: Vec<bool>,
    /// Floats that did not fit on their line: placed after it.
    deferred: Vec<crate::block::LaidFloat>,
    out_floats: Vec<LayoutBox>,
    /// Scratch: inline box index → line node index.
    vindex: Vec<u32>,
    /// Inline boxes open at the start of the current line.
    open: Vec<u32>,
    tail_start: usize,
    /// Padding boxes of the first and last fragments of each positioned
    /// inline box (containing blocks of absolutely positioned content).
    inline_cbs: Vec<Option<(Rect, Rect)>>,
    /// Absolutely positioned boxes whose containing block is a positioned
    /// inline box of this context: (inline box, box, static position).
    inline_abs: Vec<(u32, &'t BoxNode, Point)>,
}

impl<'t> Ifc<'_, 't> {
    fn strut_height(&self) -> f32 {
        self.root_font.line_height.max(1.0)
    }

    fn run(&mut self, start_y: f32) -> IfcOutput {
        let mut lines: Vec<LineBox> = Vec::new();
        let mut y = start_y;
        let mut pos = Cursor { item: 0, offset: 0 };
        let mut first_line = true;
        let mut guard = 0usize;
        let mut retries = 0usize;
        // Every line consumes at least one item or one byte of a word.
        let text_len: usize = self.items.runs.iter().map(|r| r.text.len()).sum();
        let limit = self
            .items
            .items
            .len()
            .saturating_add(text_len)
            .saturating_add(64);
        while pos.item < self.items.items.len() {
            guard += 1;
            if guard > limit {
                break;
            }
            match self.scan(pos, y, first_line) {
                Scan::Retry => {
                    // Each retry moves below at least one float: bounded by
                    // the number of floats, not by the line budget.
                    guard -= 1;
                    retries += 1;
                    let h = self.strut_height();
                    match self.floats.next_bottom(y, h) {
                        Some(ny) if ny > y && retries <= self.floats.len() + 1 => y = ny,
                        _ => {
                            // Cannot move further down: lay out as is.
                            if let Scan::Line {
                                pieces,
                                next,
                                forced,
                                shy_end,
                            } = self.scan_no_retry(pos, y, first_line)
                            {
                                if let Some(l) =
                                    self.build_line(&pieces, y, first_line, next, forced, shy_end)
                                {
                                    y = l.rect.bottom();
                                    lines.push(l);
                                    first_line = false;
                                }
                                y = self.after_line(y, forced);
                                pos = next;
                            }
                        }
                    }
                }
                Scan::Line {
                    pieces,
                    next,
                    forced,
                    shy_end,
                } => {
                    if let Some(l) = self.build_line(&pieces, y, first_line, next, forced, shy_end)
                    {
                        y = l.rect.bottom();
                        lines.push(l);
                        first_line = false;
                    }
                    y = self.after_line(y, forced);
                    if next == pos {
                        // No progress (should not happen): skip an item.
                        pos = Cursor {
                            item: pos.item + 1,
                            offset: 0,
                        };
                    } else {
                        pos = next;
                    }
                }
            }
        }
        // Floats deferred after the last line.
        let deferred = std::mem::take(&mut self.deferred);
        for f in deferred {
            let lb = self.cx.place_float(f, self.floats, y, self.cb);
            self.out_floats.push(lb);
        }
        self.layout_inline_abspos();
        // `y` is below the last line, and below the floats a final `<br
        // clear>` cleared: the content (and the container's auto height)
        // extends down to there, like browsers do.
        let height = lines
            .last()
            .map_or(0.0, |l| sane_size(l.rect.bottom().max(y) - start_y));
        IfcOutput {
            lines,
            floats: std::mem::take(&mut self.out_floats),
            height,
        }
    }

    /// Lays out the absolutely positioned boxes whose containing block is a
    /// positioned inline box, now that its fragments are known: the bounding
    /// box of the padding boxes of its first and last fragments. They become
    /// children of the container (like floats). A box whose inline box got
    /// no fragment waits for the enclosing containing block instead.
    fn layout_inline_abspos(&mut self) {
        for (ib, b, static_pos) in std::mem::take(&mut self.inline_abs) {
            match self.inline_cbs.get(ib as usize).copied().flatten() {
                Some((first, last)) => {
                    let cbr = first.union(&last);
                    let cbr = if cbr.is_empty() {
                        // A zero-size fragment still anchors its content.
                        Rect::new(
                            first.x.min(last.x),
                            first.y.min(last.y),
                            sane_size(first.right().max(last.right()) - first.x.min(last.x)),
                            sane_size(first.bottom().max(last.bottom()) - first.y.min(last.y)),
                        )
                    } else {
                        cbr
                    };
                    let p = crate::cx::PendingAbs {
                        b,
                        static_pos,
                        cb: crate::cx::AbsCb::Icb,
                        seq: 0,
                    };
                    let lb = self.cx.layout_abspos(p, cbr);
                    self.out_floats.push(lb);
                }
                None => self.cx.register_abs(b, static_pos),
            }
        }
    }

    /// Registers absolutely positioned box `a` (an inline-content item with
    /// enclosing inline box `parent`) at its static position: with its
    /// positioned inline ancestor, if any, else with the layout context.
    fn register_inline_abs(&mut self, a: u32, parent: u32, at: Point) {
        let Some(&ab) = self.items.abspos.get(a as usize) else {
            return;
        };
        let cb_box = self
            .items
            .boxes
            .get(parent as usize)
            .map_or(NO_BOX, |ib| ib.cb_box);
        if cb_box != NO_BOX && ab.style.position == xbro_style::Position::Absolute {
            self.inline_abs.push((cb_box, ab, at));
        } else {
            self.cx.register_abs(ab, at);
        }
    }

    /// Records the padding box of a fragment of positioned inline box `b`.
    fn note_inline_cb(&mut self, b: u32, f: &InlineBoxFragment) {
        let positioned = self
            .items
            .boxes
            .get(b as usize)
            .is_some_and(|ib| ib.b.style.position.is_positioned());
        if !positioned {
            return;
        }
        let pad = f.rect.inset(f.border);
        if let Some(slot) = self.inline_cbs.get_mut(b as usize) {
            *slot = Some(match *slot {
                Some((first, _)) => (first, pad),
                None => (pad, pad),
            });
        }
    }

    /// Places deferred floats and applies `clear` of a `<br>`; returns the new y.
    fn after_line(&mut self, mut y: f32, forced: Option<Clear>) -> f32 {
        let deferred = std::mem::take(&mut self.deferred);
        for f in deferred {
            let lb = self.cx.place_float(f, self.floats, y, self.cb);
            self.out_floats.push(lb);
        }
        if let Some(clear) = forced {
            if clear != Clear::None {
                if let Some(cy) = self.floats.clear_y(clear) {
                    y = y.max(cy);
                }
            }
        }
        y
    }

    fn scan_no_retry(&mut self, start: Cursor, y: f32, first_line: bool) -> Scan {
        self.scan_impl(start, y, first_line, false)
    }

    fn scan(&mut self, start: Cursor, y: f32, first_line: bool) -> Scan {
        self.scan_impl(start, y, first_line, true)
    }

    fn line_space(&self, y: f32, first_line: bool) -> (f32, f32, f32) {
        let (l, r) =
            self.floats
                .available(y, self.strut_height(), self.cb.x, self.cb.x + self.cb.width);
        let indent = if first_line { self.indent } else { 0.0 };
        (l, r, sane_size(r - l - indent))
    }

    fn run_of(&self, i: u32) -> Option<&Run> {
        self.items.runs.get(i as usize)
    }

    fn measure_range(&mut self, run: u32, start: u32, end: u32) -> f32 {
        let Some(r) = self.items.runs.get(run as usize) else {
            return 0.0;
        };
        let Some(s) = r.text.get(start as usize..end as usize) else {
            return 0.0;
        };
        self.cx
            .measure(r.font.id, r.font.size, s, r.letter_spacing, r.word_spacing)
    }

    /// Largest char boundary `split` in `start..end` such that the prefix fits
    /// in `space` (at least one char when `at_least_one`).
    fn split_word(
        &mut self,
        run: u32,
        start: u32,
        end: u32,
        space: f32,
        at_least_one: bool,
    ) -> Option<(u32, f32)> {
        let r = self.items.runs.get(run as usize)?;
        let s = r.text.get(start as usize..end as usize)?;
        let (font, size, ls, wsp) = (r.font.id, r.font.size, r.letter_spacing, r.word_spacing);
        let mut acc = 0.0f32;
        let mut best: Option<(u32, f32)> = None;
        let mut buf = [0u8; 4];
        for (i, ch) in s.char_indices() {
            let cw = self
                .cx
                .measure(font, size, ch.encode_utf8(&mut buf), ls, wsp);
            let end_i = (i + ch.len_utf8()) as u32;
            if acc + cw > space + EPSILON {
                if best.is_none() && at_least_one {
                    best = Some((start + end_i, acc + cw));
                }
                break;
            }
            acc += cw;
            best = Some((start + end_i, acc));
        }
        best.filter(|(split, _)| *split > start)
    }

    fn scan_impl(&mut self, start: Cursor, y: f32, first_line: bool, allow_retry: bool) -> Scan {
        let (_, _, mut avail) = self.line_space(y, first_line);
        let strut = self.strut_height();
        let narrows = |fc: &FloatContext, cb: Cb| fc.narrows(y, strut, cb.x, cb.x + cb.width);
        let mut pieces: Vec<Piece> = Vec::new();
        let mut width = 0.0f32;
        let mut has_content = false;
        // (pieces kept, cursor after the opportunity, soft hyphen)
        let mut last_break: Option<(usize, Cursor, bool)> = None;
        let mut i = start.item;
        let mut offset = start.offset;
        let items = self.items;
        while let Some(item) = items.items.get(i) {
            match item.kind {
                ItemKind::Space {
                    collapsible: true, ..
                } if !has_content => {}
                ItemKind::Word { run, start: s, end } => {
                    let s2 = s.saturating_add(offset).min(end);
                    let anywhere = self.run_of(run).is_some_and(|r| r.break_anywhere);
                    let room = avail - width;
                    // Width of the (rest of the) word, `None` when it does not fit.
                    let fit = if offset == 0 {
                        (item.width <= room + EPSILON).then_some(item.width)
                    } else {
                        // Rest of a word split by `break_anywhere`: measure only
                        // what fits, so long words stay linear.
                        match self.split_word(run, s2, end, room, false) {
                            Some((split, sw)) if split >= end => Some(sw),
                            _ => None,
                        }
                    };
                    let w = match fit {
                        Some(w) => w,
                        None => {
                            let split_at = |cx: &mut Self, at_least_one: bool| {
                                if anywhere {
                                    cx.split_word(run, s2, end, room, at_least_one)
                                } else {
                                    None
                                }
                            };
                            if has_content {
                                if let Some((n, cur, shy)) = last_break {
                                    pieces.truncate(n);
                                    return Scan::Line {
                                        pieces,
                                        next: cur,
                                        forced: None,
                                        shy_end: shy,
                                    };
                                }
                                if anywhere {
                                    // Break inside the word, or before it when
                                    // nothing of it fits.
                                    let next = match split_at(self, false) {
                                        Some((split, sw)) if split < end => {
                                            pieces.push(Piece {
                                                item: i,
                                                start: s2,
                                                end: split,
                                                width: sw,
                                            });
                                            Cursor {
                                                item: i,
                                                offset: split - s,
                                            }
                                        }
                                        _ => Cursor { item: i, offset },
                                    };
                                    return Scan::Line {
                                        pieces,
                                        next,
                                        forced: None,
                                        shy_end: false,
                                    };
                                }
                            } else {
                                if allow_retry && narrows(self.floats, self.cb) {
                                    return Scan::Retry;
                                }
                                if let Some((split, sw)) = split_at(self, true) {
                                    if split < end {
                                        pieces.push(Piece {
                                            item: i,
                                            start: s2,
                                            end: split,
                                            width: sw,
                                        });
                                        return Scan::Line {
                                            pieces,
                                            next: Cursor {
                                                item: i,
                                                offset: split - s,
                                            },
                                            forced: None,
                                            shy_end: false,
                                        };
                                    }
                                }
                            }
                            // Unbreakable: the word overflows the line.
                            if offset == 0 {
                                item.width
                            } else {
                                self.measure_range(run, s2, end)
                            }
                        }
                    };
                    pieces.push(Piece {
                        item: i,
                        start: s2,
                        end,
                        width: w,
                    });
                    width += w;
                    has_content = true;
                    // A soft hyphen opportunity is only usable when the
                    // hyphen drawn there still fits (else an earlier
                    // opportunity is kept, if any).
                    let usable = !item.shy
                        || last_break.is_none()
                        || width + self.run_hyphen_width(run) <= avail + EPSILON;
                    if item.break_after && usable {
                        last_break = Some((
                            pieces.len(),
                            Cursor {
                                item: i + 1,
                                offset: 0,
                            },
                            item.shy,
                        ));
                    }
                }
                ItemKind::Space {
                    collapsible,
                    start: s,
                    end,
                    ..
                } => {
                    pieces.push(Piece {
                        item: i,
                        start: s,
                        end,
                        width: item.width,
                    });
                    width += item.width;
                    if !collapsible {
                        has_content = true;
                    }
                    if item.break_after {
                        last_break = Some((
                            pieces.len(),
                            Cursor {
                                item: i + 1,
                                offset: 0,
                            },
                            false,
                        ));
                    }
                }
                ItemKind::Tab { run, start: s, end } => {
                    let sw = self.run_of(run).map_or(0.0, |r| r.space_width);
                    let stop = 8.0 * sw;
                    let indent = if first_line { self.indent } else { 0.0 };
                    let pos = width + indent;
                    let w = if stop > EPSILON {
                        let mut next = ((pos / stop).floor() + 1.0) * stop;
                        if next - pos < sw / 2.0 {
                            next += stop;
                        }
                        sane_size(next - pos)
                    } else {
                        0.0
                    };
                    pieces.push(Piece {
                        item: i,
                        start: s,
                        end,
                        width: w,
                    });
                    width += w;
                    has_content = true;
                    if item.break_after {
                        last_break = Some((
                            pieces.len(),
                            Cursor {
                                item: i + 1,
                                offset: 0,
                            },
                            false,
                        ));
                    }
                }
                ItemKind::Break { clear } => {
                    pieces.push(Piece {
                        item: i,
                        start: 0,
                        end: 0,
                        width: 0.0,
                    });
                    return Scan::Line {
                        pieces,
                        next: Cursor {
                            item: i + 1,
                            offset: 0,
                        },
                        forced: Some(clear),
                        shy_end: false,
                    };
                }
                ItemKind::Open(_) => {
                    pieces.push(Piece {
                        item: i,
                        start: 0,
                        end: 0,
                        width: item.width,
                    });
                    width += item.width;
                }
                ItemKind::Close(_) => {
                    pieces.push(Piece {
                        item: i,
                        start: 0,
                        end: 0,
                        width: item.width,
                    });
                    width += item.width;
                    // An opportunity just before a closing tag moves after it.
                    if let Some((n, _, shy)) = last_break {
                        if n + 1 == pieces.len() {
                            last_break = Some((
                                pieces.len(),
                                Cursor {
                                    item: i + 1,
                                    offset: 0,
                                },
                                shy,
                            ));
                        }
                    }
                }
                ItemKind::Atomic(_) => {
                    if item.break_before && has_content {
                        last_break = Some((pieces.len(), Cursor { item: i, offset: 0 }, false));
                    }
                    if width + item.width > avail + EPSILON {
                        if has_content {
                            if let Some((n, cur, shy)) = last_break {
                                pieces.truncate(n);
                                return Scan::Line {
                                    pieces,
                                    next: cur,
                                    forced: None,
                                    shy_end: shy,
                                };
                            }
                        } else if allow_retry && narrows(self.floats, self.cb) {
                            return Scan::Retry;
                        }
                    }
                    pieces.push(Piece {
                        item: i,
                        start: 0,
                        end: 0,
                        width: item.width,
                    });
                    width += item.width;
                    has_content = true;
                    if item.break_after {
                        last_break = Some((
                            pieces.len(),
                            Cursor {
                                item: i + 1,
                                offset: 0,
                            },
                            false,
                        ));
                    }
                }
                ItemKind::Float(f) => {
                    let fi = f as usize;
                    if self.float_done.get(fi) == Some(&false) {
                        if let Some(slot) = self.float_done.get_mut(fi) {
                            *slot = true;
                        }
                        if let Some(&fb) = items.floats.get(fi) {
                            let laid = self.cx.prepare_float(fb, self.cb);
                            if width + laid.outer_width() <= avail + EPSILON
                                && self.deferred.is_empty()
                            {
                                let lb = self.cx.place_float(laid, self.floats, y, self.cb);
                                self.out_floats.push(lb);
                                avail = self.line_space(y, first_line).2;
                            } else {
                                self.deferred.push(laid);
                            }
                        }
                    }
                }
                ItemKind::Abspos(_) => {
                    pieces.push(Piece {
                        item: i,
                        start: 0,
                        end: 0,
                        width: 0.0,
                    });
                }
                ItemKind::Opportunity => {
                    last_break = Some((
                        pieces.len(),
                        Cursor {
                            item: i + 1,
                            offset: 0,
                        },
                        false,
                    ));
                }
            }
            i += 1;
            offset = 0;
        }
        Scan::Line {
            pieces,
            next: Cursor {
                item: items.items.len(),
                offset: 0,
            },
            forced: None,
            shy_end: false,
        }
    }

    /// Builds the line box for `pieces` at `y`. Returns `None` for an empty
    /// line (which then takes no space).
    fn build_line(
        &mut self,
        pieces: &[Piece],
        y: f32,
        first_line: bool,
        next: Cursor,
        forced: Option<Clear>,
        shy_end: bool,
    ) -> Option<LineBox> {
        let items = self.items;
        let kind_of = |p: &Piece| items.items.get(p.item).map(|it| it.kind);
        let ws_wraps = self.root_style.white_space.wraps();
        let is_last = forced.is_none() && next.item >= self.tail_start;

        // 1. Trailing white space removal (collapsible) or hanging (preserved, soft wrap).
        let mut keep = vec![true; pieces.len()];
        for (k, p) in pieces.iter().enumerate().rev() {
            match kind_of(p) {
                Some(ItemKind::Close(_) | ItemKind::Abspos(_) | ItemKind::Break { .. }) => {}
                Some(ItemKind::Space {
                    collapsible, run, ..
                }) => {
                    let run_wraps = self
                        .run_of(run)
                        .map_or(ws_wraps, |r| r.style.white_space.wraps());
                    if collapsible || (run_wraps && forced.is_none() && !is_last) {
                        if let Some(slot) = keep.get_mut(k) {
                            *slot = false;
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        // 2. Emptiness.
        let non_empty = pieces.iter().zip(&keep).any(|(p, &k)| {
            k && match kind_of(p) {
                Some(
                    ItemKind::Word { .. }
                    | ItemKind::Tab { .. }
                    | ItemKind::Atomic(_)
                    | ItemKind::Break { .. }
                    | ItemKind::Space {
                        collapsible: false, ..
                    },
                ) => true,
                Some(ItemKind::Open(b) | ItemKind::Close(b)) => {
                    items.boxes.get(b as usize).is_some_and(|ib| ib.has_mbp())
                }
                _ => false,
            }
        });

        let (left, right, avail) = self.line_space(y, first_line);
        let indent = if first_line { self.indent } else { 0.0 };
        if !non_empty {
            // Still track inline boxes and static positions.
            for p in pieces {
                match kind_of(p) {
                    Some(ItemKind::Open(b)) => self.open.push(b),
                    Some(ItemKind::Close(b)) => self.close_box(b),
                    Some(ItemKind::Abspos(a)) => {
                        let parent = items.items.get(p.item).map_or(NO_BOX, |it| it.parent);
                        self.register_inline_abs(a, parent, Point::new(left + indent, y));
                    }
                    _ => {}
                }
            }
            return None;
        }

        // 3. Horizontal metrics and alignment.
        let content_width: f32 = pieces
            .iter()
            .zip(&keep)
            .filter(|(_, &k)| k)
            .map(|(p, _)| p.width)
            .sum();
        let hyphen = if shy_end {
            self.hyphen_width(pieces, &keep)
        } else {
            0.0
        };
        let content_width = content_width + hyphen;
        let slack = avail - content_width;
        let align = self.root_style.text_align;
        let mut justify_extra = 0.0f32;
        let offset = match align {
            TextAlign::Left => 0.0,
            TextAlign::Right => slack.max(0.0),
            TextAlign::Center => (slack / 2.0).max(0.0),
            TextAlign::Justify => {
                if !is_last && forced.is_none() && slack > 0.0 {
                    let spaces: usize = pieces
                        .iter()
                        .zip(&keep)
                        .filter(|(_, &k)| k)
                        .filter_map(|(p, _)| match kind_of(p) {
                            Some(ItemKind::Space { .. }) => Some((p.end - p.start) as usize),
                            _ => None,
                        })
                        .sum();
                    if spaces > 0 {
                        justify_extra = slack / spaces as f32;
                    }
                }
                0.0
            }
        };

        // 4. Vertical alignment.
        let mut nodes: Vec<VNode> = vec![VNode {
            group: 0,
            rel: 0.0,
            above: self.root_font.above,
            below: self.root_font.below,
            align: VerticalAlign::Baseline,
            font: self.root_font,
        }];
        let mut touched: Vec<u32> = Vec::new();
        let open_at_start = self.open.clone();
        for &b in &open_at_start {
            self.add_box_node(&mut nodes, &mut touched, b);
        }
        // (node index of each atomic piece)
        let mut atomic_nodes: Vec<(usize, usize)> = Vec::new();
        // text contributions: (parent node, above, below)
        let mut text_contrib: Vec<(usize, f32, f32)> = Vec::new();
        let mut last_run: Option<u32> = None;
        for (k, p) in pieces.iter().enumerate() {
            if !keep.get(k).copied().unwrap_or(false) {
                continue;
            }
            match kind_of(p) {
                Some(ItemKind::Open(b)) => {
                    self.add_box_node(&mut nodes, &mut touched, b);
                }
                Some(ItemKind::Atomic(a)) => {
                    let parent =
                        self.parent_node(items.items.get(p.item).map_or(NO_BOX, |it| it.parent));
                    let pn = nodes.get(parent).or(nodes.first()).copied();
                    if let (Some(Some(ab)), Some(pn)) = (self.atomics.get(a as usize), pn) {
                        let above = ab.baseline;
                        let below = (ab.height - ab.baseline).max(0.0);
                        let align = ab.lb.style.vertical_align;
                        let idx = nodes.len();
                        nodes.push(make_vnode(idx, &pn, above, below, align, pn.font));
                        atomic_nodes.push((k, idx));
                    }
                }
                Some(
                    ItemKind::Word { run, .. }
                    | ItemKind::Space { run, .. }
                    | ItemKind::Tab { run, .. },
                ) if last_run != Some(run) => {
                    last_run = Some(run);
                    let parent =
                        self.parent_node(items.items.get(p.item).map_or(NO_BOX, |it| it.parent));
                    if let Some(r) = self.run_of(run) {
                        text_contrib.push((parent, r.font.above, r.font.below));
                    }
                }
                _ => {}
            }
        }
        // Group extents.
        let mut ext: Vec<(f32, f32)> = vec![(f32::INFINITY, f32::NEG_INFINITY); nodes.len()];
        for n in &nodes {
            if let Some(e) = ext.get_mut(n.group) {
                e.0 = e.0.min(n.rel - n.above);
                e.1 = e.1.max(n.rel + n.below);
            }
        }
        for &(parent, above, below) in &text_contrib {
            if let Some(pn) = nodes.get(parent) {
                if let Some(e) = ext.get_mut(pn.group) {
                    e.0 = e.0.min(pn.rel - above);
                    e.1 = e.1.max(pn.rel + below);
                }
            }
        }
        // Extent of the line relative to the root's baseline: the root
        // group's, enlarged by the `top`/`bottom` groups taller than the
        // line (CSS 2.1 §10.8.1). Like browsers, a taller `top` group
        // extends the line downwards and a taller `bottom` group upwards,
        // so the rest of the line then sits at the line's bottom.
        let (t0, b0) = ext
            .first()
            .copied()
            .filter(|(t, b)| t.is_finite() && b.is_finite())
            .unwrap_or((0.0, 0.0));
        let mut above = -t0;
        let mut below = b0;
        for (i, n) in nodes.iter().enumerate().skip(1) {
            if n.group != i {
                continue;
            }
            if let Some(&(t, b)) = ext.get(i) {
                let h = b - t;
                if h.is_finite() && above + below < h {
                    match n.align {
                        VerticalAlign::Top => below = h - above,
                        _ => above = h - below,
                    }
                }
            }
        }
        let top = sane(-above);
        let bottom = sane(below).max(top);
        let mut shifts: Vec<f32> = vec![0.0; nodes.len()];
        for (i, n) in nodes.iter().enumerate().skip(1) {
            if n.group == i {
                if let (Some(&(t, b)), Some(shift)) = (ext.get(i), shifts.get_mut(i)) {
                    *shift = sane(match n.align {
                        VerticalAlign::Top => top - t,
                        _ => bottom - b,
                    });
                }
            }
        }
        let line_height = sane_size(bottom - top);
        let baseline = y - top;
        let node_baseline = |idx: usize| -> f32 {
            nodes.get(idx).map_or(baseline, |n| {
                baseline + n.rel + shifts.get(n.group).copied().unwrap_or(0.0)
            })
        };

        // 5. Fragments.
        let mut frags: Vec<Fragment> = Vec::with_capacity(pieces.len() / 2 + 2);
        let mut x = left + indent + offset;
        // Open inline box fragments: (box index, fragment index).
        let mut open_frags: Vec<(u32, usize)> = Vec::new();
        // Every inline box fragment of the line, for containing blocks.
        let mut all_frags: Vec<(u32, usize)> = Vec::new();
        for &b in &open_at_start {
            let idx = self.vindex.get(b as usize).map_or(0, |&v| v as usize);
            let bl = node_baseline(idx);
            if let Some(ib) = items.boxes.get(b as usize) {
                frags.push(Fragment::InlineBox(inline_fragment(ib, x, bl, false)));
                shift_last(&mut frags, ib.rel);
                open_frags.push((b, frags.len() - 1));
                all_frags.push((b, frags.len() - 1));
            }
        }
        let mut acc: Option<TextAcc> = None;
        let last_kept = keep.iter().rposition(|&k| k);
        let mut atomic_iter = atomic_nodes.iter().peekable();
        for (k, p) in pieces.iter().enumerate() {
            if !keep.get(k).copied().unwrap_or(false) {
                continue;
            }
            let item = match items.items.get(p.item) {
                Some(it) => *it,
                None => continue,
            };
            match item.kind {
                ItemKind::Word { run, .. } | ItemKind::Space { run, .. } => {
                    let extra = if matches!(item.kind, ItemKind::Space { .. }) {
                        justify_extra * (p.end - p.start) as f32
                    } else {
                        0.0
                    };
                    let contiguous = acc
                        .as_ref()
                        .is_some_and(|a| a.run == run && a.end == p.start);
                    if !contiguous {
                        if let Some(a) = acc.take() {
                            self.flush_text(
                                a,
                                &nodes,
                                &shifts,
                                baseline,
                                justify_extra,
                                &mut frags,
                            );
                        }
                        let parent = self.parent_node(item.parent);
                        acc = Some(TextAcc {
                            run,
                            start: p.start,
                            end: p.start,
                            x,
                            width: 0.0,
                            parent,
                            bx: item.parent,
                            hyphen: false,
                        });
                    }
                    if let Some(a) = acc.as_mut() {
                        a.end = p.end;
                        a.width += p.width + extra;
                        if shy_end && Some(k) == last_kept {
                            a.hyphen = true;
                            a.width += hyphen;
                        }
                    }
                    x += p.width + extra;
                    if shy_end && Some(k) == last_kept {
                        x += hyphen;
                    }
                }
                ItemKind::Tab { .. } => {
                    if let Some(a) = acc.take() {
                        self.flush_text(a, &nodes, &shifts, baseline, justify_extra, &mut frags);
                    }
                    x += p.width;
                }
                ItemKind::Open(b) => {
                    if let Some(a) = acc.take() {
                        self.flush_text(a, &nodes, &shifts, baseline, justify_extra, &mut frags);
                    }
                    if let Some(ib) = items.boxes.get(b as usize) {
                        x += ib.margin.left;
                        let idx = self.vindex.get(b as usize).map_or(0, |&v| v as usize);
                        let bl = node_baseline(idx);
                        frags.push(Fragment::InlineBox(inline_fragment(ib, x, bl, true)));
                        shift_last(&mut frags, ib.rel);
                        open_frags.push((b, frags.len() - 1));
                        all_frags.push((b, frags.len() - 1));
                        x += ib.border.left + ib.padding.left;
                    }
                    self.open.push(b);
                }
                ItemKind::Close(b) => {
                    if let Some(a) = acc.take() {
                        self.flush_text(a, &nodes, &shifts, baseline, justify_extra, &mut frags);
                    }
                    if let Some(ib) = items.boxes.get(b as usize) {
                        x += ib.padding.right + ib.border.right;
                        if let Some(pos) = open_frags.iter().rposition(|&(ob, _)| ob == b) {
                            let (_, fi) = open_frags.remove(pos);
                            if let Some(Fragment::InlineBox(f)) = frags.get_mut(fi) {
                                f.rect.width = sane_size(x + ib.rel.0 - f.rect.x);
                                f.border.right = ib.border.right;
                                f.padding.right = ib.padding.right;
                            }
                        }
                        x += ib.margin.right;
                    }
                    self.close_box(b);
                }
                ItemKind::Atomic(a) => {
                    if let Some(t) = acc.take() {
                        self.flush_text(t, &nodes, &shifts, baseline, justify_extra, &mut frags);
                    }
                    let node_idx = match atomic_iter.peek() {
                        Some(&&(pk, ni)) if pk == k => {
                            atomic_iter.next();
                            Some(ni)
                        }
                        _ => None,
                    };
                    if let Some(ab) = self.atomics.get_mut(a as usize).and_then(Option::take) {
                        let bl = node_idx.map_or(baseline, node_baseline);
                        let top_y = bl - ab.baseline;
                        let AtomicBox {
                            mut lb,
                            width,
                            pending,
                            ..
                        } = ab;
                        // Laid out with the margin box at the origin.
                        let (rx, ry) = self.rel_of(item.parent);
                        crate::tree::translate_box(&mut lb, x + rx, top_y + ry);
                        self.cx
                            .translate_pending_range(pending.0, pending.1, x + rx, top_y + ry);
                        self.cx.apply_relative_range(&mut lb, self.cb, pending);
                        frags.push(Fragment::Atomic(lb));
                        x += width;
                    } else {
                        x += p.width;
                    }
                }
                ItemKind::Abspos(a) => {
                    self.register_inline_abs(a, item.parent, Point::new(x, y));
                }
                ItemKind::Break { .. } | ItemKind::Opportunity | ItemKind::Float(_) => {
                    if let Some(a) = acc.take() {
                        self.flush_text(a, &nodes, &shifts, baseline, justify_extra, &mut frags);
                    }
                }
            }
        }
        if let Some(a) = acc.take() {
            self.flush_text(a, &nodes, &shifts, baseline, justify_extra, &mut frags);
        }
        for (b, fi) in open_frags {
            let dx = self.rel_of(b).0;
            if let Some(Fragment::InlineBox(f)) = frags.get_mut(fi) {
                f.rect.width = sane_size(x + dx - f.rect.x);
            }
        }
        for (b, fi) in all_frags {
            if let Some(Fragment::InlineBox(f)) = frags.get(fi) {
                self.note_inline_cb(b, f);
            }
        }
        // Inline boxes closed in trimmed pieces still close.
        for (k, p) in pieces.iter().enumerate() {
            if !keep.get(k).copied().unwrap_or(true) {
                if let Some(ItemKind::Close(b)) = kind_of(p) {
                    self.close_box(b);
                }
            }
        }
        for b in touched {
            if let Some(v) = self.vindex.get_mut(b as usize) {
                *v = u32::MAX;
            }
        }
        Some(LineBox {
            rect: Rect::new(left, y, sane_size(right - left), line_height),
            baseline,
            fragments: frags,
        })
    }

    /// Cumulative relative offset of inline box `b` (zero for the root).
    fn rel_of(&self, b: u32) -> (f32, f32) {
        self.items
            .boxes
            .get(b as usize)
            .map_or((0.0, 0.0), |ib| ib.rel)
    }

    fn close_box(&mut self, b: u32) {
        if let Some(pos) = self.open.iter().rposition(|&o| o == b) {
            self.open.remove(pos);
        }
    }

    /// Line node index of inline box `b` (0 = root).
    fn parent_node(&self, b: u32) -> usize {
        if b == NO_BOX {
            return 0;
        }
        match self.vindex.get(b as usize) {
            Some(&v) if v != u32::MAX => v as usize,
            _ => 0,
        }
    }

    fn add_box_node(&mut self, nodes: &mut Vec<VNode>, touched: &mut Vec<u32>, b: u32) {
        let Some(ib) = self.items.boxes.get(b as usize) else {
            return;
        };
        let parent = self.parent_node(ib.parent);
        let Some(pn) = nodes.get(parent).or(nodes.first()).copied() else {
            return;
        };
        let idx = nodes.len();
        nodes.push(make_vnode(
            idx,
            &pn,
            ib.font.above,
            ib.font.below,
            ib.b.style.vertical_align,
            ib.font,
        ));
        if let Some(v) = self.vindex.get_mut(b as usize) {
            *v = idx as u32;
            touched.push(b);
        }
    }

    /// Width of a hyphen in the font of run `run`.
    fn run_hyphen_width(&mut self, run: u32) -> f32 {
        let Some(r) = self.run_of(run) else {
            return 0.0;
        };
        let (font, size, ls) = (r.font.id, r.font.size, r.letter_spacing);
        self.cx.measure(font, size, "-", ls, 0.0)
    }

    /// Width of the hyphen drawn at a soft-hyphen break.
    fn hyphen_width(&mut self, pieces: &[Piece], keep: &[bool]) -> f32 {
        let last = pieces
            .iter()
            .zip(keep)
            .rev()
            .find(|(_, &k)| k)
            .map(|(p, _)| *p);
        let Some(p) = last else {
            return 0.0;
        };
        let Some(ItemKind::Word { run, .. }) = self.items.items.get(p.item).map(|i| i.kind) else {
            return 0.0;
        };
        let Some(r) = self.run_of(run) else {
            return 0.0;
        };
        let (font, size, ls) = (r.font.id, r.font.size, r.letter_spacing);
        self.cx.measure(font, size, "-", ls, 0.0)
    }

    /// Emits the text fragment accumulated in `a`.
    fn flush_text(
        &mut self,
        a: TextAcc,
        nodes: &[VNode],
        shifts: &[f32],
        line_baseline: f32,
        justify_extra: f32,
        frags: &mut Vec<Fragment>,
    ) {
        let Some(r) = self.items.runs.get(a.run as usize) else {
            return;
        };
        let Some(s) = r.text.get(a.start as usize..a.end as usize) else {
            return;
        };
        if s.is_empty() && !a.hyphen {
            return;
        }
        let mut text = s.to_string();
        if a.hyphen {
            text.push('-');
        }
        let shaped = self.cx.fonts.shape(
            r.font.id,
            r.font.size,
            &text,
            r.letter_spacing,
            r.word_spacing,
        );
        let mut glyphs = shaped.glyphs;
        if justify_extra > 0.0 {
            let bytes = text.as_bytes();
            let mut shift = 0.0f32;
            for g in &mut glyphs {
                g.x += shift;
                if bytes.get(g.cluster) == Some(&b' ') {
                    g.advance += justify_extra;
                    shift += justify_extra;
                }
            }
        }
        let (rx, ry) = self.rel_of(a.bx);
        let baseline = ry
            + nodes.get(a.parent).map_or(line_baseline, |n| {
                line_baseline + n.rel + shifts.get(n.group).copied().unwrap_or(0.0)
            });
        let ascent = sane_size(r.font.metrics.ascent);
        let descent = sane_size(r.font.metrics.descent);
        let node = match r.node {
            Some(n) => n,
            None => return,
        };
        frags.push(Fragment::Text(TextFragment {
            node,
            rect: Rect::new(
                a.x + rx,
                baseline - ascent,
                sane_size(a.width),
                ascent + descent,
            ),
            baseline,
            text,
            font: r.font.id,
            font_size: r.font.size,
            glyphs,
            style: r.style.clone(),
        }));
    }
}

/// Moves the last fragment (an inline box fragment) by a relative offset.
fn shift_last(frags: &mut [Fragment], (dx, dy): (f32, f32)) {
    if let Some(Fragment::InlineBox(f)) = frags.last_mut() {
        f.rect = f.rect.translate(dx, dy);
    }
}

/// Text accumulated for one fragment.
struct TextAcc {
    run: u32,
    start: u32,
    end: u32,
    x: f32,
    width: f32,
    parent: usize,
    /// Enclosing inline box (for its relative offset).
    bx: u32,
    hyphen: bool,
}

/// Creates the alignment node of a box aligned per `align` inside `parent`.
fn make_vnode(
    idx: usize,
    p: &VNode,
    above: f32,
    below: f32,
    align: VerticalAlign,
    font: FontInfo,
) -> VNode {
    let pm = &p.font.metrics;
    let (group, rel) = match align {
        VerticalAlign::Top | VerticalAlign::Bottom => (idx, 0.0),
        _ => {
            let shift = match align {
                VerticalAlign::Baseline => 0.0,
                VerticalAlign::Sub => p.font.size * 0.2,
                VerticalAlign::Super => -p.font.size * 0.34,
                VerticalAlign::TextTop => -pm.ascent + above,
                VerticalAlign::TextBottom => pm.descent - below,
                VerticalAlign::Middle => -pm.x_height / 2.0 + (above - below) / 2.0,
                VerticalAlign::Length(v) => -sane(v),
                VerticalAlign::Top | VerticalAlign::Bottom => 0.0,
            };
            (p.group, p.rel + sane(shift))
        }
    };
    VNode {
        group,
        rel,
        above,
        below,
        align,
        font,
    }
}

/// Inline box fragment starting at `x` (border edge) for a box whose baseline is `baseline`.
fn inline_fragment(
    ib: &InlineBoxInfo<'_>,
    x: f32,
    baseline: f32,
    first: bool,
) -> InlineBoxFragment {
    let m = &ib.font.metrics;
    let ascent = sane_size(m.ascent);
    let descent = sane_size(m.descent);
    let top = baseline - ascent - ib.padding.top - ib.border.top;
    let height = ascent + descent + ib.padding.vertical() + ib.border.vertical();
    let (bl, pl) = if first {
        (ib.border.left, ib.padding.left)
    } else {
        (0.0, 0.0)
    };
    // Node of an inline box is always an element (anonymous inline boxes do not exist).
    InlineBoxFragment {
        node: ib.b.node.unwrap_or(xbro_dom::NodeId(0)),
        style: ib.b.style.clone(),
        rect: Rect::new(x, top, 0.0, sane_size(height)),
        border: Edges::new(ib.border.top, 0.0, ib.border.bottom, bl),
        padding: Edges::new(ib.padding.top, 0.0, ib.padding.bottom, pl),
    }
}

/// Min-content and max-content widths of the inline content of `root`.
pub(crate) fn ifc_intrinsic<'t>(cx: &mut LayoutCx<'t>, root: &'t BoxNode) -> Intrinsic {
    let items = collect_items(cx, root, 0.0);
    let indent = lp(&root.style.text_indent, 0.0);
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    // Unbreakable segment and line widths.
    let mut seg = 0.0f32;
    let mut line = indent;
    let mut trailing = 0.0f32;
    let end_seg = |min: &mut f32, seg: &mut f32| {
        *min = min.max(*seg);
        *seg = 0.0;
    };
    for it in &items.items {
        match it.kind {
            ItemKind::Word { run, start, end } => {
                let anywhere = items.runs.get(run as usize).filter(|r| r.break_anywhere);
                if let Some(r) = anywhere {
                    // Any character boundary is a break opportunity.
                    let widest = widest_char(cx, r, start, end);
                    seg += widest;
                    end_seg(&mut min, &mut seg);
                } else {
                    seg += it.width;
                }
                line += it.width;
                trailing = 0.0;
                if it.break_after {
                    end_seg(&mut min, &mut seg);
                }
            }
            ItemKind::Space { collapsible, .. } => {
                line += it.width;
                if it.break_after {
                    end_seg(&mut min, &mut seg);
                    trailing += it.width;
                } else if collapsible {
                    // Non-wrapping space: part of the segment.
                    seg += it.width;
                    trailing += it.width;
                } else {
                    seg += it.width;
                    trailing = 0.0;
                }
            }
            ItemKind::Tab { run, .. } => {
                let w = items
                    .runs
                    .get(run as usize)
                    .map_or(0.0, |r| r.space_width * 8.0);
                line += w;
                seg += w;
                trailing = 0.0;
                if it.break_after {
                    end_seg(&mut min, &mut seg);
                }
            }
            ItemKind::Break { .. } => {
                max = max.max(line - trailing);
                line = 0.0;
                trailing = 0.0;
                end_seg(&mut min, &mut seg);
            }
            ItemKind::Open(_) | ItemKind::Close(_) => {
                seg += it.width;
                line += it.width;
            }
            ItemKind::Atomic(a) => {
                let outer = match items.atomics.get(a as usize) {
                    Some(&b) => cx.outer_intrinsic(b),
                    None => Intrinsic::default(),
                };
                if it.break_before {
                    end_seg(&mut min, &mut seg);
                }
                seg += outer.min;
                if it.break_after {
                    end_seg(&mut min, &mut seg);
                }
                line += outer.max;
                trailing = 0.0;
            }
            ItemKind::Float(f) => {
                if let Some(&b) = items.floats.get(f as usize) {
                    let outer = cx.outer_intrinsic(b);
                    min = min.max(outer.min);
                    line += outer.max;
                }
            }
            ItemKind::Opportunity => end_seg(&mut min, &mut seg),
            ItemKind::Abspos(_) => {}
        }
    }
    max = max.max(line - trailing);
    min = min.max(seg);
    let _: Option<FontId> = None;
    Intrinsic {
        min: sane_size(min),
        max: sane_size(max.max(min)),
    }
}

/// Advance of the widest character of `run.text[start..end]`.
fn widest_char(cx: &mut LayoutCx<'_>, r: &Run, start: u32, end: u32) -> f32 {
    let Some(text) = r.text.get(start as usize..end as usize) else {
        return 0.0;
    };
    let mut buf = [0u8; 4];
    let mut widest = 0.0f32;
    for ch in text.chars() {
        let w = cx.measure(
            r.font.id,
            r.font.size,
            ch.encode_utf8(&mut buf),
            r.letter_spacing,
            r.word_spacing,
        );
        widest = widest.max(w);
    }
    widest
}
