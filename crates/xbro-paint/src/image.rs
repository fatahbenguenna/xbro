//! Decoded images (raster or SVG) and the provider trait used while painting.

use std::sync::Arc;

use xbro_dom::NodeId;
use xbro_layout::Size;

/// A decoded image ready to be drawn.
#[derive(Clone)]
pub enum Image {
    /// Premultiplied RGBA pixels (PNG, JPEG, GIF (first frame), WebP, BMP, ICO).
    Raster(Arc<tiny_skia::Pixmap>),
    /// Parsed SVG document, rendered at the destination size when drawn (crisp at any scale).
    Svg(Arc<resvg::usvg::Tree>),
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.natural_size();
        match self {
            Image::Raster(_) => write!(f, "Image::Raster({}x{})", s.width, s.height),
            Image::Svg(_) => write!(f, "Image::Svg({}x{})", s.width, s.height),
        }
    }
}

impl Image {
    /// Decodes image bytes, detecting the format from content (falls back to
    /// SVG when the bytes look like XML/`<svg`). `content_type` is a hint.
    /// Returns `None` for undecodable data. Images larger than 16384 px in
    /// either dimension are rejected.
    pub fn decode(bytes: &[u8], content_type: Option<&str>) -> Option<Image> {
        let _ = (bytes, content_type);
        todo!("xbro-paint: implement Image::decode")
    }

    /// Parses SVG markup (used for inline `<svg>` elements serialized by the engine).
    pub fn from_svg_str(svg: &str) -> Option<Image> {
        let _ = svg;
        todo!("xbro-paint: implement Image::from_svg_str")
    }

    /// Natural size in CSS px (pixels for raster images, the SVG's width/height
    /// or viewBox size for SVG).
    pub fn natural_size(&self) -> Size {
        match self {
            Image::Raster(p) => Size::new(p.width() as f32, p.height() as f32),
            Image::Svg(t) => {
                let s = t.size();
                Size::new(s.width(), s.height())
            }
        }
    }
}

/// Source of images while painting (implemented by the engine).
pub trait ImageProvider: Sync {
    /// Image for a replaced element (`<img>`, inline `<svg>`, `<input type=image>`, `<video poster>`…).
    fn node_image(&self, node: NodeId) -> Option<Image>;
    /// Image by absolute URL (backgrounds, list-style-image, `content: url()`).
    fn url_image(&self, url: &str) -> Option<Image>;
}

/// An [`ImageProvider`] with no images.
pub struct NoImageProvider;

impl ImageProvider for NoImageProvider {
    fn node_image(&self, _: NodeId) -> Option<Image> {
        None
    }
    fn url_image(&self, _: &str) -> Option<Image> {
        None
    }
}
