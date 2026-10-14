//! Operations on laid-out boxes: translation of whole subtrees and baseline
//! queries.

use crate::{BoxKind, Fragment, LayoutBox, LineBox, MarkerContent};

/// Moves a laid-out box and everything it contains by `(dx, dy)`.
pub(crate) fn translate_box(b: &mut LayoutBox, dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    b.content = b.content.translate(dx, dy);
    if let Some((a, e)) = &mut b.border_top_gap {
        *a += dx;
        *e += dx;
    }
    translate_lines(&mut b.lines, dx, dy);
    if let Some(m) = &mut b.marker {
        m.rect = m.rect.translate(dx, dy);
        if let MarkerContent::Text(t) = &mut m.content {
            t.rect = t.rect.translate(dx, dy);
            t.baseline += dy;
        }
    }
    for c in &mut b.children {
        translate_box(c, dx, dy);
    }
}

/// Moves line boxes and their fragments by `(dx, dy)`.
pub(crate) fn translate_lines(lines: &mut [LineBox], dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    for line in lines {
        line.rect = line.rect.translate(dx, dy);
        line.baseline += dy;
        for f in &mut line.fragments {
            match f {
                Fragment::Text(t) => {
                    t.rect = t.rect.translate(dx, dy);
                    t.baseline += dy;
                }
                Fragment::InlineBox(ib) => ib.rect = ib.rect.translate(dx, dy),
                Fragment::Atomic(a) => translate_box(a, dx, dy),
            }
        }
    }
}

/// True for boxes taken out of the normal flow (floats, absolute, fixed).
pub(crate) fn is_out_of_flow_box(b: &LayoutBox) -> bool {
    b.style.is_out_of_flow()
}

/// True for a table caption box (children of [`BoxKind::Table`] boxes laid
/// out above the table box, which do not define the table's baseline).
pub(crate) fn is_caption_box(b: &LayoutBox) -> bool {
    b.style.display == xbro_style::Display::TableCaption && b.kind == BoxKind::Block
}

/// How far the bottom captions of a table box reach below its border box
/// (0 for other boxes).
pub(crate) fn bottom_caption_extent(b: &LayoutBox) -> f32 {
    if b.kind != BoxKind::Table {
        return 0.0;
    }
    let bottom = b.border_box().bottom();
    b.children
        .iter()
        .filter(|c| is_caption_box(c) && c.style.caption_side == xbro_style::CaptionSide::Bottom)
        .map(|c| c.margin_box().bottom() - bottom)
        .fold(0.0f32, f32::max)
        .max(0.0)
}

/// Baseline (absolute y) of the first line box in the box's in-flow content.
/// The baseline of a table is the one of its first row (captions are skipped).
pub(crate) fn first_baseline(b: &LayoutBox) -> Option<f32> {
    if let Some(l) = b.lines.first() {
        return Some(l.baseline);
    }
    let table = b.kind == BoxKind::Table;
    b.children
        .iter()
        .filter(|c| !is_out_of_flow_box(c) && !(table && is_caption_box(c)))
        .find_map(first_baseline)
}

/// Baseline (absolute y) of the last line box in the box's in-flow content.
pub(crate) fn last_baseline(b: &LayoutBox) -> Option<f32> {
    if let Some(l) = b.lines.last() {
        return Some(l.baseline);
    }
    b.children
        .iter()
        .rev()
        .filter(|c| !is_out_of_flow_box(c))
        .find_map(last_baseline)
}
