//! Headless modes: PNG screenshots and DOM/layout dumps.

use xbro_engine::{Engine, NavigationRequest, Page, Point, RasterOptions, Size};
use xbro_paint::tiny_skia::Pixmap;

use crate::{Mode, Options};

/// Largest screenshot side, in device pixels (larger pages are cut).
const MAX_SCREENSHOT_HEIGHT: f32 = 32_768.0;
const MAX_SCREENSHOT_WIDTH: f32 = 32_768.0;
/// Largest screenshot area in device pixels (1 GiB of RGBA); taller
/// images are cut to fit.
const MAX_SCREENSHOT_PIXELS: f32 = (1 << 28) as f32;

/// Viewport height used for layout when a full-page screenshot is requested.
const DEFAULT_VIEWPORT_HEIGHT: u32 = 768;

pub fn run(opts: &Options) -> Result<(), String> {
    let input = opts.url.as_deref().unwrap_or_default();
    let url = crate::resolve_input(input);
    let viewport_height = if opts.height == 0 {
        DEFAULT_VIEWPORT_HEIGHT
    } else {
        opts.height
    };
    let viewport = Size::new(opts.width as f32, viewport_height as f32);

    let engine = Engine::new();
    let page = engine.load(&NavigationRequest::Get(url), viewport);
    for line in page.console() {
        eprintln!("xbro: {line}");
    }

    match &opts.mode {
        Mode::Screenshot { output } => screenshot(&page, opts, output),
        Mode::DumpDom => {
            print!("{}", page.document().to_debug_string());
            Ok(())
        }
        Mode::DumpLayout => {
            print!("{}", page.layout_tree().dump(page.document()));
            Ok(())
        }
        Mode::Gui | Mode::Help | Mode::Version => Ok(()),
    }
}

fn screenshot(page: &Page, opts: &Options, output: &str) -> Result<(), String> {
    let (width, height) = screenshot_size(opts, page.viewport(), page.document_size());
    let mut pixmap = Pixmap::new(width, height)
        .ok_or_else(|| format!("cannot allocate a {width}x{height} image"))?;
    let options = RasterOptions {
        scroll: Point::ZERO,
        scale: opts.scale,
        ..Default::default()
    };
    page.render(&mut pixmap, &options);
    pixmap
        .save_png(output)
        .map_err(|e| format!("cannot write {output}: {e}"))?;
    println!("saved {output} ({width}x{height})");
    Ok(())
}

/// Device-pixel size of the screenshot: the viewport, or the whole document
/// when `--height 0` was given (capped at [`MAX_SCREENSHOT_WIDTH`] ×
/// [`MAX_SCREENSHOT_HEIGHT`] and [`MAX_SCREENSHOT_PIXELS`], so that huge
/// `--width`/`--scale` values cannot exhaust memory).
fn screenshot_size(opts: &Options, viewport: Size, document: Size) -> (u32, u32) {
    let css_height = if opts.height == 0 {
        document.height.max(viewport.height)
    } else {
        viewport.height
    };
    let width = (viewport.width * opts.scale)
        .round()
        .clamp(1.0, MAX_SCREENSHOT_WIDTH);
    let height = (css_height * opts.scale)
        .round()
        .clamp(1.0, MAX_SCREENSHOT_HEIGHT)
        .min((MAX_SCREENSHOT_PIXELS / width).floor());
    (width as u32, height as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(width: u32, height: u32, scale: f32) -> Options {
        Options {
            mode: Mode::DumpDom,
            url: None,
            width,
            height,
            scale,
        }
    }

    #[test]
    fn viewport_screenshot_size_is_scaled() {
        let size = screenshot_size(
            &opts(800, 600, 2.0),
            Size::new(800.0, 600.0),
            Size::new(800.0, 5000.0),
        );
        assert_eq!(size, (1600, 1200));
    }

    #[test]
    fn full_page_screenshot_uses_document_height_and_is_capped() {
        let size = screenshot_size(
            &opts(800, 0, 1.0),
            Size::new(800.0, 768.0),
            Size::new(800.0, 2500.5),
        );
        assert_eq!(size, (800, 2501));
        let short = screenshot_size(
            &opts(800, 0, 1.0),
            Size::new(800.0, 768.0),
            Size::new(800.0, 100.0),
        );
        assert_eq!(short, (800, 768));
        let huge = screenshot_size(
            &opts(800, 0, 2.0),
            Size::new(800.0, 768.0),
            Size::new(800.0, 1e7),
        );
        assert_eq!(huge, (1600, MAX_SCREENSHOT_HEIGHT as u32));
        // A huge viewport stays within the pixel budget.
        let wide = screenshot_size(
            &opts(100_000, 20_000, 8.0),
            Size::new(100_000.0, 20_000.0),
            Size::new(100_000.0, 20_000.0),
        );
        assert_eq!(wide, (32_768, 8_192));
        assert!(u64::from(wide.0) * u64::from(wide.1) <= 1 << 28);
    }
}
