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
//! SVG documents are pre-scanned before parsing (see `svg_scan`): element
//! nesting deeper than 256 levels, chains of clip-path/mask references and
//! DTD entities expanding to more than 1 MiB are rejected, because usvg
//! recurses over the tree and its references (a stack overflow cannot be
//! caught) and expands entities eagerly. Nested SVG documents in `data:`
//! URLs (`<image href>`) go through the same scan. As a second line of
//! defence, parsing and rendering run on a worker thread with a large
//! stack (see `with_svg_stack`).
//!
//! SVG limitations: `resvg`/`usvg` are built without their `text` feature
//! (see the workspace manifest; enabling it would pull in a text shaping
//! stack for a rarely needed feature), so SVG `<text>` elements are not
//! rendered; `<image>` elements only resolve `data:` URLs (never local
//! files), and embedded raster images are not drawn (no `raster-images`
//! feature). Gzip-compressed SVG (`.svgz` bytes) is not decoded (no `svgz`
//! feature; HTTP `Content-Encoding: gzip` is undone by the network layer).

use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use xbro_dom::NodeId;
use xbro_layout::Size;

mod svg_scan;

use svg_scan::structure_ok;

/// Stack size of the threads that parse and render SVG documents. usvg
/// and resvg recurse over the element tree and its references; the
/// pre-scan bounds that recursion, and this generous stack (reserved, only
/// touched pages are committed) keeps the bounded worst case far from an
/// overflow whatever stack the calling thread has.
const SVG_STACK_BYTES: usize = 64 << 20;

/// Largest accepted raster image side, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 16_384;

/// Largest accepted raster image area, in pixels (100 megapixels).
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;

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
    /// GIF/WebP images decode to their first frame. JPEG/PNG/WebP images
    /// are rotated or flipped according to their EXIF orientation, as
    /// browsers do (`image-orientation: from-image`). SVG documents nesting
    /// elements deeper than 256 levels, chaining clip paths/masks or
    /// expanding DTD entities beyond 1 MiB are rejected (see the module
    /// documentation).
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
    ///
    /// The same structural limits as for [`Image::decode`] apply.
    pub fn from_svg_str(svg: &str) -> Option<Image> {
        let svg = svg.trim_start_matches('\u{feff}').trim_start();
        if !structure_ok(svg.as_bytes()) {
            return None;
        }
        let tree = with_svg_stack(|| resvg::usvg::Tree::from_str(svg, &svg_options()).ok())??;
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
/// starts with `<svg`, or with an XML prolog (`<?xml …?>`, comments, a
/// DOCTYPE) whose first element is `<svg>` (any namespace prefix), however
/// long the prolog is.
pub(crate) fn looks_like_svg(bytes: &[u8]) -> bool {
    svg_scan::root_is_svg(trim_xml_start(bytes))
}

/// usvg options: defaults, except that `<image href>` strings are never
/// resolved against the local file system (web content must not read
/// files) and that nested SVG documents in `data:` URLs must pass the same
/// structural pre-scan as top-level documents.
fn svg_options() -> resvg::usvg::Options<'static> {
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let default_data = resvg::usvg::ImageHrefResolver::default_data_resolver();
    options.image_href_resolver.resolve_data = Box::new(move |mime, data, opts| {
        let raster = matches!(
            mime,
            "image/jpg" | "image/jpeg" | "image/png" | "image/gif" | "image/webp"
        );
        // Everything else may be parsed as a nested SVG document.
        if raster || structure_ok(trim_xml_start(&data)) {
            default_data(mime, data, opts)
        } else {
            None
        }
    });
    options
}

/// Runs `f` (SVG parsing or rendering) on a scoped worker thread with a
/// [`SVG_STACK_BYTES`] stack. `None` when the thread cannot be spawned or
/// `f` panics (the panic is contained like with `catch_unwind`).
pub(crate) fn with_svg_stack<T: Send>(f: impl FnOnce() -> T + Send) -> Option<T> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("xbro-svg".into())
            .stack_size(SVG_STACK_BYTES)
            .spawn_scoped(scope, f)
            .ok()?
            .join()
            .ok()
    })
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

/// Parses SVG bytes after the structural pre-scan.
fn parse_svg_data(bytes: &[u8]) -> Option<Image> {
    let data = trim_xml_start(bytes);
    if !structure_ok(data) {
        return None;
    }
    let tree = with_svg_stack(|| resvg::usvg::Tree::from_data(data, &svg_options()).ok())??;
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

/// Decodes a raster image (first frame) into a premultiplied pixmap,
/// rotated/flipped as its EXIF orientation says.
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
    let mut reader = reader()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    // Browsers honour the EXIF orientation of photos (`image-orientation:
    // from-image`); unreadable metadata just means "as stored".
    let orientation = image::ImageDecoder::orientation(&mut decoder)
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder).ok()?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
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
        // However long the prolog is.
        let licensed = format!(
            "<?xml version='1.0'?>\n<!-- {} -->\n<!DOCTYPE svg [ <!ENTITY a 'b'> ]>\n<svg/>",
            "license text ".repeat(1000)
        );
        assert!(looks_like_svg(licensed.as_bytes()));
        assert!(looks_like_svg(
            b"<?xml version='1.0'?><svg:svg xmlns:svg='x'/>"
        ));
        // The root element must be `svg`, not merely contain one.
        assert!(!looks_like_svg(b"<svgx/>"));
        assert!(!looks_like_svg(
            b"<?xml version='1.0'?><html><body><svg/></body></html>"
        ));
        assert!(!looks_like_svg(b"<?xml version='1.0'?> text <svg/>"));
        assert!(!looks_like_svg(b"<!-- unterminated"));
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
