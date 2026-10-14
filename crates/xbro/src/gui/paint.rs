//! Drawing primitives for the browser chrome: a painter working in logical
//! px on a device-pixel pixmap, UI fonts, vector icons and the final pixel
//! conversion for the window surface.

use std::sync::Arc;

use xbro_engine::Rect;
use xbro_paint::tiny_skia::{
    self, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform,
};
use xbro_text::{FontId, FontQuery, FontSystem};

/// An sRGB colour with straight alpha.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 255)
    }

    fn skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.0, self.1, self.2, self.3)
    }

    fn css(self) -> xbro_css::Color {
        xbro_css::Color::rgba(self.0, self.1, self.2, self.3)
    }
}

/// The UI fonts (sans-serif regular and bold) and text measurement.
#[derive(Clone)]
pub struct UiFonts {
    pub system: Arc<FontSystem>,
    pub regular: FontId,
    pub bold: FontId,
}

impl UiFonts {
    pub fn new(system: Arc<FontSystem>) -> Self {
        let families = ["sans-serif".to_string()];
        let query = |weight| FontQuery {
            families: &families,
            weight,
            italic: false,
        };
        let regular = system.select(&query(400));
        let bold = system.select(&query(700));
        Self {
            system,
            regular,
            bold,
        }
    }

    fn font(&self, bold: bool) -> FontId {
        if bold {
            self.bold
        } else {
            self.regular
        }
    }

    /// Advance width in logical px of `text` at `size` logical px, as drawn
    /// at `dpi` device px per logical px.
    pub fn measure(&self, text: &str, size: f32, bold: bool, dpi: f32) -> f32 {
        self.system
            .measure(self.font(bold), size * dpi, text, 0.0, 0.0)
            / dpi
    }

    /// Caret stops of `text`: (byte offset, x in logical px) for the start
    /// of every char that produces a glyph, plus the end of the text.
    pub fn caret_stops(&self, text: &str, size: f32, dpi: f32) -> Vec<(usize, f32)> {
        let run = self.system.shape(self.regular, size * dpi, text, 0.0, 0.0);
        let mut stops: Vec<(usize, f32)> =
            run.glyphs.iter().map(|g| (g.cluster, g.x / dpi)).collect();
        stops.push((text.len(), run.width / dpi));
        stops
    }

    /// The longest prefix of `text` that fits in `max_width` logical px,
    /// with "…" appended when cut.
    pub fn truncate(&self, text: &str, size: f32, bold: bool, dpi: f32, max_width: f32) -> String {
        if self.measure(text, size, bold, dpi) <= max_width {
            return text.to_string();
        }
        let ellipsis = "…";
        let room = max_width - self.measure(ellipsis, size, bold, dpi);
        if room <= 0.0 {
            return String::new();
        }
        let run = self
            .system
            .shape(self.font(bold), size * dpi, text, 0.0, 0.0);
        let cut = run
            .glyphs
            .iter()
            .find(|g| (g.x + g.advance) / dpi > room)
            .map_or(text.len(), |g| g.cluster);
        let mut out = text.get(..cut).unwrap_or_default().trim_end().to_string();
        out.push_str(ellipsis);
        out
    }
}

/// Caret x for byte offset `pos` given [`UiFonts::caret_stops`].
pub fn caret_x(stops: &[(usize, f32)], pos: usize) -> f32 {
    stops
        .iter()
        .find(|&&(i, _)| i >= pos)
        .or(stops.last())
        .map_or(0.0, |&(_, x)| x)
}

/// Byte offset of the caret stop closest to `x`.
pub fn caret_at(stops: &[(usize, f32)], x: f32) -> usize {
    stops
        .iter()
        .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
        .map_or(0, |&(i, _)| i)
}

/// Converts premultiplied RGBA pixels to the `0x00RRGGBB` words of a window
/// surface, compositing over white (translucent pixels never occur in a
/// finished frame, but stay well defined).
pub fn rgba_to_xrgb(src: &[u8], dst: &mut [u32]) {
    for (d, p) in dst.iter_mut().zip(src.chunks_exact(4)) {
        let &[r, g, b, a] = p else {
            continue;
        };
        let (r, g, b) = (u32::from(r), u32::from(g), u32::from(b));
        *d = if a == 255 {
            (r << 16) | (g << 8) | b
        } else {
            // Premultiplied: over white, each channel gains (255 − alpha).
            let back = 255 - u32::from(a);
            ((r + back).min(255) << 16) | ((g + back).min(255) << 8) | (b + back).min(255)
        };
    }
}

/// Draws in logical px on a device-pixel pixmap.
pub struct Painter<'a> {
    pub pixmap: &'a mut Pixmap,
    pub dpi: f32,
    pub fonts: &'a UiFonts,
}

impl<'a> Painter<'a> {
    pub fn new(pixmap: &'a mut Pixmap, dpi: f32, fonts: &'a UiFonts) -> Self {
        Self { pixmap, dpi, fonts }
    }

    fn transform(&self) -> Transform {
        Transform::from_scale(self.dpi, self.dpi)
    }

    fn paint(color: Rgba) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color(color.skia());
        paint.anti_alias = true;
        paint
    }

    /// Fills a rect snapped to device pixels.
    pub fn fill_rect(&mut self, r: Rect, color: Rgba) {
        self.fill_device_rect(self.to_device(r), color);
    }

    /// Fills a rect given in device pixels (snapped to whole pixels).
    pub fn fill_device_rect(&mut self, r: Rect, color: Rgba) {
        let (x0, y0) = (r.x.round(), r.y.round());
        let (x1, y1) = (r.right().round(), r.bottom().round());
        let (w, h) = (self.pixmap.width() as f32, self.pixmap.height() as f32);
        let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(w), y1.min(h));
        if !(x1 > x0 && y1 > y0) {
            return;
        }
        if let Some(rect) = tiny_skia::Rect::from_ltrb(x0, y0, x1, y1) {
            let mut paint = Self::paint(color);
            paint.anti_alias = false;
            self.pixmap
                .fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    pub fn to_device(&self, r: Rect) -> Rect {
        r.scale(self.dpi)
    }

    /// Fills a rounded rect; `radii` are top-left, top-right, bottom-right, bottom-left.
    pub fn fill_rounded(&mut self, r: Rect, radii: [f32; 4], color: Rgba) {
        if let Some(path) = rounded_path(r, radii) {
            self.pixmap.fill_path(
                &path,
                &Self::paint(color),
                FillRule::Winding,
                self.transform(),
                None,
            );
        }
    }

    pub fn stroke_rounded(&mut self, r: Rect, radius: f32, width: f32, color: Rgba) {
        let inset = width / 2.0;
        let r = Rect::new(r.x + inset, r.y + inset, r.width - width, r.height - width);
        if let Some(path) = rounded_path(r, [radius - inset; 4]) {
            self.stroke_path(&path, width, color);
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: Rgba) {
        if let Some(path) = PathBuilder::from_circle(cx, cy, radius) {
            self.pixmap.fill_path(
                &path,
                &Self::paint(color),
                FillRule::Winding,
                self.transform(),
                None,
            );
        }
    }

    pub fn stroke_path(&mut self, path: &tiny_skia::Path, width: f32, color: Rgba) {
        let stroke = Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        self.pixmap
            .stroke_path(path, &Self::paint(color), &stroke, self.transform(), None);
    }

    pub fn fill_path(&mut self, path: &tiny_skia::Path, color: Rgba) {
        self.pixmap.fill_path(
            path,
            &Self::paint(color),
            FillRule::Winding,
            self.transform(),
            None,
        );
    }

    /// Strokes a polyline (logical px).
    pub fn polyline(&mut self, points: &[(f32, f32)], width: f32, color: Rgba) {
        let mut pb = PathBuilder::new();
        for (i, &(x, y)) in points.iter().enumerate() {
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        if let Some(path) = pb.finish() {
            self.stroke_path(&path, width, color);
        }
    }

    /// Baseline that vertically centres text of `size` on `center_y` (logical px).
    pub fn baseline(&self, center_y: f32, size: f32, bold: bool) -> f32 {
        let m = self
            .fonts
            .system
            .metrics(self.fonts.font(bold), size * self.dpi);
        center_y + (m.ascent - m.descent) / 2.0 / self.dpi
    }

    /// Draws text whose vertical centre is `center_y`; returns its advance
    /// in logical px.
    pub fn text(
        &mut self,
        x: f32,
        center_y: f32,
        text: &str,
        size: f32,
        color: Rgba,
        bold: bool,
    ) -> f32 {
        let baseline = self.baseline(center_y, size, bold);
        xbro_paint::draw_text(
            self.pixmap,
            &self.fonts.system,
            self.fonts.font(bold),
            size * self.dpi,
            x * self.dpi,
            baseline * self.dpi,
            text,
            color.css(),
        ) / self.dpi
    }

    /// Draws text clipped to `clip` (logical px): glyphs are drawn on a
    /// scratch layer the size of the clip, then composited.
    #[allow(clippy::too_many_arguments)]
    pub fn text_clipped(
        &mut self,
        x: f32,
        center_y: f32,
        text: &str,
        size: f32,
        color: Rgba,
        bold: bool,
        clip: Rect,
    ) {
        let dev = self.to_device(clip);
        let (x0, y0) = (dev.x.round(), dev.y.round());
        let w = (dev.right().round() - x0).max(0.0) as u32;
        let h = (dev.bottom().round() - y0).max(0.0) as u32;
        let Some(mut layer) = Pixmap::new(w, h) else {
            return;
        };
        let baseline = self.baseline(center_y, size, bold);
        xbro_paint::draw_text(
            &mut layer,
            &self.fonts.system,
            self.fonts.font(bold),
            size * self.dpi,
            x * self.dpi - x0,
            baseline * self.dpi - y0,
            text,
            color.css(),
        );
        self.pixmap.draw_pixmap(
            x0 as i32,
            y0 as i32,
            layer.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
}

/// A rounded-rect path; radii are clamped to half the rect size.
pub fn rounded_path(r: Rect, radii: [f32; 4]) -> Option<tiny_skia::Path> {
    if !(r.width > 0.0 && r.height > 0.0) {
        return None;
    }
    let max = r.width.min(r.height) / 2.0;
    let [tl, tr, br, bl] = radii.map(|v| v.clamp(0.0, max));
    // Cubic approximation of a quarter circle.
    const K: f32 = 0.552_284_8;
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + tl, y0);
    pb.line_to(x1 - tr, y0);
    pb.cubic_to(x1 - tr + tr * K, y0, x1, y0 + tr - tr * K, x1, y0 + tr);
    pb.line_to(x1, y1 - br);
    pb.cubic_to(x1, y1 - br + br * K, x1 - br + br * K, y1, x1 - br, y1);
    pb.line_to(x0 + bl, y1);
    pb.cubic_to(x0 + bl - bl * K, y1, x0, y1 - bl + bl * K, x0, y1 - bl);
    pb.line_to(x0, y0 + tl);
    pb.cubic_to(x0, y0 + tl - tl * K, x0 + tl - tl * K, y0, x0 + tl, y0);
    pb.close();
    pb.finish()
}

/// Points of a circular arc (angles in radians, clockwise on screen from +x).
fn arc_points(cx: f32, cy: f32, r: f32, start: f32, sweep: f32) -> Vec<(f32, f32)> {
    let steps = 24;
    (0..=steps)
        .map(|i| {
            let a = start + sweep * i as f32 / steps as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Vector icons, drawn centred on (`cx`, `cy`) in logical px.
pub mod icons {
    use super::*;

    const STROKE: f32 = 1.8;

    pub fn arrow_left(p: &mut Painter, cx: f32, cy: f32, color: Rgba) {
        p.polyline(&[(cx + 6.5, cy), (cx - 6.5, cy)], STROKE, color);
        p.polyline(
            &[(cx - 1.0, cy - 5.5), (cx - 6.5, cy), (cx - 1.0, cy + 5.5)],
            STROKE,
            color,
        );
    }

    pub fn arrow_right(p: &mut Painter, cx: f32, cy: f32, color: Rgba) {
        p.polyline(&[(cx - 6.5, cy), (cx + 6.5, cy)], STROKE, color);
        p.polyline(
            &[(cx + 1.0, cy - 5.5), (cx + 6.5, cy), (cx + 1.0, cy + 5.5)],
            STROKE,
            color,
        );
    }

    /// A circular arrow (reload).
    pub fn reload(p: &mut Painter, cx: f32, cy: f32, color: Rgba) {
        use std::f32::consts::PI;
        let r = 6.0;
        // From the top, clockwise, leaving a gap at the top right.
        let start = -PI / 2.0 + 0.55;
        let sweep = 2.0 * PI - 1.1;
        let points = arc_points(cx, cy, r, start, sweep);
        p.polyline(&points, STROKE, color);
        // Arrow head at the end of the arc (pointing along the tangent).
        let end = start + sweep;
        let (ex, ey) = (cx + r * end.cos(), cy + r * end.sin());
        let (tx, ty) = (-end.sin(), end.cos()); // clockwise tangent
        let (nx, ny) = (end.cos(), end.sin()); // outward normal
        let tip = (ex + tx * 3.2, ey + ty * 3.2);
        let a = (ex + nx * 3.0 - tx * 0.8, ey + ny * 3.0 - ty * 0.8);
        let b = (ex - nx * 3.0 - tx * 0.8, ey - ny * 3.0 - ty * 0.8);
        let mut pb = PathBuilder::new();
        pb.move_to(tip.0, tip.1);
        pb.line_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        pb.close();
        if let Some(path) = pb.finish() {
            p.fill_path(&path, color);
        }
    }

    /// A "×" of half-size `r`.
    pub fn cross(p: &mut Painter, cx: f32, cy: f32, r: f32, width: f32, color: Rgba) {
        p.polyline(&[(cx - r, cy - r), (cx + r, cy + r)], width, color);
        p.polyline(&[(cx + r, cy - r), (cx - r, cy + r)], width, color);
    }

    pub fn plus(p: &mut Painter, cx: f32, cy: f32, color: Rgba) {
        p.polyline(&[(cx - 6.0, cy), (cx + 6.0, cy)], STROKE, color);
        p.polyline(&[(cx, cy - 6.0), (cx, cy + 6.0)], STROKE, color);
    }

    pub fn home(p: &mut Painter, cx: f32, cy: f32, color: Rgba) {
        p.polyline(
            &[(cx - 7.5, cy - 0.5), (cx, cy - 7.0), (cx + 7.5, cy - 0.5)],
            STROKE,
            color,
        );
        p.polyline(
            &[
                (cx - 5.0, cy - 2.5),
                (cx - 5.0, cy + 6.5),
                (cx - 1.5, cy + 6.5),
                (cx - 1.5, cy + 2.0),
                (cx + 1.5, cy + 2.0),
                (cx + 1.5, cy + 6.5),
                (cx + 5.0, cy + 6.5),
                (cx + 5.0, cy - 2.5),
            ],
            STROKE,
            color,
        );
    }

    pub fn chevron_up(p: &mut Painter, cx: f32, cy: f32, color: Rgba) {
        p.polyline(
            &[(cx - 5.0, cy + 2.5), (cx, cy - 2.5), (cx + 5.0, cy + 2.5)],
            STROKE,
            color,
        );
    }

    pub fn chevron_down(p: &mut Painter, cx: f32, cy: f32, color: Rgba) {
        p.polyline(
            &[(cx - 5.0, cy - 2.5), (cx, cy + 2.5), (cx + 5.0, cy - 2.5)],
            STROKE,
            color,
        );
    }

    /// A loading spinner: a 270° arc rotated by `phase` (radians).
    pub fn spinner(p: &mut Painter, cx: f32, cy: f32, r: f32, phase: f32, color: Rgba) {
        let points = arc_points(cx, cy, r, phase, std::f32::consts::PI * 1.5);
        p.polyline(&points, 2.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_conversion() {
        let src = [
            255, 0, 0, 255, // opaque red
            0, 0, 0, 255, // opaque black
            0, 0, 0, 0, // transparent → white
            64, 0, 0, 128, // half-transparent dark red over white
            10, 20, 30, 255,
        ];
        let mut dst = [0u32; 5];
        rgba_to_xrgb(&src, &mut dst);
        assert_eq!(dst, [0xff0000, 0x000000, 0xffffff, 0xbf7f7f, 0x0a141e]);
        // A short destination is not overrun.
        let mut short = [0u32; 1];
        rgba_to_xrgb(&src, &mut short);
        assert_eq!(short, [0xff0000]);
    }

    #[test]
    fn caret_stop_lookup() {
        let stops = [(0, 0.0), (1, 7.0), (3, 15.0), (4, 20.0)];
        assert_eq!(caret_x(&stops, 0), 0.0);
        assert_eq!(caret_x(&stops, 3), 15.0);
        assert_eq!(caret_x(&stops, 2), 15.0, "inside a char: next stop");
        assert_eq!(caret_x(&stops, 99), 20.0);
        assert_eq!(caret_at(&stops, -5.0), 0);
        assert_eq!(caret_at(&stops, 10.0), 1);
        assert_eq!(caret_at(&stops, 12.0), 3);
        assert_eq!(caret_at(&stops, 500.0), 4);
        assert_eq!(caret_at(&[], 5.0), 0);
        assert_eq!(caret_x(&[], 5), 0.0);
    }

    #[test]
    fn rounded_paths() {
        assert!(rounded_path(Rect::new(0.0, 0.0, 0.0, 10.0), [2.0; 4]).is_none());
        assert!(rounded_path(Rect::new(0.0, 0.0, 10.0, 10.0), [50.0; 4]).is_some());
        assert!(rounded_path(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]).is_some());
    }

    #[test]
    fn painter_draws_text_and_shapes() {
        let fonts = UiFonts::new(Arc::new(FontSystem::new()));
        let mut pixmap = Pixmap::new(200, 40).unwrap();
        pixmap.fill(tiny_skia::Color::WHITE);
        let mut p = Painter::new(&mut pixmap, 2.0, &fonts);
        p.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Rgba::rgb(255, 0, 0));
        icons::reload(&mut p, 50.0, 10.0, Rgba::rgb(0, 0, 0));
        let width = p.text(30.0, 10.0, "xbro", 13.0, Rgba::rgb(0, 0, 0), false);
        p.text_clipped(
            -5.0,
            10.0,
            "clipped",
            13.0,
            Rgba::rgb(0, 0, 0),
            true,
            Rect::new(0.0, 0.0, 20.0, 20.0),
        );
        p.fill_rect(Rect::new(-50.0, 0.0, 1e9, 1.0), Rgba::rgb(0, 0, 255));
        p.fill_rect(Rect::new(-50.0, -50.0, 1e9, 1.0), Rgba::rgb(0, 255, 0));
        assert_eq!(&pixmap.data()[..4], &[0, 0, 255, 255]);
        assert_eq!(&pixmap.data()[(2 * 200 + 2) * 4..][..4], &[255, 0, 0, 255]);
        if fonts.system.has_fonts() {
            assert!(width > 10.0);
            let measured = fonts.measure("xbro", 13.0, false, 2.0);
            assert!((measured - width).abs() < 0.01);
            let cut = fonts.truncate("a fairly long tab title", 13.0, false, 1.0, 60.0);
            assert!(cut.ends_with('…'));
            assert!(fonts.measure(&cut, 13.0, false, 1.0) <= 60.0 + 0.01);
            assert_eq!(fonts.truncate("ok", 13.0, false, 1.0, 60.0), "ok");
            let stops = fonts.caret_stops("héllo", 13.0, 1.0);
            assert_eq!(stops.first().unwrap().0, 0);
            assert_eq!(stops.last().unwrap().0, "héllo".len());
            assert!(stops.windows(2).all(|w| w[0].1 <= w[1].1));
        }
    }
}
