//! CPU rasterisation of display lists with tiny-skia.

use xbro_layout::Point;
use xbro_style::Color;
use xbro_text::{FontId, FontSystem};

use crate::display_list::DisplayList;
use crate::image::ImageProvider;

/// Where and how to rasterise.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RasterOptions {
    /// Document point (CSS px) drawn at the target's top-left (the scroll position).
    pub scroll: Point,
    /// Device pixels per CSS px (HiDPI factor × zoom).
    pub scale: f32,
    /// Offset in device pixels at which to draw inside the target (e.g. below the toolbar).
    pub offset: (i32, i32),
    /// Height in device pixels of the area to draw, starting at `offset.1`
    /// (the target may be taller, e.g. when it also holds the browser UI).
    /// `None` = to the bottom of the target.
    pub height: Option<u32>,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            scroll: Point::ZERO,
            scale: 1.0,
            offset: (0, 0),
            height: None,
        }
    }
}

/// Paints `list` into `target`: first fills the drawing area with
/// `list.background`, then draws every item intersecting the visible region
/// (items outside it are culled cheaply). Text is drawn from cached glyph
/// coverage masks blended with the run color; images are scaled with bilinear
/// filtering; SVG images are rendered with resvg at the destination size.
pub fn rasterize(
    list: &DisplayList,
    fonts: &FontSystem,
    images: &dyn ImageProvider,
    target: &mut tiny_skia::Pixmap,
    options: &RasterOptions,
) {
    let _ = (list, fonts, images, target, options);
    todo!("xbro-paint: implement rasterize")
}

/// Draws `text` with the given font on `target` at device-pixel position
/// (`x`, `baseline`), returning the advance width in device px. Used by the
/// browser UI (address bar, tabs, status bar).
pub fn draw_text(
    target: &mut tiny_skia::Pixmap,
    fonts: &FontSystem,
    font: FontId,
    size_px: f32,
    x: f32,
    baseline: f32,
    text: &str,
    color: Color,
) -> f32 {
    let _ = (target, fonts, font, size_px, x, baseline, text, color);
    todo!("xbro-paint: implement draw_text")
}
