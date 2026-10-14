//! Client-side image maps: `<img usemap="#name">` with `<map name>` /
//! `<area shape coords href>`. Areas generate no boxes, so hit testing finds
//! the image; this module finds the area under a point of that image.

use xbro_dom::{Document, NodeId};
use xbro_layout::{Point, Rect};

/// Maximum number of coordinates read from a `coords` attribute.
const MAX_COORDS: usize = 4096;

/// Parses a `coords` list (numbers separated by commas and/or spaces).
fn parse_coords(s: &str) -> Vec<f32> {
    s.split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|t| !t.is_empty())
        .take(MAX_COORDS)
        .map(|t| {
            // Lenient: leading number of each token ("10px" → 10).
            let end = t
                .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
                .unwrap_or(t.len());
            t[..end]
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .unwrap_or(0.0)
        })
        .collect()
}

/// Ray-casting point-in-polygon test (`coords` = x1,y1,x2,y2,…).
fn in_polygon(c: &[f32], x: f32, y: f32) -> bool {
    let points: Vec<(f32, f32)> = c.chunks_exact(2).map(|p| (p[0], p[1])).collect();
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (xi, yi) = points[i];
        let (xj, yj) = points[j];
        if (yi > y) != (yj > y) {
            let t = (y - yi) / (yj - yi);
            if x < xi + t * (xj - xi) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// True when the local point `(x, y)` (relative to the image) is in the area.
fn area_contains(doc: &Document, area: NodeId, x: f32, y: f32) -> bool {
    let shape = doc
        .attr(area, "shape")
        .unwrap_or("rect")
        .trim()
        .to_ascii_lowercase();
    let c = parse_coords(doc.attr(area, "coords").unwrap_or(""));
    match shape.as_str() {
        "default" => true,
        "circle" | "circ" => match c.as_slice() {
            [cx, cy, r, ..] => {
                let (dx, dy) = (x - cx, y - cy);
                dx * dx + dy * dy <= r * r
            }
            _ => false,
        },
        "poly" | "polygon" => in_polygon(&c, x, y),
        _ => match c.as_slice() {
            [x1, y1, x2, y2, ..] => {
                x >= x1.min(*x2) && x < x1.max(*x2) && y >= y1.min(*y2) && y < y1.max(*y2)
            }
            _ => false,
        },
    }
}

/// The `<area href>` of the image map of `img` under `point`, given the
/// image's rectangle (document coordinates).
pub(crate) fn area_at(
    doc: &Document,
    img: NodeId,
    image_rect: Rect,
    point: Point,
) -> Option<NodeId> {
    let usemap = doc.attr(img, "usemap")?.trim();
    let name = usemap.strip_prefix('#').unwrap_or(usemap);
    if name.is_empty() {
        return None;
    }
    let map = doc.descendants(doc.root()).find(|&n| {
        doc.is_element_named(n, "map")
            && (doc.attr(n, "name") == Some(name) || doc.attr(n, "id") == Some(name))
    })?;
    let (x, y) = (point.x - image_rect.x, point.y - image_rect.y);
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    doc.descendants(map)
        .filter(|&n| doc.is_element_named(n, "area"))
        .take(10_000)
        .find(|&a| area_contains(doc, a, x, y))
        .filter(|&a| doc.attr(a, "href").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        let doc = xbro_html::parse_document(
            "<img id=i usemap='#m'><map name=m>\
             <area id=r shape=rect coords='0,0,10,10' href=r>\
             <area id=c shape=circle coords='30 30 5' href=c>\
             <area id=p shape=poly coords='50,0 60,0 60,10' href=p>\
             <area id=n shape=rect coords='70,0,80,10'>\
             <area id=d shape=default href=d></map>",
        );
        let img = doc.get_element_by_id("i").unwrap();
        let rect = Rect::new(100.0, 100.0, 200.0, 50.0);
        let at = |x: f32, y: f32| {
            area_at(&doc, img, rect, Point::new(100.0 + x, 100.0 + y))
                .and_then(|a| doc.attr(a, "id").map(str::to_string))
        };
        assert_eq!(at(5.0, 5.0).as_deref(), Some("r"));
        assert_eq!(at(31.0, 29.0).as_deref(), Some("c"));
        assert_eq!(at(59.0, 2.0).as_deref(), Some("p"));
        assert_eq!(at(51.0, 8.0).as_deref(), Some("d"), "outside the triangle");
        assert_eq!(
            at(75.0, 5.0),
            None,
            "an area without href blocks the default"
        );
        assert_eq!(at(150.0, 40.0).as_deref(), Some("d"));
        assert_eq!(parse_coords("1, 2px  3.5,,x"), vec![1.0, 2.0, 3.5, 0.0]);
        assert!(!in_polygon(&[0.0, 0.0, 1.0], 0.0, 0.0));
    }
}
