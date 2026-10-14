//! Device rectangles and the clip stack state.
//!
//! A clip state is the intersection of every clip rect pushed so far (used
//! for geometric clipping and culling) plus the chain of rounded clips. The
//! coverage mask is only built when a drawing actually needs it.

use std::cell::OnceCell;
use std::rc::Rc;

use tiny_skia::{FillRule, Mask, Transform};

use super::shapes;

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

/// A rounded clip shape in a clip chain.
#[derive(Debug)]
struct RoundedClip {
    rect: DevRect,
    radii: [f32; 4],
    parent: Option<Rc<RoundedClip>>,
}

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

    pub fn has_rounded(&self) -> bool {
        self.rounded.is_some()
    }

    /// Coverage mask of the clip region for a `width` × `height` area
    /// (`None` if the mask cannot be allocated).
    pub fn mask(&self, width: u32, height: u32) -> Option<Rc<Mask>> {
        self.mask
            .get_or_init(|| self.build_mask(width, height).map(Rc::new))
            .clone()
    }

    fn build_mask(&self, width: u32, height: u32) -> Option<Mask> {
        let mut mask = Mask::new(width, height)?;
        fill_rect_coverage(&mut mask, self.rect);
        let mut next = self.rounded.as_deref();
        let mut guard = 0usize;
        while let Some(clip) = next {
            match shapes::rounded_rect_path(clip.rect, clip.radii) {
                Some(path) => {
                    mask.intersect_path(&path, FillRule::Winding, true, Transform::identity())
                }
                None => mask.clear(),
            }
            next = clip.parent.as_deref();
            guard += 1;
            if guard > 4096 {
                break;
            }
        }
        Some(mask)
    }
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
    let xs: Vec<f32> = (cx0..cx1).map(|x| coverage(x, r.x0, r.x1)).collect();
    let data = mask.data_mut();
    for y in cy0..cy1 {
        let cy = coverage(y, r.y0, r.y1);
        let Some(row) = data.get_mut(y * w + cx0..y * w + cx1) else {
            continue;
        };
        for (m, cx) in row.iter_mut().zip(&xs) {
            *m = (cx * cy * 255.0).round() as u8;
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
    fn lazy_rounded_mask() {
        let full = ClipState::full(20, 20);
        let clip = full.push(DevRect::new(0.0, 0.0, 20.0, 20.0), [10.0; 4]);
        assert!(clip.has_rounded());
        let mask = clip.mask(20, 20).unwrap();
        // Centre inside, corner outside the circle.
        assert_eq!(mask.data()[10 * 20 + 10], 255);
        assert_eq!(mask.data()[0], 0);
        // Same Rc on the second call.
        assert!(Rc::ptr_eq(&mask, &clip.mask(20, 20).unwrap()));
    }
}
