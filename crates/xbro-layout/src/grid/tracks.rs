//! Track sizing (CSS Grid 1 §11.4–§11.8), shared by columns and rows.
//!
//! Each track has a min and a max sizing function ([`MinSizing`],
//! [`MaxSizing`]). The algorithm ([`size_tracks`]) works on the items'
//! contributions in the axis ([`Contribution`]: the minimum contribution
//! and the min-content and max-content contributions of their margin
//! boxes):
//!
//! 1. **Initialise** base sizes (fixed mins, else 0) and growth limits
//!    (fixed maxes, else unset/infinite).
//! 2. **Items spanning one non-flexible track** (§11.5 step 2) set its
//!    base size (`auto` min: the minimum contribution, or under an
//!    intrinsic constraint the min-content contribution limited by a
//!    fixed max and floored by the minimum contribution; `min-content` /
//!    `max-content` mins: those contributions) and its growth limit
//!    (`min-content` max: min-content; `auto`/`max-content`: max-content;
//!    `fit-content(L)`: max-content clamped to `L`).
//! 3. **Items spanning several non-flexible tracks**, by increasing span
//!    (§11.5 step 3): their contributions are distributed
//!    ([`Distributor::run`], §11.5.1) to the base sizes (intrinsic,
//!    content-based and max-content minimums) then to the growth limits
//!    (intrinsic and max-content maximums) of the tracks they span: the
//!    space is shared equally by the affected tracks up to their growth
//!    limits first (so a track without items takes what a track already
//!    at its limit cannot), then beyond them, preferably to tracks with
//!    an intrinsic (or max-content) max. Planned increases of the items
//!    of a span group are combined with `max`, so the order of the items
//!    does not matter.
//! 4. **Items crossing flexible tracks** (§11.5 step 4) grow the base
//!    sizes of the flexible tracks they span with an intrinsic min, in
//!    proportion to their flex factors (`1fr` never shrinks below an
//!    item's minimum contribution: `min-width: 0`, scroll containers and
//!    items spanning several tracks with a flexible one have none, see
//!    `grid::column_minimum`). Unset growth limits become the base size.
//! 5. **Maximise**: positive free space grows the base sizes of
//!    non-flexible tracks equally up to their growth limits (all of them
//!    under a max-content constraint, none under a min-content one).
//! 6. **Flexible tracks**: with a definite size, the leftover space is
//!    divided by the sum of the flex factors (at least 1), tracks whose
//!    base size is larger than their share being treated as inflexible;
//!    under a max-content constraint the fr size is the largest of the
//!    flexible tracks' base sizes per fr and of what the items crossing
//!    them need per fr (§11.7.1).
//! 7. **Stretch `auto` tracks** (`justify-content` / `align-content`
//!    `normal`/`stretch`): remaining definite free space is shared equally
//!    by the tracks whose max sizing function is `auto`.
//!
//! Hostile grids stay bounded: the distribution work (items × spanned
//! tracks) has a budget beyond which the remaining items no longer grow
//! the tracks.

use std::ops::Range;

use xbro_style::{GridTrack, LengthPercentage};

use crate::sizing::{sane, sane_size, EPSILON};

/// Largest number of (item, spanned track) visits of the distribution
/// steps of one [`size_tracks`] call.
const WORK_BUDGET: usize = 8_000_000;

/// Min track sizing function.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum MinSizing {
    Fixed(f32),
    MinContent,
    MaxContent,
    Auto,
}

/// Max track sizing function.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum MaxSizing {
    Fixed(f32),
    MinContent,
    MaxContent,
    Auto,
    /// `fit-content(L)`.
    FitContent(f32),
    /// `<flex>` factor (`fr`).
    Flex(f32),
}

impl MinSizing {
    fn is_intrinsic(self) -> bool {
        !matches!(self, MinSizing::Fixed(_))
    }
}

impl MaxSizing {
    fn is_intrinsic(self) -> bool {
        matches!(
            self,
            MaxSizing::MinContent
                | MaxSizing::MaxContent
                | MaxSizing::Auto
                | MaxSizing::FitContent(_)
        )
    }

    /// A max-content maximum (`auto` and `fit-content()` included, the
    /// latter up to its argument).
    fn is_max_content(self) -> bool {
        matches!(
            self,
            MaxSizing::MaxContent | MaxSizing::Auto | MaxSizing::FitContent(_)
        )
    }

    fn flex(self) -> Option<f32> {
        match self {
            MaxSizing::Flex(f) => Some(f),
            _ => None,
        }
    }

    /// The fixed size limiting the track: a fixed max or the argument of
    /// `fit-content()`.
    fn fixed_limit(self) -> Option<f32> {
        match self {
            MaxSizing::Fixed(v) | MaxSizing::FitContent(v) => Some(v),
            _ => None,
        }
    }
}

/// A track: its sizing functions and, after [`size_tracks`], its size.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Track {
    pub min: MinSizing,
    pub max: MaxSizing,
    /// Base size: the used size once sizing is done.
    pub base: f32,
    /// Growth limit (`INFINITY` while unset).
    limit: f32,
}

impl Track {
    pub fn new(min: MinSizing, max: MaxSizing) -> Self {
        Track {
            min,
            max,
            base: 0.0,
            limit: f32::INFINITY,
        }
    }

    /// An implicit track (`grid-auto-rows/columns: auto`).
    pub fn auto() -> Self {
        Self::new(MinSizing::Auto, MaxSizing::Auto)
    }

    /// A fixed track of `size` px (already sized).
    #[cfg(test)]
    pub fn fixed(size: f32) -> Self {
        let s = sane_size(size);
        Track {
            base: s,
            ..Self::new(MinSizing::Fixed(s), MaxSizing::Fixed(s))
        }
    }
}

/// Whether an item spanning `ts` has an automatic minimum size (CSS Grid
/// 1 §6.6) — when it is not a scroll container and its min size is
/// `auto`: it spans a track with an `auto` min, and no flexible track
/// unless it spans only one track.
pub(crate) fn has_auto_minimum(ts: &[Track]) -> bool {
    ts.iter().any(|t| t.min == MinSizing::Auto)
        && (ts.len() == 1 || ts.iter().all(|t| t.max.flex().is_none()))
}

/// Sum of the fixed limits ([`MaxSizing::fixed_limit`]) of `ts` plus the
/// `inner` gaps between them, when all of them have one.
pub(crate) fn fixed_cap(ts: &[Track], inner: f32) -> Option<f32> {
    ts.iter().try_fold(sane_size(inner), |acc, t| {
        t.max.fixed_limit().map(|v| acc + v)
    })
}

/// `v` limited by `cap` (when some), floored by `floor`.
fn limited(v: f32, cap: Option<f32>, floor: f32) -> f32 {
    cap.map_or(v, |c| v.min(c)).max(floor)
}

/// Resolves a `<length-percentage>` against `basis` (`None` when it is a
/// percentage of an indefinite size).
fn resolve(l: &LengthPercentage, basis: Option<f32>) -> Option<f32> {
    if l.has_percent() {
        basis.map(|b| sane_size(l.resolve(sane(b))))
    } else {
        Some(sane_size(l.px))
    }
}

/// The track of a computed track sizing function; percentages resolve
/// against `basis` (treated as `auto` when it is indefinite).
pub(crate) fn track_of(t: &GridTrack, basis: Option<f32>) -> Track {
    let min = |t: &GridTrack| match t {
        GridTrack::Length(l) => resolve(l, basis).map_or(MinSizing::Auto, MinSizing::Fixed),
        GridTrack::MinContent => MinSizing::MinContent,
        GridTrack::MaxContent => MinSizing::MaxContent,
        _ => MinSizing::Auto,
    };
    let max = |t: &GridTrack| match t {
        GridTrack::Length(l) => resolve(l, basis).map_or(MaxSizing::Auto, MaxSizing::Fixed),
        GridTrack::Fr(f) => MaxSizing::Flex(sane_size(*f)),
        GridTrack::MinContent => MaxSizing::MinContent,
        GridTrack::MaxContent => MaxSizing::MaxContent,
        GridTrack::FitContent(l) => {
            resolve(l, basis).map_or(MaxSizing::MaxContent, MaxSizing::FitContent)
        }
        GridTrack::Auto | GridTrack::MinMax(..) => MaxSizing::Auto,
    };
    match t {
        GridTrack::MinMax(a, b) => Track::new(min(a), max(b)),
        GridTrack::Fr(_) | GridTrack::FitContent(_) => Track::new(MinSizing::Auto, max(t)),
        _ => Track::new(min(t), max(t)),
    }
}

/// Size of a track sizing function for the auto-repeat count (CSS Grid 1
/// §7.2.3.2): its max if definite, else its min if definite; `None` when
/// neither is.
pub(crate) fn definite_size(t: &GridTrack, basis: f32) -> Option<f32> {
    let fixed = |t: &GridTrack| match t {
        GridTrack::Length(l) => resolve(l, Some(basis)),
        _ => None,
    };
    match t {
        GridTrack::MinMax(a, b) => fixed(b).or_else(|| fixed(a)),
        _ => fixed(t),
    }
}

/// An item's contribution to the tracks it spans (margin-box sizes).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Contribution {
    pub start: usize,
    pub span: usize,
    /// Minimum contribution (CSS Grid 1 §11.5): the size at the item's
    /// used minimum size, what an `auto` min track must hold under a
    /// definite size.
    pub minimum: f32,
    /// Min-content and max-content contributions.
    pub min: f32,
    pub max: f32,
}

impl Contribution {
    /// A contribution whose minimum contribution is its min-content one.
    #[cfg(test)]
    pub fn new(start: usize, span: usize, min: f32, max: f32) -> Self {
        Contribution {
            start,
            span,
            minimum: min,
            min,
            max,
        }
    }
}

/// The constraint the tracks are sized under.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Space {
    /// A definite size of the grid container's content box.
    Definite(f32),
    MinContent,
    MaxContent,
}

/// A sanitised contribution and the range of tracks it spans.
struct Span {
    range: Range<usize>,
    minimum: f32,
    min: f32,
    max: f32,
}

/// Which size of the tracks a distribution grows.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Target {
    Base,
    /// Growth limits; `mark`: tracks whose limit goes from infinite to
    /// finite are "infinitely growable" for the next step (§11.5 3.5).
    Limit {
        mark: bool,
    },
}

/// The tracks that keep growing once every affected track reached its
/// limit (§11.5.1 "distribute space beyond limits").
#[derive(Copy, Clone, Debug, PartialEq)]
enum Beyond {
    /// Those with an intrinsic max (else all the affected tracks).
    IntrinsicMax,
    /// Those with a max-content max, `fit-content()` ones up to their
    /// argument (else all the affected tracks).
    MaxContentMax,
    /// All the affected tracks, `fit-content()` ones up to their argument.
    AllCapFit,
    /// All the affected tracks.
    All,
}

/// Distribution of extra space across spanned tracks (§11.5.1), with
/// buffers reused across the steps of one [`size_tracks`] call.
struct Distributor {
    gap: f32,
    /// Planned increase of each track, and the tracks having one.
    planned: Vec<f32>,
    touched: Vec<bool>,
    touched_list: Vec<usize>,
    /// "Infinitely growable" tracks (§11.5 3.5–3.6), and their list.
    growable: Vec<bool>,
    growable_list: Vec<usize>,
    budget: usize,
    affected: Vec<usize>,
    inc: Vec<f32>,
    room: Vec<f32>,
    eligible: Vec<bool>,
}

impl Distributor {
    fn new(n: usize, gap: f32) -> Self {
        Distributor {
            gap,
            planned: vec![0.0; n],
            touched: vec![false; n],
            touched_list: Vec::new(),
            growable: vec![false; n],
            growable_list: Vec::new(),
            budget: WORK_BUDGET,
            affected: Vec::new(),
            inc: Vec::new(),
            room: Vec::new(),
            eligible: Vec::new(),
        }
    }

    /// Spends the work of visiting `n` tracks; false when the budget is
    /// exhausted.
    fn spend(&mut self, n: usize) -> bool {
        match self.budget.checked_sub(n) {
            Some(left) => {
                self.budget = left;
                true
            }
            None => {
                self.budget = 0;
                false
            }
        }
    }

    /// Collects the tracks of `range` selected by `affected` and marks
    /// them touched; false when there is none.
    fn collect(
        &mut self,
        tracks: &[Track],
        range: Range<usize>,
        affected: &impl Fn(&Track) -> bool,
    ) -> bool {
        self.affected.clear();
        for i in range {
            if tracks.get(i).is_some_and(affected) {
                self.affected.push(i);
                if let Some(t) = self.touched.get_mut(i) {
                    if !*t {
                        *t = true;
                        self.touched_list.push(i);
                    }
                }
            }
        }
        !self.affected.is_empty()
    }

    /// Records the item-incurred increases `inc` of the affected tracks.
    fn plan(&mut self) {
        for (&i, &v) in self.affected.iter().zip(&self.inc) {
            if let Some(p) = self.planned.get_mut(i) {
                *p = p.max(v);
            }
        }
    }

    /// Adds the planned increases to the targeted sizes and resets them.
    fn apply(&mut self, tracks: &mut [Track], target: Target) {
        for i in self.touched_list.drain(..) {
            let p = self.planned.get_mut(i).map_or(0.0, std::mem::take);
            if let Some(t) = self.touched.get_mut(i) {
                *t = false;
            }
            let Some(t) = tracks.get_mut(i) else {
                continue;
            };
            match target {
                Target::Base => t.base = sane_size(t.base + p),
                Target::Limit { mark } => {
                    if t.limit.is_finite() {
                        t.limit = sane_size(t.limit + p);
                    } else {
                        t.limit = sane_size(t.base + p);
                        if mark {
                            if let Some(g) = self.growable.get_mut(i) {
                                *g = true;
                                self.growable_list.push(i);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Forgets the "infinitely growable" marks.
    fn clear_growable(&mut self) {
        for i in self.growable_list.drain(..) {
            if let Some(g) = self.growable.get_mut(i) {
                *g = false;
            }
        }
    }

    /// Distributes the contributions `size(item, spanned tracks, inner
    /// gaps)` of the items of `group` (none crossing a flexible track) to
    /// the `target` sizes of the tracks selected by `affected` (§11.5.1).
    fn run(
        &mut self,
        tracks: &mut [Track],
        group: &[&Span],
        target: Target,
        affected: impl Fn(&Track) -> bool,
        size: impl Fn(&Span, &[Track], f32) -> f32,
        beyond: Beyond,
    ) {
        let cur = |t: &Track| match target {
            Target::Base => t.base,
            Target::Limit { .. } => {
                if t.limit.is_finite() {
                    t.limit
                } else {
                    t.base
                }
            }
        };
        for s in group {
            let r = s.range.clone();
            if !self.spend(r.len()) {
                break;
            }
            if !self.collect(tracks, r.clone(), &affected) {
                continue;
            }
            let Some(ts) = tracks.get(r.clone()) else {
                continue;
            };
            let inner = self.gap * r.len().saturating_sub(1) as f32;
            let have = ts.iter().map(cur).sum::<f32>() + inner;
            let space = sane(size(s, ts, inner)) - have;
            if space <= EPSILON {
                continue;
            }
            // Up to the limits.
            let k = self.affected.len();
            self.inc.clear();
            self.inc.resize(k, 0.0);
            self.room.clear();
            self.eligible.clear();
            for &i in &self.affected {
                let Some(t) = tracks.get(i) else {
                    self.room.push(0.0);
                    self.eligible.push(false);
                    continue;
                };
                let limit = match target {
                    Target::Base => {
                        let l = t.limit;
                        t.max.fit_limit_or(l)
                    }
                    Target::Limit { .. } => {
                        let growable = self.growable.get(i).copied().unwrap_or(false);
                        if t.limit.is_finite() && !growable {
                            t.limit
                        } else if let MaxSizing::FitContent(f) = t.max {
                            f
                        } else {
                            f32::INFINITY
                        }
                    }
                };
                self.room.push((limit - cur(t)).max(0.0));
                self.eligible.push(true);
            }
            let mut left = water_fill(&mut self.inc, &self.room, &self.eligible, space);
            // Beyond the limits.
            if left > EPSILON {
                let fit_room = |t: &Track, inc: f32| match t.max {
                    MaxSizing::FitContent(f) => (f - cur(t) - inc).max(0.0),
                    _ => f32::INFINITY,
                };
                let prefer = |t: &Track| match beyond {
                    Beyond::IntrinsicMax => t.max.is_intrinsic(),
                    Beyond::MaxContentMax => t.max.is_max_content(),
                    Beyond::AllCapFit | Beyond::All => true,
                };
                let any = self
                    .affected
                    .iter()
                    .any(|&i| tracks.get(i).is_some_and(prefer));
                self.room.clear();
                self.eligible.clear();
                for (&i, &inc) in self.affected.iter().zip(&self.inc) {
                    let t = tracks.get(i);
                    self.eligible.push(!any || t.is_some_and(prefer));
                    self.room.push(match (beyond, t) {
                        (Beyond::MaxContentMax | Beyond::AllCapFit, Some(t)) => fit_room(t, inc),
                        _ => f32::INFINITY,
                    });
                }
                left = water_fill(&mut self.inc, &self.room, &self.eligible, left);
                if left > EPSILON && beyond == Beyond::MaxContentMax {
                    // `fit-content()` tracks at their argument count as
                    // fixed: the space goes to all the affected tracks.
                    self.eligible.iter_mut().for_each(|e| *e = true);
                    self.room.iter_mut().for_each(|r| *r = f32::INFINITY);
                    water_fill(&mut self.inc, &self.room, &self.eligible, left);
                }
            }
            self.plan();
        }
        self.apply(tracks, target);
    }

    /// Distributes the contributions `size(item, spanned tracks, inner
    /// gaps)` of `items` (all crossing a flexible track) to the base sizes
    /// of the flexible tracks selected by `affected`, in proportion to
    /// their flex factors (§11.5 step 4).
    fn run_flex(
        &mut self,
        tracks: &mut [Track],
        items: &[&Span],
        affected: impl Fn(&Track) -> bool,
        size: impl Fn(&Span, &[Track], f32) -> f32,
    ) {
        let affected = |t: &Track| t.max.flex().is_some() && affected(t);
        for s in items {
            let r = s.range.clone();
            if !self.spend(r.len()) {
                break;
            }
            if !self.collect(tracks, r.clone(), &affected) {
                continue;
            }
            let Some(ts) = tracks.get(r.clone()) else {
                continue;
            };
            let inner = self.gap * r.len().saturating_sub(1) as f32;
            let have = ts.iter().map(|t| t.base).sum::<f32>() + inner;
            let space = sane(size(s, ts, inner)) - have;
            if space <= EPSILON {
                continue;
            }
            let flex = |i: usize| tracks.get(i).and_then(|t| t.max.flex()).unwrap_or(0.0);
            let sum: f32 = self.affected.iter().map(|&i| flex(i)).sum();
            let k = self.affected.len() as f32;
            self.inc.clear();
            for &i in &self.affected {
                let f = flex(i);
                // A sum of at least 1: by the flex ratios; below 1, that
                // proportion of the space by the ratios, the rest equally.
                let v = if sum >= 1.0 {
                    space * f / sum
                } else {
                    space * f + space * (1.0 - sum) / k
                };
                self.inc.push(sane_size(v));
            }
            self.plan();
        }
        self.apply(tracks, Target::Base);
    }
}

impl MaxSizing {
    /// `limit` capped by the argument of `fit-content()`.
    fn fit_limit_or(self, limit: f32) -> f32 {
        match self {
            MaxSizing::FitContent(f) => limit.min(f),
            _ => limit,
        }
    }
}

/// Distributes `space` equally over the eligible entries of `inc`, none
/// growing by more than its `room` (water filling); returns what is left.
fn water_fill(inc: &mut [f32], room: &[f32], eligible: &[bool], mut space: f32) -> f32 {
    let mut order: Vec<usize> = (0..inc.len())
        .filter(|&j| eligible.get(j).copied().unwrap_or(false))
        .collect();
    order.sort_by(|&a, &b| {
        let ra = room.get(a).copied().unwrap_or(0.0);
        let rb = room.get(b).copied().unwrap_or(0.0);
        ra.total_cmp(&rb)
    });
    let mut left = order.len();
    for &j in &order {
        if space <= EPSILON || left == 0 {
            break;
        }
        let share = space / left as f32;
        let give = room.get(j).copied().unwrap_or(0.0).min(share).max(0.0);
        if let Some(v) = inc.get_mut(j) {
            *v += give;
        }
        space -= give;
        left -= 1;
    }
    space.max(0.0)
}

/// Sizes `tracks` (their `base` becomes their used size) for the items'
/// contributions, `gap` between consecutive tracks. `stretch` stretches
/// the `auto` tracks into the remaining definite free space.
pub(crate) fn size_tracks(
    tracks: &mut [Track],
    items: &[Contribution],
    space: Space,
    gap: f32,
    stretch: bool,
) {
    let n = tracks.len();
    if n == 0 {
        return;
    }
    let gap = sane_size(gap);
    let gaps = gap * (n - 1) as f32;
    // 1. Initialise.
    for t in tracks.iter_mut() {
        t.base = match t.min {
            MinSizing::Fixed(v) => sane_size(v),
            _ => 0.0,
        };
        t.limit = match t.max {
            MaxSizing::Fixed(v) => sane_size(v).max(t.base),
            _ => f32::INFINITY,
        };
    }
    let spans: Vec<Span> = items
        .iter()
        .filter_map(|c| {
            let end = c.start.saturating_add(c.span).min(n);
            let start = c.start.min(end);
            let min = sane_size(c.min);
            (start < end).then(|| Span {
                range: start..end,
                minimum: sane_size(c.minimum),
                min,
                max: sane_size(c.max).max(min),
            })
        })
        .collect();
    let crosses_flex: Vec<bool> = spans
        .iter()
        .map(|s| {
            tracks
                .get(s.range.clone())
                .is_some_and(|ts| ts.iter().any(|t| t.max.flex().is_some()))
        })
        .collect();
    let intrinsic = !matches!(space, Space::Definite(_));
    // What an `auto` min accommodates: the minimum contribution, or under
    // an intrinsic constraint the limited min-content contribution.
    let auto_min = |s: &Span, ts: &[Track], inner: f32| {
        if intrinsic {
            limited(s.min, fixed_cap(ts, inner), s.minimum)
        } else {
            s.minimum
        }
    };
    let limited_max =
        |s: &Span, ts: &[Track], inner: f32| limited(s.max, fixed_cap(ts, inner), s.minimum);
    // 2. Items spanning one non-flexible track.
    for (s, _) in spans
        .iter()
        .zip(&crosses_flex)
        .filter(|(s, flex)| !**flex && s.range.len() == 1)
    {
        let Some(t) = tracks.get_mut(s.range.start) else {
            continue;
        };
        match t.min {
            MinSizing::Auto => {
                let v = auto_min(s, std::slice::from_ref(t), 0.0);
                t.base = t.base.max(v);
            }
            MinSizing::MinContent => t.base = t.base.max(s.min),
            MinSizing::MaxContent => t.base = t.base.max(s.max),
            MinSizing::Fixed(_) => {}
        }
        let want = match t.max {
            MaxSizing::MinContent => Some(s.min),
            MaxSizing::MaxContent | MaxSizing::Auto => Some(s.max),
            MaxSizing::FitContent(l) => Some(s.max.min(l)),
            MaxSizing::Fixed(_) | MaxSizing::Flex(_) => None,
        };
        if let Some(w) = want {
            t.limit = if t.limit.is_finite() {
                t.limit.max(w)
            } else {
                w
            };
        }
    }
    raise_limits(tracks);
    let mut d = Distributor::new(n, gap);
    // 3. Items spanning several non-flexible tracks, by increasing span.
    let mut multi: Vec<&Span> = spans
        .iter()
        .zip(&crosses_flex)
        .filter(|(s, flex)| !**flex && s.range.len() > 1)
        .map(|(s, _)| s)
        .collect();
    multi.sort_by_key(|s| s.range.len());
    for group in multi.chunk_by(|a, b| a.range.len() == b.range.len()) {
        // Intrinsic, content-based and max-content minimums.
        d.run(
            tracks,
            group,
            Target::Base,
            |t| t.min.is_intrinsic(),
            auto_min,
            Beyond::IntrinsicMax,
        );
        d.run(
            tracks,
            group,
            Target::Base,
            |t| matches!(t.min, MinSizing::MinContent | MinSizing::MaxContent),
            |s, _, _| s.min,
            Beyond::IntrinsicMax,
        );
        if space == Space::MaxContent {
            d.run(
                tracks,
                group,
                Target::Base,
                |t| matches!(t.min, MinSizing::Auto | MinSizing::MaxContent),
                limited_max,
                Beyond::MaxContentMax,
            );
        }
        d.run(
            tracks,
            group,
            Target::Base,
            |t| t.min == MinSizing::MaxContent,
            |s, _, _| s.max,
            Beyond::MaxContentMax,
        );
        raise_limits(tracks);
        // Intrinsic and max-content maximums.
        d.run(
            tracks,
            group,
            Target::Limit { mark: true },
            |t| t.max.is_intrinsic(),
            |s, _, _| s.min,
            Beyond::All,
        );
        d.run(
            tracks,
            group,
            Target::Limit { mark: false },
            |t| t.max.is_max_content(),
            |s, _, _| s.max,
            Beyond::AllCapFit,
        );
        d.clear_growable();
    }
    // 4. Items crossing flexible tracks, all together.
    let flexed: Vec<&Span> = spans
        .iter()
        .zip(&crosses_flex)
        .filter(|(_, flex)| **flex)
        .map(|(s, _)| s)
        .collect();
    if !flexed.is_empty() {
        d.run_flex(tracks, &flexed, |t| t.min.is_intrinsic(), auto_min);
        d.run_flex(
            tracks,
            &flexed,
            |t| matches!(t.min, MinSizing::MinContent | MinSizing::MaxContent),
            |s, _, _| s.min,
        );
        if space == Space::MaxContent {
            d.run_flex(
                tracks,
                &flexed,
                |t| matches!(t.min, MinSizing::Auto | MinSizing::MaxContent),
                limited_max,
            );
        }
        d.run_flex(
            tracks,
            &flexed,
            |t| t.min == MinSizing::MaxContent,
            |s, _, _| s.max,
        );
    }
    // Unset growth limits become the base size.
    for t in tracks.iter_mut() {
        if !t.limit.is_finite() || t.limit < t.base {
            t.limit = t.base;
        }
        if t.max.flex().is_some() {
            t.limit = t.base;
        }
    }
    // 5. Maximise.
    match space {
        Space::MaxContent => {
            for t in tracks.iter_mut() {
                t.base = t.limit;
            }
        }
        Space::Definite(avail) => {
            let free = sane(avail) - gaps - tracks.iter().map(|t| t.base).sum::<f32>();
            if free > EPSILON {
                let room: Vec<f32> = tracks.iter().map(|t| (t.limit - t.base).max(0.0)).collect();
                let eligible: Vec<bool> = tracks.iter().map(|t| t.max.flex().is_none()).collect();
                let mut inc = vec![0.0; n];
                water_fill(&mut inc, &room, &eligible, free);
                for (t, v) in tracks.iter_mut().zip(inc) {
                    t.base += v;
                }
            }
        }
        Space::MinContent => {}
    }
    // 6. Flexible tracks.
    if tracks.iter().any(|t| t.max.flex().is_some()) {
        let fr = match space {
            Space::Definite(avail) => definite_fr(tracks, sane(avail) - gaps),
            Space::MaxContent => intrinsic_fr(tracks, &spans, gap, &mut d),
            Space::MinContent => 0.0,
        };
        for t in tracks.iter_mut() {
            if let Some(f) = t.max.flex() {
                t.base = t.base.max(sane_size(fr * f));
            }
        }
    }
    // 7. Stretch auto tracks.
    if let (true, Space::Definite(avail)) = (stretch, space) {
        let free = sane(avail) - gaps - tracks.iter().map(|t| t.base).sum::<f32>();
        let k = tracks.iter().filter(|t| t.max == MaxSizing::Auto).count();
        if free > EPSILON && k > 0 {
            let share = free / k as f32;
            for t in tracks.iter_mut().filter(|t| t.max == MaxSizing::Auto) {
                t.base += share;
            }
        }
    }
    for t in tracks.iter_mut() {
        t.base = sane_size(t.base);
    }
}

/// Raises the (finite) growth limits below their base size to it.
fn raise_limits(tracks: &mut [Track]) {
    for t in tracks.iter_mut() {
        if t.limit < t.base {
            t.limit = t.base;
        }
    }
}

/// The size of `1fr` for a definite space `avail` (gaps excluded): the
/// leftover space after the inflexible tracks, divided by the flex
/// factors (at least 1); flexible tracks wider than their share are
/// treated as inflexible (§11.7.1). Flexible tracks are made inflexible
/// one at a time, largest base size per fr first: the hypothetical fr
/// size only decreases as they are, so this finds the same set as
/// restarting with all the offending tracks at once, in O(n log n).
fn definite_fr(tracks: &[Track], avail: f32) -> f32 {
    let mut used = 0.0f32;
    let mut flex_sum = 0.0f32;
    // (base size per fr, base size, flex factor) of the flexible tracks.
    let mut flexible: Vec<(f32, f32, f32)> = Vec::new();
    for t in tracks {
        match t.max.flex() {
            Some(f) => {
                let ratio = if f > 0.0 { t.base / f } else { f32::INFINITY };
                flexible.push((ratio, t.base, f));
                flex_sum += f;
            }
            None => used += t.base,
        }
    }
    flexible.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut next = flexible.iter();
    loop {
        let leftover = avail - used;
        if leftover.is_nan() || leftover <= 0.0 {
            return 0.0;
        }
        let fr = leftover / flex_sum.max(1.0);
        match next.next() {
            Some(&(_, base, f)) if base > fr * f + EPSILON => {
                used += base;
                flex_sum -= f;
            }
            _ => return sane_size(fr),
        }
    }
}

/// The size of `1fr` under a max-content constraint.
fn intrinsic_fr(tracks: &[Track], items: &[Span], gap: f32, d: &mut Distributor) -> f32 {
    let mut fr = 0.0f32;
    for t in tracks {
        if let Some(f) = t.max.flex() {
            fr = fr.max(if f > 1.0 { t.base / f } else { t.base });
        }
    }
    for s in items {
        let Some(ts) = tracks.get(s.range.clone()) else {
            continue;
        };
        if !ts.iter().any(|t| t.max.flex().is_some()) || !d.spend(ts.len()) {
            continue;
        }
        let inner = gap * ts.len().saturating_sub(1) as f32;
        fr = fr.max(definite_fr(ts, s.max - inner));
    }
    sane_size(fr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(start: usize, span: usize, min: f32, max: f32) -> Contribution {
        Contribution::new(start, span, min, max)
    }

    fn sizes(tracks: &[Track]) -> Vec<f32> {
        tracks
            .iter()
            .map(|t| (t.base * 100.0).round() / 100.0)
            .collect()
    }

    fn fr(f: f32) -> Track {
        Track::new(MinSizing::Auto, MaxSizing::Flex(f))
    }

    #[test]
    fn fixed_and_fr_tracks() {
        // 100px 1fr 2fr in 430px with 10px gaps: (430 - 20 - 100) / 3 = 103.33.
        let mut t = [Track::fixed(100.0), fr(1.0), fr(2.0)];
        size_tracks(&mut t, &[], Space::Definite(430.0), 10.0, true);
        assert_eq!(sizes(&t), vec![100.0, 103.33, 206.67]);
        // A wide item keeps its 1fr column at its min-content width.
        let mut t = [fr(1.0), fr(1.0)];
        size_tracks(
            &mut t,
            &[c(0, 1, 300.0, 400.0)],
            Space::Definite(400.0),
            0.0,
            true,
        );
        assert_eq!(sizes(&t), vec![300.0, 100.0]);
        // Fractions below 1 do not fill the space.
        let mut t = [fr(0.25)];
        size_tracks(&mut t, &[], Space::Definite(400.0), 0.0, true);
        assert_eq!(sizes(&t), vec![100.0]);
    }

    #[test]
    fn auto_tracks_size_to_content_and_stretch() {
        let items = [c(0, 1, 50.0, 120.0), c(1, 1, 30.0, 60.0)];
        // Room for both max-contents, the rest stretches the auto tracks.
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &items, Space::Definite(280.0), 0.0, true);
        assert_eq!(sizes(&t), vec![170.0, 110.0]);
        // Without stretching: max-content.
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &items, Space::Definite(280.0), 0.0, false);
        assert_eq!(sizes(&t), vec![120.0, 60.0]);
        // Not enough room: min-content, then the free space equally.
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &items, Space::Definite(100.0), 0.0, true);
        assert_eq!(sizes(&t), vec![60.0, 40.0]);
        // auto next to 1fr: the fr track takes the free space.
        let mut t = [Track::auto(), fr(1.0)];
        size_tracks(&mut t, &items, Space::Definite(400.0), 0.0, true);
        assert_eq!(sizes(&t), vec![120.0, 280.0]);
    }

    #[test]
    fn min_max_content_minmax_and_fit_content() {
        let items = [
            c(0, 1, 50.0, 120.0),
            c(1, 1, 30.0, 60.0),
            c(2, 1, 40.0, 500.0),
        ];
        let mut t = [
            Track::new(MinSizing::MinContent, MaxSizing::MinContent),
            Track::new(MinSizing::MaxContent, MaxSizing::MaxContent),
            Track::new(MinSizing::Auto, MaxSizing::FitContent(200.0)),
        ];
        size_tracks(&mut t, &items, Space::Definite(1000.0), 0.0, false);
        assert_eq!(sizes(&t), vec![50.0, 60.0, 200.0]);
        // minmax(200px, 1fr): at least 200px, flexible above.
        let mm = Track::new(MinSizing::Fixed(200.0), MaxSizing::Flex(1.0));
        let mut t = [mm, mm, mm];
        size_tracks(&mut t, &[], Space::Definite(500.0), 0.0, true);
        assert_eq!(sizes(&t), vec![200.0, 200.0, 200.0]);
        let mut t = [mm, mm];
        size_tracks(&mut t, &[], Space::Definite(500.0), 0.0, true);
        assert_eq!(sizes(&t), vec![250.0, 250.0]);
        // minmax(100px, 300px) grows to its max with free space.
        let mut t = [Track::new(MinSizing::Fixed(100.0), MaxSizing::Fixed(300.0))];
        size_tracks(&mut t, &[], Space::Definite(1000.0), 0.0, true);
        assert_eq!(sizes(&t), vec![300.0]);
    }

    #[test]
    fn spanning_items_and_intrinsic_constraints() {
        // A 300px item spanning two auto tracks with a 20px gap.
        let items = [c(0, 2, 300.0, 300.0), c(0, 1, 50.0, 50.0)];
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &items, Space::MaxContent, 20.0, false);
        let total: f32 = t.iter().map(|t| t.base).sum::<f32>() + 20.0;
        assert!((total - 300.0).abs() < 0.01, "{t:?}");
        // repeat(2, 1fr) under max-content: both as wide as the widest item.
        let items = [c(0, 1, 10.0, 30.0), c(1, 1, 10.0, 50.0)];
        let mut t = [fr(1.0), fr(1.0)];
        size_tracks(&mut t, &items, Space::MaxContent, 10.0, false);
        assert_eq!(sizes(&t), vec![50.0, 50.0]);
        // ...and under min-content: their min-content contributions.
        let mut t = [fr(1.0), fr(1.0)];
        size_tracks(&mut t, &items, Space::MinContent, 10.0, false);
        assert_eq!(sizes(&t), vec![10.0, 10.0]);
    }

    #[test]
    fn spanning_items_fill_tracks_without_items_first() {
        // CSS Grid 1 §11.5.1: the space a spanning item lacks goes first
        // to the tracks below their growth limit — here the second track
        // has no item (infinite limit), the first is at its limit.
        let items = [c(0, 1, 100.0, 100.0), c(0, 2, 300.0, 300.0)];
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &items, Space::Definite(300.0), 0.0, true);
        assert_eq!(sizes(&t), vec![100.0, 200.0]);
        // Under a max-content constraint too (inline-grid widths).
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &items, Space::MaxContent, 0.0, false);
        assert_eq!(sizes(&t), vec![100.0, 200.0]);
        // Tracks grow up to their growth limit (max-content) before the
        // others: 50..150 and 50..50 tracks, a 200px spanning item.
        let items = [
            c(0, 1, 50.0, 150.0),
            c(1, 1, 50.0, 50.0),
            c(0, 2, 200.0, 200.0),
        ];
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &items, Space::Definite(1000.0), 0.0, false);
        assert_eq!(sizes(&t), vec![150.0, 50.0]);
        // Beyond the limits, tracks with an intrinsic max take the space,
        // not the fixed-max ones: minmax(auto, 40px) and auto.
        let items = [
            c(0, 1, 10.0, 10.0),
            c(1, 1, 10.0, 10.0),
            c(0, 2, 120.0, 120.0),
        ];
        let mut t = [
            Track::new(MinSizing::Auto, MaxSizing::Fixed(40.0)),
            Track::auto(),
        ];
        size_tracks(&mut t, &items, Space::Definite(1000.0), 0.0, false);
        assert_eq!(sizes(&t), vec![40.0, 80.0]);
        // The order of the items of a span group does not matter.
        let a = [c(0, 2, 100.0, 100.0), c(1, 2, 300.0, 300.0)];
        let b = [a[1], a[0]];
        let mut ta = [Track::auto(); 3];
        let mut tb = [Track::auto(); 3];
        size_tracks(&mut ta, &a, Space::MaxContent, 10.0, false);
        size_tracks(&mut tb, &b, Space::MaxContent, 10.0, false);
        assert_eq!(sizes(&ta), sizes(&tb));
    }

    #[test]
    fn minimum_contributions_of_auto_min_tracks() {
        // An item whose minimum contribution is 0 (`min-width: 0`, a
        // scroll container) does not widen its `1fr` column under a
        // definite size, but counts with its min-content size for the
        // grid's intrinsic sizes.
        let item = Contribution {
            minimum: 0.0,
            ..c(0, 1, 600.0, 600.0)
        };
        let mut t = [fr(1.0), fr(1.0)];
        size_tracks(&mut t, &[item], Space::Definite(400.0), 0.0, true);
        assert_eq!(sizes(&t), vec![200.0, 200.0]);
        let mut t = [fr(1.0), fr(1.0)];
        size_tracks(&mut t, &[item], Space::MinContent, 0.0, false);
        assert_eq!(sizes(&t), vec![600.0, 0.0]);
        // Same for an `auto` track: it stays at its share.
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &[item], Space::Definite(400.0), 0.0, true);
        assert_eq!(sizes(&t), vec![400.0, 0.0]);
        let narrow = c(1, 1, 50.0, 50.0);
        let mut t = [Track::auto(), Track::auto()];
        size_tracks(&mut t, &[item, narrow], Space::Definite(300.0), 0.0, true);
        assert_eq!(sizes(&t), vec![250.0, 50.0]);
        // An item spanning flexible tracks with a minimum contribution
        // (an explicit `min-width`) grows them by their flex ratios.
        let wide = c(0, 2, 300.0, 300.0);
        let mut t = [fr(1.0), fr(2.0)];
        size_tracks(&mut t, &[wide], Space::Definite(90.0), 0.0, true);
        assert_eq!(sizes(&t), vec![100.0, 200.0]);
    }

    /// The fr size of CSS Grid 1 §11.7.1 as specified (restart while
    /// some flexible track is larger than its share).
    fn reference_fr(tracks: &[Track], avail: f32) -> f32 {
        let mut inflexible: Vec<bool> = tracks.iter().map(|t| t.max.flex().is_none()).collect();
        loop {
            let used: f32 = tracks
                .iter()
                .zip(&inflexible)
                .filter(|(_, &i)| i)
                .map(|(t, _)| t.base)
                .sum();
            let flex: f32 = tracks
                .iter()
                .zip(&inflexible)
                .filter(|(_, &i)| !i)
                .filter_map(|(t, _)| t.max.flex())
                .sum();
            let leftover = avail - used;
            if leftover <= 0.0 {
                return 0.0;
            }
            let fr = leftover / flex.max(1.0);
            let mut changed = false;
            for (t, inf) in tracks.iter().zip(inflexible.iter_mut()) {
                if !*inf && t.base > fr * t.max.flex().unwrap_or(0.0) + EPSILON {
                    *inf = true;
                    changed = true;
                }
            }
            if !changed {
                return fr;
            }
        }
    }

    #[test]
    fn fr_size_matches_the_specified_restarts() {
        let with = |base: f32, f: f32| Track { base, ..fr(f) };
        let cases: Vec<(Vec<Track>, f32)> = vec![
            (vec![with(300.0, 1.0), with(0.0, 1.0)], 400.0),
            (
                vec![with(10.0, 0.5), with(200.0, 0.25), with(0.0, 0.1)],
                300.0,
            ),
            (
                vec![
                    with(90.0, 1.0),
                    with(80.0, 2.0),
                    with(5.0, 3.0),
                    Track::fixed(50.0),
                ],
                250.0,
            ),
            (vec![with(10.0, 0.0), with(20.0, 1.0)], 100.0),
            (vec![with(500.0, 1.0)], 100.0),
            (vec![with(1.0, 1.0); 7], 1000.0),
        ];
        for (tracks, avail) in cases {
            let a = definite_fr(&tracks, avail);
            let b = reference_fr(&tracks, avail);
            assert!((a - b).abs() < 0.01, "{tracks:?} in {avail}: {a} vs {b}");
        }
        // An item spanning a wide flexible track and a narrow one under a
        // max-content constraint: the wide track is inflexible for it.
        let items = [c(0, 1, 300.0, 300.0), c(0, 2, 400.0, 400.0)];
        let mut t = [fr(1.0), fr(1.0)];
        size_tracks(&mut t, &items, Space::MaxContent, 0.0, false);
        assert_eq!(sizes(&t), vec![300.0, 300.0]);
        // With factors below 1 the base sizes do not bound the item's fr
        // size: `minmax(100px, 0.5fr) minmax(0, 0.5fr)` and an item
        // spanning both with a 150px max-content width. The first track
        // is inflexible for the item, whose 50px left need 50px per fr
        // (the sum of factors counts as 1), so the grid is exactly 150px
        // wide (not 100 + 75).
        let items = [c(0, 2, 0.0, 150.0)];
        let mut t = [
            Track::new(MinSizing::Fixed(100.0), MaxSizing::Flex(0.5)),
            Track::new(MinSizing::Fixed(0.0), MaxSizing::Flex(0.5)),
        ];
        size_tracks(&mut t, &items, Space::MaxContent, 0.0, false);
        assert_eq!(sizes(&t), vec![100.0, 50.0]);
    }

    #[test]
    fn hostile_values_stay_finite() {
        let items = [c(0, 1000, f32::MAX, f32::INFINITY), c(5, 1, f32::NAN, -1.0)];
        let mut t = vec![fr(f32::MAX); 3];
        t.push(Track::fixed(f32::NAN));
        size_tracks(
            &mut t,
            &items,
            Space::Definite(f32::INFINITY),
            f32::MAX,
            true,
        );
        assert!(t.iter().all(|t| t.base.is_finite() && t.base >= 0.0));
        size_tracks(&mut t, &items, Space::MaxContent, 1e30, true);
        assert!(t.iter().all(|t| t.base.is_finite() && t.base >= 0.0));
        size_tracks(&mut [], &items, Space::MinContent, 0.0, true);
    }
}
