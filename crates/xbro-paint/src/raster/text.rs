//! Text: glyph coverage masks blended straight into the target pixels.

use tiny_skia::Mask;
use xbro_style::Color;
use xbro_text::GlyphBitmap;

use super::clip::DevRect;
use super::{div255, Painter};
use crate::display_list::GlyphRun;

/// Integer clip rectangle (device px, exclusive right/bottom).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntClip {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

/// Blends a glyph coverage mask at (`gx`, `gy`) (top-left of the bitmap)
/// with `color`, source-over on premultiplied RGBA `data` of width `width`,
/// restricted to `clip` (which must lie inside the pixmap) and modulated by
/// `mask` (same size as the pixmap) when given.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_glyph(
    data: &mut [u8],
    width: u32,
    bitmap: &GlyphBitmap,
    gx: i32,
    gy: i32,
    color: Color,
    clip: IntClip,
    mask: Option<&Mask>,
) {
    let (bw, bh) = (bitmap.width as i32, bitmap.height as i32);
    let x0 = gx.max(clip.x0);
    let x1 = gx.saturating_add(bw).min(clip.x1);
    let y0 = gy.max(clip.y0);
    let y1 = gy.saturating_add(bh).min(clip.y1);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let (r, g, b, a) = (
        u32::from(color.r),
        u32::from(color.g),
        u32::from(color.b),
        u32::from(color.a),
    );
    let width = width as usize;
    let span = (x1 - x0) as usize;
    let mask_data = mask.map(|m| m.data());
    for y in y0..y1 {
        let src_start = (y - gy) as usize * bw as usize + (x0 - gx) as usize;
        let Some(coverage) = bitmap.coverage.get(src_start..src_start + span) else {
            continue;
        };
        let dst_start = y as usize * width + x0 as usize;
        let Some(dst) = data.get_mut(dst_start * 4..(dst_start + span) * 4) else {
            continue;
        };
        let mask_row = match mask_data {
            Some(m) => match m.get(dst_start..dst_start + span) {
                Some(row) => Some(row),
                None => continue,
            },
            None => None,
        };
        for (i, (px, &c)) in dst.chunks_exact_mut(4).zip(coverage).enumerate() {
            if c == 0 {
                continue;
            }
            let mut c = u32::from(c);
            if let Some(row) = mask_row {
                c = div255(c * u32::from(row.get(i).copied().unwrap_or(0)));
            }
            let sa = div255(c * a);
            if sa == 0 {
                continue;
            }
            if sa == 255 {
                px[0] = r as u8;
                px[1] = g as u8;
                px[2] = b as u8;
                px[3] = 255;
            } else {
                let inv = 255 - sa;
                px[0] = (div255(r * sa) + div255(u32::from(px[0]) * inv)).min(255) as u8;
                px[1] = (div255(g * sa) + div255(u32::from(px[1]) * inv)).min(255) as u8;
                px[2] = (div255(b * sa) + div255(u32::from(px[2]) * inv)).min(255) as u8;
                px[3] = (sa + div255(u32::from(px[3]) * inv)).min(255) as u8;
            }
        }
    }
}

/// Draws a glyph run through the current clip.
pub(crate) fn draw_run(p: &mut Painter<'_>, run: &GlyphRun) {
    let scale = p.scale();
    let size = run.font_size * scale;
    if run.color.is_transparent() || run.glyphs.is_empty() || !(size.is_finite() && size > 0.0) {
        return;
    }
    let (ox, oy) = p.to_dev_point(run.origin);
    // Generous ink bounds: accents, descenders and fallback glyphs overflow the em box.
    let width = run.width() * scale;
    let bounds = DevRect::new(
        ox - size,
        oy - 1.5 * size,
        ox + width + size,
        oy + 0.75 * size,
    );
    let Some((visible, mask)) = p.rect_clip(bounds) else {
        return;
    };
    let clip_rect = p.clip().rect.intersect(&p.area());
    let clip = IntClip {
        x0: clip_rect.x0.round() as i32,
        y0: clip_rect.y0.round() as i32,
        x1: clip_rect.x1.round() as i32,
        y1: clip_rect.y1.round() as i32,
    };
    let fonts = p.fonts();
    let (area_w, _) = p.size();
    let baseline = oy.round() as i32;
    let (vx0, vx1) = (visible.x0 - 2.0 * size, visible.x1 + size);
    let Some(mut target) = p.target() else {
        return;
    };
    let data = target.data_mut();
    for glyph in &run.glyphs {
        let pen = ox + glyph.x * scale;
        if !pen.is_finite() || pen < vx0 || pen > vx1 {
            continue;
        }
        let floor = pen.floor();
        let Some(bitmap) = fonts.rasterize(glyph.font, glyph.glyph_id, size, pen - floor) else {
            continue;
        };
        blend_glyph(
            data,
            area_w,
            &bitmap,
            floor as i32 + bitmap.left,
            baseline + bitmap.top,
            run.color,
            clip,
            mask.as_deref(),
        );
    }
    p.mark_dirty(visible);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap() -> GlyphBitmap {
        GlyphBitmap {
            width: 2,
            height: 2,
            left: 0,
            top: 0,
            coverage: vec![255, 128, 0, 255],
        }
    }

    #[test]
    fn blends_coverage() {
        let mut data = vec![255u8; 3 * 3 * 4];
        let clip = IntClip {
            x0: 0,
            y0: 0,
            x1: 3,
            y1: 3,
        };
        blend_glyph(&mut data, 3, &bitmap(), 1, 1, Color::BLACK, clip, None);
        let px = |x: usize, y: usize| &data[(y * 3 + x) * 4..(y * 3 + x) * 4 + 4];
        assert_eq!(px(1, 1), &[0, 0, 0, 255]);
        assert_eq!(px(2, 1), &[127, 127, 127, 255]);
        assert_eq!(px(1, 2), &[255, 255, 255, 255]);
        assert_eq!(px(0, 0), &[255, 255, 255, 255]);
    }

    #[test]
    fn respects_clip_and_bounds() {
        let mut data = vec![255u8; 3 * 3 * 4];
        let clip = IntClip {
            x0: 0,
            y0: 0,
            x1: 2,
            y1: 3,
        };
        blend_glyph(&mut data, 3, &bitmap(), 1, 1, Color::BLACK, clip, None);
        assert_eq!(&data[(3 + 2) * 4..(3 + 2) * 4 + 4], &[255, 255, 255, 255]);
        // Far outside: nothing happens, no panic.
        blend_glyph(
            &mut data,
            3,
            &bitmap(),
            i32::MAX - 1,
            i32::MIN,
            Color::BLACK,
            clip,
            None,
        );
        blend_glyph(&mut data, 3, &bitmap(), -5, -5, Color::BLACK, clip, None);
    }

    #[test]
    fn mask_modulates() {
        let mut data = vec![255u8; 2 * 2 * 4];
        let mut mask = Mask::new(2, 2).unwrap();
        mask.data_mut().copy_from_slice(&[0, 255, 255, 255]);
        let clip = IntClip {
            x0: 0,
            y0: 0,
            x1: 2,
            y1: 2,
        };
        blend_glyph(
            &mut data,
            2,
            &bitmap(),
            0,
            0,
            Color::BLACK,
            clip,
            Some(&mask),
        );
        assert_eq!(&data[0..4], &[255, 255, 255, 255]);
        assert_eq!(&data[12..16], &[0, 0, 0, 255]);
    }
}
