//! Device rectangles and the clip stack state.
//!
//! A clip state is the intersection of every clip rect pushed so far (used
//! for geometric clipping and culling) plus the chain of rounded clips. The
//! coverage mask is only built when a drawing actually needs it (it touches
//! the clip's anti-aliased edges or one of its rounded corners), and it is
//! built analytically in time proportional to the clip rect plus the corner
//! squares, not to the whole drawing area.

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use tiny_skia::Mask;

use super::div255;

/// An axis-aligned rectangle in device pixels (area-local), as edges.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct DevRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// Coordinates are clamped to this magnitude (device px) so geometry stays
/// well inside tiny-skia's numeric limits.
const LIMIT: f32 = 1.0e7;

impl DevRect {
    pub const EMPTY: DevRect = DevRect {
        x0: 0.0,
        y0: 0.0,
        x1: 0.0,
        y1: 0.0,
    };

    /// Builds a rect, replacing NaN with 0 and clamping huge values.
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> DevRect {
        let c = |v: f32| {
            if v.is_nan() {
                0.0
            } else {
                v.clamp(-LIMIT, LIMIT)
            }
        };
        DevRect {
            x0: c(x0),
            y0: c(y0),
            x1: c(x1),
            y1: c(y1),
        }
    }

    pub fn width(&self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }

    pub fn height(&self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }

    pub fn is_empty(&self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }

    pub fn intersect(&self, o: &DevRect) -> DevRect {
        DevRect {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }

    pub fn intersects(&self, o: &DevRect) -> bool {
        !self.intersect(o).is_empty()
    }

    /// True when `o` lies inside `self` (an empty `o` is always contained).
    pub fn contains(&self, o: &DevRect) -> bool {
        o.is_empty() || (o.x0 >= self.x0 && o.y0 >= self.y0 && o.x1 <= self.x1 && o.y1 <= self.y1)
    }

    /// Smallest rect containing both (empty rects are ignored).
    pub fn union(&self, o: &DevRect) -> DevRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        DevRect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    /// Grows the rect by `d` on every side.
    pub fn outset(&self, d: f32) -> DevRect {
        DevRect::new(self.x0 - d, self.y0 - d, self.x1 + d, self.y1 + d)
    }

    /// Smallest integer-aligned rect containing this one.
    pub fn round_out(&self) -> DevRect {
        DevRect::new(
            self.x0.floor(),
            self.y0.floor(),
            self.x1.ceil(),
            self.y1.ceil(),
        )
    }

    /// Edges rounded to whole device pixels; a non-empty rect stays at
    /// least one pixel wide/tall (hairlines remain visible).
    pub fn snap(&self) -> DevRect {
        let snap_axis = |a: f32, b: f32| {
            let (ra, rb) = (a.round(), b.round());
            if b > a && rb <= ra {
                (ra, ra + 1.0)
            } else {
                (ra, rb)
            }
        };
        let (x0, x1) = snap_axis(self.x0, self.x1);
        let (y0, y1) = snap_axis(self.y0, self.y1);
        DevRect { x0, y0, x1, y1 }
    }

    pub fn to_skia(self) -> Option<tiny_skia::Rect> {
        if self.is_empty() {
            return None;
        }
        tiny_skia::Rect::from_ltrb(self.x0, self.y0, self.x1, self.y1)
    }
}

/// Recycled coverage masks of the drawing area's size. Allocating (and
/// zeroing) a full-area mask for every clip that needs one dominated pages
/// with many rounded `overflow: hidden` boxes; recycled masks only need the
/// region they were written in cleared.
#[derive(Default)]
pub(crate) struct MaskPool {
    free: RefCell<Vec<Mask>>,
}

/// Most masks kept for reuse.
const POOLED_MASKS: usize = 8;

impl MaskPool {
    /// A zeroed `width` × `height` mask (`None` if it cannot be allocated).
    pub fn take(&self, width: u32, height: u32) -> Option<Mask> {
        let reused = self.free.try_borrow_mut().ok().and_then(|mut f| f.pop());
        match reused {
            Some(mask) if mask.width() == width && mask.height() == height => Some(mask),
            _ => Mask::new(width, height),
        }
    }

    /// Takes `mask` back for reuse, zeroing `written` (device px), which
    /// must contain every non-zero value of the mask.
    pub fn give(&self, mut mask: Mask, written: DevRect) {
        let (w, h) = (mask.width() as usize, mask.height() as usize);
        let r = written
            .round_out()
            .intersect(&DevRect::new(0.0, 0.0, w as f32, h as f32));
        if !r.is_empty() {
            let (x0, x1) = (r.x0 as usize, r.x1 as usize);
            let data = mask.data_mut();
            for y in r.y0 as usize..r.y1 as usize {
                if let Some(row) = data.get_mut(y * w + x0..y * w + x1) {
                    row.fill(0);
                }
            }
        }
        if let Ok(mut free) = self.free.try_borrow_mut() {
            if free.len() < POOLED_MASKS {
                free.push(mask);
            }
        }
    }
}

/// A rounded clip shape in a clip chain.
#[derive(Debug)]
struct RoundedClip {
    rect: DevRect,
    radii: [f32; 4],
    parent: Option<Rc<RoundedClip>>,
}

impl RoundedClip {
    /// True when `b` (which must lie inside `rect`) avoids the four corner
    /// squares, i.e. lies wholly inside the rounded shape.
    fn avoids_corners(&self, b: &DevRect) -> bool {
        let r = &self.rect;
        let [tl, tr, br, bl] = self.radii;
        (b.x0 >= r.x0 + tl || b.y0 >= r.y0 + tl)
            && (b.x1 <= r.x1 - tr || b.y0 >= r.y0 + tr)
            && (b.x1 <= r.x1 - br || b.y1 <= r.y1 - br)
            && (b.x0 >= r.x0 + bl || b.y1 <= r.y1 - bl)
    }
}

/// Longest rounded-clip chain walked (deeper chains are treated as needing
/// the mask, which is always correct).
const MAX_CHAIN: usize = 4096;

/// One level of the clip stack.
#[derive(Clone, Debug)]
pub(crate) struct ClipState {
    /// Intersection of all clip rects (bounds of the clip region).
    pub rect: DevRect,
    rounded: Option<Rc<RoundedClip>>,
    /// Lazily built coverage mask of the whole clip region.
    mask: OnceCell<Option<Rc<Mask>>>,
}

impl ClipState {
    /// No clipping beyond the area itself.
    pub fn full(width: u32, height: u32) -> ClipState {
        ClipState {
            rect: DevRect::new(0.0, 0.0, width as f32, height as f32),
            rounded: None,
            mask: OnceCell::new(),
        }
    }

    /// The state after intersecting with `rect` (rounded by `radii`).
    pub fn push(&self, rect: DevRect, radii: [f32; 4]) -> ClipState {
        let rounded = if radii.iter().any(|&r| r > 0.0) {
            Some(Rc::new(RoundedClip {
                rect,
                radii,
                parent: self.rounded.clone(),
            }))
        } else {
            self.rounded.clone()
        };
        ClipState {
            rect: self.rect.intersect(&rect),
            rounded,
            mask: OnceCell::new(),
        }
    }

    /// True when drawing inside `b` (which must already lie inside
    /// [`ClipState::rect`]) needs the coverage mask: `b` reaches into a
    /// rounded corner of some clip of the chain.
    pub fn needs_mask_within(&self, b: &DevRect) -> bool {
        let mut next = self.rounded.as_deref();
        let mut steps = 0usize;
        while let Some(clip) = next {
            steps += 1;
            if steps > MAX_CHAIN || !clip.avoids_corners(b) {
                return true;
            }
            next = clip.parent.as_deref();
        }
        false
    }

    /// Coverage mask of the clip region for a `width` × `height` area
    /// (`None` if the mask cannot be allocated), built from `pool` once.
    pub fn mask(&self, width: u32, height: u32, pool: &MaskPool) -> Option<Rc<Mask>> {
        self.mask
            .get_or_init(|| self.build_mask(width, height, pool).map(Rc::new))
            .clone()
    }

    /// Returns this state's mask, if it built one and nothing else holds
    /// it, to `pool` (called when the state is popped).
    pub fn recycle(self, pool: &MaskPool) {
        if let Some(Some(mask)) = self.mask.into_inner() {
            if let Ok(mask) = Rc::try_unwrap(mask) {
                // Only the clip rect is ever written (see `build_mask`).
                pool.give(mask, self.rect);
            }
        }
    }

    fn build_mask(&self, width: u32, height: u32, pool: &MaskPool) -> Option<Mask> {
        let mut mask = pool.take(width, height)?;
        fill_rect_coverage(&mut mask, self.rect);
        let bounds =
            self.rect
                .round_out()
                .intersect(&DevRect::new(0.0, 0.0, width as f32, height as f32));
        let mut next = self.rounded.as_deref();
        let mut steps = 0usize;
        while let Some(clip) = next {
            steps += 1;
            if steps > MAX_CHAIN {
                break;
            }
            round_corners(&mut mask, clip.rect, clip.radii, bounds);
            next = clip.parent.as_deref();
        }
        Some(mask)
    }
}

/// Multiplies the mask by the coverage of the rounded corners of `rect`
/// (circular, radii tl, tr, br, bl), within `bounds` (integer-aligned,
/// inside the mask). Pixels whose centre lies beyond a corner's centre (in
/// its outer quadrant) get `clamp(r + ½ − distance, 0, 1)`; the straight
/// edges are already covered by the rect coverage.
fn round_corners(mask: &mut Mask, rect: DevRect, radii: [f32; 4], bounds: DevRect) {
    let w = mask.width() as usize;
    let data = mask.data_mut();
    // (radius, centre x, centre y, outer side is left, outer side is top)
    let corners = [
        (radii[0], rect.x0 + radii[0], rect.y0 + radii[0], true, true),
        (
            radii[1],
            rect.x1 - radii[1],
            rect.y0 + radii[1],
            false,
            true,
        ),
        (
            radii[2],
            rect.x1 - radii[2],
            rect.y1 - radii[2],
            false,
            false,
        ),
        (
            radii[3],
            rect.x0 + radii[3],
            rect.y1 - radii[3],
            true,
            false,
        ),
    ];
    for (r, cx, cy, left, top) in corners {
        if !(r > 0.0 && r.is_finite()) {
            continue;
        }
        let (sx0, sx1) = if left { (cx - r, cx) } else { (cx, cx + r) };
        let (sy0, sy1) = if top { (cy - r, cy) } else { (cy, cy + r) };
        // Include the anti-aliased pixel row/column outside the rect.
        let square = DevRect::new(sx0 - 1.0, sy0 - 1.0, sx1 + 1.0, sy1 + 1.0)
            .round_out()
            .intersect(&bounds);
        if square.is_empty() {
            continue;
        }
        let (x0, x1) = (square.x0 as usize, square.x1 as usize);
        for py in square.y0 as usize..square.y1 as usize {
            let dy = py as f32 + 0.5 - cy;
            if (top && dy > 0.0) || (!top && dy < 0.0) {
                continue;
            }
            let Some(row) = data.get_mut(py * w + x0..py * w + x1) else {
                continue;
            };
            for (i, m) in row.iter_mut().enumerate() {
                if *m == 0 {
                    continue;
                }
                let dx = (x0 + i) as f32 + 0.5 - cx;
                if (left && dx > 0.0) || (!left && dx < 0.0) {
                    continue;
                }
                let d = (dx * dx + dy * dy).sqrt();
                let c = u32::from(coverage_u8(r + 0.5 - d));
                *m = div255(u32::from(*m) * c) as u8;
            }
        }
    }
}

/// Converts a coverage in 0..=1 to 0..=255 (rounded; no libm call).
#[inline]
fn coverage_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Writes the exact anti-aliased coverage of `rect` into an empty mask.
pub(crate) fn fill_rect_coverage(mask: &mut Mask, rect: DevRect) {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let r = rect.intersect(&DevRect::new(0.0, 0.0, w as f32, h as f32));
    if r.is_empty() {
        return;
    }
    let coverage = |i: usize, a: f32, b: f32| -> f32 {
        let (lo, hi) = (i as f32, i as f32 + 1.0);
        (hi.min(b) - lo.max(a)).clamp(0.0, 1.0)
    };
    let (cx0, cx1) = (r.x0.floor() as usize, (r.x1.ceil() as usize).min(w));
    let (cy0, cy1) = (r.y0.floor() as usize, (r.y1.ceil() as usize).min(h));
    let xs: Vec<u8> = (cx0..cx1)
        .map(|x| coverage_u8(coverage(x, r.x0, r.x1)))
        .collect();
    let data = mask.data_mut();
    for y in cy0..cy1 {
        let cy = coverage_u8(coverage(y, r.y0, r.y1));
        let Some(row) = data.get_mut(y * w + cx0..y * w + cx1) else {
            continue;
        };
        if cy == 255 {
            row.copy_from_slice(&xs);
        } else {
            let cy = u32::from(cy);
            for (m, &cx) in row.iter_mut().zip(&xs) {
                *m = div255(u32::from(cx) * cy) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_ops() {
        let a = DevRect::new(0.0, 0.0, 10.0, 10.0);
        let b = DevRect::new(5.0, 5.0, 15.0, 15.0);
        assert_eq!(a.intersect(&b), DevRect::new(5.0, 5.0, 10.0, 10.0));
        assert!(a.intersects(&b));
        assert!(!a.contains(&b));
        assert!(a.contains(&DevRect::new(1.0, 1.0, 2.0, 2.0)));
        assert_eq!(a.union(&b), DevRect::new(0.0, 0.0, 15.0, 15.0));
        assert_eq!(
            DevRect::new(0.4, 0.6, 10.2, 1.1).snap(),
            DevRect::new(0.0, 1.0, 10.0, 2.0)
        );
        assert_eq!(DevRect::new(f32::NAN, 0.0, f32::INFINITY, 1.0).x1, LIMIT);
        assert!(DevRect::new(5.0, 5.0, 5.0, 9.0).is_empty());
    }

    #[test]
    fn rect_coverage() {
        let mut mask = Mask::new(4, 2).unwrap();
        fill_rect_coverage(&mut mask, DevRect::new(0.5, 0.0, 3.0, 1.0));
        assert_eq!(mask.data(), &[128, 255, 255, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn recycled_masks_come_back_clean() {
        let pool = MaskPool::default();
        let clip = ClipState::full(30, 30).push(DevRect::new(2.5, 3.0, 20.0, 25.0), [6.0; 4]);
        let first = clip.mask(30, 30, &pool).unwrap();
        assert!(first.data().iter().any(|&m| m > 0));
        drop(first);
        clip.recycle(&pool);
        let reused = pool.take(30, 30).unwrap();
        assert!(reused.data().iter().all(|&m| m == 0));
        // A mask of another size is not reused.
        pool.give(reused, DevRect::EMPTY);
        assert_eq!(pool.take(10, 10).unwrap().width(), 10);
        // A mask still shared is simply dropped.
        let clip = ClipState::full(30, 30).push(DevRect::new(0.0, 0.0, 9.0, 9.0), [3.0; 4]);
        let held = clip.mask(30, 30, &pool).unwrap();
        clip.recycle(&pool);
        assert!(held.data().iter().any(|&m| m > 0));
    }

    #[test]
    fn corner_avoidance() {
        let clip = ClipState::full(100, 100).push(DevRect::new(0.0, 0.0, 100.0, 100.0), [10.0; 4]);
        // Inside the cross between the corner squares: no mask.
        assert!(!clip.needs_mask_within(&DevRect::new(10.0, 0.0, 90.0, 100.0)));
        assert!(!clip.needs_mask_within(&DevRect::new(0.0, 10.0, 100.0, 90.0)));
        assert!(!clip.needs_mask_within(&DevRect::new(20.0, 20.0, 30.0, 30.0)));
        // Reaching into a corner square.
        assert!(clip.needs_mask_within(&DevRect::new(5.0, 5.0, 30.0, 30.0)));
        assert!(clip.needs_mask_within(&DevRect::new(0.0, 0.0, 100.0, 100.0)));
        assert!(clip.needs_mask_within(&DevRect::new(95.0, 95.0, 100.0, 100.0)));
        // Square clips never need a mask for contained shapes.
        let square = ClipState::full(100, 100).push(DevRect::new(0.0, 0.0, 50.0, 50.0), [0.0; 4]);
        assert!(!square.needs_mask_within(&DevRect::new(0.0, 0.0, 50.0, 50.0)));
    }

    #[test]
    fn analytic_corner_coverage() {
        let clip = ClipState::full(40, 40).push(DevRect::new(0.0, 0.0, 40.0, 40.0), [16.0; 4]);
        let mask = clip.mask(40, 40, &MaskPool::default()).unwrap();
        let at = |x: usize, y: usize| mask.data()[y * 40 + x];
        assert_eq!(at(20, 20), 255);
        assert_eq!(at(20, 0), 255, "straight edge");
        for (x, y) in [(0, 0), (39, 0), (39, 39), (0, 39), (2, 2)] {
            assert_eq!(at(x, y), 0, "corner ({x},{y})");
        }
        // Symmetric anti-aliased arc: a pixel on the 45° diagonal near the circle.
        let d = 16.0 - 16.0 / std::f32::consts::SQRT_2;
        let p = d.floor() as usize;
        assert!(at(p, p) > 0 && at(p, p) < 255, "{}", at(p, p));
        assert_eq!(at(p, p), at(39 - p, p));
        assert_eq!(at(p, p), at(39 - p, 39 - p));
        assert_eq!(at(p, p), at(p, 39 - p));
        // Nested rounded clips intersect.
        let inner = clip.push(DevRect::new(10.0, 10.0, 30.0, 30.0), [10.0; 4]);
        let mask = inner.mask(40, 40, &MaskPool::default()).unwrap();
        assert_eq!(mask.data()[20 * 40 + 20], 255);
        assert_eq!(mask.data()[10 * 40 + 10], 0);
        assert_eq!(mask.data()[5 * 40 + 20], 0, "outside the inner rect");
    }

    #[test]
    fn lazy_rounded_mask() {
        let full = ClipState::full(20, 20);
        let clip = full.push(DevRect::new(0.0, 0.0, 20.0, 20.0), [10.0; 4]);
        assert!(clip.needs_mask_within(&DevRect::new(0.0, 0.0, 20.0, 20.0)));
        let pool = MaskPool::default();
        let mask = clip.mask(20, 20, &pool).unwrap();
        // Centre inside, corner outside the circle.
        assert_eq!(mask.data()[10 * 20 + 10], 255);
        assert_eq!(mask.data()[0], 0);
        // Same Rc on the second call.
        assert!(Rc::ptr_eq(&mask, &clip.mask(20, 20, &pool).unwrap()));
    }
}
