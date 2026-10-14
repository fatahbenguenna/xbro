//! Decoded images (raster or SVG) and the provider trait used while painting.
//!
//! Raster formats (PNG, JPEG, GIF, WebP, BMP, ICO) are decoded with the
//! `image` crate into premultiplied [`tiny_skia::Pixmap`]s; SVG documents are
//! parsed with `usvg` and rendered at their destination size when drawn.
//!
//! Decoding is defensive: dimensions are read from the header first and
//! oversized images are rejected before any pixel buffer is allocated, and
//! every decoder call is wrapped in [`std::panic::catch_unwind`] as a last
//! resort against decoder bugs on malformed input.
//!
//! SVG limitations: `resvg`/`usvg` are built without their `text` feature
//! (see the workspace manifest), so SVG `<text>` elements are not rendered;
//! `<image>` elements only resolve `data:` URLs (never local files), and
//! embedded raster images are not drawn (no `raster-images` feature).

use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use xbro_dom::NodeId;
use xbro_layout::Size;

/// Largest accepted raster image side, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 16_384;

/// Largest accepted raster image area, in pixels (100 megapixels).
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;

/// How many leading bytes are searched for an `<svg` tag when sniffing XML.
const SVG_SNIFF_BYTES: usize = 4096;

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
    ///
    /// Raster images larger than 100 megapixels are rejected as well; both
    /// checks happen on the header, before pixels are allocated. Animated
    /// GIF/WebP images decode to their first frame.
    pub fn decode(bytes: &[u8], content_type: Option<&str>) -> Option<Image> {
        if bytes.is_empty() {
            return None;
        }
        let svg_hint = content_type.is_some_and(is_svg_content_type);
        if svg_hint || looks_like_svg(bytes) {
            if let Some(image) = parse_svg_data(bytes) {
                return Some(image);
            }
        }
        decode_raster(bytes).map(|p| Image::Raster(Arc::new(p)))
    }

    /// Parses SVG markup (used for inline `<svg>` elements serialized by the engine).
    pub fn from_svg_str(svg: &str) -> Option<Image> {
        let svg = svg.trim_start_matches('\u{feff}').trim_start();
        let tree = catch_unwind(AssertUnwindSafe(|| {
            resvg::usvg::Tree::from_str(svg, &svg_options()).ok()
        }))
        .ok()
        .flatten()?;
        svg_image(tree)
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

/// True for `image/svg+xml` (any case, parameters ignored).
fn is_svg_content_type(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("image/svg+xml"))
}

/// Content sniffing: after an optional UTF-8 BOM and whitespace, the data
/// starts with `<svg`, or with `<?xml` / `<!--` / `<!DOCTYPE` and contains
/// `<svg` near the start.
pub(crate) fn looks_like_svg(bytes: &[u8]) -> bool {
    let data = trim_xml_start(bytes);
    let starts = |prefix: &[u8]| {
        data.len() >= prefix.len()
            && data
                .iter()
                .zip(prefix)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    };
    if starts(b"<svg") {
        return true;
    }
    if starts(b"<?xml") || starts(b"<!--") || starts(b"<!doctype") {
        let head = &data[..data.len().min(SVG_SNIFF_BYTES)];
        return head.windows(4).any(|w| w.eq_ignore_ascii_case(b"<svg"));
    }
    false
}

/// usvg options: defaults, except that `<image href>` strings are never
/// resolved against the local file system (web content must not read files).
fn svg_options() -> resvg::usvg::Options<'static> {
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options
}

/// Strips a UTF-8 BOM and leading whitespace (an XML declaration must
/// come first, but servers and inline markup often prepend whitespace).
fn trim_xml_start(bytes: &[u8]) -> &[u8] {
    let mut data = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    while let Some((first, rest)) = data.split_first() {
        if first.is_ascii_whitespace() {
            data = rest;
        } else {
            break;
        }
    }
    data
}

fn parse_svg_data(bytes: &[u8]) -> Option<Image> {
    let bytes = trim_xml_start(bytes);
    let tree = catch_unwind(AssertUnwindSafe(|| {
        resvg::usvg::Tree::from_data(bytes, &svg_options()).ok()
    }))
    .ok()
    .flatten()?;
    svg_image(tree)
}

/// Wraps a parsed tree, rejecting degenerate sizes.
fn svg_image(tree: resvg::usvg::Tree) -> Option<Image> {
    let size = tree.size();
    let ok = |v: f32| v.is_finite() && v > 0.0;
    (ok(size.width()) && ok(size.height())).then(|| Image::Svg(Arc::new(tree)))
}

/// True when a raster image of these dimensions may be decoded.
pub(crate) fn dimensions_allowed(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_IMAGE_SIDE
        && height <= MAX_IMAGE_SIDE
        && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS
}

/// Decodes a raster image (first frame) into a premultiplied pixmap.
fn decode_raster(bytes: &[u8]) -> Option<tiny_skia::Pixmap> {
    catch_unwind(AssertUnwindSafe(|| decode_raster_inner(bytes)))
        .ok()
        .flatten()
}

fn decode_raster_inner(bytes: &[u8]) -> Option<tiny_skia::Pixmap> {
    let reader = || {
        image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()
    };
    // Header only: reject huge images before allocating anything.
    let probe = reader()?;
    probe.format()?;
    let (width, height) = probe.into_dimensions().ok()?;
    if !dimensions_allowed(width, height) {
        return None;
    }
    let mut decoder = reader()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    decoder.limits(limits);
    let rgba = decoder.decode().ok()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    if !dimensions_allowed(width, height) {
        return None;
    }
    let mut data = rgba.into_raw();
    premultiply(&mut data);
    let size = tiny_skia::IntSize::from_wh(width, height)?;
    tiny_skia::Pixmap::from_vec(data, size)
}

/// Converts straight RGBA8 to premultiplied RGBA8 in place.
pub(crate) fn premultiply(data: &mut [u8]) {
    for px in data.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        match a {
            255 => {}
            0 => {
                px[0] = 0;
                px[1] = 0;
                px[2] = 0;
            }
            _ => {
                for c in &mut px[..3] {
                    *c = crate::raster::div255(u32::from(*c) * a) as u8;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffing() {
        assert!(looks_like_svg(b"<svg xmlns='http://www.w3.org/2000/svg'/>"));
        assert!(looks_like_svg(b"\xEF\xBB\xBF  \n<SVG>"));
        assert!(looks_like_svg(b"<?xml version='1.0'?>\n<svg/>"));
        assert!(looks_like_svg(b"<!-- c --><!DOCTYPE svg><svg/>"));
        assert!(!looks_like_svg(b"<?xml version='1.0'?><html/>"));
        assert!(!looks_like_svg(b"\x89PNG\r\n"));
        assert!(!looks_like_svg(b""));
        assert!(!looks_like_svg(b"   "));
    }

    #[test]
    fn content_type_hint() {
        assert!(is_svg_content_type("image/svg+xml"));
        assert!(is_svg_content_type("Image/SVG+XML; charset=utf-8"));
        assert!(!is_svg_content_type("image/png"));
    }

    #[test]
    fn dimension_limits() {
        assert!(dimensions_allowed(1, 1));
        assert!(dimensions_allowed(16_384, 6_000));
        assert!(!dimensions_allowed(16_385, 1));
        assert!(!dimensions_allowed(1, 16_385));
        assert!(!dimensions_allowed(16_384, 16_384)); // 268 MP
        assert!(!dimensions_allowed(0, 10));
    }

    #[test]
    fn premultiplication() {
        let mut px = [255, 128, 0, 128, 10, 20, 30, 0, 1, 2, 3, 255];
        premultiply(&mut px);
        assert_eq!(&px[..4], &[128, 64, 0, 128]);
        assert_eq!(&px[4..8], &[0, 0, 0, 0]);
        assert_eq!(&px[8..], &[1, 2, 3, 255]);
    }
}
