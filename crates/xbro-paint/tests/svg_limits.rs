//! SVG images with geometry beyond the rasteriser's numeric range: resvg's
//! tiny-skia panicked (caught, but a panic all the same) on strokes around
//! 1e10 device px wide, found by the pipeline fuzzer. Such content is not
//! drawn; nothing panics.
//!
//! A separate test binary: the panic hook it installs counts panics of the
//! whole process.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use xbro_dom::NodeId;
use xbro_layout::{Point, Rect};
use xbro_paint::{DisplayItem, DisplayList, Image, RasterOptions};

static PANICS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn huge_svg_geometry_never_reaches_the_rasteriser() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        PANICS.fetch_add(1, Ordering::SeqCst);
        default(info);
    }));
    let svgs = [
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" stroke="blue" stroke-width="1e6"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" stroke="blue" stroke-width="1e5"/><rect x="-5" y="1" width="1e9" height="8" fill="red" rx="2"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><g transform="scale(1e9)"><rect width="1" height="1"/></g><path d="M 0 0 C 1e30 1e30 -1e30 2 5 5" stroke="red"/></svg>"#,
    ];
    for svg in svgs {
        let image = Image::from_svg_str(svg).expect("svg");
        let mut images = MapImages::default();
        images.nodes.insert(1, image);
        for size in [100.0, 10_000.0, 100_000.0, 1_000_000.0, 1e7] {
            for scale in [1.0, 64.0] {
                let list = DisplayList {
                    items: vec![DisplayItem::NodeImage {
                        rect: Rect::new(0.0, 0.0, size, size),
                        node: NodeId(1),
                    }],
                    background: WHITE,
                };
                let options = RasterOptions {
                    scroll: Point::new(size / 2.0, size / 2.0),
                    scale,
                    ..RasterOptions::default()
                };
                let _ = render_with(&list, 44, 92, &options, &images);
            }
        }
    }
    assert_eq!(PANICS.load(Ordering::SeqCst), 0);
    // Ordinary SVG content is still drawn.
    let image = Image::from_svg_str(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><circle cx="5" cy="5" r="4" stroke="blue" stroke-width="2" fill="lime"/></svg>"#,
    )
    .expect("svg");
    let mut images = MapImages::default();
    images.nodes.insert(1, image);
    let list = DisplayList {
        items: vec![DisplayItem::NodeImage {
            rect: Rect::new(0.0, 0.0, 100_000.0, 100_000.0),
            node: NodeId(1),
        }],
        background: WHITE,
    };
    let options = RasterOptions {
        scroll: Point::new(50_000.0, 50_000.0),
        ..RasterOptions::default()
    };
    let p = render_with(&list, 40, 40, &options, &images);
    assert_px(&p, 20, 20, LIME);
}
