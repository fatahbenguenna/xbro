//! Painting for xbro: converts a [`LayoutTree`] into a [`DisplayList`] (in
//! CSS-px document coordinates) and rasterises display lists into
//! `tiny_skia::Pixmap`s at any scale/scroll offset. Also provides image
//! decoding ([`image::Image`]) and a text-drawing helper used by the browser UI.

pub mod display_list;
pub mod image;
pub mod raster;

pub use display_list::{
    background_tile, build_display_list, build_display_list_full, build_display_list_with_images,
    resolve_radii, BorderSideItem, DisplayItem, DisplayList, GlyphRun,
};
pub use image::{probe_image_size, Image, ImageProvider, NoImageProvider};
pub use raster::{draw_text, rasterize, RasterOptions};
pub use tiny_skia;
pub use xbro_layout::{Edges, LayoutTree, Point, Rect, Size};
