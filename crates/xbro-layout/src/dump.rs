//! Text dumps of a laid-out tree (`xbro --dump-layout`, tests).

use std::fmt::Write as _;

use xbro_dom::{Document, NodeData, NodeId};

use crate::geom::{Edges, Rect};
use crate::{
    BoxKind, ControlKind, Fragment, LayoutBox, LayoutTree, LineBox, MarkerContent, Pseudo,
};

/// Recursion guard.
const MAX_DEPTH: usize = 2048;

/// Longest text excerpt printed per fragment, in chars.
const MAX_TEXT: usize = 60;

/// `12`, `12.5` or `12.25`: at most two decimals, no trailing zeros.
fn num(v: f32) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    let r = (v * 100.0).round() / 100.0;
    let s = format!("{r:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

fn rect(r: Rect) -> String {
    format!(
        "({}, {}) {}×{}",
        num(r.x),
        num(r.y),
        num(r.width),
        num(r.height)
    )
}

fn edges(e: Edges) -> String {
    if e.top == e.right && e.top == e.bottom && e.top == e.left {
        num(e.top)
    } else {
        format!(
            "{} {} {} {}",
            num(e.top),
            num(e.right),
            num(e.bottom),
            num(e.left)
        )
    }
}

fn excerpt(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_TEXT).collect();
    if s.chars().count() > MAX_TEXT {
        out.push('…');
    }
    format!("{out:?}")
}

/// `<tag>`, `#text`, `<tag>::before` or `(anonymous)`.
fn node_name(doc: &Document, node: Option<NodeId>, pseudo: Option<Pseudo>) -> String {
    let Some(n) = node.filter(|n| n.index() < doc.len()) else {
        return "(anonymous)".to_string();
    };
    let base = match &doc.node(n).data {
        NodeData::Element(e) => {
            let mut s = format!("<{}", e.name);
            if let Some(id) = e.id() {
                let _ = write!(s, " id={id:?}");
            }
            s.push('>');
            s
        }
        NodeData::Text(_) => "#text".to_string(),
        _ => format!("#{}", n.index()),
    };
    match pseudo {
        Some(Pseudo::Before) => format!("{base}::before"),
        Some(Pseudo::After) => format!("{base}::after"),
        None => base,
    }
}

fn kind_name(k: &BoxKind) -> String {
    match k {
        BoxKind::Block => "Block".into(),
        BoxKind::Flex => "Flex".into(),
        BoxKind::Table => "Table".into(),
        BoxKind::TableRowGroup => "TableRowGroup".into(),
        BoxKind::TableRow => "TableRow".into(),
        BoxKind::TableCell => "TableCell".into(),
        BoxKind::Image => "Image".into(),
        BoxKind::Placeholder => "Placeholder".into(),
        BoxKind::Control(c) => match c {
            ControlKind::TextField => "Control(TextField)".into(),
            ControlKind::Button => "Control(Button)".into(),
            ControlKind::Checkbox { checked } => format!("Control(Checkbox checked={checked})"),
            ControlKind::Radio { checked } => format!("Control(Radio checked={checked})"),
            ControlKind::Select => "Control(Select)".into(),
            ControlKind::TextArea => "Control(TextArea)".into(),
            ControlKind::Range { value } => format!("Control(Range {})", num(*value)),
            ControlKind::Color(c) => format!("Control(Color {c:?})"),
            ControlKind::Progress { value } => format!("Control(Progress {})", num(*value)),
        },
    }
}

pub(crate) fn dump(tree: &LayoutTree, doc: &Document) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "viewport {}×{}, document {}×{}",
        num(tree.viewport.width),
        num(tree.viewport.height),
        num(tree.document_size.width),
        num(tree.document_size.height)
    );
    match &tree.root {
        Some(root) => dump_box(&mut out, doc, root, 0),
        None => out.push_str("(no root box)\n"),
    }
    out
}

fn dump_box(out: &mut String, doc: &Document, b: &LayoutBox, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let ind = "  ".repeat(depth);
    let _ = write!(
        out,
        "{ind}{} {} border-box {}",
        kind_name(&b.kind),
        node_name(doc, b.node, b.pseudo),
        rect(b.border_box())
    );
    if b.margin != Edges::ZERO {
        let _ = write!(out, " margin {}", edges(b.margin));
    }
    if b.border != Edges::ZERO {
        let _ = write!(out, " border {}", edges(b.border));
    }
    if b.padding != Edges::ZERO {
        let _ = write!(out, " padding {}", edges(b.padding));
    }
    let s = &b.style;
    if s.position != xbro_style::Position::Static {
        let _ = write!(out, " position:{:?}", s.position);
    }
    if s.float != xbro_style::Float::None {
        let _ = write!(out, " float:{:?}", s.float);
    }
    out.push('\n');
    if let Some(m) = &b.marker {
        let what = match &m.content {
            MarkerContent::Disc => "disc".to_string(),
            MarkerContent::Circle => "circle".to_string(),
            MarkerContent::Square => "square".to_string(),
            MarkerContent::Text(t) => excerpt(&t.text),
            MarkerContent::Image(url) => format!("image {}", excerpt(url)),
        };
        let _ = writeln!(out, "{ind}  marker {what} {}", rect(m.rect));
    }
    for line in &b.lines {
        dump_line(out, doc, line, depth + 1);
    }
    for c in &b.children {
        dump_box(out, doc, c, depth + 1);
    }
}

fn dump_line(out: &mut String, doc: &Document, line: &LineBox, depth: usize) {
    let ind = "  ".repeat(depth);
    let _ = writeln!(
        out,
        "{ind}line {} baseline {}",
        rect(line.rect),
        num(line.baseline)
    );
    for f in &line.fragments {
        match f {
            Fragment::Text(t) => {
                let _ = writeln!(
                    out,
                    "{ind}  text {} {} {} size {}",
                    node_name(doc, Some(t.node), None),
                    excerpt(&t.text),
                    rect(t.rect),
                    num(t.font_size)
                );
            }
            Fragment::InlineBox(ib) => {
                let _ = writeln!(
                    out,
                    "{ind}  inline {} {}",
                    node_name(doc, Some(ib.node), None),
                    rect(ib.rect)
                );
            }
            Fragment::Atomic(a) => dump_box(out, doc, a, depth + 1),
        }
    }
}

pub(crate) fn outline(tree: &LayoutTree, doc: &Document) -> String {
    let mut out = String::new();
    if let Some(root) = &tree.root {
        outline_box(&mut out, doc, root, 0);
    }
    out
}

fn short_rect(r: Rect) -> String {
    format!(
        "{},{} {}x{}",
        r.x.round(),
        r.y.round(),
        r.width.round(),
        r.height.round()
    )
}

fn outline_box(out: &mut String, doc: &Document, b: &LayoutBox, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let ind = "  ".repeat(depth);
    let name = match (b.node, b.pseudo) {
        (None, _) => "anon".to_string(),
        (Some(n), p) => {
            let base = doc
                .tag_name(n)
                .filter(|_| n.index() < doc.len())
                .unwrap_or("?")
                .to_string();
            match p {
                Some(Pseudo::Before) => format!("{base}::before"),
                Some(Pseudo::After) => format!("{base}::after"),
                None => base,
            }
        }
    };
    let _ = writeln!(out, "{ind}{name} {}", short_rect(b.border_box()));
    for line in &b.lines {
        let mut text = String::new();
        for f in &line.fragments {
            match f {
                Fragment::Text(t) => text.push_str(&t.text),
                Fragment::Atomic(_) => text.push('\u{FFFC}'),
                Fragment::InlineBox(_) => {}
            }
        }
        let _ = writeln!(out, "{ind}  | {} {:?}", short_rect(line.rect), text);
        for f in &line.fragments {
            if let Fragment::Atomic(a) = f {
                outline_box(out, doc, a, depth + 2);
            }
        }
    }
    for c in &b.children {
        outline_box(out, doc, c, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(num(12.0), "12");
        assert_eq!(num(12.5), "12.5");
        assert_eq!(num(-0.001), "0");
        assert_eq!(num(1.23456), "1.23");
        assert_eq!(excerpt("ab"), "\"ab\"");
    }
}
