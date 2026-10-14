//! Cascade, inheritance and computation of the grid template / placement,
//! table (`table-layout`, `caption-side`) and word-breaking
//! (`overflow-wrap`, `word-break`) properties.

mod common;

use common::{style, style_with, viewport};
use xbro_style::*;

fn px(v: f32) -> GridTrack {
    GridTrack::Length(LengthPercentage::px(v))
}

fn fr(v: f32) -> GridTrack {
    GridTrack::Fr(v)
}

fn tracks(t: &Option<std::sync::Arc<GridTemplate>>) -> Vec<GridTrack> {
    t.as_ref().expect("a template").tracks.clone()
}

#[test]
fn table_layout_and_caption_side() {
    let s = style(
        "<table id=t><caption id=c>c</caption><tr><td id=td><table id=inner></table></td></tr></table>\
         <table id=u><caption id=cb align=BOTTOM>b</caption><caption id=ca align=bottom>a</caption></table>",
        "#t { table-layout: fixed; caption-side: bottom }
         #ca { caption-side: top }",
    );
    let t = s.get("t");
    assert_eq!(t.table_layout, TableLayout::Fixed);
    assert_eq!(t.caption_side, CaptionSide::Bottom);
    // caption-side is inherited, table-layout is not.
    assert_eq!(s.get("c").caption_side, CaptionSide::Bottom);
    assert_eq!(s.get("td").caption_side, CaptionSide::Bottom);
    assert_eq!(s.get("inner").table_layout, TableLayout::Auto);
    assert_eq!(s.get("u").caption_side, CaptionSide::Top);
    // `<caption align=bottom>` (UA rule), overridden by author rules.
    assert_eq!(s.get("cb").caption_side, CaptionSide::Bottom);
    assert_eq!(s.get("ca").caption_side, CaptionSide::Top);

    let s = style(
        "<table id=t><tr><td id=td><table id=inner></table></td></tr></table>",
        "#t { table-layout: fixed; caption-side: bottom }
         #inner { table-layout: inherit; caption-side: initial }
         #td { caption-side: bogus; table-layout: fixed }",
    );
    assert_eq!(s.get("td").caption_side, CaptionSide::Bottom);
    assert_eq!(s.get("inner").table_layout, TableLayout::Fixed);
    assert_eq!(s.get("inner").caption_side, CaptionSide::Top);
}

#[test]
fn overflow_wrap_and_word_break() {
    let s = style(
        "<div id=a><p id=a1>x</p></div><div id=b><p id=b1>x</p></div><div id=c></div>\
         <div id=d><p id=d1>x</p></div><div id=e></div><div id=f></div><textarea id=t></textarea>",
        "#a { overflow-wrap: anywhere }
         #b { word-wrap: break-word; word-break: break-all }
         #b1 { word-break: normal }
         #c { word-break: keep-all }
         #d { word-break: break-word }
         #d1 { overflow-wrap: normal }
         #e { word-break: break-word; overflow-wrap: anywhere }
         #f { overflow-wrap: anywhere; overflow-wrap: bogus; word-break: auto-phrase }",
    );
    let check = |id: &str, wrap: OverflowWrap, brk: WordBreak, anywhere: bool| {
        let st = s.get(id);
        assert_eq!(
            (st.overflow_wrap, st.word_break, st.break_anywhere),
            (wrap, brk, anywhere),
            "#{id}"
        );
    };
    check("a", OverflowWrap::Anywhere, WordBreak::Normal, true);
    check("a1", OverflowWrap::Anywhere, WordBreak::Normal, true);
    check("b", OverflowWrap::BreakWord, WordBreak::BreakAll, true);
    check("b1", OverflowWrap::BreakWord, WordBreak::Normal, true);
    check("c", OverflowWrap::Normal, WordBreak::KeepAll, false);
    // `word-break: break-word` = normal + overflow-wrap: break-word, even
    // when a descendant resets overflow-wrap (word-break is inherited).
    check("d", OverflowWrap::BreakWord, WordBreak::Normal, true);
    check("d1", OverflowWrap::BreakWord, WordBreak::Normal, true);
    check("e", OverflowWrap::Anywhere, WordBreak::Normal, true);
    check("f", OverflowWrap::Anywhere, WordBreak::Normal, true);
    check("t", OverflowWrap::BreakWord, WordBreak::Normal, true);
    let st = s.get("b");
    assert!(st.overflow_wrap.breaks_overflowing_words());
    assert!(!st.overflow_wrap.reduces_min_content());
    assert!(s.get("a").overflow_wrap.reduces_min_content());

    // `initial` / `unset` on the underlying longhands.
    let s = style(
        "<div id=p><p id=i>x</p><p id=u>x</p></div>",
        "#p { overflow-wrap: anywhere; word-break: break-all }
         #i { overflow-wrap: initial; word-break: initial }
         #u { overflow-wrap: unset }",
    );
    assert_eq!(s.get("i").overflow_wrap, OverflowWrap::Normal);
    assert_eq!(s.get("i").word_break, WordBreak::Normal);
    assert!(!s.get("i").break_anywhere);
    assert_eq!(s.get("u").overflow_wrap, OverflowWrap::Anywhere);
    assert_eq!(s.get("u").word_break, WordBreak::BreakAll);
}

#[test]
fn grid_templates() {
    let s = style(
        "<div id=g><div id=item>x</div></div><div id=h></div><div id=em></div><div id=r></div>\
         <div id=n></div><div id=i><div id=ii></div></div>",
        "#g { display: grid; grid-template-columns: [a] 100px repeat(2, 1fr) minmax(min-content, 20%);
              grid-template-rows: auto fit-content(50px) }
         #h { display: block; grid-template-columns: 1fr 1fr }
         #em { display: grid; font-size: 10px; grid-template-columns: 2em calc(1em + 5%) }
         #r { display: grid; grid-template-columns: 50px repeat(auto-fill, minmax(10em, 1fr)) 50px }
         #n { display: grid; grid-template-columns: 1fr; grid-template-columns: none }
         #i { display: grid; grid-template-columns: 1fr 2fr }
         #ii { display: grid; grid-template-columns: inherit }",
    );
    let g = s.get("g");
    assert_eq!(
        tracks(&g.grid_template_columns),
        vec![
            px(100.0),
            fr(1.0),
            fr(1.0),
            GridTrack::MinMax(
                Box::new(GridTrack::MinContent),
                Box::new(GridTrack::Length(LengthPercentage::percent(20.0)))
            )
        ]
    );
    assert_eq!(g.grid_columns, 4, "explicit track count");
    assert_eq!(
        tracks(&g.grid_template_rows),
        vec![
            GridTrack::Auto,
            GridTrack::FitContent(LengthPercentage::px(50.0))
        ]
    );
    // Not inherited.
    assert_eq!(s.get("item").grid_template_columns, None);
    assert_eq!(s.get("item").grid_columns, 0);
    // The template is kept on non-grids, the count is 0 there.
    let h = s.get("h");
    assert_eq!(h.grid_columns, 0);
    assert_eq!(tracks(&h.grid_template_columns).len(), 2);
    // Lengths are computed with the element's font.
    assert_eq!(
        tracks(&s.get("em").grid_template_columns),
        vec![
            px(20.0),
            GridTrack::Length(LengthPercentage {
                px: 10.0,
                percent: 5.0
            })
        ]
    );
    let r = s.get("r");
    let t = r.grid_template_columns.as_ref().unwrap();
    assert_eq!(t.tracks, vec![px(50.0), px(50.0)]);
    assert_eq!(t.auto_repeat_index, 1);
    let auto = t.auto_repeat.as_ref().unwrap();
    assert!(auto.fill);
    assert_eq!(
        auto.tracks,
        vec![GridTrack::MinMax(Box::new(px(160.0)), Box::new(fr(1.0)))]
    );
    assert_eq!(r.grid_columns, 0, "auto-repeat: unknown count");
    assert_eq!(s.get("n").grid_template_columns, None);
    assert_eq!(s.get("n").grid_columns, 0);
    let ii = s.get("ii");
    assert_eq!(tracks(&ii.grid_template_columns), vec![fr(1.0), fr(2.0)]);
    assert_eq!(ii.grid_columns, 2);
    // Context-free templates are shared, not copied per element.
    assert!(std::sync::Arc::ptr_eq(
        s.get("i").grid_template_columns.as_ref().unwrap(),
        ii.grid_template_columns.as_ref().unwrap()
    ));
}

#[test]
fn grid_shorthands_and_vars() {
    let s = style_with(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div>",
        &["div { display: grid; --cols: repeat(3, 2vw) }
           #a { grid-template: 40px 1fr / auto 1fr }
           #b { grid: auto-flow dense / 1fr 1fr 1fr }
           #c { grid-template: \"head head\" 30px \"nav main\" / 120px 1fr }
           #d { grid-template-columns: var(--cols) }
           #e { grid-template-columns: 1fr 1fr; grid: 100px / auto-flow }"],
        &viewport(1000.0, 800.0),
    );
    let a = s.get("a");
    assert_eq!(tracks(&a.grid_template_rows), vec![px(40.0), fr(1.0)]);
    assert_eq!(
        tracks(&a.grid_template_columns),
        vec![GridTrack::Auto, fr(1.0)]
    );
    assert_eq!(a.grid_columns, 2);
    let b = s.get("b");
    assert_eq!(b.grid_template_rows, None);
    assert_eq!(b.grid_columns, 3);
    let c = s.get("c");
    assert_eq!(
        tracks(&c.grid_template_rows),
        vec![px(30.0), GridTrack::Auto]
    );
    assert_eq!(tracks(&c.grid_template_columns), vec![px(120.0), fr(1.0)]);
    let d = s.get("d");
    assert_eq!(tracks(&d.grid_template_columns), vec![px(20.0); 3]);
    assert_eq!(d.grid_columns, 3);
    let e = s.get("e");
    assert_eq!(tracks(&e.grid_template_rows), vec![px(100.0)]);
    assert_eq!(
        e.grid_template_columns, None,
        "reset by the `grid` shorthand"
    );
    assert_eq!(e.grid_columns, 0);
}

#[test]
fn grid_placement() {
    let s = style(
        "<div id=g><p id=a>x</p><p id=b>x</p><p id=c>x</p><p id=d>x</p><p id=e>x</p></div>",
        "#g { display: grid; grid-column: 1 / 3 }
         #a { grid-column: 1 / -1; grid-row: span 2 }
         #b { grid-column: 2 / span 3; grid-column-end: 5; grid-row-start: 3 }
         #c { grid-area: 2 / 1 / 4 / 3 }
         #d { grid-area: header; grid-row: 0 / 2 }
         #e { grid-column: inherit; grid-row-end: -2 }",
    );
    let line = GridLine::Line;
    let a = s.get("a");
    assert_eq!(
        a.grid_column,
        GridPlacement {
            start: line(1),
            end: line(-1)
        }
    );
    assert_eq!(
        a.grid_row,
        GridPlacement {
            start: GridLine::Span(2),
            end: GridLine::Auto
        }
    );
    let b = s.get("b");
    assert_eq!(
        b.grid_column,
        GridPlacement {
            start: line(2),
            end: line(5)
        }
    );
    assert_eq!(b.grid_row.start, line(3));
    let c = s.get("c");
    assert_eq!(
        (c.grid_row, c.grid_column),
        (
            GridPlacement {
                start: line(2),
                end: line(4)
            },
            GridPlacement {
                start: line(1),
                end: line(3)
            }
        )
    );
    // Named areas compute to auto; the invalid `grid-row` is ignored.
    let d = s.get("d");
    assert!(d.grid_row.is_auto() && d.grid_column.is_auto());
    let e = s.get("e");
    assert_eq!(e.grid_column, s.get("g").grid_column, "inherit");
    assert_eq!(e.grid_row.end, line(-2));
    // Not inherited.
    let s = style(
        "<div id=g><p id=p>x</p></div>",
        "#g { grid-column: 2; grid-row: 1 / 2 }",
    );
    assert!(s.get("p").grid_column.is_auto());
    assert!(s.get("p").grid_row.is_auto());
}

#[test]
fn hostile_grid_templates_are_bounded() {
    let html: String = (0..300)
        .map(|i| format!("<div style=\"font-size: {}px\">x</div>", 10 + i % 7))
        .collect();
    let s = style(
        &format!("<main id=m>{html}</main>"),
        "* { display: grid; grid-template-columns: repeat(100000000, 1em) [a] 1fr;
             grid-template-rows: repeat(5000, 1px) }",
    );
    let m = s.get("m");
    let cols = m.grid_template_columns.as_ref().unwrap();
    assert_eq!(cols.tracks.len(), MAX_GRID_TRACKS);
    assert_eq!(m.grid_columns as usize, MAX_GRID_TRACKS);
    let rows = m.grid_template_rows.as_ref().unwrap();
    assert_eq!(rows.tracks.len(), MAX_GRID_TRACKS);
    // Same font size → the same computed template.
    let divs = s.doc.get_elements_by_tag_name("div");
    let first = s.map.get(divs[0]).unwrap().grid_template_columns.clone();
    let eighth = s.map.get(divs[7]).unwrap().grid_template_columns.clone();
    assert!(std::sync::Arc::ptr_eq(
        first.as_ref().unwrap(),
        eighth.as_ref().unwrap()
    ));
}
