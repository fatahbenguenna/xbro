//! Form control decorations (check boxes, radio buttons, select arrows,
//! range sliders, color swatches, progress bars).

use xbro_layout::{ControlKind, Edges, LayoutBox, Point, Rect};
use xbro_style::{BorderStyle, Color};

use super::background::{inner_radii, resolve_radii, sane_rect};
use super::builder::Builder;
use super::{BorderSideItem, DisplayItem};

/// Accent color of checked/filled controls.
pub(crate) const ACCENT: Color = Color::rgb(0x00, 0x75, 0xff);
/// Border color of unchecked controls.
pub(crate) const CONTROL_BORDER: Color = Color::rgb(0x76, 0x76, 0x76);
/// Track color of range and progress controls.
pub(crate) const TRACK: Color = Color::rgb(0xef, 0xef, 0xef);
/// Border of range tracks.
const TRACK_BORDER: Color = Color::rgb(0xb2, 0xb2, 0xb2);

/// A value in 0..=1 (NaN → 0).
fn unit(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// The area a control draws into: its content box, or its border box when
/// the content box is empty.
fn control_area(b: &LayoutBox) -> Rect {
    let content = sane_rect(b.content);
    if content.is_empty() {
        sane_rect(b.border_box())
    } else {
        content
    }
}

/// The largest square centred in `r`.
fn centered_square(r: Rect) -> Rect {
    let s = r.width.min(r.height);
    Rect::new(r.x + (r.width - s) / 2.0, r.y + (r.height - s) / 2.0, s, s)
}

fn solid_border(width: f32, color: Color) -> [BorderSideItem; 4] {
    [BorderSideItem {
        width,
        style: BorderStyle::Solid,
        color,
    }; 4]
}

impl<'t> Builder<'_, 't> {
    /// Paints a control's decorations and its line content (text fields,
    /// text areas and selects clip their text to the padding box).
    pub(super) fn paint_control(
        &mut self,
        b: &'t LayoutBox,
        kind: &ControlKind,
        visible: bool,
        depth: usize,
    ) {
        match kind {
            ControlKind::TextField | ControlKind::TextArea | ControlKind::Select => {
                if visible && *kind == ControlKind::Select {
                    self.select_arrow(b);
                }
                let radii = resolve_radii(&b.style.border_radius, sane_rect(b.border_box()));
                self.push_clip(b.padding_box(), inner_radii(radii, b.border));
                self.paint_lines(&b.lines, depth);
                self.pop_clip();
                return;
            }
            ControlKind::Button => {}
            ControlKind::Checkbox { checked } if visible => self.checkbox(b, *checked),
            ControlKind::Radio { checked } if visible => self.radio(b, *checked),
            ControlKind::Range { value } if visible => self.range(b, unit(*value)),
            ControlKind::Color(color) if visible => self.color_swatch(b, *color),
            ControlKind::Progress { value } if visible => self.progress(b, unit(*value)),
            _ => {}
        }
        self.paint_lines(&b.lines, depth);
    }

    fn checkbox(&mut self, b: &LayoutBox, checked: bool) {
        let r = centered_square(control_area(b));
        if r.is_empty() {
            return;
        }
        let radii = [2.0f32.min(r.width / 2.0); 4];
        if checked {
            self.items.push(DisplayItem::Rect {
                rect: r,
                color: ACCENT,
                radii,
            });
            let s = r.width;
            let p = |fx: f32, fy: f32| Point::new(r.x + fx * s, r.y + fy * s);
            self.items.push(DisplayItem::Polyline {
                points: vec![p(0.22, 0.52), p(0.42, 0.72), p(0.78, 0.30)],
                width: (s * 0.14).max(1.0),
                color: Color::WHITE,
            });
        } else {
            self.items.push(DisplayItem::Rect {
                rect: r,
                color: Color::WHITE,
                radii,
            });
            self.items.push(DisplayItem::Border {
                rect: r,
                sides: solid_border(1.0, CONTROL_BORDER),
                radii,
            });
        }
    }

    fn radio(&mut self, b: &LayoutBox, checked: bool) {
        let r = centered_square(control_area(b));
        if r.is_empty() {
            return;
        }
        let ellipse =
            |rect: Rect, color: Color, fill: bool, stroke_width: f32| DisplayItem::Ellipse {
                rect,
                color,
                fill,
                stroke_width,
            };
        self.items.push(ellipse(r, Color::WHITE, true, 0.0));
        let ring = if checked { ACCENT } else { CONTROL_BORDER };
        let stroke = if checked {
            (r.width / 13.0 * 1.5).max(1.0)
        } else {
            1.0
        };
        let stroke = stroke.min(r.width / 2.0);
        self.items.push(ellipse(
            r.inset(Edges::all(stroke / 2.0)),
            ring,
            false,
            stroke,
        ));
        if checked {
            let d = r.width * 0.5;
            let dot = Rect::new(r.x + (r.width - d) / 2.0, r.y + (r.height - d) / 2.0, d, d);
            self.items.push(ellipse(dot, ACCENT, true, 0.0));
        }
    }

    fn select_arrow(&mut self, b: &LayoutBox) {
        let pb = sane_rect(b.padding_box());
        if pb.is_empty() || b.style.color.is_transparent() {
            return;
        }
        let w = (b.style.font_size * 0.6).clamp(4.0, 12.0).min(pb.width);
        let h = w / 2.0;
        let cx = pb.right() - 4.0 - w / 2.0;
        let cy = pb.y + pb.height / 2.0;
        self.items.push(DisplayItem::Polygon {
            points: vec![
                Point::new(cx - w / 2.0, cy - h / 2.0),
                Point::new(cx + w / 2.0, cy - h / 2.0),
                Point::new(cx, cy + h / 2.0),
            ],
            color: b.style.color,
        });
    }

    fn range(&mut self, b: &LayoutBox, value: f32) {
        let area = control_area(b);
        if area.is_empty() {
            return;
        }
        let thumb = area.height.min(16.0);
        let track_h = 4.0f32.min(area.height);
        let track = Rect::new(
            area.x,
            area.y + (area.height - track_h) / 2.0,
            area.width,
            track_h,
        );
        let radii = [track_h / 2.0; 4];
        self.items.push(DisplayItem::Rect {
            rect: track,
            color: TRACK,
            radii,
        });
        self.items.push(DisplayItem::Border {
            rect: track,
            sides: solid_border(1.0f32.min(track_h / 2.0), TRACK_BORDER),
            radii,
        });
        let travel = (area.width - thumb).max(0.0);
        let cx = area.x + thumb / 2.0 + value * travel;
        let filled = Rect::new(area.x, track.y, (cx - area.x).max(0.0), track_h);
        if !filled.is_empty() {
            self.items.push(DisplayItem::Rect {
                rect: filled,
                color: ACCENT,
                radii,
            });
        }
        if thumb > 0.0 {
            self.items.push(DisplayItem::Ellipse {
                rect: Rect::new(
                    cx - thumb / 2.0,
                    area.y + (area.height - thumb) / 2.0,
                    thumb,
                    thumb,
                ),
                color: ACCENT,
                fill: true,
                stroke_width: 0.0,
            });
        }
    }

    fn color_swatch(&mut self, b: &LayoutBox, color: Color) {
        let area = control_area(b);
        let inset = if area.width > 6.0 && area.height > 6.0 {
            2.0
        } else {
            0.0
        };
        let swatch = area.inset(Edges::all(inset));
        if swatch.is_empty() {
            return;
        }
        self.items.push(DisplayItem::Rect {
            rect: swatch,
            color,
            radii: [0.0; 4],
        });
        self.items.push(DisplayItem::Border {
            rect: swatch,
            sides: solid_border(1.0, CONTROL_BORDER),
            radii: [0.0; 4],
        });
    }

    fn progress(&mut self, b: &LayoutBox, value: f32) {
        let area = control_area(b);
        if area.is_empty() {
            return;
        }
        let radii = [(area.height / 2.0).min(4.0); 4];
        self.items.push(DisplayItem::Rect {
            rect: area,
            color: TRACK,
            radii,
        });
        let bar = Rect::new(area.x, area.y, area.width * value, area.height);
        if !bar.is_empty() {
            self.items.push(DisplayItem::Rect {
                rect: bar,
                color: ACCENT,
                radii,
            });
        }
    }
}
