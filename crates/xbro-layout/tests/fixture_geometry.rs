//! Plausible flex, grid and table geometry on the fixtures, at desktop
//! and phone widths.

mod common;

use common::*;
use xbro_dom::NodeId;
use xbro_layout::{BoxKind, LayoutBox, Rect, Size};

fn load(name: &str, width: f32) -> Option<Laid> {
    let html = std::fs::read_to_string(fixtures_dir().join(name)).expect("fixture");
    Some(lay_full(&html, Size::new(width, 768.0), fonts()?, None))
}

/// Elements with class `c`, in document order.
fn all_class(l: &Laid, c: &str) -> Vec<NodeId> {
    l.doc
        .descendants(l.doc.root())
        .filter(|&n| l.doc.element(n).is_some_and(|e| e.has_class(c)))
        .collect()
}

fn rect_of(l: &Laid, n: NodeId) -> Rect {
    l.box_of(n)
        .unwrap_or_else(|| panic!("no box for {n:?}"))
        .border_box()
}

fn class_rects(l: &Laid, c: &str) -> Vec<Rect> {
    all_class(l, c)
        .into_iter()
        .filter_map(|n| l.box_of(n).map(LayoutBox::border_box))
        .collect()
}

fn first_class(l: &Laid, c: &str) -> NodeId {
    *all_class(l, c).first().unwrap_or_else(|| panic!("no .{c}"))
}

fn inside(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x - 0.5
        && inner.right() <= outer.right() + 0.5
        && inner.y >= outer.y - 0.5
        && inner.bottom() <= outer.bottom() + 0.5
}

#[test]
fn modern_header_and_features_at_desktop_width() {
    let Some(l) = load("modern.html", 1024.0) else {
        return;
    };
    // Header: logo at the start, sign-up button at the end, centered in 64px.
    let header = all_class(&l, "container")[0];
    let hc = l.box_of(header).unwrap();
    assert_eq!(hc.kind, BoxKind::Flex);
    close(hc.border_box().width, 992.0, 0.5);
    let logo = rect_of(&l, first_class(&l, "logo"));
    close(logo.x, hc.content.x, 0.01);
    let signup = rect_of(&l, first_class(&l, "btn"));
    close(signup.right(), hc.content.right(), 0.01);
    close(
        signup.y - hc.content.y,
        hc.content.bottom() - signup.bottom(),
        0.05,
    );
    let nav = rect_of(&l, l.tag("nav", 0));
    assert!(nav.x > logo.right() && nav.right() < signup.x);
    // Nav links side by side.
    let ul = l.tag("ul", 0);
    let links: Vec<Rect> = l.doc.element_children(ul).map(|n| rect_of(&l, n)).collect();
    assert_eq!(links.len(), 4);
    for w in links.windows(2) {
        assert_eq!(w[0].y, w[1].y);
        close(w[1].x - w[0].right(), 24.0, 0.05);
    }

    // Feature cards side by side, equal widths and heights.
    let cards = class_rects(&l, "card");
    assert_eq!(cards.len(), 3);
    for c in &cards {
        close(c.y, cards[0].y, 0.01);
        close(c.width, cards[0].width, 0.05);
        close(c.height, cards[0].height, 0.05);
    }
    // 2vw gaps (20.48px) and the whole container width used.
    close(cards[1].x - cards[0].right(), 20.48, 0.05);
    close(cards[2].right() - cards[0].x, 992.0, 0.5);

    // Hero buttons centered.
    let actions = rect_of(&l, first_class(&l, "actions"));
    let hero_btns: Vec<Rect> = class_rects(&l, "btn").into_iter().skip(1).take(2).collect();
    close(
        hero_btns[0].x - actions.x,
        actions.right() - hero_btns[1].right(),
        0.5,
    );
}

#[test]
fn modern_pricing_plans() {
    let Some(l) = load("modern.html", 1024.0) else {
        return;
    };
    let pricing = rect_of(&l, l.id("pricing"));
    let plans = class_rects(&l, "plan");
    assert_eq!(plans.len(), 3);
    for p in &plans {
        close(p.y, plans[0].y, 0.01);
        // `flex: 1` shares content sizes; the last plan has no right border.
        close(p.width, plans[0].width, 1.01);
        // Stretched to the tallest plan.
        close(p.height, plans[0].height, 0.05);
        assert!(inside(*p, pricing));
    }
    // In each plan (a column flex container) the list grows so that the
    // button sits at the bottom, not stretched (align-self: flex-start).
    for plan in all_class(&l, "plan") {
        let pb = l.box_of(plan).unwrap();
        let btn = pb.children.last().unwrap();
        close(btn.margin_box().bottom(), pb.content.bottom(), 0.05);
        close(btn.border_box().x, pb.content.x, 0.01);
        assert!(btn.border_box().width < pb.content.width / 2.0);
    }
}

#[test]
fn modern_layout_stacks_at_phone_width() {
    let Some(l) = load("modern.html", 360.0) else {
        return;
    };
    let cards = class_rects(&l, "card");
    assert_eq!(cards.len(), 3);
    for w in cards.windows(2) {
        close(w[0].x, w[1].x, 0.01);
        assert!(w[1].y >= w[0].bottom() + 10.0);
    }
    close(cards[0].width, 328.0, 0.05);
    let plans = class_rects(&l, "plan");
    for w in plans.windows(2) {
        close(w[0].x, w[1].x, 0.01);
        close(w[1].y, w[0].bottom(), 0.05);
    }
    // The nav's list is hidden: logo at the start, button at the end.
    let header = l.box_of(all_class(&l, "container")[0]).unwrap();
    assert_eq!(rect_of(&l, l.tag("nav", 0)).width, 0.0);
    let signup = rect_of(&l, first_class(&l, "btn"));
    close(signup.right(), header.content.right(), 0.01);
    // Footer items wrap onto two lines.
    let footer = l.box_of(all_class(&l, "container")[3]).unwrap();
    assert_eq!(footer.children.len(), 2);
    assert!(footer.children[1].border_box().y > footer.children[0].border_box().y);
}

#[test]
fn hacker_news_rows_inside_the_centered_table() {
    for width in [1024.0, 360.0] {
        let Some(l) = load("hn.html", width) else {
            return;
        };
        let main = l.box_of(l.id("hnmain")).unwrap();
        assert_eq!(main.kind, BoxKind::Table);
        let body = l.box_of(l.doc.body().unwrap()).unwrap().content;
        let mr = main.border_box();
        let expected = if width > 750.0 { 0.85 } else { 1.0 };
        close(mr.width, body.width * expected, 0.1);
        close(mr.x - body.x, body.right() - mr.right(), 0.1);
        // Every story row (and its cells) is inside the main table.
        for row in all_class(&l, "athing") {
            let r = rect_of(&l, row);
            assert!(inside(r, mr), "{r:?} not in {mr:?} at {width}");
        }
        for t in all_class(&l, "titleline") {
            let titles = l.tree.node_rect(t).unwrap();
            assert!(titles.x >= mr.x && titles.x < mr.right(), "at {width}");
        }
        // Ranks are right-aligned in their column, before the titles.
        let ranks: Vec<Rect> = all_class(&l, "rank")
            .into_iter()
            .filter_map(|n| l.tree.node_rect(n))
            .collect();
        let titles: Vec<Rect> = all_class(&l, "titleline")
            .into_iter()
            .filter_map(|n| l.tree.node_rect(n))
            .collect();
        assert_eq!(ranks.len(), 3);
        for (r, t) in ranks.iter().zip(&titles) {
            assert!(r.right() <= t.x + 0.5);
        }
        // Rows are stacked in order.
        let rows: Vec<Rect> = all_class(&l, "athing")
            .into_iter()
            .map(|n| rect_of(&l, n))
            .collect();
        for w in rows.windows(2) {
            assert!(w[1].y > w[0].bottom());
        }
        // The orange header bar spans the main table.
        let header_cell = l.box_of(l.tag("td", 0)).unwrap().border_box();
        close(header_cell.width, mr.width, 0.1);
    }
}

#[test]
fn tables_fixture_structure() {
    let Some(l) = load("tables.html", 1024.0) else {
        return;
    };
    let grid = l.box_of(first_class(&l, "grid")).unwrap();
    let gb = grid.border_box();
    let body = l.box_of(l.doc.body().unwrap()).unwrap().content;
    close(gb.width, body.width, 0.1);
    // Caption first and above the table box.
    let caption = &grid.children[0];
    assert_eq!(caption.node, Some(l.tag("caption", 0)));
    close(caption.border_box().bottom(), gb.y, 0.05);
    // thead, tbody, tfoot in order.
    let groups: Vec<_> = grid.children[1..].iter().map(|g| g.node).collect();
    assert_eq!(
        groups,
        vec![
            Some(l.tag("thead", 0)),
            Some(l.tag("tbody", 0)),
            Some(l.tag("tfoot", 0))
        ]
    );
    // "Est + Ouest" spans two rows; "2 000 (S1)" spans two columns.
    let tds = l.doc.get_elements_by_tag_name("td");
    let span = tds
        .iter()
        .copied()
        .find(|&n| l.doc.attr(n, "rowspan").is_some())
        .unwrap();
    let tbody_rows = &grid.children[2].children;
    let sr = rect_of(&l, span);
    close(sr.y, tbody_rows[2].border_box().y, 0.05);
    close(sr.bottom(), tbody_rows[3].border_box().bottom(), 0.05);
    let wide = tds
        .iter()
        .copied()
        .find(|&n| l.doc.attr(n, "colspan") == Some("2"))
        .unwrap();
    let ths: Vec<Rect> = l.doc.get_elements_by_tag_name("th")[1..3]
        .iter()
        .map(|&n| rect_of(&l, n))
        .collect();
    let wr = rect_of(&l, wide);
    close(wr.x, ths[0].x, 0.05);
    close(wr.right(), ths[1].right(), 0.05);
    // Numeric cells are right-aligned.
    let num = all_class(&l, "num")[0];
    let cell = l.box_of(num).unwrap();
    close(
        cell.lines[0].fragments.iter().fold(0.0f32, |m, f| match f {
            xbro_layout::Fragment::Text(t) => m.max(t.rect.right()),
            _ => m,
        }),
        cell.content.right(),
        0.05,
    );

    // The table-layout: fixed table: first column is 50% of the grid.
    let fixed = l.box_of(l.tag("table", 4)).unwrap();
    assert_eq!(fixed.border_box().width, 300.0);
    let tds_fixed: Vec<Rect> = tds[tds.len() - 2..]
        .iter()
        .map(|&n| rect_of(&l, n))
        .collect();
    close(tds_fixed[0].width, tds_fixed[1].width, 0.05);
    // The centered 60% table.
    let attr_table = l.box_of(l.tag("table", 2)).unwrap().border_box();
    close(attr_table.width, body.width * 0.6, 0.1);
    close(
        attr_table.x - body.x,
        body.right() - attr_table.right(),
        0.1,
    );
}

#[test]
fn wiki_page_columns_infobox_and_wikitable() {
    let Some(l) = load("wiki.html", 1024.0) else {
        return;
    };
    let toc = rect_of(&l, first_class(&l, "vector-toc"));
    let body = rect_of(&l, first_class(&l, "mw-body"));
    close(toc.width, 200.0, 0.01);
    close(body.x - toc.right(), 24.0, 0.01);
    let container = l.box_of(first_class(&l, "mw-page-container")).unwrap();
    close(body.right(), container.content.right(), 0.01);
    // Infobox: a floated table on the right, rows inside.
    let infobox = l.box_of(first_class(&l, "infobox")).unwrap();
    assert_eq!(infobox.kind, BoxKind::Table);
    let ib = infobox.border_box();
    for label in all_class(&l, "infobox-label") {
        assert!(inside(rect_of(&l, label), ib));
    }
    // Labels and data side by side.
    let label = rect_of(&l, first_class(&l, "infobox-label"));
    let data = rect_of(&l, first_class(&l, "infobox-data"));
    close(label.y, data.y, 0.01);
    assert!(data.x >= label.right());
    // Wikitable: caption above, collapsed 1px borders.
    let wt = l.box_of(first_class(&l, "wikitable")).unwrap();
    let caption = &wt.children[0];
    close(caption.border_box().bottom(), wt.border_box().y, 0.05);
    let first_row = &wt.children[1].children[0];
    let cells: Vec<Rect> = first_row
        .children
        .iter()
        .map(LayoutBox::border_box)
        .collect();
    assert_eq!(cells.len(), 3);
    for w in cells.windows(2) {
        close(w[1].x, w[0].right() - 1.0, 0.01);
    }

    let Some(phone) = load("wiki.html", 360.0) else {
        return;
    };
    // The table of contents is hidden, the body takes the whole width.
    let pbody = rect_of(&phone, first_class(&phone, "mw-body"));
    let pc = phone
        .box_of(first_class(&phone, "mw-page-container"))
        .unwrap();
    close(pbody.x, pc.content.x, 0.01);
    close(pbody.width, pc.content.width, 0.01);
    // The infobox no longer floats and fills the content width.
    let pib = phone.box_of(first_class(&phone, "infobox")).unwrap();
    let output = phone
        .box_of(first_class(&phone, "mw-parser-output"))
        .unwrap();
    close(pib.margin_box().width, output.content.width, 0.5);
}

/// Every table cell box under `b`.
fn cell_boxes<'a>(b: &'a LayoutBox, out: &mut Vec<&'a LayoutBox>) {
    if b.kind == BoxKind::TableCell {
        out.push(b);
    }
    for c in &b.children {
        cell_boxes(c, out);
    }
}

#[test]
fn tables_fixture_at_phone_width() {
    let Some(l) = load("tables.html", 360.0) else {
        return;
    };
    let body = l.box_of(l.doc.body().unwrap()).unwrap().content;
    let tables = l.doc.get_elements_by_tag_name("table");
    // The 100% grid still fits: its columns shrink between their min and
    // max-content widths (the numbers wrap), cells stay inside.
    let grid = l.box_of(tables[0]).unwrap();
    let gb = grid.border_box();
    close(gb.width, body.width, 0.1);
    let mut cells = Vec::new();
    cell_boxes(grid, &mut cells);
    assert_eq!(cells.len(), 5 + 5 + 5 + 4 + 3 + 5);
    for c in &cells {
        assert!(inside(c.border_box(), gb), "{:?}", c.border_box());
        assert!(c.border_box().width > 20.0);
    }
    // Every table but the fixed-width one is within the body.
    for (i, &t) in tables.iter().enumerate() {
        let r = rect_of(&l, t);
        assert!(
            r.x >= body.x - 0.01 && r.right() <= body.right() + 0.01,
            "table {i}: {r:?}"
        );
    }
    // The long text cell of the separated table wraps onto several lines
    // and its row is as tall as it.
    let long = l.doc.get_elements_by_tag_name("td")[22];
    let lc = l.box_of(long).unwrap();
    assert!(lc.lines.len() >= 3, "{}", l.outline());
    // The 60% table is centered and at least 60% wide.
    let attr_table = rect_of(&l, tables[2]);
    assert!(attr_table.width >= body.width * 0.6 - 0.1);
    close(
        attr_table.x - body.x,
        body.right() - attr_table.right(),
        0.1,
    );
    // The fixed layout table keeps its 300px.
    assert_eq!(rect_of(&l, tables[4]).width, 300.0);
}
