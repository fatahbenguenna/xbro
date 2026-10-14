//! Frame composition: the page (rasterised into a cached layer), overlays
//! (find highlights, scrollbars, status bubble) and the browser chrome.

use std::f32::consts::PI;
use std::time::Instant;

use xbro_engine::{Point, RasterOptions, Rect};
use xbro_paint::tiny_skia::{self, Pixmap};

use super::address_bar::{self, url_segments};
use super::browser::{Browser, Drag, Focus};
use super::chrome::{ChromeElement, ChromeLayout, CHROME_HEIGHT, UI_TEXT_SIZE};
use super::line_edit::LineEdit;
use super::paint::{caret_x, icons, visible_range, Painter, Rgba};
use super::tabs::TabId;
use super::viewport::{self, Axis, PageArea};

mod colors {
    use super::Rgba;

    pub const STRIP: Rgba = Rgba::rgb(222, 225, 230);
    pub const TOOLBAR: Rgba = Rgba::rgb(255, 255, 255);
    pub const TAB_HOVER: Rgba = Rgba::rgb(235, 237, 240);
    pub const TAB_SEPARATOR: Rgba = Rgba::rgb(170, 174, 180);
    pub const TEXT: Rgba = Rgba::rgb(32, 33, 36);
    pub const TEXT_DIM: Rgba = Rgba::rgb(95, 99, 104);
    pub const PLACEHOLDER: Rgba = Rgba::rgb(128, 134, 139);
    pub const ICON: Rgba = Rgba::rgb(60, 64, 67);
    pub const ICON_DISABLED: Rgba = Rgba::rgb(189, 193, 198);
    pub const BUTTON_HOVER: Rgba = Rgba(0, 0, 0, 22);
    pub const BUTTON_PRESSED: Rgba = Rgba(0, 0, 0, 46);
    pub const FIELD: Rgba = Rgba::rgb(241, 243, 244);
    pub const FIELD_HOVER: Rgba = Rgba::rgb(232, 234, 237);
    pub const FIELD_FOCUSED: Rgba = Rgba::rgb(255, 255, 255);
    pub const BADGE_HOVER: Rgba = Rgba::rgb(232, 240, 254);
    pub const BADGE_PRESSED: Rgba = Rgba::rgb(210, 227, 252);
    pub const NOT_FOUND: Rgba = Rgba::rgb(252, 228, 228);
    pub const ACCENT: Rgba = Rgba::rgb(26, 115, 232);
    pub const SELECTION: Rgba = Rgba::rgb(187, 214, 251);
    pub const BORDER: Rgba = Rgba::rgb(218, 220, 224);
    pub const MATCH: Rgba = Rgba(255, 221, 0, 110);
    pub const CURRENT_MATCH: Rgba = Rgba(255, 128, 0, 150);
    pub const THUMB: Rgba = Rgba(0, 0, 0, 80);
    pub const THUMB_ACTIVE: Rgba = Rgba(0, 0, 0, 140);
    pub const STATUS: Rgba = Rgba::rgb(248, 249, 250);
    pub const STATUS_BORDER: Rgba = Rgba::rgb(196, 199, 204);
    pub const PAGE_BLANK: Rgba = Rgba::rgb(255, 255, 255);
}

/// Largest window area drawn, in device pixels (two buffers of this size
/// are kept: the frame and the page layer).
const MAX_FRAME_PIXELS: u64 = 1 << 27;

/// Field texts longer than this (bytes) are drawn from their visible part only.
const LONG_FIELD_TEXT: usize = 256;

/// What the cached page layer shows.
#[derive(Copy, Clone, Debug, PartialEq)]
struct LayerKey {
    tab: TabId,
    has_page: bool,
    scroll: Point,
    scale: f32,
    width: u32,
    height: u32,
}

/// Owns the frame and page-layer pixmaps between frames.
#[derive(Default)]
pub struct Renderer {
    frame: Option<Pixmap>,
    layer: Option<Pixmap>,
    layer_key: Option<LayerKey>,
}

impl Renderer {
    /// Draws a frame of the browser; `None` if the window has no area.
    pub fn render(&mut self, b: &mut Browser, now: Instant) -> Option<&Pixmap> {
        let (width, height) = (b.metrics.width, b.metrics.height);
        if u64::from(width) * u64::from(height) > MAX_FRAME_PIXELS {
            return None; // absurd window size: do not try to allocate it
        }
        if self.frame.as_ref().map(|f| (f.width(), f.height())) != Some((width, height)) {
            self.frame = Some(Pixmap::new(width, height)?);
        }
        let page_dirty = b.take_page_dirty();
        let area = b.page_area();
        let layout = b.layout();
        self.draw_page(b, &area, page_dirty);

        let frame = self.frame.as_mut()?;
        let mut p = Painter::new(frame, b.metrics.dpi, &b.fonts);
        draw_overlays(&mut p, b, &layout, &area);
        draw_tab_strip(&mut p, b, &layout, now);
        draw_toolbar(&mut p, b, &layout, now);
        draw_find_bar(&mut p, b, &layout);
        self.frame.as_ref()
    }

    /// Rasterises the active page into the layer when needed and copies it
    /// into the frame.
    fn draw_page(&mut self, b: &Browser, area: &PageArea, dirty: bool) {
        if area.height == 0 || area.width == 0 {
            self.layer_key = None;
            return;
        }
        let tab = b.active();
        let key = LayerKey {
            tab: tab.map_or(0, |t| t.id),
            has_page: tab.is_some_and(|t| t.page.is_some()),
            scroll: tab.map_or(Point::ZERO, |t| t.scroll),
            scale: area.scale,
            width: area.width,
            height: area.height,
        };
        if self.layer.as_ref().map(|l| (l.width(), l.height())) != Some((area.width, area.height)) {
            self.layer = Pixmap::new(area.width, area.height);
            self.layer_key = None;
        }
        let Some(layer) = self.layer.as_mut() else {
            return;
        };
        if dirty || self.layer_key != Some(key) {
            match tab.and_then(|t| t.page.as_ref().map(|p| (t, p))) {
                Some((tab, page)) => page.render(
                    layer,
                    &RasterOptions {
                        scroll: tab.scroll,
                        scale: area.scale,
                        offset: (0, 0),
                        height: None,
                    },
                ),
                None => {
                    let c = colors::PAGE_BLANK;
                    layer.fill(tiny_skia::Color::from_rgba8(c.0, c.1, c.2, c.3));
                }
            }
            self.layer_key = Some(key);
        }
        let Some(frame) = self.frame.as_mut() else {
            return;
        };
        let row = area.width as usize * 4;
        let start = area.top.max(0) as usize * row;
        let len = area.height as usize * row;
        if let (Some(dst), Some(src)) = (
            frame.data_mut().get_mut(start..start + len),
            layer.data().get(..len),
        ) {
            dst.copy_from_slice(src);
        }
    }
}

/// Find highlights, scrollbars and the status bubble, over the page.
fn draw_overlays(p: &mut Painter, b: &Browser, layout: &ChromeLayout, area: &PageArea) {
    let Some(tab) = b.active() else {
        return;
    };
    let page_rect = area.rect();
    if let (true, Some(page)) = (tab.find.open && !tab.find.matches.is_empty(), &tab.page) {
        // Content scrolling under a fixed header (or any fixed box with a
        // background) is hidden, and so are its highlights.
        let backdrops: Vec<Rect> = page
            .fixed_backdrops()
            .into_iter()
            .map(|r| area.to_device(r, Point::ZERO))
            .collect();
        for (i, m) in tab.find.matches.iter().enumerate() {
            // Matches in fixed content stay at their viewport position.
            let dev = area.to_device(m.viewport_rect(tab.scroll), Point::ZERO);
            let Some(visible) = dev.intersection(&page_rect) else {
                continue;
            };
            let color = if Some(i) == tab.find.current {
                colors::CURRENT_MATCH
            } else {
                colors::MATCH
            };
            let mut pieces = vec![visible];
            if !m.fixed {
                for hole in &backdrops {
                    pieces = pieces
                        .into_iter()
                        .flat_map(|r| viewport::subtract(r, *hole))
                        .collect();
                }
            }
            for piece in pieces {
                p.fill_device_rect(piece, color);
            }
        }
    }

    for axis in [Axis::Vertical, Axis::Horizontal] {
        let Some(bar) = b.scrollbar(axis) else {
            continue;
        };
        let active = matches!(b.drag, Some(Drag::Scrollbar { axis: a, .. }) if a == axis);
        let hovered = b.scrollbar_hover == Some(axis);
        let (thickness, color) = if active || hovered {
            (8.0, colors::THUMB_ACTIVE)
        } else {
            (6.0, colors::THUMB)
        };
        let t = bar.thumb;
        let thumb = match axis {
            Axis::Vertical => Rect::new(
                t.right() - thickness - 2.0,
                t.y + 2.0,
                thickness,
                t.height - 4.0,
            ),
            Axis::Horizontal => Rect::new(
                t.x + 2.0,
                t.bottom() - thickness - 2.0,
                t.width - 4.0,
                thickness,
            ),
        };
        p.fill_rounded(thumb, [thickness / 2.0; 4], color);
    }

    if let Some(text) = b.status_text() {
        // Huge (data:) URLs: only the start can be shown anyway.
        let text: String = text.chars().take(1000).collect();
        let page = layout.page;
        let height = STATUS_HEIGHT;
        let max_width = (page.width * 0.6).max(160.0).min(page.width - 4.0);
        let text = b
            .fonts
            .truncate(&text, UI_TEXT_SIZE - 1.0, false, p.dpi, max_width - 16.0);
        let width = b.fonts.measure(&text, UI_TEXT_SIZE - 1.0, false, p.dpi) + 16.0;
        let pointer = b.mouse.map(|(x, y)| Point::new(x / p.dpi, y / p.dpi));
        let (rect, right) = status_bubble(page, width, height, pointer);
        // A one-pixel border on the sides facing the page, rounded at the top.
        let (outer_radii, inner_radii, inner) = if right {
            (
                [5.0, 0.0, 0.0, 0.0],
                [4.0, 0.0, 0.0, 0.0],
                Rect::new(
                    rect.x + 1.0,
                    rect.y + 1.0,
                    rect.width - 1.0,
                    rect.height - 1.0,
                ),
            )
        } else {
            (
                [0.0, 5.0, 0.0, 0.0],
                [0.0, 4.0, 0.0, 0.0],
                Rect::new(rect.x, rect.y + 1.0, rect.width - 1.0, rect.height - 1.0),
            )
        };
        p.fill_rounded(rect, outer_radii, colors::STATUS_BORDER);
        p.fill_rounded(inner, inner_radii, colors::STATUS);
        p.text(
            inner.x + if right { 8.0 } else { 9.0 },
            inner.y + inner.height / 2.0,
            &text,
            UI_TEXT_SIZE - 1.0,
            colors::TEXT_DIM,
            false,
        );
    }
}

/// Height of the status bubble, logical px.
const STATUS_HEIGHT: f32 = 22.0;
/// Margin above the status bubble where the pointer moves it aside.
const STATUS_MARGIN: f32 = 8.0;

/// Whether a pointer at `p` (logical px) can affect where the status bubble
/// goes: the band along the bottom of the page area where it is drawn.
pub fn in_status_band(page: Rect, p: Point) -> bool {
    p.y >= page.bottom() - STATUS_HEIGHT - STATUS_MARGIN + 1.0 && p.y < page.bottom() + 1.0
}

/// Where the status bubble goes (logical px): in the bottom-left corner of
/// the page, or in the bottom-right one when the pointer is over that place
/// (it would hide the link being pointed at). Returns the rect and whether
/// it is on the right.
fn status_bubble(page: Rect, width: f32, height: f32, pointer: Option<Point>) -> (Rect, bool) {
    let left = Rect::new(
        page.x - 1.0,
        page.bottom() - height + 1.0,
        width + 1.0,
        height,
    );
    // With a margin above and after it: the pointer is often a little
    // above or after the part of a link that the bubble would hide.
    let near = Rect::new(
        left.x,
        left.y - STATUS_MARGIN,
        left.width + 2.0 * STATUS_MARGIN,
        left.height + STATUS_MARGIN,
    );
    if pointer.is_some_and(|pt| near.contains(pt)) {
        let right = Rect::new(page.right() - width, left.y, width + 1.0, height);
        (right, true)
    } else {
        (left, false)
    }
}

fn draw_tab_strip(p: &mut Painter, b: &Browser, layout: &ChromeLayout, now: Instant) {
    p.fill_rect(layout.tab_strip, colors::STRIP);
    let active = b.tabs.active_index();
    let phase = now.saturating_duration_since(b.epoch).as_secs_f32() * 2.0 * PI;
    let highlighted = |i: usize| i == active || b.hover == Some(ChromeElement::Tab(i));
    for (i, (slot, tab)) in layout.tabs.iter().zip(b.tabs.iter()).enumerate() {
        let r = slot.rect;
        let hovered =
            matches!(b.hover, Some(ChromeElement::Tab(j) | ChromeElement::TabClose(j)) if j == i);
        if i == active {
            // Joined to the toolbar below.
            let shape = Rect::new(r.x, r.y, r.width, layout.tab_strip.bottom() - r.y + 1.0);
            p.fill_rounded(shape, [8.0, 8.0, 0.0, 0.0], colors::TOOLBAR);
        } else if hovered {
            let shape = Rect::new(r.x + 2.0, r.y + 2.0, r.width - 4.0, r.height - 6.0);
            p.fill_rounded(shape, [6.0; 4], colors::TAB_HOVER);
        }
        // Separator between two plain tabs.
        if i + 1 < layout.tabs.len() && !highlighted(i) && !highlighted(i + 1) && !hovered {
            p.fill_rect(
                Rect::new(r.right() - 0.5, r.y + 8.0, 1.0, r.height - 16.0),
                colors::TAB_SEPARATOR,
            );
        }
        let cy = r.y + (r.height - 2.0) / 2.0;
        let mut x = r.x + 12.0;
        let right = slot.close.map_or(r.right() - 10.0, |c| c.x - 4.0);
        if tab.is_loading() && right - x > 24.0 {
            icons::spinner(p, x + 6.0, cy, 6.0, phase, colors::ACCENT);
            x += 20.0;
        }
        if right - x > 8.0 {
            let bold = i == active;
            let title = b
                .fonts
                .truncate(&tab.title(), UI_TEXT_SIZE, bold, p.dpi, right - x);
            let color = if i == active {
                colors::TEXT
            } else {
                colors::TEXT_DIM
            };
            p.text(x, cy, &title, UI_TEXT_SIZE, color, bold);
        }
        if let Some(c) = slot.close {
            let (ccx, ccy) = (c.x + c.width / 2.0, c.y + c.height / 2.0);
            if b.hover == Some(ChromeElement::TabClose(i)) {
                let bg = if b.pressed == Some(ChromeElement::TabClose(i)) {
                    colors::BUTTON_PRESSED
                } else {
                    colors::BUTTON_HOVER
                };
                p.fill_circle(ccx, ccy, c.width / 2.0, bg);
            }
            icons::cross(p, ccx, ccy, 4.0, 1.5, colors::ICON);
        }
    }
    let n = layout.new_tab;
    button_background(p, b, ChromeElement::NewTab, n, 6.0);
    icons::plus(p, n.x + n.width / 2.0, n.y + n.height / 2.0, colors::ICON);
}

/// Hover / pressed background of a chrome button.
fn button_background(p: &mut Painter, b: &Browser, element: ChromeElement, r: Rect, radius: f32) {
    let color = if b.pressed == Some(element) && b.hover == Some(element) {
        colors::BUTTON_PRESSED
    } else if b.hover == Some(element) {
        colors::BUTTON_HOVER
    } else {
        return;
    };
    p.fill_rounded(r, [radius; 4], color);
}

fn draw_toolbar(p: &mut Painter, b: &Browser, layout: &ChromeLayout, now: Instant) {
    p.fill_rect(layout.toolbar, colors::TOOLBAR);
    p.fill_rect(
        Rect::new(0.0, CHROME_HEIGHT - 1.0, layout.width, 1.0),
        colors::BORDER,
    );
    let tab = b.active();
    let can_back = tab.and_then(|t| t.back_index()).is_some();
    let can_forward = tab.and_then(|t| t.forward_index()).is_some();
    let loading = tab.is_some_and(|t| t.is_loading());
    let buttons = [
        (ChromeElement::Back, layout.back, can_back),
        (ChromeElement::Forward, layout.forward, can_forward),
        (ChromeElement::Reload, layout.reload, tab.is_some()),
        (ChromeElement::Home, layout.home, true),
    ];
    for (element, r, enabled) in buttons {
        if enabled {
            button_background(p, b, element, r, r.width / 2.0);
        }
        let color = if enabled {
            colors::ICON
        } else {
            colors::ICON_DISABLED
        };
        let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        match element {
            ChromeElement::Back => icons::arrow_left(p, cx, cy, color),
            ChromeElement::Forward => icons::arrow_right(p, cx, cy, color),
            ChromeElement::Reload if loading => icons::cross(p, cx, cy, 5.5, 1.8, color),
            ChromeElement::Reload => icons::reload(p, cx, cy, color),
            _ => icons::home(p, cx, cy, color),
        }
    }

    // Address field.
    let r = layout.address;
    let focused = b.focus == Focus::Address;
    let radius = r.height / 2.0;
    if focused {
        p.fill_rounded(r, [radius; 4], colors::FIELD_FOCUSED);
        p.stroke_rounded(r, radius, 2.0, colors::ACCENT);
    } else {
        let bg = if b.hover == Some(ChromeElement::Address) {
            colors::FIELD_HOVER
        } else {
            colors::FIELD
        };
        p.fill_rounded(r, [radius; 4], bg);
    }
    let inner = layout.address_text();
    if focused {
        draw_edit(
            p,
            b,
            inner,
            &b.address.edit,
            b.address.scroll_x,
            true,
            "Search or enter address",
        );
    } else {
        let text = address_bar::display_text(tab.and_then(|t| t.display_url()));
        if text.is_empty() {
            draw_edit(
                p,
                b,
                inner,
                &LineEdit::default(),
                0.0,
                false,
                "Search or enter address",
            );
        } else {
            let cy = inner.y + inner.height / 2.0;
            let mut x = inner.x;
            for (range, dim) in url_segments(&text) {
                if x >= inner.right() {
                    break;
                }
                let segment = text.get(range).unwrap_or_default();
                // Long URLs: only what can be visible is shaped.
                let segment: String = segment.chars().take(400).collect();
                let color = if dim { colors::TEXT_DIM } else { colors::TEXT };
                p.text_clipped(x, cy, &segment, UI_TEXT_SIZE, color, false, inner);
                x += b.fonts.measure(&segment, UI_TEXT_SIZE, false, p.dpi);
            }
        }
    }

    if let Some(badge) = layout.zoom_badge {
        draw_zoom_badge(p, b, badge);
    }

    if let Some(progress) = b.load_progress(now) {
        p.fill_rect(
            Rect::new(0.0, CHROME_HEIGHT - 2.0, layout.width * progress, 2.0),
            colors::ACCENT,
        );
    }
}

/// The zoom level ("125%") at the right end of the address field; a click
/// resets it.
fn draw_zoom_badge(p: &mut Painter, b: &Browser, r: Rect) {
    let radius = r.height / 2.0;
    let element = ChromeElement::ZoomReset;
    let bg = if b.hover == Some(element) && b.pressed == Some(element) {
        colors::BADGE_PRESSED
    } else if b.hover == Some(element) {
        colors::BADGE_HOVER
    } else {
        colors::FIELD_FOCUSED
    };
    p.fill_rounded(r, [radius; 4], bg);
    p.stroke_rounded(r, radius, 1.0, colors::BORDER);
    let label = format!("{}%", (b.zoom() * 100.0).round());
    let size = UI_TEXT_SIZE - 1.0;
    let w = b.fonts.measure(&label, size, false, p.dpi);
    p.text(
        r.x + (r.width - w) / 2.0,
        r.y + r.height / 2.0,
        &label,
        size,
        colors::TEXT,
        false,
    );
}

/// Draws an editable line of text in `inner` (selection, text, caret), or
/// the placeholder when empty and unfocused.
fn draw_edit(
    p: &mut Painter,
    b: &Browser,
    inner: Rect,
    edit: &LineEdit,
    scroll_x: f32,
    focused: bool,
    placeholder: &str,
) {
    let cy = inner.y + inner.height / 2.0;
    if edit.text().is_empty() && !focused {
        p.text_clipped(
            inner.x,
            cy,
            placeholder,
            UI_TEXT_SIZE,
            colors::PLACEHOLDER,
            false,
            inner,
        );
        return;
    }
    let stops = b.fonts.caret_stops(edit.text(), UI_TEXT_SIZE, p.dpi);
    let x0 = inner.x - scroll_x;
    let line_height = UI_TEXT_SIZE + 5.0;
    if focused {
        if let Some(sel) = edit.selection() {
            let (a, z) = (caret_x(&stops, sel.start), caret_x(&stops, sel.end));
            let rect = Rect::new(x0 + a, cy - line_height / 2.0, z - a, line_height);
            if let Some(visible) = rect.intersection(&inner) {
                p.fill_rect(visible, colors::SELECTION);
            }
        }
    }
    if edit.text().is_empty() && focused {
        p.text_clipped(
            inner.x + 2.0,
            cy,
            placeholder,
            UI_TEXT_SIZE,
            colors::PLACEHOLDER,
            false,
            inner,
        );
    } else {
        // Only the part of a long text that can show is shaped and drawn
        // again (the caret stops are cached, see `UiFonts`).
        let text = edit.text();
        let range = if text.len() > LONG_FIELD_TEXT {
            visible_range(&stops, scroll_x, scroll_x + inner.width, 4.0 * UI_TEXT_SIZE)
        } else {
            0..text.len()
        };
        let (start, shown) = match text.get(range.clone()) {
            Some(slice) => (range.start, slice),
            None => (0, text),
        };
        p.text_clipped(
            x0 + caret_x(&stops, start),
            cy,
            shown,
            UI_TEXT_SIZE,
            colors::TEXT,
            false,
            inner,
        );
    }
    if focused {
        let x = (x0 + caret_x(&stops, edit.caret())).clamp(inner.x, inner.right());
        p.fill_rect(
            Rect::new(
                x,
                cy - line_height / 2.0 + 1.0,
                1.0 / p.dpi.max(1.0) * 1.5,
                line_height - 2.0,
            ),
            colors::TEXT,
        );
    }
}

fn draw_find_bar(p: &mut Painter, b: &Browser, layout: &ChromeLayout) {
    let (Some(find), Some(tab)) = (layout.find, b.active()) else {
        return;
    };
    p.fill_rect(find.bar, colors::TOOLBAR);
    p.fill_rect(
        Rect::new(0.0, find.bar.y, layout.width, 1.0),
        colors::BORDER,
    );

    let focused = b.focus == Focus::Find;
    let r = find.input;
    let bg = if tab.find.not_found() {
        colors::NOT_FOUND
    } else {
        colors::FIELD_FOCUSED
    };
    p.fill_rounded(r, [6.0; 4], bg);
    let (border, width) = if focused {
        (colors::ACCENT, 2.0)
    } else {
        (colors::BORDER, 1.0)
    };
    p.stroke_rounded(r, 6.0, width, border);
    let inner = find.input_text();
    draw_edit(
        p,
        b,
        inner,
        &tab.find.query,
        b.find_scroll_x,
        focused,
        "Find in page",
    );

    let counter = tab.find.counter();
    if !counter.is_empty() {
        let c = find.counter;
        let w = b.fonts.measure(&counter, UI_TEXT_SIZE, false, p.dpi);
        let x = c.x + (c.width - w) / 2.0;
        p.text(
            x,
            c.y + c.height / 2.0,
            &counter,
            UI_TEXT_SIZE,
            colors::TEXT_DIM,
            false,
        );
    }

    let has_matches = !tab.find.matches.is_empty();
    for (element, r) in [
        (ChromeElement::FindPrev, find.prev),
        (ChromeElement::FindNext, find.next),
        (ChromeElement::FindClose, find.close),
    ] {
        let enabled = has_matches || element == ChromeElement::FindClose;
        if enabled {
            button_background(p, b, element, r, 6.0);
        }
        let color = if enabled {
            colors::ICON
        } else {
            colors::ICON_DISABLED
        };
        let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        match element {
            ChromeElement::FindPrev => icons::chevron_up(p, cx, cy, color),
            ChromeElement::FindNext => icons::chevron_down(p, cx, cy, color),
            _ => icons::cross(p, cx, cy, 4.5, 1.6, color),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::browser::tests::{browser, engine, finish, fixed_header_page, url};
    use super::*;

    fn pixel(frame: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * frame.width() + x) * 4) as usize;
        frame.data()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn the_status_bubble_moves_away_from_the_pointer() {
        let page = Rect::new(0.0, 74.0, 800.0, 526.0);
        let (rect, right) = status_bubble(page, 200.0, 22.0, None);
        assert!(!right);
        assert_eq!((rect.x, rect.bottom()), (-1.0, 601.0));
        let (_, right) = status_bubble(page, 200.0, 22.0, Some(Point::new(400.0, 590.0)));
        assert!(!right, "the pointer is elsewhere");
        let (rect, right) = status_bubble(page, 200.0, 22.0, Some(Point::new(100.0, 590.0)));
        assert!(right);
        assert_eq!(rect.right(), 801.0);
        assert_eq!(rect.bottom(), 601.0);
        // Just above or after the bubble counts too.
        let (_, right) = status_bubble(page, 200.0, 22.0, Some(Point::new(100.0, 573.0)));
        assert!(right);
        let (_, right) = status_bubble(page, 200.0, 22.0, Some(Point::new(210.0, 590.0)));
        assert!(right);
    }

    #[test]
    fn renders_chrome_page_and_find_bar() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        let mut r = Renderer::default();
        let now = Instant::now();
        // Loading, nothing committed yet: blank page area and a progress line.
        let frame = r.render(&mut b, now).unwrap();
        assert_eq!((frame.width(), frame.height()), (800, 600));
        assert_eq!(pixel(frame, 400, 300), [255, 255, 255, 255]);
        assert_eq!(
            pixel(frame, 1, CHROME_HEIGHT as u32 - 1)[..3],
            [26, 115, 232]
        );
        assert_eq!(pixel(frame, 790, 10)[..3], [222, 225, 230], "tab strip");

        finish(&mut b, &jobs, "A", 30);
        b.open_find();
        b.tabs.active_mut().unwrap().find.query.insert("needle");
        b.run_find();
        let frame = r.render(&mut b, now).unwrap();
        assert_eq!(
            pixel(frame, 790, 600 - 20)[..3],
            [255, 255, 255],
            "find bar"
        );
        // Same state: the cached page layer is reused and the frame is identical.
        let before = frame.data().to_vec();
        let frame = r.render(&mut b, now).unwrap();
        assert_eq!(frame.data(), &before[..]);

        // HiDPI: the frame follows the window in device pixels.
        b.resize(1600, 1200, 2.0);
        let frame = r.render(&mut b, now).unwrap();
        assert_eq!((frame.width(), frame.height()), (1600, 1200));
    }

    #[test]
    fn fixed_find_matches_are_highlighted_where_they_are_shown() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        let job = jobs.borrow_mut().pop().unwrap();
        let page = engine().load_html(
            &fixed_header_page(),
            job.request.url().clone(),
            job.viewport,
        );
        b.on_loaded(job.tab_id, job.nav_id, page);
        b.scroll_to(Point::new(0.0, 1500.0));
        b.open_find();
        b.tabs.active_mut().unwrap().find.query.insert("needle");
        b.run_find();
        let m = b.active().unwrap().find.current_match().unwrap();
        assert!(m.fixed);
        let mut r = Renderer::default();
        let frame = r.render(&mut b, Instant::now()).unwrap();
        // The current-match colour over the white header, at the match's
        // viewport position (the page is scrolled by 1500 px).
        let area = b.page_area();
        let dev = area.to_device(m.rect, Point::ZERO);
        let [red, green, blue, _] = pixel(frame, dev.x as u32 + 1, dev.y as u32 + 1);
        assert!(
            red > 200 && green < 230 && blue < 160,
            "{:?}",
            (red, green, blue)
        );
    }

    #[test]
    fn huge_field_texts_are_shaped_once() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 3);
        b.focus_address();
        let huge = format!("data:text/plain,{}", "0123456789".repeat(50_000));
        b.address.edit.insert(&huge);
        b.update_field_scrolls();
        let mut r = Renderer::default();
        let shapes = b.fonts.caret_shapes();
        let frame = r.render(&mut b, Instant::now()).unwrap();
        // The end of the text (where the caret is) is drawn.
        let inner = b.layout().address_text();
        let dark = (inner.x as u32..inner.right() as u32)
            .flat_map(|x| (inner.y as u32..inner.bottom() as u32).map(move |y| (x, y)))
            .filter(|&(x, y)| pixel(frame, x, y)[0] < 128)
            .count();
        if b.fonts.system.has_fonts() {
            assert!(dark > 50, "{dark} dark pixels");
        }
        r.render(&mut b, Instant::now()).unwrap();
        b.update_field_scrolls();
        assert_eq!(b.fonts.caret_shapes(), shapes, "no new shaping");
        // An edit shapes the new text once.
        b.address.edit.insert("!");
        b.update_field_scrolls();
        r.render(&mut b, Instant::now()).unwrap();
        assert_eq!(b.fonts.caret_shapes(), shapes + 1);
    }

    #[test]
    fn matches_under_a_fixed_header_are_not_highlighted() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        let job = jobs.borrow_mut().pop().unwrap();
        let page = engine().load_html(
            &fixed_header_page(),
            job.request.url().clone(),
            job.viewport,
        );
        b.on_loaded(job.tab_id, job.nav_id, page);
        b.open_find();
        b.tabs.active_mut().unwrap().find.query.insert("needle");
        b.run_find();
        // Put a match of the flow under the 40 px header (below its text).
        let m = b.active().unwrap().find.matches[10];
        assert!(!m.fixed);
        b.scroll_to(Point::new(0.0, m.rect.y - 22.0));
        let mut r = Renderer::default();
        let frame = r.render(&mut b, Instant::now()).unwrap();
        let area = b.page_area();
        let dev = area.to_device(m.viewport_rect(b.active().unwrap().scroll), Point::ZERO);
        assert!(dev.y > area.top as f32 && dev.y + 4.0 < area.top as f32 + 40.0);
        assert_eq!(
            pixel(frame, dev.x as u32 + 1, dev.y as u32 + 1),
            [255, 255, 255, 255]
        );
        // The next match, in view below the header, is highlighted.
        let next = b.active().unwrap().find.matches[11];
        let dev = area.to_device(next.viewport_rect(b.active().unwrap().scroll), Point::ZERO);
        let [_, _, blue, _] = pixel(frame, dev.x as u32 + 1, dev.y as u32 + 1);
        assert!(blue < 200, "highlighted: {blue}");
    }
}
