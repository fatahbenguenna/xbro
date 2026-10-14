//! Grid layout for `display: grid` / `inline-grid` containers (CSS Grid 1,
//! simplified).
//!
//! # Contract
//!
//! Same as flex layout ([`crate::flex::layout_flex_contents`]): the
//! container is a [`Kind::Flex`](crate::box_tree::Kind::Flex) box whose
//! in-flow children are the grid items; [`layout_grid_contents`] gets the
//! container's content box (`cb`, top `y`) and returns the laid-out items
//! (in `order`-modified document order) and the auto content height;
//! [`grid_intrinsic`] gives the container's min/max-content widths.
//!
//! # Algorithm
//!
//! 1. **Explicit grid** from `grid-template-columns` / `-rows`: fixed
//!    lengths and percentages (of the content box; `auto` when it is
//!    indefinite), `fr`, `auto`, `min-content`, `max-content`, `minmax()`,
//!    `fit-content()`. An auto-repeat (`repeat(auto-fill | auto-fit, …)`)
//!    repeats its tracks as many times as fit in the definite size
//!    (`floor((avail + gap) / (size + gap))` for one track, each track
//!    counted at its max if definite, else its min, at least 1px; at
//!    least one repetition; one with an indefinite size). Without a
//!    template the grid has one column (and implicit rows).
//! 2. **Placement** ([`placement`]): row-major auto-placement honouring
//!    `grid-column` / `grid-row` line numbers and spans. With `auto-fit`,
//!    repeated columns (rows) left empty collapse: they are removed with
//!    their gaps.
//! 3. **Columns** are sized ([`tracks`]) against the content width with
//!    the items' minimum and min/max-content contributions (the minimum
//!    contribution — what an `auto` min track holds — is the item's
//!    min-content size unless it has a definite width, a `min-width`
//!    other than `auto`, is a scroll container, or spans several tracks
//!    one of which is flexible: `min-width: 0` or `overflow: hidden`
//!    items do not widen `1fr` columns); then items are laid out at the
//!    width of their area (auto widths stretch, specified widths and
//!    replaced elements keep their size; auto margins shrink the item to
//!    fit-content and align it) to measure their heights.
//! 4. **Rows** are sized with those heights (implicit rows and `auto` rows
//!    fit their content; fixed template rows keep their size; `fr` rows
//!    share a definite container height, else behave as `max-content`;
//!    with an indefinite height, rows shorter than `min-height` or taller
//!    than `max-height` are sized again against it). Heights include the
//!    room of table captions and of a fieldset's legend
//!    ([`item::extra_height`]); minimum contributions follow the same
//!    rules as for columns (`min-height`, scroll containers).
//! 5. `justify-content` / `align-content` (`normal`, `stretch`: `auto`
//!    tracks share the free space; `start`, `end`, `center`, `space-*`
//!    otherwise) position the tracks; items are aligned in their area with
//!    `align-self` / `align-items` (`normal`/`stretch` stretches auto
//!    heights, `start`/`end`/`center`; `baseline` behaves as `start`;
//!    auto margins absorb the free space).
//!
//! # Approximations
//!
//! `grid-auto-flow` is always `row` (sparse) and implicit tracks are
//! `auto` (no `grid-auto-rows/columns` in the computed style); named
//! lines and areas (`grid-template-areas`) are not supported (such items
//! are auto-placed); there are no implicit tracks before the explicit
//! grid; `justify-self`/`justify-items` behave as `normal`; percentage
//! heights of items behave as `auto`; `justify-content: start` stretches
//! `auto` tracks like `normal` (the computed style does not distinguish
//! them). A style with a column count but no template (`grid_columns`
//! only, built by hand) gets `repeat(n, 1fr)` columns.

mod placement;
mod tracks;

use std::ops::Range;

use xbro_style::{
    AlignItems, ComputedStyle, GridTemplate, GridTrack, JustifyContent, LengthPercentageAuto,
    MAX_GRID_TRACKS,
};

use crate::block::{Cb, ContentLayout};
use crate::box_tree::BoxNode;
use crate::cx::LayoutCx;
use crate::flex::{in_flow_items, item};
use crate::geom::Point;
use crate::intrinsic::Intrinsic;
use crate::sizing::{
    box_edges, clamp_size, height_limits, is_scroll_container, lp, min_height, min_width, sane,
    sane_size, specified_height, specified_width, width_limits, BoxEdges,
};
use placement::{Area, MAX_TRACKS};
use tracks::{fixed_cap, has_auto_minimum, size_tracks, track_of, Contribution, Space, Track};

/// Column gap (percentages against the container width).
fn column_gap(style: &ComputedStyle, width: f32) -> f32 {
    sane_size(lp(&style.column_gap, width))
}

/// Row gap (a percentage of an indefinite height counts as zero).
fn row_gap(style: &ComputedStyle, height: Option<f32>) -> f32 {
    sane_size(match height {
        Some(h) => lp(&style.row_gap, h),
        None => {
            if style.row_gap.has_percent() {
                0.0
            } else {
                sane(style.row_gap.px)
            }
        }
    })
}

/// The explicit tracks of one axis: the tracks and the range of tracks
/// generated by an `auto-fit` repeat (collapsed when empty).
struct Explicit {
    tracks: Vec<Track>,
    auto_fit: Option<Range<usize>>,
}

/// Number of repetitions of the auto-repeat of `t` in a definite size
/// `avail` (CSS Grid 1 §7.2.3.2).
fn repeat_count(t: &GridTemplate, avail: Option<f32>, gap: f32) -> usize {
    let Some(rep) = &t.auto_repeat else {
        return 0;
    };
    let k = rep.tracks.len();
    if k == 0 {
        return 0;
    }
    let others = t.tracks.len();
    let max_reps = (MAX_GRID_TRACKS.min(MAX_TRACKS).saturating_sub(others) / k).max(1);
    let Some(avail) = avail.map(sane_size) else {
        return 1;
    };
    let size = |g: &GridTrack| tracks::definite_size(g, avail).unwrap_or(0.0);
    let others_sum: f32 = t.tracks.iter().map(size).sum();
    let unit: f32 = rep.tracks.iter().map(|g| size(g).max(1.0)).sum();
    // total(n) = others + n·unit + (others_count + n·k − 1)·gap ≤ avail
    let room = avail - others_sum - (others as f32 - 1.0) * gap;
    let n = (room / (unit + k as f32 * gap)).floor();
    if n.is_finite() && n >= 1.0 {
        (n as usize).min(max_reps)
    } else {
        1
    }
}

/// Explicit tracks of an axis with template `t` (`count`: the column
/// count of a style without template), percentages and auto-repeats
/// resolved against `avail`.
fn explicit_tracks(t: Option<&GridTemplate>, count: u32, avail: Option<f32>, gap: f32) -> Explicit {
    let Some(t) = t else {
        let n = (count as usize).min(MAX_TRACKS);
        return Explicit {
            tracks: (0..n)
                .map(|_| track_of(&GridTrack::Fr(1.0), avail))
                .collect(),
            auto_fit: None,
        };
    };
    let reps = repeat_count(t, avail, gap);
    let split = t.auto_repeat_index.min(t.tracks.len());
    let mut out: Vec<Track> = Vec::with_capacity(t.tracks.len() + reps);
    let (before, after) = t.tracks.split_at(split);
    out.extend(before.iter().map(|g| track_of(g, avail)));
    let mut auto_fit = None;
    if let Some(rep) = &t.auto_repeat {
        let start = out.len();
        for _ in 0..reps {
            out.extend(rep.tracks.iter().map(|g| track_of(g, avail)));
        }
        if !rep.fill {
            auto_fit = Some(start..out.len());
        }
    }
    out.extend(after.iter().map(|g| track_of(g, avail)));
    out.truncate(MAX_TRACKS);
    Explicit {
        tracks: out,
        auto_fit,
    }
}

/// Placed items and the tracks of both axes.
struct Setup<'t> {
    items: Vec<&'t BoxNode>,
    areas: Vec<Area>,
    cols: Vec<Track>,
    rows: Vec<Track>,
}

/// Removes the empty tracks of an `auto-fit` range (index `axis` of the
/// areas: columns when true) and renumbers the areas.
fn collapse_auto_fit(tracks: &mut Vec<Track>, areas: &mut [Area], fit: Range<usize>, cols: bool) {
    let n = tracks.len();
    let mut used = vec![false; n];
    for a in areas.iter() {
        let (s, span) = if cols {
            (a.col, a.col_span)
        } else {
            (a.row, a.row_span)
        };
        if let Some(u) = used.get_mut(s..(s + span).min(n)) {
            u.iter_mut().for_each(|x| *x = true);
        }
    }
    let keep: Vec<bool> = (0..n)
        .map(|i| !fit.contains(&i) || used.get(i).copied().unwrap_or(true))
        .collect();
    if keep.iter().all(|&k| k) {
        return;
    }
    // New index of every old track (collapsed ones map to the next kept one).
    let mut new_index = Vec::with_capacity(n + 1);
    let mut k = 0usize;
    for &kept in &keep {
        new_index.push(k);
        if kept {
            k += 1;
        }
    }
    new_index.push(k);
    for a in areas.iter_mut() {
        let (s, span) = if cols {
            (&mut a.col, &mut a.col_span)
        } else {
            (&mut a.row, &mut a.row_span)
        };
        let end = (*s + *span).min(n);
        let ns = new_index.get(*s).copied().unwrap_or(k);
        let ne = new_index.get(end).copied().unwrap_or(k);
        *s = ns;
        *span = ne.saturating_sub(ns).max(1);
    }
    let mut i = 0;
    tracks.retain(|_| {
        let kept = keep.get(i).copied().unwrap_or(true);
        i += 1;
        kept
    });
}

/// Items, placement and tracks of grid container `b` for a content box of
/// `width` × `height` (`None`: indefinite).
fn setup<'t>(
    b: &'t BoxNode,
    width: Option<f32>,
    height: Option<f32>,
    cgap: f32,
    rgap: f32,
) -> Setup<'t> {
    let style = &b.style;
    let mut items: Vec<&'t BoxNode> = in_flow_items(b).collect();
    items.sort_by_key(|c| c.style.order);
    let ec = explicit_tracks(
        style.grid_template_columns.as_deref(),
        style.grid_columns,
        width,
        cgap,
    );
    let er = explicit_tracks(style.grid_template_rows.as_deref(), 0, height, rgap);
    let positions: Vec<_> = items
        .iter()
        .map(|i| (i.style.grid_column, i.style.grid_row))
        .collect();
    let p = placement::place(&positions, ec.tracks.len(), er.tracks.len());
    let mut areas = p.areas;
    let mut cols = ec.tracks;
    cols.resize(p.cols.max(cols.len()), Track::auto());
    let mut rows = er.tracks;
    rows.resize(p.rows.max(rows.len()), Track::auto());
    if let Some(fit) = ec.auto_fit {
        collapse_auto_fit(&mut cols, &mut areas, fit, true);
    }
    if let Some(fit) = er.auto_fit {
        collapse_auto_fit(&mut rows, &mut areas, fit, false);
    }
    Setup {
        items,
        areas,
        cols,
        rows,
    }
}

/// Whether the `auto` tracks stretch (`normal` / `stretch` distribution).
fn stretches(j: JustifyContent) -> bool {
    matches!(j, JustifyContent::FlexStart | JustifyContent::Stretch)
}

/// Start position and size of every track: tracks laid out from `start`
/// with `gap`, the free space of a definite `avail` distributed per `mode`.
fn positions(
    tracks: &[Track],
    start: f32,
    avail: Option<f32>,
    gap: f32,
    mode: JustifyContent,
) -> Vec<(f32, f32)> {
    let n = tracks.len();
    if n == 0 {
        return Vec::new();
    }
    let total = tracks.iter().map(|t| t.base).sum::<f32>() + gap * (n - 1) as f32;
    let free = avail.map_or(0.0, |a| sane_size(a - total));
    let nf = n as f32;
    let (offset, extra) = match mode {
        JustifyContent::FlexEnd => (free, 0.0),
        JustifyContent::Center => (free / 2.0, 0.0),
        JustifyContent::SpaceBetween if n > 1 => (0.0, free / (nf - 1.0)),
        JustifyContent::SpaceAround => (free / (2.0 * nf), free / nf),
        JustifyContent::SpaceEvenly => (free / (nf + 1.0), free / (nf + 1.0)),
        _ => (0.0, 0.0),
    };
    let mut x = start + offset;
    tracks
        .iter()
        .map(|t| {
            let p = (sane(x), t.base);
            x += t.base + gap + extra;
            p
        })
        .collect()
}

/// Start and size of the area spanning `span` tracks from `first`.
fn span_of(pos: &[(f32, f32)], first: usize, span: usize, fallback: f32) -> (f32, f32) {
    let last = (first + span.max(1) - 1).min(pos.len().saturating_sub(1));
    match (pos.get(first), pos.get(last)) {
        (Some(&(a, _)), Some(&(b, w))) => (a, sane_size(b + w - a)),
        _ => (fallback, 0.0),
    }
}

/// The tracks spanned from `start`.
fn spanned(tracks: &[Track], start: usize, span: usize) -> &[Track] {
    let end = start.saturating_add(span).min(tracks.len());
    tracks.get(start.min(end)..end).unwrap_or(&[])
}

/// A definite length (not `auto`, no percentage of the grid area).
fn is_fixed_length(v: &LengthPercentageAuto) -> bool {
    matches!(v, LengthPercentageAuto::Length(l) if !l.has_percent())
}

/// Minimum contribution (CSS Grid 1 §11.5) of an item along one axis,
/// whose min-content contribution is `min_content` (margin box) and whose
/// padding, borders and margins (plus captions or legend room) add up to
/// `mbp`, spanning the tracks `ts` with `inner` gaps between them: its
/// min-content contribution when its size (`fixed`) is definite;
/// otherwise its outer size at its used minimum size — the `min-width` /
/// `min-height` (`min`) when not `auto`, else the automatic minimum size
/// (§6.6): none for a scroll container or an item spanning several tracks
/// with a flexible one (or no `auto` min), otherwise its content size
/// (capped by the total of the spanned tracks when they all have a fixed
/// max).
fn minimum_contribution(
    style: &ComputedStyle,
    fixed: bool,
    min: Option<f32>,
    min_content: f32,
    mbp: f32,
    ts: &[Track],
    inner: f32,
) -> f32 {
    if fixed {
        return min_content;
    }
    let used = match min {
        Some(m) => m,
        None if !is_scroll_container(style) && has_auto_minimum(ts) => {
            let content = sane_size(min_content - mbp);
            match fixed_cap(ts, inner) {
                Some(cap) => content.min(sane_size(cap - mbp)),
                None => content,
            }
        }
        None => 0.0,
    };
    sane_size(used + mbp)
}

/// Column contributions (margin-box minimum, min-content and max-content
/// widths) of the items, `gap` between columns.
fn column_contributions<'t>(cx: &mut LayoutCx<'t>, s: &Setup<'t>, gap: f32) -> Vec<Contribution> {
    s.items
        .iter()
        .zip(&s.areas)
        .map(|(&it, a)| {
            let o = cx.outer_intrinsic(it);
            let st = &it.style;
            let ts = spanned(&s.cols, a.col, a.col_span);
            let edges = box_edges(st, 0.0);
            let pb = edges.pb_h();
            let mbp = pb + edges.margin.left + edges.margin.right;
            let min = (!st.min_width.is_auto()).then(|| min_width(st, None, pb));
            let inner = gap * ts.len().saturating_sub(1) as f32;
            Contribution {
                start: a.col,
                span: a.col_span,
                minimum: minimum_contribution(
                    st,
                    is_fixed_length(&st.width),
                    min,
                    o.min,
                    mbp,
                    ts,
                    inner,
                ),
                min: o.min,
                max: o.max,
            }
        })
        .collect()
}

/// An item measured at the width of its area.
struct Measured<'t> {
    b: &'t BoxNode,
    edges: BoxEdges,
    cb: Cb,
    /// Border-box x.
    x: f32,
    /// Content width.
    w: f32,
    /// Outer (margin-box) height with an auto height.
    outer: f32,
    /// Captions or legend room ([`item::extra_height`]), in `outer`.
    extra: f32,
    laid: Option<item::Laid>,
}

/// Lays out the items of grid container `b` (see the module docs).
pub(crate) fn layout_grid_contents<'t>(
    cx: &mut LayoutCx<'t>,
    b: &'t BoxNode,
    cb: Cb,
    y: f32,
    limits: (f32, f32),
) -> ContentLayout {
    let style = &b.style;
    for c in b.children.iter().filter(|c| c.is_absolute()) {
        cx.register_abs(c, Point::new(cb.x, y));
    }
    let cgap = column_gap(style, cb.width);
    let rgap = row_gap(style, cb.height);
    let mut s = setup(b, Some(cb.width), cb.height, cgap, rgap);
    let mut out = ContentLayout::default();

    // ---- columns ----
    let contribs = column_contributions(cx, &s, cgap);
    size_tracks(
        &mut s.cols,
        &contribs,
        Space::Definite(cb.width),
        cgap,
        stretches(style.justify_content),
    );
    let col_pos = positions(&s.cols, cb.x, Some(cb.width), cgap, style.justify_content);

    // ---- measure the items at their area widths ----
    let mut measured: Vec<Measured<'t>> = Vec::with_capacity(s.items.len());
    for (&it, a) in s.items.iter().zip(&s.areas) {
        let (ax, aw) = span_of(&col_pos, a.col, a.col_span, cb.x);
        let icb = Cb {
            x: ax,
            width: aw,
            height: None,
        };
        let st = &it.style;
        let edges = box_edges(st, aw);
        let pb_h = edges.pb_h();
        let avail = sane_size(aw - edges.margin.left - edges.margin.right - pb_h);
        let (min_w, max_w) = width_limits(st, Some(aw), pb_h);
        let auto_margins = edges.auto_left || edges.auto_right;
        let w = if it.is_replaced() {
            cx.replaced_size(it, &edges, icb).0
        } else if let Some(w) = specified_width(st, Some(aw), pb_h) {
            clamp_size(w, min_w, max_w)
        } else if auto_margins {
            cx.shrink_width(it, &edges, icb).0
        } else {
            clamp_size(avail, min_w, max_w)
        };
        let free = avail - w;
        let dx = match (edges.auto_left, edges.auto_right) {
            (true, true) if free > 0.0 => free / 2.0,
            (true, false) if free > 0.0 => free,
            _ => 0.0,
        };
        let x = ax + edges.margin.left + sane(dx);
        let (m, laid) = item::measure(cx, it, x, y + edges.margin.top, w, &edges, icb);
        let outer = m.height + edges.pb_v() + edges.margin.top + edges.margin.bottom + m.extra;
        measured.push(Measured {
            b: it,
            edges,
            cb: icb,
            x,
            w,
            outer: sane_size(outer),
            extra: m.extra,
            laid,
        });
    }

    // ---- rows ----
    let row_contribs: Vec<Contribution> = measured
        .iter()
        .zip(&s.areas)
        .map(|(m, a)| {
            let st = &m.b.style;
            let e = &m.edges;
            let pb = e.pb_v();
            let ts = spanned(&s.rows, a.row, a.row_span);
            let min = (!st.min_height.is_auto()).then(|| min_height(st, None, pb));
            Contribution {
                start: a.row,
                span: a.row_span,
                minimum: minimum_contribution(
                    st,
                    is_fixed_length(&st.height),
                    min,
                    m.outer,
                    pb + e.margin.top + e.margin.bottom + m.extra,
                    ts,
                    rgap * ts.len().saturating_sub(1) as f32,
                ),
                min: m.outer,
                max: m.outer,
            }
        })
        .collect();
    let row_space = cb.height.map_or(Space::MaxContent, Space::Definite);
    let explicit_rows = s.rows.clone();
    size_tracks(
        &mut s.rows,
        &row_contribs,
        row_space,
        rgap,
        stretches(style.align_content),
    );
    // An indefinite height: rows that end up shorter than `min-height`
    // (or taller than `max-height`) are sized again against that height
    // (CSS Grid 1 §12.7.1, §12.8), so `fr` and `auto` rows fill it.
    let mut rows_avail = cb.height;
    if cb.height.is_none() {
        let total = s.rows.iter().map(|t| t.base).sum::<f32>()
            + rgap * s.rows.len().saturating_sub(1) as f32;
        let (min_h, max_h) = (sane_size(limits.0), limits.1);
        let bound = if total < min_h - 0.01 {
            Some(min_h)
        } else if max_h.is_finite() && total > max_h + 0.01 {
            Some(sane_size(max_h))
        } else {
            None
        };
        if let Some(h) = bound {
            s.rows = explicit_rows;
            size_tracks(
                &mut s.rows,
                &row_contribs,
                Space::Definite(h),
                rgap,
                stretches(style.align_content),
            );
            rows_avail = Some(h);
        }
    }
    let row_pos = positions(&s.rows, y, rows_avail, rgap, style.align_content);

    // ---- final layout and alignment of the items ----
    for (m, a) in measured.into_iter().zip(&s.areas) {
        let (ry, rh) = span_of(&row_pos, a.row, a.row_span, y);
        let e = &m.edges;
        let align = match m.b.style.align_self {
            AlignItems::Auto => style.align_items,
            al => al,
        };
        let stretch = align == AlignItems::Stretch
            && !m.b.is_replaced()
            && specified_height(&m.b.style, None, e.pb_v()).is_none()
            && !e.auto_top
            && !e.auto_bottom;
        let free = rh - m.outer;
        let (forced, dy) = if stretch {
            let (min_h, max_h) = height_limits(&m.b.style, None, e.pb_v());
            let h = clamp_size(
                rh - e.margin.top - e.margin.bottom - e.pb_v() - m.extra,
                min_h,
                max_h,
            );
            (Some(h), 0.0)
        } else {
            let dy = match (e.auto_top, e.auto_bottom) {
                (true, true) => free / 2.0,
                (true, false) => free,
                (false, true) => 0.0,
                _ => match align {
                    AlignItems::FlexEnd => free,
                    AlignItems::Center => free / 2.0,
                    _ => 0.0,
                },
            };
            (None, sane(dy))
        };
        let by = ry + e.margin.top + dy;
        let laid = item::finish_item(cx, m.b, m.laid, m.x, by, m.w, forced, e, m.cb);
        let item::Laid {
            mut lb, start, end, ..
        } = laid;
        cx.apply_relative_range(&mut lb, m.cb, (start, end));
        out.children.push(lb);
    }
    let rows_total = match row_pos.last() {
        Some(&(last, h)) => sane_size(last + h - y),
        None => 0.0,
    };
    out.height = rows_total;
    out
}

/// Min/max-content widths of the content box of grid container `b`:
/// its columns sized under a min-content and a max-content constraint
/// (auto-repeats repeat once; percentages behave as `auto`).
pub(crate) fn grid_intrinsic<'t>(cx: &mut LayoutCx<'t>, b: &'t BoxNode) -> Intrinsic {
    let style = &b.style;
    let gap = if style.column_gap.has_percent() {
        0.0
    } else {
        sane_size(style.column_gap.px)
    };
    let s = setup(b, None, None, gap, 0.0);
    let contribs = column_contributions(cx, &s, gap);
    let n = s.cols.len();
    let gaps = gap * n.saturating_sub(1) as f32;
    let mut min_cols = s.cols.clone();
    size_tracks(&mut min_cols, &contribs, Space::MinContent, gap, false);
    let mut max_cols = s.cols;
    size_tracks(&mut max_cols, &contribs, Space::MaxContent, gap, false);
    let sum = |t: &[Track]| t.iter().map(|t| t.base).sum::<f32>();
    Intrinsic::new(sum(&min_cols) + gaps, sum(&max_cols) + gaps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_style::{AutoRepeat, LengthPercentage};

    fn px(v: f32) -> GridTrack {
        GridTrack::Length(LengthPercentage::px(v))
    }

    fn minmax(a: GridTrack, b: GridTrack) -> GridTrack {
        GridTrack::MinMax(Box::new(a), Box::new(b))
    }

    fn auto_repeat(fill: bool, tracks: Vec<GridTrack>, before: Vec<GridTrack>) -> GridTemplate {
        GridTemplate {
            auto_repeat_index: before.len(),
            tracks: before,
            auto_repeat: Some(AutoRepeat { fill, tracks }),
        }
    }

    #[test]
    fn auto_repeat_counts() {
        let t = auto_repeat(true, vec![minmax(px(200.0), GridTrack::Fr(1.0))], vec![]);
        // floor((avail + gap) / (200 + gap)), at least 1.
        assert_eq!(repeat_count(&t, Some(976.0), 16.0), 4);
        // Four tracks need 4 × 200 + 3 × 16 = 848px.
        assert_eq!(repeat_count(&t, Some(848.0), 16.0), 4);
        assert_eq!(repeat_count(&t, Some(847.0), 16.0), 3);
        assert_eq!(repeat_count(&t, Some(100.0), 16.0), 1);
        assert_eq!(repeat_count(&t, None, 16.0), 1);
        // Other fixed tracks take their room first.
        let t = auto_repeat(true, vec![px(100.0)], vec![px(250.0)]);
        assert_eq!(repeat_count(&t, Some(600.0), 10.0), 3);
        // Zero-sized tracks count as 1px; the count stays bounded.
        let t = auto_repeat(false, vec![px(0.0)], vec![]);
        assert_eq!(repeat_count(&t, Some(1.0e7), 0.0), MAX_TRACKS);
        assert_eq!(repeat_count(&t, Some(f32::NAN), f32::NAN), 1);
    }

    #[test]
    fn auto_fit_collapses_empty_tracks() {
        let mut tracks = vec![Track::auto(); 5];
        let mut areas = vec![
            Area {
                col: 0,
                col_span: 1,
                row: 0,
                row_span: 1,
            },
            Area {
                col: 3,
                col_span: 2,
                row: 0,
                row_span: 1,
            },
        ];
        collapse_auto_fit(&mut tracks, &mut areas, 0..4, true);
        // Tracks 1 and 2 are empty: gone.
        assert_eq!(tracks.len(), 3);
        assert_eq!((areas[1].col, areas[1].col_span), (1, 2));
    }

    #[test]
    fn track_positions_and_content_distribution() {
        let tracks = [Track::fixed(100.0), Track::fixed(50.0)];
        let p = positions(&tracks, 10.0, Some(400.0), 10.0, JustifyContent::FlexStart);
        assert_eq!(p, vec![(10.0, 100.0), (120.0, 50.0)]);
        let p = positions(&tracks, 0.0, Some(400.0), 10.0, JustifyContent::Center);
        assert_eq!(p[0].0, 120.0);
        let p = positions(
            &tracks,
            0.0,
            Some(400.0),
            10.0,
            JustifyContent::SpaceBetween,
        );
        assert_eq!(p[1].0, 350.0);
        assert_eq!(span_of(&p, 0, 2, 0.0), (0.0, 400.0));
        assert_eq!(span_of(&p, 7, 1, 3.0), (3.0, 0.0));
    }
}
