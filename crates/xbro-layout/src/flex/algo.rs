//! The flex layout algorithm (CSS Flexbox 1 §9), simplified.
//!
//! Sizes are handled in *logical* terms: the main axis runs from main-start
//! to main-end (left→right for `row`, right→left for `row-reverse`,
//! top→bottom for `column`…) and the cross axis from cross-start to
//! cross-end (swapped by `wrap-reverse`). Item positions are computed as
//! offsets from those starts and mapped to physical coordinates at the end.
//!
//! Steps (numbers from §9):
//!
//! 1. items: in-flow children in `order`-modified document order (§9.1);
//! 2. flex base size and hypothetical main size (§9.2 step 3) with the
//!    automatic minimum size of §4.5;
//! 3. lines: greedy collection, gaps included (§9.3 step 5);
//! 4. flexible lengths: the freeze loop of §9.7;
//! 5. main-axis auto margins and `justify-content` (§9.5 step 12);
//! 6. hypothetical cross sizes by laying items out at their main size,
//!    line cross sizes (baseline alignment approximated with first
//!    baselines), `align-content` (§9.4 steps 7–8, 15–16);
//! 7. stretch, cross-axis auto margins and `align-self` (§9.4 step 11,
//!    §9.6), final layout of each item.

use std::ops::Range;

use xbro_style::{
    AlignItems, BoxSizing, ComputedStyle, FlexWrap, JustifyContent, LengthPercentageAuto, Overflow,
};

use super::item::{self, Laid};
use crate::block::{Cb, ContentLayout};
use crate::box_tree::{BoxNode, Kind};
use crate::cx::LayoutCx;
use crate::geom::{Edges, Point};
use crate::sizing::{
    box_edges, clamp_size, height_limits, lp, max_height, max_width, min_height, min_width, sane,
    sane_size, shrink_to_fit, specified_height, specified_width, BoxEdges, EPSILON,
};

/// Bound on the iterations of the flexible length resolution loop (each
/// iteration freezes at least one item, so this only caps pathological
/// lines; the remaining items are then frozen at their clamped sizes).
const MAX_FREEZE_ITERATIONS: usize = 64;

/// Largest flex factor taken into account.
const MAX_FACTOR: f32 = 1.0e6;

/// Direction and wrapping of a flex container.
#[derive(Copy, Clone, Debug)]
pub(super) struct Axes {
    /// Main axis horizontal.
    pub row: bool,
    /// `row-reverse` / `column-reverse`.
    pub reverse: bool,
    /// Multi-line container.
    pub wrap: bool,
    /// `wrap-reverse`.
    pub wrap_reverse: bool,
}

impl Axes {
    pub fn of(style: &ComputedStyle) -> Self {
        Axes {
            row: style.flex_direction.is_row(),
            reverse: style.flex_direction.is_reverse(),
            wrap: style.flex_wrap != FlexWrap::NoWrap,
            wrap_reverse: style.flex_wrap == FlexWrap::WrapReverse,
        }
    }
}

/// Sanitised flex factor.
fn factor(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, MAX_FACTOR)
    } else {
        0.0
    }
}

/// `overflow` other than `visible`/`clip`: no automatic minimum size.
fn is_scroll_container(style: &ComputedStyle) -> bool {
    let scrolls = |o: Overflow| !matches!(o, Overflow::Visible | Overflow::Clip);
    scrolls(style.overflow_x) || scrolls(style.overflow_y)
}

/// Main-axis and cross-axis gaps of container `style` with content box `cb`.
pub(super) fn gaps(style: &ComputedStyle, cb: Cb, row: bool) -> (f32, f32) {
    let column_gap = sane_size(lp(&style.column_gap, cb.width));
    // A percentage of an indefinite height counts as zero.
    let row_gap = sane_size(match cb.height {
        Some(h) => lp(&style.row_gap, h),
        None => sane(style.row_gap.px),
    });
    if row {
        (column_gap, row_gap)
    } else {
        (row_gap, column_gap)
    }
}

/// One flex item while the algorithm runs. All sizes are content-box sizes
/// unless named `outer` (margin box).
struct Item<'t> {
    b: &'t BoxNode,
    edges: BoxEdges,
    /// Main-axis margins (main-start, main-end); auto margins count as 0
    /// until free space is distributed to them.
    m_main: (f32, f32),
    auto_main: (bool, bool),
    /// Cross-axis margins (cross-start, cross-end).
    m_cross: (f32, f32),
    auto_cross: (bool, bool),
    pb_main: f32,
    pb_cross: f32,
    base: f32,
    hyp: f32,
    min_main: f32,
    max_main: f32,
    grow: f32,
    shrink: f32,
    target: f32,
    frozen: bool,
    violation: f32,
    align: AlignItems,
    /// Stretched to its line's cross size.
    stretch: bool,
    /// Cross size: hypothetical, then final.
    cross: f32,
    /// The cross size is final (stretched to a definite line up front).
    cross_final: bool,
    min_cross: f32,
    max_cross: f32,
    /// First baseline, from the cross-start edge of the margin box.
    baseline: Option<f32>,
    /// Content height measured at the item's cross size (column containers).
    content_main: Option<f32>,
    /// Box laid out while measuring, kept for reuse.
    laid: Option<Laid>,
    /// Offset of the margin box's main-start edge from the line's main start.
    main_pos: f32,
}

impl<'t> Item<'t> {
    fn new(b: &'t BoxNode, ax: Axes, cb: Cb, container: &ComputedStyle) -> Self {
        let style = &b.style;
        let edges = box_edges(style, cb.width);
        let m = &edges.margin;
        let ((m_main, auto_main), (m_cross, auto_cross)) = {
            let h = ((m.left, m.right), (edges.auto_left, edges.auto_right));
            let v = ((m.top, m.bottom), (edges.auto_top, edges.auto_bottom));
            let flip = |p: ((f32, f32), (bool, bool)), rev: bool| {
                if rev {
                    ((p.0 .1, p.0 .0), (p.1 .1, p.1 .0))
                } else {
                    p
                }
            };
            if ax.row {
                (flip(h, ax.reverse), flip(v, ax.wrap_reverse))
            } else {
                (flip(v, ax.reverse), flip(h, ax.wrap_reverse))
            }
        };
        let pb_h = edges.pb_h();
        let pb_v = edges.pb_v();
        let (pb_main, pb_cross) = if ax.row { (pb_h, pb_v) } else { (pb_v, pb_h) };
        let align = match style.align_self {
            AlignItems::Auto => container.align_items,
            a => a,
        };
        let cross_auto = if ax.row {
            specified_height(style, cb.height, pb_v).is_none()
        } else {
            specified_width(style, Some(cb.width), pb_h).is_none()
        };
        let stretch = align == AlignItems::Stretch && cross_auto && !auto_cross.0 && !auto_cross.1;
        let (min_cross, max_cross) = if ax.row {
            height_limits(style, cb.height, pb_v)
        } else {
            (
                min_width(style, Some(cb.width), pb_h),
                max_width(style, Some(cb.width), pb_h),
            )
        };
        Item {
            b,
            edges,
            m_main,
            auto_main,
            m_cross,
            auto_cross,
            pb_main,
            pb_cross,
            base: 0.0,
            hyp: 0.0,
            min_main: 0.0,
            max_main: f32::INFINITY,
            grow: factor(style.flex_grow),
            shrink: factor(style.flex_shrink),
            target: 0.0,
            frozen: false,
            violation: 0.0,
            align,
            stretch,
            cross: 0.0,
            cross_final: false,
            min_cross,
            max_cross,
            baseline: None,
            content_main: None,
            laid: None,
            main_pos: 0.0,
        }
    }

    fn outer_main(&self, size: f32) -> f32 {
        size + self.pb_main + self.m_main.0 + self.m_main.1
    }

    fn outer_cross(&self) -> f32 {
        self.cross + self.pb_cross + self.m_cross.0 + self.m_cross.1
    }

    /// Physical used margins.
    fn physical_margins(&self, ax: Axes) -> Edges {
        let (ms, me) = self.m_main;
        let (cs, ce) = self.m_cross;
        // First = left (row main / column cross) or top.
        let (main_a, main_b) = if ax.reverse { (me, ms) } else { (ms, me) };
        let (cross_a, cross_b) = if ax.wrap_reverse { (ce, cs) } else { (cs, ce) };
        if ax.row {
            Edges::new(cross_a, main_b, cross_b, main_a)
        } else {
            Edges::new(main_a, cross_b, main_b, cross_a)
        }
    }

    /// Edges with the physical used margins (for laying the item out).
    fn used_edges(&self, ax: Axes) -> BoxEdges {
        let mut e = self.edges;
        e.margin = self.physical_margins(ax);
        e
    }

    /// First baseline from the margin box's cross-start edge, synthesized
    /// from the bottom of the border box when the item has no line box.
    fn baseline_offset(&self) -> f32 {
        self.baseline
            .unwrap_or(self.m_cross.0 + self.pb_cross + self.cross)
    }

    /// Whether the item takes part in baseline alignment.
    fn uses_baseline(&self, ax: Axes) -> bool {
        ax.row
            && !ax.wrap_reverse
            && self.align == AlignItems::Baseline
            && !self.auto_cross.0
            && !self.auto_cross.1
    }

    /// Specified main size (`width` for rows, `height` for columns).
    fn main_size_property(&self, ax: Axes, cb: Cb) -> Option<f32> {
        let style = &self.b.style;
        if ax.row {
            specified_width(style, Some(cb.width), self.pb_main)
        } else {
            specified_height(style, cb.height, self.pb_main)
        }
    }

    /// Definite `flex-basis` as a content-box size.
    fn definite_basis(&self, inner_main: Option<f32>) -> Option<f32> {
        let style = &self.b.style;
        match style.flex_basis {
            LengthPercentageAuto::Auto => None,
            LengthPercentageAuto::Length(l) => {
                if l.has_percent() && inner_main.is_none() {
                    // Percentage of an indefinite size: `content`.
                    return None;
                }
                let v = sane(l.resolve(inner_main.unwrap_or(0.0)));
                Some(match style.box_sizing {
                    BoxSizing::ContentBox => v.max(0.0),
                    BoxSizing::BorderBox => (v - self.pb_main).max(0.0),
                })
            }
        }
    }

    /// Cross size of an item of a column container, decided before its
    /// main size: specified width, stretched width (single-line), else
    /// fit-content.
    fn column_cross(&self, cx: &mut LayoutCx<'t>, cb: Cb, single_line: bool) -> f32 {
        let style = &self.b.style;
        let avail = cb.width - self.m_cross.0 - self.m_cross.1 - self.pb_cross;
        if let Some(w) = specified_width(style, Some(cb.width), self.pb_cross) {
            return clamp_size(w, self.min_cross, self.max_cross);
        }
        if self.stretch && single_line {
            return clamp_size(avail, self.min_cross, self.max_cross);
        }
        if self.b.is_replaced() {
            return cx.replaced_size(self.b, &self.edges, cb).0;
        }
        let i = cx.content_intrinsic(self.b);
        clamp_size(
            shrink_to_fit(i.min, i.max, avail),
            self.min_cross,
            self.max_cross,
        )
    }

    /// Content height at the item's cross size (column containers),
    /// measured once (the laid-out box is kept for reuse).
    fn column_content_height(&mut self, cx: &mut LayoutCx<'t>, ax: Axes, cb: Cb, y: f32) -> f32 {
        if let Some(h) = self.content_main {
            return h;
        }
        let h = match self.b.is_replaced() {
            true => cx.replaced_height_for_width(self.b, &self.edges, cb, self.cross),
            false => None,
        };
        let h = match h {
            Some(h) => h,
            None => {
                let e = self.used_edges(ax);
                let (m, laid) = item::measure(
                    cx,
                    self.b,
                    cb.x + e.margin.left,
                    y + e.margin.top,
                    self.cross,
                    &e,
                    cb,
                );
                if self.laid.is_none() {
                    self.laid = laid;
                } else if let Some(l) = laid {
                    l.discard(cx);
                }
                m.height
            }
        };
        self.content_main = Some(h);
        h
    }

    /// Flex base size, hypothetical main size and main-size limits.
    #[allow(clippy::too_many_arguments)]
    fn size_main(
        &mut self,
        cx: &mut LayoutCx<'t>,
        ax: Axes,
        cb: Cb,
        inner_main: Option<f32>,
        single_line: bool,
        y: f32,
    ) {
        let style = &self.b.style;
        if !ax.row {
            self.cross = self.column_cross(cx, cb, single_line);
        }
        self.max_main = if ax.row {
            max_width(style, Some(cb.width), self.pb_main)
        } else {
            max_height(style, cb.height, self.pb_main)
        };
        let spec = self.main_size_property(ax, cb);
        let base = match self.definite_basis(inner_main).or(spec) {
            Some(v) => v,
            None if ax.row => {
                if self.b.is_replaced() {
                    cx.replaced_size(self.b, &self.edges, cb).0
                } else {
                    cx.content_intrinsic(self.b).max
                }
            }
            None => self.column_content_height(cx, ax, cb, y),
        };
        self.base = sane_size(base);
        let explicit_min = if ax.row {
            &style.min_width
        } else {
            &style.min_height
        };
        self.min_main = if explicit_min.is_auto() && !is_scroll_container(style) {
            // Automatic minimum size (§4.5): the content size suggestion,
            // capped by the specified size suggestion and the max size.
            let content = if ax.row {
                cx.content_intrinsic(self.b).min
            } else {
                self.column_content_height(cx, ax, cb, y)
            };
            let content = content.min(self.max_main);
            sane_size(spec.map_or(content, |s| s.min(content)))
        } else if ax.row {
            min_width(style, Some(cb.width), self.pb_main)
        } else {
            min_height(style, cb.height, self.pb_main)
        };
        self.hyp = clamp_size(self.base, self.min_main, self.max_main);
    }
}

/// Splits items into lines (greedy, gaps included). `limit` is the
/// available main size (`INFINITY` when indefinite).
fn collect_lines(items: &[Item<'_>], wrap: bool, limit: f32, gap: f32) -> Vec<Range<usize>> {
    if items.is_empty() {
        return Vec::new();
    }
    if !wrap {
        return std::iter::once(0..items.len()).collect();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0.0f32;
    for (i, it) in items.iter().enumerate() {
        let outer = it.outer_main(it.hyp);
        if i > start && used + gap + outer > limit + EPSILON {
            lines.push(start..i);
            start = i;
            used = outer;
        } else {
            used += if i > start { gap + outer } else { outer };
        }
    }
    lines.push(start..items.len());
    lines
}

/// Sum of the outer hypothetical main sizes of a line plus its gaps.
fn line_hyp_sum(items: &[Item<'_>], gap: f32) -> f32 {
    let gaps = gap * items.len().saturating_sub(1) as f32;
    items.iter().map(|it| it.outer_main(it.hyp)).sum::<f32>() + gaps
}

/// Resolves the flexible lengths of one line (§9.7) against the available
/// main size `avail`, setting each item's `target`.
fn resolve_line(items: &mut [Item<'_>], avail: f32, gap: f32) {
    if items.is_empty() {
        return;
    }
    let gaps = gap * (items.len() - 1) as f32;
    let growing = line_hyp_sum(items, gap) < avail;
    for it in items.iter_mut() {
        it.target = it.hyp;
        let f = if growing { it.grow } else { it.shrink };
        it.frozen = f == 0.0 || (growing && it.base > it.hyp) || (!growing && it.base < it.hyp);
    }
    let free_space = |items: &[Item<'_>]| {
        avail
            - gaps
            - items
                .iter()
                .map(|it| it.outer_main(if it.frozen { it.target } else { it.base }))
                .sum::<f32>()
    };
    let initial_free = free_space(items);
    for _ in 0..MAX_FREEZE_ITERATIONS {
        if items.iter().all(|it| it.frozen) {
            break;
        }
        let mut free = free_space(items);
        let sum_factors: f32 = items
            .iter()
            .filter(|it| !it.frozen)
            .map(|it| if growing { it.grow } else { it.shrink })
            .sum();
        if sum_factors < 1.0 {
            let scaled = initial_free * sum_factors;
            if scaled.abs() < free.abs() {
                free = scaled;
            }
        }
        let sum_scaled: f32 = items
            .iter()
            .filter(|it| !it.frozen)
            .map(|it| it.shrink * it.base)
            .sum();
        for it in items.iter_mut().filter(|it| !it.frozen) {
            it.target = if !free.is_finite() || free == 0.0 {
                it.base
            } else if growing {
                if sum_factors > 0.0 {
                    it.base + free * it.grow / sum_factors
                } else {
                    it.base
                }
            } else if sum_scaled > 0.0 {
                it.base + free * (it.shrink * it.base) / sum_scaled
            } else {
                it.base
            };
        }
        let mut total = 0.0f32;
        for it in items.iter_mut().filter(|it| !it.frozen) {
            let clamped = clamp_size(it.target, it.min_main, it.max_main);
            it.violation = clamped - sane(it.target);
            total += it.violation;
            it.target = clamped;
        }
        for it in items.iter_mut().filter(|it| !it.frozen) {
            it.frozen = if total.abs() < EPSILON {
                true
            } else if total > 0.0 {
                it.violation > 0.0
            } else {
                it.violation < 0.0
            };
        }
    }
    for it in items.iter_mut() {
        it.target = clamp_size(it.target, it.min_main, it.max_main);
    }
}

/// `(offset before the first item, extra space between items)` for
/// `justify-content` / `align-content` with `free` space and `n` items.
fn distribute(mode: JustifyContent, free: f32, n: usize) -> (f32, f32) {
    let n_f = n.max(1) as f32;
    match mode {
        JustifyContent::FlexStart | JustifyContent::Stretch => (0.0, 0.0),
        JustifyContent::FlexEnd => (free, 0.0),
        JustifyContent::Center => (free / 2.0, 0.0),
        JustifyContent::SpaceBetween => {
            if free > 0.0 && n > 1 {
                (0.0, free / (n - 1) as f32)
            } else {
                (0.0, 0.0)
            }
        }
        JustifyContent::SpaceAround => {
            if free > 0.0 {
                (free / (2.0 * n_f), free / n_f)
            } else {
                (free / 2.0, 0.0)
            }
        }
        JustifyContent::SpaceEvenly => {
            if free > 0.0 {
                (free / (n_f + 1.0), free / (n_f + 1.0))
            } else {
                (free / 2.0, 0.0)
            }
        }
    }
}

/// Main-axis auto margins and `justify-content` for one line: sets each
/// item's `main_pos`.
fn place_main(items: &mut [Item<'_>], avail: f32, gap: f32, justify: JustifyContent) {
    let n = items.len();
    if n == 0 {
        return;
    }
    let used = items.iter().map(|it| it.outer_main(it.target)).sum::<f32>() + gap * (n - 1) as f32;
    let free = avail - used;
    let autos: usize = items
        .iter()
        .map(|it| usize::from(it.auto_main.0) + usize::from(it.auto_main.1))
        .sum();
    let (offset, between) = if free > 0.0 && autos > 0 {
        let each = free / autos as f32;
        for it in items.iter_mut() {
            if it.auto_main.0 {
                it.m_main.0 += each;
            }
            if it.auto_main.1 {
                it.m_main.1 += each;
            }
        }
        (0.0, 0.0)
    } else {
        distribute(justify, free, n)
    };
    let mut pos = offset;
    for it in items.iter_mut() {
        it.main_pos = pos;
        pos += it.outer_main(it.target) + gap + between;
    }
}

/// Lays out the items of flex container `b` (see the module docs of
/// [`crate::flex`]). `limits` are the container's used min/max content
/// heights.
pub(super) fn layout<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    cb: Cb,
    y: f32,
    limits: (f32, f32),
) -> ContentLayout {
    let style = &b.style;
    let ax = Axes::of(style);
    let single_line = !ax.wrap;
    let (main_gap, cross_gap) = gaps(style, cb, ax.row);
    let cmin_h = sane_size(limits.0);
    let cmax_h = if limits.1.is_nan() {
        f32::INFINITY
    } else {
        limits.1.max(cmin_h)
    };

    let mut kids: Vec<&'t BoxNode> = Vec::with_capacity(b.children.len());
    for c in &b.children {
        if c.is_absolute() {
            cx.register_abs(c, Point::new(cb.x, y));
        } else if !c.is_text_like() && !matches!(c.kind, Kind::Inline) {
            // (Box construction wraps text runs in anonymous blocks.)
            kids.push(c);
        }
    }
    // Stable: document order is kept within an `order` value.
    kids.sort_by_key(|c| c.style.order);

    let inner_main = if ax.row { Some(cb.width) } else { cb.height };
    let mut items: Vec<Item<'t>> = kids
        .into_iter()
        .map(|k| Item::new(k, ax, cb, style))
        .collect();
    for it in &mut items {
        it.size_main(cx, ax, cb, inner_main, single_line, y);
    }

    // ---- lines and main sizes ----
    let break_limit = if ax.row {
        cb.width
    } else {
        cb.height.unwrap_or(cmax_h)
    };
    let lines = collect_lines(&items, ax.wrap, break_limit, main_gap);
    let main_size = match inner_main {
        Some(m) => m,
        None => {
            let natural = lines
                .iter()
                .filter_map(|r| items.get(r.clone()))
                .map(|l| line_hyp_sum(l, main_gap))
                .fold(0.0f32, f32::max);
            clamp_size(natural, cmin_h, cmax_h)
        }
    };
    for r in &lines {
        if let Some(line) = items.get_mut(r.clone()) {
            resolve_line(line, main_size, main_gap);
            place_main(line, main_size, main_gap, style.justify_content);
        }
    }
    let main_origin = if ax.row { cb.x } else { y };
    let main_phys = |it: &Item<'_>| {
        if ax.reverse {
            main_origin + main_size - it.main_pos - it.outer_main(it.target)
        } else {
            main_origin + it.main_pos
        }
    };

    // ---- hypothetical cross sizes ----
    let definite_cross = if ax.row { cb.height } else { Some(cb.width) };
    if ax.row {
        for it in &mut items {
            let e = it.used_edges(ax);
            let bx = main_phys(it) + e.margin.left;
            let by = y + e.margin.top;
            match definite_cross {
                Some(c) if single_line && it.stretch => {
                    // Stretched to the definite line: lay out once.
                    let cross = clamp_size(
                        c - it.m_cross.0 - it.m_cross.1 - it.pb_cross,
                        it.min_cross,
                        it.max_cross,
                    );
                    let laid = item::layout_item(cx, it.b, bx, by, it.target, Some(cross), &e, cb);
                    it.baseline =
                        crate::tree::first_baseline(&laid.lb).map(|bl| bl - laid.y + e.margin.top);
                    it.cross = cross;
                    it.cross_final = true;
                    it.laid = Some(laid);
                }
                _ => {
                    let (m, laid) = item::measure(cx, it.b, bx, by, it.target, &e, cb);
                    it.cross = m.height;
                    it.baseline = m.baseline.map(|bl| bl + e.margin.top);
                    it.laid = laid;
                }
            }
        }
    }

    // ---- line cross sizes ----
    let mut line_cross = Vec::with_capacity(lines.len());
    let mut line_above = Vec::with_capacity(lines.len());
    for r in &lines {
        let line = items.get(r.clone()).unwrap_or(&[]);
        let mut max_outer = 0.0f32;
        let mut above = 0.0f32;
        let mut below = 0.0f32;
        for it in line {
            let outer = it.outer_cross();
            if it.uses_baseline(ax) {
                let bl = it.baseline_offset();
                above = above.max(bl);
                below = below.max(outer - bl);
            } else {
                max_outer = max_outer.max(outer);
            }
        }
        let natural = sane_size(max_outer.max(above + below));
        let lc = match (single_line, definite_cross) {
            (true, Some(c)) => c,
            (true, None) => clamp_size(natural, cmin_h, cmax_h),
            (false, _) => natural,
        };
        line_cross.push(lc);
        line_above.push(above);
    }

    // ---- container cross size, align-content ----
    let n_lines = line_cross.len();
    let gaps_total = cross_gap * n_lines.saturating_sub(1) as f32;
    let total: f32 = line_cross.iter().sum::<f32>() + gaps_total;
    let (offset, between, cross_extent) = if single_line {
        (0.0, 0.0, line_cross.first().copied().unwrap_or(0.0))
    } else {
        let used = match definite_cross {
            Some(c) => c,
            None => clamp_size(total, cmin_h, cmax_h),
        };
        let free = used - total;
        if style.align_content == JustifyContent::Stretch {
            if free > 0.0 && n_lines > 0 {
                let each = free / n_lines as f32;
                for lc in &mut line_cross {
                    *lc += each;
                }
            }
            (0.0, 0.0, used)
        } else {
            let (o, bt) = distribute(style.align_content, free, n_lines);
            (o, bt, used)
        }
    };
    let cross_origin = if ax.row { y } else { cb.x };

    // ---- final cross sizes, positions and layout ----
    let mut out = ContentLayout::default();
    let mut line_pos = offset;
    for (li, r) in lines.iter().enumerate() {
        let lc = line_cross.get(li).copied().unwrap_or(0.0);
        let above = line_above.get(li).copied().unwrap_or(0.0);
        let Some(line) = items.get_mut(r.clone()) else {
            continue;
        };
        for it in line.iter_mut() {
            if it.stretch && !it.cross_final {
                it.cross = clamp_size(
                    lc - it.m_cross.0 - it.m_cross.1 - it.pb_cross,
                    it.min_cross,
                    it.max_cross,
                );
            }
            let q = if it.auto_cross.0 || it.auto_cross.1 {
                let free = lc - it.outer_cross();
                if free > 0.0 {
                    match it.auto_cross {
                        (true, true) => {
                            it.m_cross.0 += free / 2.0;
                            it.m_cross.1 += free / 2.0;
                        }
                        (true, false) => it.m_cross.0 += free,
                        _ => it.m_cross.1 += free,
                    }
                }
                0.0
            } else {
                match it.align {
                    AlignItems::FlexEnd => lc - it.outer_cross(),
                    AlignItems::Center => (lc - it.outer_cross()) / 2.0,
                    AlignItems::Baseline if it.uses_baseline(ax) => above - it.baseline_offset(),
                    _ => 0.0,
                }
            };
            let cross_start = if ax.wrap_reverse {
                cross_origin + cross_extent - line_pos - q - it.outer_cross()
            } else {
                cross_origin + line_pos + q
            };
            let main_start = main_phys(it);
            let e = it.used_edges(ax);
            let (bx, by, w, forced) = if ax.row {
                (
                    main_start + e.margin.left,
                    cross_start + e.margin.top,
                    it.target,
                    it.stretch.then_some(it.cross),
                )
            } else {
                (
                    cross_start + e.margin.left,
                    main_start + e.margin.top,
                    it.cross,
                    Some(it.target),
                )
            };
            let mut laid = item::finish_item(cx, it.b, it.laid.take(), bx, by, w, forced, &e, cb);
            laid.set_margins(e.margin);
            let Laid {
                mut lb, start, end, ..
            } = laid;
            cx.apply_relative_range(&mut lb, cb, (start, end));
            out.children.push(lb);
        }
        line_pos += lc + cross_gap + between;
    }
    out.height = sane_size(if ax.row { cross_extent } else { main_size });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_modes() {
        assert_eq!(distribute(JustifyContent::FlexEnd, 30.0, 3), (30.0, 0.0));
        assert_eq!(distribute(JustifyContent::Center, 30.0, 3), (15.0, 0.0));
        assert_eq!(
            distribute(JustifyContent::SpaceBetween, 30.0, 3),
            (0.0, 15.0)
        );
        assert_eq!(
            distribute(JustifyContent::SpaceBetween, 30.0, 1),
            (0.0, 0.0)
        );
        assert_eq!(
            distribute(JustifyContent::SpaceAround, 30.0, 3),
            (5.0, 10.0)
        );
        assert_eq!(
            distribute(JustifyContent::SpaceEvenly, 40.0, 3),
            (10.0, 10.0)
        );
        // Negative free space: space-* fall back to start / center.
        assert_eq!(
            distribute(JustifyContent::SpaceBetween, -10.0, 3),
            (0.0, 0.0)
        );
        assert_eq!(
            distribute(JustifyContent::SpaceAround, -10.0, 3),
            (-5.0, 0.0)
        );
        assert_eq!(distribute(JustifyContent::Stretch, 10.0, 2), (0.0, 0.0));
    }

    #[test]
    fn factors_are_sanitised() {
        assert_eq!(factor(f32::NAN), 0.0);
        assert_eq!(factor(-1.0), 0.0);
        assert_eq!(factor(1e30), MAX_FACTOR);
        assert_eq!(factor(2.0), 2.0);
    }
}
