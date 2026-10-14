//! Inline formatting: line breaking, alignment, white-space, line height,
//! vertical-align, inline box fragments and atomic inlines.

mod common;

use common::*;
use xbro_layout::{Fragment, LineBox, TextFragment};

const BASE: &str = "<style>body{margin:0;font:16px/20px sans-serif}p{margin:0}</style>";

fn page(body: &str) -> String {
    format!("<!doctype html>{BASE}<body>{body}</body>")
}

fn texts(line: &LineBox) -> Vec<&TextFragment> {
    line.fragments
        .iter()
        .filter_map(|f| match f {
            Fragment::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

/// Right edge of the last text fragment of a line.
fn text_right(line: &LineBox) -> f32 {
    texts(line).last().map_or(0.0, |t| t.rect.right())
}

#[test]
fn greedy_wrapping_respects_width() {
    let Some(l) = lay(
        &page("<p id=p style='width:120px'>aaa bbb ccc ddd eee fff ggg hhh iii jjj</p>"),
        400.0,
    ) else {
        return;
    };
    let p = l.box_id("p");
    assert!(p.lines.len() >= 3, "{}", l.dump());
    for line in &p.lines {
        assert!(text_right(line) <= 120.0 + 0.01, "{}", l.dump());
        assert_eq!(line.rect.height, 20.0);
        assert_eq!(line.rect.width, 120.0);
    }
    // Every word appears exactly once, lines start without spaces.
    let all: Vec<String> = l.line_texts("p");
    assert_eq!(all.join(" "), "aaa bbb ccc ddd eee fff ggg hhh iii jjj");
    for t in &all {
        assert!(!t.starts_with(' ') && !t.ends_with(' '), "{t:?}");
    }
    assert_eq!(p.border_box().height, 20.0 * p.lines.len() as f32);
    // Consecutive lines stack.
    for w in p.lines.windows(2) {
        assert_eq!(w[1].rect.y, w[0].rect.bottom());
    }
}

#[test]
fn long_word_overflows_unless_break_anywhere() {
    let Some(l) = lay(
        &page(
            "<p id=a style='width:50px'>xxxxxxxxxxxxxxxxxxxx yy</p>\
             <p id=b style='width:50px;overflow-wrap:anywhere'>xxxxxxxxxxxxxxxxxxxx</p>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.line_texts("a"), vec!["xxxxxxxxxxxxxxxxxxxx", "yy"]);
    let b = l.line_texts("b");
    assert!(b.len() >= 3, "{b:?}");
    assert_eq!(b.concat(), "xxxxxxxxxxxxxxxxxxxx");
    for line in &l.box_id("b").lines {
        assert!(text_right(line) <= 50.01);
    }
}

#[test]
fn break_opportunities_hyphen_shy_zwsp_wbr_cjk() {
    let Some(l) = lay(
        &page(
            "<p id=a style='width:60px'>aaaa-bbbb-cccc</p>\
             <p id=b style='width:60px'>aaaa\u{AD}bbbb\u{AD}cccc</p>\
             <p id=c style='width:60px'>aaaa\u{200B}bbbb\u{200B}cccc</p>\
             <p id=d style='width:60px'>aaaa<wbr>bbbb<wbr>cccc</p>\
             <p id=e style='width:40px'>日本語日本語</p>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.line_texts("a"), vec!["aaaa-", "bbbb-", "cccc"]);
    // Soft hyphens are invisible unless the line breaks there.
    assert_eq!(l.line_texts("b"), vec!["aaaa-", "bbbb-", "cccc"]);
    assert_eq!(l.line_texts("c"), vec!["aaaa", "bbbb", "cccc"]);
    assert_eq!(l.line_texts("d"), vec!["aaaa", "bbbb", "cccc"]);
    assert!(l.line_texts("e").len() >= 2);
    let Some(l) = lay(&page("<p id=b>aa\u{AD}bb</p>"), 400.0) else {
        return;
    };
    assert_eq!(l.line_texts("b"), vec!["aabb"]);
}

#[test]
fn text_align_left_right_center() {
    let Some(l) = lay(
        &page(
            "<p id=l style='width:200px'>word</p>\
             <p id=r style='width:200px;text-align:right'>word</p>\
             <p id=c style='width:200px;text-align:center'>word</p>",
        ),
        400.0,
    ) else {
        return;
    };
    let w = texts(&l.box_id("l").lines[0])[0].rect.width;
    assert_eq!(texts(&l.box_id("l").lines[0])[0].rect.x, 0.0);
    close(texts(&l.box_id("r").lines[0])[0].rect.x, 200.0 - w, 0.01);
    close(
        texts(&l.box_id("c").lines[0])[0].rect.x,
        (200.0 - w) / 2.0,
        0.01,
    );
}

#[test]
fn justify_stretches_all_lines_but_the_last() {
    let Some(l) = lay(
        &page("<p id=p style='width:150px;text-align:justify'>aa bb cc dd ee ff gg hh ii jj kk ll mm<br>nn oo</p>"),
        400.0,
    ) else {
        return;
    };
    let p = l.box_id("p");
    let n = p.lines.len();
    assert!(n >= 3, "{}", l.dump());
    for line in &p.lines[..n - 2] {
        close(text_right(line), 150.0, 0.05);
        // Glyph positions follow the stretched spaces.
        let t = texts(line)[0];
        let last = t.glyphs.last().unwrap();
        close(t.rect.x + last.x + last.advance, text_right(line), 0.05);
    }
    // The line ending with <br> and the last line are not justified.
    assert!(text_right(&p.lines[n - 2]) < 149.0);
    assert!(text_right(&p.lines[n - 1]) < 100.0);
}

#[test]
fn text_indent_on_first_line_only() {
    let Some(l) = lay(
        &page("<p id=p style='width:100px;text-indent:30px'>aaa bbb ccc ddd eee fff</p>"),
        400.0,
    ) else {
        return;
    };
    let p = l.box_id("p");
    assert_eq!(texts(&p.lines[0])[0].rect.x, 30.0);
    assert_eq!(texts(&p.lines[1])[0].rect.x, 0.0);
}

#[test]
fn white_space_modes() {
    let Some(l) = lay(
        &page(
            "<p id=n>  a \n\t b  </p>\
             <pre id=pre style='margin:0'>a  b\n\tc</pre>\
             <p id=nw style='white-space:nowrap;width:30px'>aaa bbb ccc</p>\
             <p id=pl style='white-space:pre-line'>  a   b  \n  c</p>\
             <p id=pw style='white-space:pre-wrap;width:60px'>aaa   bbb</p>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.line_texts("n"), vec!["a b"]);
    assert_eq!(l.line_texts("pre"), vec!["a  b", "c"]);
    // Tab stops: 8 spaces.
    let pre = l.box_id("pre");
    let space = texts(&pre.lines[0])[0].rect.width / 4.0;
    close(texts(&pre.lines[1])[0].rect.x, 8.0 * space, 0.05);
    assert_eq!(l.line_texts("nw"), vec!["aaa bbb ccc"]);
    assert_eq!(l.line_texts("pl"), vec!["a b", "c"]);
    assert_eq!(l.line_texts("pw"), vec!["aaa", "bbb"]);
}

#[test]
fn text_transform() {
    let Some(l) = lay(
        &page(
            "<p id=u style='text-transform:uppercase'>straße</p>\
             <p id=c style='text-transform:capitalize'>hello <b>wo</b>rld it's</p>\
             <p id=lo style='text-transform:lowercase'>ABC</p>",
        ),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.line_texts("u"), vec!["STRASSE"]);
    assert_eq!(l.line_texts("c"), vec!["Hello World It's"]);
    assert_eq!(l.line_texts("lo"), vec!["abc"]);
}

#[test]
fn br_forces_breaks_and_empty_lines() {
    let Some(l) = lay(
        &page("<p id=a>one<br>two</p><p id=b>x<br><br>y</p><p id=c>z<br></p><div id=d><br></div>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.line_texts("a"), vec!["one", "two"]);
    assert_eq!(l.line_texts("b"), vec!["x", "", "y"]);
    assert_eq!(l.rect("b").height, 60.0);
    // A trailing <br> does not add a line; a lone <br> makes one.
    assert_eq!(l.rect("c").height, 20.0);
    assert_eq!(l.rect("d").height, 20.0);
}

#[test]
fn collapsible_space_across_inline_boundaries() {
    let Some(l) = lay(&page("<p id=p>a <b> b</b> <i> </i> c</p>"), 400.0) else {
        return;
    };
    assert_eq!(l.line_texts("p"), vec!["a b c"]);
}

#[test]
fn whitespace_only_block_has_zero_height() {
    let Some(l) = lay(&page("<div id=d> \n <span> </span> </div>"), 400.0) else {
        return;
    };
    assert_eq!(l.rect("d").height, 0.0);
    assert!(l.box_id("d").lines.is_empty());
}

#[test]
fn mixed_font_sizes_grow_the_line() {
    let Some(l) = lay(
        &page("<p id=p style='line-height:normal'>a<span style='font-size:40px'>B</span>c</p><p id=q>a<span style='font-size:40px'>B</span>c</p>"),
        400.0,
    ) else {
        return;
    };
    let p = &l.box_id("p").lines[0];
    let ts = texts(p);
    // All runs share the baseline; the line holds the big glyph's extent.
    assert_eq!(ts[0].baseline, ts[1].baseline);
    assert!(p.rect.height > 40.0, "{}", l.dump());
    assert!(ts[1].rect.y >= p.rect.y - 0.01);
    assert!(ts[1].rect.bottom() <= p.rect.bottom() + 0.01);
    // With a fixed line-height inherited in px, the big span's half-leading
    // is negative and the line is smaller than its glyphs.
    let q = &l.box_id("q").lines[0];
    assert!(
        q.rect.height < 46.0 && q.rect.height > 20.0,
        "{}",
        q.rect.height
    );
}

#[test]
fn vertical_align_values() {
    let Some(l) = lay(
        &page(
            "<p id=p>x<span id=sup style='vertical-align:super'>s</span>\
             <span id=sub style='vertical-align:sub'>s</span>\
             <span id=len style='vertical-align:5px'>s</span>\
             <span id=tt style='vertical-align:text-top;font-size:8px;line-height:normal'>s</span>\
             <span id=tb style='vertical-align:text-bottom;font-size:8px;line-height:normal'>s</span></p>\
             <p id=q>x<img id=top style='vertical-align:top' width=10 height=50>\
             <img id=bot style='vertical-align:bottom' width=10 height=10>\
             <img id=mid style='vertical-align:middle' width=10 height=10></p>",
        ),
        400.0,
    ) else {
        return;
    };
    let p = &l.box_id("p").lines[0];
    let base = p.baseline;
    let bl = |id: &str| {
        texts(p)
            .into_iter()
            .find(|t| l.doc.parent(t.node) == Some(l.id(id)))
            .map(|t| (t.baseline, t.rect))
            .unwrap()
    };
    close(bl("sup").0, base - 0.34 * 16.0, 0.01);
    close(bl("sub").0, base + 0.2 * 16.0, 0.01);
    close(bl("len").0, base - 5.0, 0.01);
    let x = texts(p)[0].rect;
    // text-top: tops aligned; text-bottom: bottoms aligned.
    close(bl("tt").1.y, x.y, 0.01);
    close(bl("tb").1.bottom(), x.bottom(), 0.01);

    let q = &l.box_id("q").lines[0];
    let top = l.rect("top");
    let bot = l.rect("bot");
    assert_eq!(top.y, q.rect.y);
    assert_eq!(top.height, 50.0);
    assert_eq!(q.rect.height, 50.0);
    close(bot.bottom(), q.rect.bottom(), 0.01);
    let mid = l.rect("mid");
    let xh = 16.0 * 0.5;
    assert!(
        (mid.y + 5.0 - (q.baseline - xh / 2.0)).abs() < 2.0,
        "{mid:?} {}",
        q.baseline
    );
}

#[test]
fn inline_box_fragments_carry_padding_and_borders() {
    let Some(l) = lay(
        &page("<p id=p style='width:120px'>x <span id=s style='padding:0 5px;border:2px solid;margin:0 3px'>aaa bbb ccc ddd eee fff</span> y</p>"),
        400.0,
    ) else {
        return;
    };
    let p = l.box_id("p");
    let frags: Vec<_> = p
        .lines
        .iter()
        .flat_map(|line| line.fragments.iter())
        .filter_map(|f| match f {
            Fragment::InlineBox(ib) if ib.node == l.id("s") => Some(ib),
            _ => None,
        })
        .collect();
    assert!(frags.len() >= 2, "{}", l.dump());
    let first = frags[0];
    let last = frags[frags.len() - 1];
    assert_eq!(first.border.left, 2.0);
    assert_eq!(first.padding.left, 5.0);
    assert_eq!(first.border.right, 0.0);
    assert_eq!(last.border.right, 2.0);
    assert_eq!(last.padding.right, 5.0);
    assert_eq!(last.border.left, 0.0);
    // The fragment spans its text: left margin before it.
    let x_text = texts(&p.lines[0])[0].rect.right();
    close(first.rect.x, x_text + 3.0, 0.01);
    // Vertical extent: ascent+descent plus border (no padding vertically).
    let t = texts(&p.lines[0])[1];
    close(first.rect.y, t.rect.y - 2.0, 0.01);
    close(first.rect.height, t.rect.height + 4.0, 0.01);
    // Inline box fragments precede their text.
    let line0 = &p.lines[0].fragments;
    let ib_pos = line0
        .iter()
        .position(|f| matches!(f, Fragment::InlineBox(_)))
        .unwrap();
    assert!(matches!(line0[ib_pos + 1], Fragment::Text(_)));
    // Horizontal border/padding/margin consume line space: text fits.
    for line in &p.lines {
        assert!(text_right(line) <= 120.01);
    }
}

#[test]
fn inline_block_shrinks_to_fit_and_uses_last_baseline() {
    let Some(l) = lay(
        &page(
            "<p id=p>x<span id=ib style='display:inline-block;padding:4px'>one<br>two</span>\
             <span id=e style='display:inline-block;width:30px;height:30px'></span>\
             <span id=o style='display:inline-block;overflow:hidden'>z</span></p>",
        ),
        400.0,
    ) else {
        return;
    };
    let line = &l.box_id("p").lines[0];
    let ib = l.box_id("ib");
    // Shrink-to-fit width: the widest line.
    let widest = ib.lines.iter().map(text_right).fold(0.0f32, f32::max) - ib.content.x;
    close(ib.content.width, widest, 0.01);
    // Baseline of the inline-block = baseline of its last line.
    close(ib.lines[1].baseline, line.baseline, 0.01);
    // Empty inline-block: bottom margin edge on the baseline.
    close(l.rect("e").bottom(), line.baseline, 0.01);
    // overflow != visible: bottom margin edge on the baseline.
    close(l.rect("o").bottom(), line.baseline, 0.01);
}

#[test]
fn inline_block_percentage_and_explicit_width() {
    let Some(l) = lay(
        &page("<div style='width:300px'><span id=a style='display:inline-block;width:50%'>x</span><span id=b style='display:inline-block;width:40px;margin-left:10px'>y</span></div>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.rect("a").width, 150.0);
    assert_eq!(l.rect("b").width, 40.0);
    assert_eq!(l.rect("b").x, 160.0);
}

#[test]
fn atomic_inlines_wrap_as_units() {
    let Some(l) = lay(
        &page("<p id=p style='width:100px'><span style='display:inline-block;width:60px;height:10px'></span><span id=b style='display:inline-block;width:60px;height:10px'></span></p>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.box_id("p").lines.len(), 2);
    assert_eq!(l.rect("b").x, 0.0);
}

#[test]
fn nowrap_never_wraps_atomics_or_text() {
    let Some(l) = lay(
        &page("<p id=p style='width:50px;white-space:nowrap'>aaa <span style='display:inline-block;width:60px'>x</span> bbb</p>"),
        400.0,
    ) else {
        return;
    };
    assert_eq!(l.box_id("p").lines.len(), 1);
}

#[test]
fn letter_and_word_spacing_widen_text() {
    let Some(l) = lay(
        &page("<p id=a>ab cd</p><p id=b style='letter-spacing:2px'>ab cd</p><p id=c style='word-spacing:10px'>ab cd</p>"),
        400.0,
    ) else {
        return;
    };
    let w = |id: &str| text_right(&l.box_id(id).lines[0]);
    close(w("b"), w("a") + 5.0 * 2.0, 0.05);
    close(w("c"), w("a") + 10.0, 0.05);
}

#[test]
fn text_fragments_have_glyphs_relative_to_rect() {
    let Some(l) = lay(&page("<p id=p>Hello world</p>"), 400.0) else {
        return;
    };
    let t = texts(&l.box_id("p").lines[0])[0];
    assert_eq!(t.text, "Hello world");
    assert_eq!(t.glyphs.len(), 11);
    assert_eq!(t.glyphs[0].x, 0.0);
    let last = t.glyphs.last().unwrap();
    close(last.x + last.advance, t.rect.width, 0.5);
    assert_eq!(t.font_size, 16.0);
    assert_eq!(t.node, l.doc.children(l.id("p"))[0]);
}

// ------------------------------------------- overflow-wrap and word-break

const URL: &str = "https://example.com/a/very/long/path/that/never/ends/index.html";

#[test]
fn break_word_does_not_shrink_min_content_in_tables() {
    let Some(l) = lay(
        &page(&format!(
            "<div style='width:200px'>\
               <table id=bw style='border-spacing:0'><tr><td id=bwc style='overflow-wrap:break-word;padding:0'>{URL}</td></tr></table>\
               <table id=any style='border-spacing:0'><tr><td id=anyc style='overflow-wrap:anywhere;padding:0'>{URL}</td></tr></table>\
               <table id=ba style='border-spacing:0'><tr><td id=bac style='word-break:break-all;padding:0'>{URL}</td></tr></table>\
               <table id=ww style='border-spacing:0'><tr><td id=wwc style='word-wrap:break-word;padding:0'>{URL}</td></tr></table>\
             </div>"
        )),
        800.0,
    ) else {
        return;
    };
    // break-word: the column keeps the URL's width (the table overflows
    // its 200px container) and the URL stays on one line.
    let url_w = l.rect("bwc").width;
    assert!(url_w > 300.0, "{url_w}");
    assert_eq!(l.line_texts("bwc").len(), 1);
    assert_eq!(l.rect("bw").width, url_w);
    assert_eq!(l.rect("ww").width, url_w);
    // anywhere / break-all: the table fits and the URL wraps.
    for (t, c) in [("any", "anyc"), ("ba", "bac")] {
        assert_eq!(l.rect(t).width, 200.0, "{t}");
        let lines = l.line_texts(c);
        assert!(lines.len() >= 2, "{t}: {lines:?}");
        assert_eq!(lines.concat(), URL);
    }
}

#[test]
fn break_word_does_not_shrink_flex_items() {
    let Some(l) = lay(
        &page(&format!(
            "<div style='display:flex;width:200px'><div id=bw style='overflow-wrap:break-word'>{URL}</div></div>\
             <div style='display:flex;width:200px'><div id=any style='overflow-wrap:anywhere'>{URL}</div></div>\
             <div style='width:200px;overflow-wrap:break-word'><p id=block style='margin:0'>{URL}</p></div>"
        )),
        800.0,
    ) else {
        return;
    };
    // The flex item's automatic minimum size is its min-content width:
    // the whole URL with break-word (the item overflows), the widest
    // character with anywhere (the item shrinks and the URL wraps).
    let bw = l.rect("bw");
    assert!(bw.width > 300.0, "{bw:?}");
    assert_eq!(l.line_texts("bw").len(), 1);
    let any = l.rect("any");
    assert_eq!(any.width, 200.0);
    assert!(l.line_texts("any").len() >= 2);
    // In a block of definite width, break-word still breaks the URL.
    let lines = l.line_texts("block");
    assert!(lines.len() >= 2, "{lines:?}");
    for line in &l.box_id("block").lines {
        assert!(text_right(line) <= 200.01);
    }
}

#[test]
fn break_all_breaks_words_wherever_the_line_is_full() {
    let Some(l) = lay(
        &page(
            "<p id=bw style='width:100px;overflow-wrap:break-word'>aaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbb</p>\
             <p id=ba style='width:100px;word-break:break-all'>aaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbb</p>",
        ),
        400.0,
    ) else {
        return;
    };
    // break-word: the long word moves to its own line first.
    assert_eq!(l.line_texts("bw")[0], "aaaa");
    // break-all: the first line is filled with the start of the word.
    let ba = l.line_texts("ba");
    assert!(ba[0].starts_with("aaaa b"), "{ba:?}");
    assert_eq!(
        ba.concat().replace(' ', ""),
        "aaaabbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
}
