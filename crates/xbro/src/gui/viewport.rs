//! Page-area geometry: zoom levels, CSS px ↔ device px conversions, scroll
//! clamping, scrollbars and scrolling things into view.

use xbro_engine::{Point, Rect, Size};

/// Zoom levels offered by Ctrl+= / Ctrl+-.
pub const ZOOM_STEPS: [f32; 12] = [
    0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

/// The next zoom step above `zoom` (stays at the largest).
pub fn zoom_in(zoom: f32) -> f32 {
    ZOOM_STEPS
        .iter()
        .copied()
        .find(|&z| z > zoom + 1e-3)
        .unwrap_or(ZOOM_STEPS[ZOOM_STEPS.len() - 1])
}

/// The next zoom step below `zoom` (stays at the smallest).
pub fn zoom_out(zoom: f32) -> f32 {
    ZOOM_STEPS
        .iter()
        .rev()
        .copied()
        .find(|&z| z < zoom - 1e-3)
        .unwrap_or(ZOOM_STEPS[0])
}

/// Keeps a scroll position inside `[0, document − viewport]` on both axes.
pub fn clamp_scroll(scroll: Point, document: Size, viewport: Size) -> Point {
    let max_x = (document.width - viewport.width).max(0.0);
    let max_y = (document.height - viewport.height).max(0.0);
    let clamp = |v: f32, max: f32| {
        if v.is_finite() {
            v.clamp(0.0, max)
        } else {
            0.0
        }
    };
    Point::new(clamp(scroll.x, max_x), clamp(scroll.y, max_y))
}

/// Where the page is drawn in the window, in device (physical) pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PageArea {
    /// Top of the page area (bottom of the browser chrome).
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Device pixels per CSS px: HiDPI factor × zoom.
    pub scale: f32,
}

impl PageArea {
    /// The page area of a `width`×`height` px window whose chrome takes
    /// `top_px` above and `bottom_px` below the page.
    pub fn new(width: u32, height: u32, top_px: u32, bottom_px: u32, dpi: f32, zoom: f32) -> Self {
        let scale = dpi * zoom;
        Self {
            top: top_px.min(height) as i32,
            width,
            height: height.saturating_sub(top_px).saturating_sub(bottom_px),
            scale: if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            },
        }
    }

    /// Layout viewport in CSS px (at least 1×1).
    pub fn viewport(self) -> Size {
        Size::new(
            (self.width as f32 / self.scale).max(1.0),
            (self.height as f32 / self.scale).max(1.0),
        )
    }

    /// Whether a window point (device px) lies inside the page area.
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= 0.0
            && x < self.width as f32
            && y >= self.top as f32
            && y < self.top as f32 + self.height as f32
    }

    /// Document point (CSS px) under a window point (device px).
    pub fn to_document(self, x: f32, y: f32, scroll: Point) -> Point {
        Point::new(
            x / self.scale + scroll.x,
            (y - self.top as f32) / self.scale + scroll.y,
        )
    }

    /// A document rect mapped to window device pixels.
    pub fn to_device(self, rect: Rect, scroll: Point) -> Rect {
        Rect::new(
            (rect.x - scroll.x) * self.scale,
            (rect.y - scroll.y) * self.scale + self.top as f32,
            rect.width * self.scale,
            rect.height * self.scale,
        )
    }

    /// The page area as a device-pixel rect.
    pub fn rect(self) -> Rect {
        Rect::new(0.0, self.top as f32, self.width as f32, self.height as f32)
    }
}

/// Scroll position (CSS px, on one axis) that brings `[start, start + len)`
/// into a view of `view` px currently scrolled to `scroll`: unchanged when
/// already fully visible, else the item is shown with a `margin` (centred
/// when `center` is set).
pub fn reveal(start: f32, len: f32, scroll: f32, view: f32, margin: f32, center: bool) -> f32 {
    let end = start + len;
    if start >= scroll && end <= scroll + view {
        return scroll;
    }
    if center || len + 2.0 * margin > view {
        return start + len / 2.0 - view / 2.0;
    }
    if start < scroll {
        start - margin
    } else {
        end + margin - view
    }
}

/// Scroll position that brings a document rect into view (see [`reveal`]),
/// clamped to the document.
pub fn scroll_into_view(
    rect: Rect,
    scroll: Point,
    viewport: Size,
    document: Size,
    center: bool,
) -> Point {
    let target = Point::new(
        reveal(rect.x, rect.width, scroll.x, viewport.width, 20.0, center),
        reveal(rect.y, rect.height, scroll.y, viewport.height, 20.0, center),
    );
    clamp_scroll(target, document, viewport)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

/// Thickness of the scrollbar hit area (logical px); the thumb is drawn thinner.
pub const SCROLLBAR_SIZE: f32 = 12.0;
const MIN_THUMB: f32 = 24.0;

/// Geometry of an overlay scrollbar along one edge of the page area, in
/// logical px.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Scrollbar {
    pub axis: Axis,
    pub track: Rect,
    pub thumb: Rect,
    /// Scroll range in CSS px (document − viewport, > 0).
    pub max_scroll: f32,
}

impl Scrollbar {
    /// The scrollbar for `page` (logical rect) when the document overflows
    /// the viewport on `axis`.
    pub fn new(
        axis: Axis,
        page: Rect,
        document: Size,
        viewport: Size,
        scroll: Point,
    ) -> Option<Self> {
        let (doc, view, pos) = match axis {
            Axis::Vertical => (document.height, viewport.height, scroll.y),
            Axis::Horizontal => (document.width, viewport.width, scroll.x),
        };
        let max_scroll = doc - view;
        if !(max_scroll > 0.5 && doc.is_finite() && view > 0.0) {
            return None;
        }
        let track = match axis {
            // The vertical track stops above the horizontal one.
            Axis::Vertical => Rect::new(
                page.right() - SCROLLBAR_SIZE,
                page.y,
                SCROLLBAR_SIZE,
                page.height - SCROLLBAR_SIZE,
            ),
            Axis::Horizontal => Rect::new(
                page.x,
                page.bottom() - SCROLLBAR_SIZE,
                page.width - SCROLLBAR_SIZE,
                SCROLLBAR_SIZE,
            ),
        };
        let track_len = match axis {
            Axis::Vertical => track.height,
            Axis::Horizontal => track.width,
        };
        if track_len < MIN_THUMB {
            return None;
        }
        let thumb_len = (track_len * view / doc).clamp(MIN_THUMB, track_len);
        let offset = (track_len - thumb_len) * (pos / max_scroll).clamp(0.0, 1.0);
        let thumb = match axis {
            Axis::Vertical => Rect::new(track.x, track.y + offset, track.width, thumb_len),
            Axis::Horizontal => Rect::new(track.x + offset, track.y, thumb_len, track.height),
        };
        Some(Self {
            axis,
            track,
            thumb,
            max_scroll,
        })
    }

    /// Scroll position for a thumb whose start is dragged to `thumb_start`
    /// (logical px along the axis).
    pub fn scroll_for_thumb(&self, thumb_start: f32) -> f32 {
        let (track_start, track_len, thumb_len) = match self.axis {
            Axis::Vertical => (self.track.y, self.track.height, self.thumb.height),
            Axis::Horizontal => (self.track.x, self.track.width, self.thumb.width),
        };
        let free = track_len - thumb_len;
        if free <= 0.0 {
            return 0.0;
        }
        ((thumb_start - track_start) / free).clamp(0.0, 1.0) * self.max_scroll
    }

    /// Start of the thumb along the axis.
    pub fn thumb_start(&self) -> f32 {
        match self.axis {
            Axis::Vertical => self.thumb.y,
            Axis::Horizontal => self.thumb.x,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_go_up_and_down_and_saturate() {
        assert_eq!(zoom_in(1.0), 1.1);
        assert_eq!(zoom_out(1.0), 0.9);
        assert_eq!(zoom_in(0.67), 0.8);
        assert_eq!(zoom_in(3.0), 3.0);
        assert_eq!(zoom_out(0.5), 0.5);
        // Off-step values snap to the neighbouring steps.
        assert_eq!(zoom_in(1.3), 1.5);
        assert_eq!(zoom_out(1.3), 1.25);
        let mut z = 0.5;
        let mut seen = vec![z];
        while zoom_in(z) != z {
            z = zoom_in(z);
            seen.push(z);
        }
        assert_eq!(seen, ZOOM_STEPS);
    }

    #[test]
    fn scroll_is_clamped_to_the_document() {
        let doc = Size::new(1000.0, 3000.0);
        let view = Size::new(800.0, 600.0);
        assert_eq!(clamp_scroll(Point::new(-5.0, -1.0), doc, view), Point::ZERO);
        assert_eq!(
            clamp_scroll(Point::new(500.0, 9999.0), doc, view),
            Point::new(200.0, 2400.0)
        );
        // A document smaller than the viewport cannot scroll.
        let small = Size::new(300.0, 200.0);
        assert_eq!(
            clamp_scroll(Point::new(10.0, 10.0), small, view),
            Point::ZERO
        );
        assert_eq!(
            clamp_scroll(Point::new(f32::NAN, f32::INFINITY), doc, view),
            Point::ZERO
        );
    }

    #[test]
    fn page_area_conversions() {
        // 800×600 logical window at 2× with 74 px of chrome on top.
        let area = PageArea::new(1600, 1200, 148, 0, 2.0, 1.0);
        assert_eq!(area.height, 1052);
        assert_eq!(area.viewport(), Size::new(800.0, 526.0));
        let scroll = Point::new(0.0, 100.0);
        let doc = area.to_document(200.0, 148.0 + 40.0, scroll);
        assert_eq!(doc, Point::new(100.0, 120.0));
        let dev = area.to_device(Rect::new(100.0, 120.0, 10.0, 5.0), scroll);
        assert_eq!(dev, Rect::new(200.0, 188.0, 20.0, 10.0));
        assert!(area.contains(10.0, 150.0));
        assert!(!area.contains(10.0, 100.0));

        // Zoom shrinks the CSS viewport.
        let zoomed = PageArea::new(1000, 800, 100, 50, 1.0, 2.0);
        assert_eq!(zoomed.viewport(), Size::new(500.0, 325.0));
        // A window smaller than the chrome still gives a usable viewport.
        let tiny = PageArea::new(10, 20, 74, 34, 1.0, 1.0);
        assert_eq!(tiny.height, 0);
        assert_eq!(tiny.viewport(), Size::new(10.0, 1.0));
    }

    #[test]
    fn reveal_keeps_visible_items_and_scrolls_to_hidden_ones() {
        assert_eq!(reveal(100.0, 20.0, 50.0, 500.0, 20.0, false), 50.0);
        // Below the view: scrolled so that it ends `margin` above the bottom.
        assert_eq!(reveal(800.0, 20.0, 0.0, 500.0, 20.0, false), 340.0);
        // Above: shown `margin` below the top.
        assert_eq!(reveal(100.0, 20.0, 400.0, 500.0, 20.0, false), 80.0);
        // Centred on request.
        assert_eq!(reveal(1000.0, 20.0, 0.0, 500.0, 20.0, true), 760.0);
        let p = scroll_into_view(
            Rect::new(0.0, 5000.0, 10.0, 10.0),
            Point::ZERO,
            Size::new(800.0, 600.0),
            Size::new(800.0, 5100.0),
            true,
        );
        assert_eq!(p, Point::new(0.0, 4500.0));
    }

    #[test]
    fn scrollbar_geometry_and_dragging() {
        let page = Rect::new(0.0, 74.0, 800.0, 524.0);
        let doc = Size::new(800.0, 2096.0);
        let view = Size::new(800.0, 524.0);
        assert!(Scrollbar::new(Axis::Vertical, page, view, view, Point::ZERO).is_none());
        assert!(Scrollbar::new(Axis::Horizontal, page, doc, view, Point::ZERO).is_none());

        let bar = Scrollbar::new(Axis::Vertical, page, doc, view, Point::ZERO).unwrap();
        assert_eq!(bar.track, Rect::new(788.0, 74.0, 12.0, 512.0));
        assert_eq!(bar.thumb, Rect::new(788.0, 74.0, 12.0, 128.0));
        assert_eq!(bar.max_scroll, 1572.0);
        let end = Scrollbar::new(Axis::Vertical, page, doc, view, Point::new(0.0, 1572.0)).unwrap();
        assert_eq!(end.thumb.bottom(), end.track.bottom());
        // Dragging the thumb to the middle of its range scrolls halfway.
        let mid = bar.track.y + (bar.track.height - bar.thumb.height) / 2.0;
        assert_eq!(bar.scroll_for_thumb(mid), 786.0);
        assert_eq!(bar.scroll_for_thumb(-100.0), 0.0);
        assert_eq!(bar.scroll_for_thumb(10_000.0), 1572.0);

        // Very long documents keep a grabbable thumb.
        let long = Scrollbar::new(
            Axis::Vertical,
            page,
            Size::new(800.0, 1e6),
            view,
            Point::ZERO,
        )
        .unwrap();
        assert_eq!(long.thumb.height, 24.0);
    }
}
