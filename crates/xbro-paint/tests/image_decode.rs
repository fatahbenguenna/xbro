//! `Image::decode` / `Image::from_svg_str` on generated and fixture images,
//! corrupt data and oversized headers.

use std::io::Cursor;
use std::path::PathBuf;

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use xbro_layout::Size;
use xbro_paint::Image;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/img")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A `w` × `h` image: left half red, right half semi-transparent blue.
fn sample(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, _| {
        if x < w / 2 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 128])
        }
    })
}

fn encode(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, format).expect("encode");
    out.into_inner()
}

fn raster(img: &Image) -> &tiny_skia::Pixmap {
    match img {
        Image::Raster(p) => p,
        Image::Svg(_) => panic!("expected a raster image"),
    }
}

fn pixel(img: &Image, x: u32, y: u32) -> (u8, u8, u8, u8) {
    let c = raster(img).pixel(x, y).expect("in bounds");
    (c.red(), c.green(), c.blue(), c.alpha())
}

#[test]
fn png_roundtrip_is_premultiplied() {
    let bytes = encode(DynamicImage::ImageRgba8(sample(8, 4)), ImageFormat::Png);
    let img = Image::decode(&bytes, Some("image/png")).expect("png");
    assert_eq!(img.natural_size(), Size::new(8.0, 4.0));
    assert_eq!(pixel(&img, 0, 0), (255, 0, 0, 255));
    assert_eq!(pixel(&img, 7, 3), (0, 0, 128, 128));
    assert!(format!("{img:?}").contains("8x4"));
}

#[test]
fn jpeg_decodes() {
    let rgb = DynamicImage::ImageRgba8(sample(16, 16)).to_rgb8();
    let bytes = encode(DynamicImage::ImageRgb8(rgb), ImageFormat::Jpeg);
    // Wrong content-type hint: detection is by content.
    let img = Image::decode(&bytes, Some("image/gif")).expect("jpeg");
    assert_eq!(img.natural_size(), Size::new(16.0, 16.0));
    let (r, g, b, a) = pixel(&img, 2, 8);
    assert!(
        r > 200 && g < 60 && b < 60 && a == 255,
        "{:?}",
        (r, g, b, a)
    );
}

#[test]
fn gif_decodes_first_frame() {
    use image::codecs::gif::GifEncoder;
    use image::Frame;
    let mut bytes = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut bytes);
        let red = RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 255]));
        let blue = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 255, 255]));
        enc.encode_frames(vec![Frame::new(red), Frame::new(blue)])
            .expect("gif");
    }
    let img = Image::decode(&bytes, None).expect("gif");
    assert_eq!(img.natural_size(), Size::new(4.0, 4.0));
    assert_eq!(pixel(&img, 1, 1), (255, 0, 0, 255));
}

#[test]
fn bmp_ico_webp_decode() {
    let img = DynamicImage::ImageRgba8(sample(6, 6));
    for format in [ImageFormat::Bmp, ImageFormat::Ico, ImageFormat::WebP] {
        let bytes = encode(img.clone(), format);
        let decoded = Image::decode(&bytes, None).unwrap_or_else(|| panic!("{format:?}"));
        assert_eq!(decoded.natural_size(), Size::new(6.0, 6.0), "{format:?}");
        assert_eq!(pixel(&decoded, 0, 0), (255, 0, 0, 255), "{format:?}");
    }
}

#[test]
fn fixture_images() {
    for (name, w, h) in [
        ("logo.png", 0, 0),
        ("photo.png", 0, 0),
        ("tile.png", 0, 0),
        ("landscape.jpg", 0, 0),
        ("y18.gif", 18, 18),
    ] {
        let img = Image::decode(&fixture(name), None).unwrap_or_else(|| panic!("{name}"));
        let size = img.natural_size();
        assert!(size.width >= 1.0 && size.height >= 1.0, "{name}: {size:?}");
        if w > 0 {
            assert_eq!(size, Size::new(w as f32, h as f32), "{name}");
        }
        assert!(matches!(img, Image::Raster(_)));
    }
    let svg = Image::decode(&fixture("icon.svg"), None).expect("svg fixture");
    assert!(matches!(svg, Image::Svg(_)));
    let s = svg.natural_size();
    assert!(s.width > 0.0 && s.height > 0.0);
}

#[test]
fn svg_detection() {
    let doc = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"><rect width="10" height="5"/></svg>"#;
    let img = Image::decode(doc, None).expect("sniffed svg");
    assert_eq!(img.natural_size(), Size::new(10.0, 5.0));
    let mut with_prolog = b"\xEF\xBB\xBF\n<?xml version=\"1.0\"?>\n<!-- comment -->\n".to_vec();
    with_prolog.extend_from_slice(doc);
    assert!(matches!(
        Image::decode(&with_prolog, None),
        Some(Image::Svg(_))
    ));
    assert!(matches!(
        Image::decode(doc, Some("image/svg+xml; charset=utf-8")),
        Some(Image::Svg(_))
    ));
    // viewBox-only size.
    let vb =
        Image::from_svg_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 12"/>"#)
            .expect("viewBox");
    assert_eq!(vb.natural_size(), Size::new(24.0, 12.0));
    assert!(format!("{vb:?}").starts_with("Image::Svg"));
}

#[test]
fn svg_does_not_read_local_files() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
        <image width="10" height="10" xlink:href="/etc/passwd"/></svg>"#;
    // Parses fine; the image reference is simply not resolved.
    assert!(Image::from_svg_str(svg).is_some());
}

#[test]
fn corrupt_data_is_rejected() {
    assert!(Image::decode(b"", None).is_none());
    assert!(Image::decode(b"hello world", None).is_none());
    assert!(Image::decode(b"<svg", Some("image/svg+xml")).is_none());
    assert!(Image::decode(b"<?xml version='1.0'?><svg><g>", None).is_none());
    assert!(Image::from_svg_str("not svg at all").is_none());
    assert!(Image::from_svg_str(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#
    )
    .is_none());
    let png = encode(DynamicImage::ImageRgba8(sample(32, 32)), ImageFormat::Png);
    for cut in [8, 16, 33, 50, png.len() / 2, png.len() - 13] {
        assert!(
            Image::decode(&png[..cut], None).is_none(),
            "truncated at {cut}"
        );
    }
    let mut garbled = png.clone();
    for b in garbled.iter_mut().skip(40) {
        *b ^= 0x5a;
    }
    assert!(Image::decode(&garbled, None).is_none());
    let jpeg = encode(
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(sample(32, 32)).to_rgb8()),
        ImageFormat::Jpeg,
    );
    let _ = Image::decode(&jpeg[..jpeg.len() / 2], None); // may decode partially, must not panic
    let gif = encode(DynamicImage::ImageRgba8(sample(8, 8)), ImageFormat::Gif);
    assert!(Image::decode(&gif[..20], None).is_none());
}

/// CRC-32 (PNG chunk checksum).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A PNG made of a valid IHDR announcing `w` × `h` and a tiny IDAT.
fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
    chunk(b"IEND", &[]);
    out
}

#[test]
fn huge_headers_are_rejected_without_allocating() {
    assert!(Image::decode(&png_header(100_000, 100_000), None).is_none());
    assert!(Image::decode(&png_header(16_385, 1), None).is_none());
    assert!(Image::decode(&png_header(1, 16_385), None).is_none());
    assert!(
        Image::decode(&png_header(10_001, 10_000), None).is_none(),
        "over 100 megapixels"
    );
    // GIF with a 65535×65535 logical screen and frame.
    let mut gif = b"GIF89a".to_vec();
    gif.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00]);
    gif.extend_from_slice(&[
        0x2c, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
    ]);
    assert!(Image::decode(&gif, None).is_none());
}

#[test]
fn largest_side_limit_is_inclusive() {
    let wide = RgbaImage::from_pixel(16_384, 1, Rgba([1, 2, 3, 255]));
    let bytes = encode(DynamicImage::ImageRgba8(wide), ImageFormat::Png);
    let img = Image::decode(&bytes, None).expect("16384 px is allowed");
    assert_eq!(img.natural_size(), Size::new(16_384.0, 1.0));
    let wider = RgbaImage::from_pixel(16_385, 1, Rgba([1, 2, 3, 255]));
    let bytes = encode(DynamicImage::ImageRgba8(wider), ImageFormat::Png);
    assert!(Image::decode(&bytes, None).is_none());
}

#[test]
fn fuzzish_inputs_never_panic() {
    let seeds = [
        fixture("logo.png"),
        fixture("landscape.jpg"),
        fixture("y18.gif"),
        fixture("icon.svg"),
        encode(DynamicImage::ImageRgba8(sample(5, 5)), ImageFormat::WebP),
        encode(DynamicImage::ImageRgba8(sample(5, 5)), ImageFormat::Bmp),
        encode(DynamicImage::ImageRgba8(sample(5, 5)), ImageFormat::Ico),
    ];
    let mut state = 0x1234_5678u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for seed in &seeds {
        for _ in 0..40 {
            let mut data = seed.clone();
            let flips = 1 + next() % 8;
            for _ in 0..flips {
                let i = next() as usize % data.len();
                data[i] = next() as u8;
            }
            if next() % 3 == 0 {
                let len = next() as usize % data.len();
                data.truncate(len);
            }
            let _ = Image::decode(&data, None);
        }
    }
}
