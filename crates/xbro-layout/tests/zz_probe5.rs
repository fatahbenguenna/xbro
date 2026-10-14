mod common;
use common::*;

#[test]
fn t1_wide_rowspans() {
    let mut s = String::from("<table>");
    for _ in 0..60 {
        s.push_str("<tr>");
        for _ in 0..1000 {
            s.push_str("<td rowspan=2>x");
        }
        s.push_str("<tr><td>y");
    }
    s.push_str("</table>");
    let t = std::time::Instant::now();
    let Some(_l) = lay(&s, 400.0) else { return };
    eprintln!("T1 {:?}", t.elapsed());
}

#[test]
fn t2_many_rows_wide_cover() {
    let mut s = String::from("<table><tr><td colspan=1000 rowspan=0>x");
    for _ in 0..20000 {
        s.push_str("<tr><td>y");
    }
    s.push_str("</table>");
    let t = std::time::Instant::now();
    let Some(_l) = lay(&s, 400.0) else { return };
    eprintln!("T2 {:?}", t.elapsed());
}

#[test]
fn t3_button_centering() {
    let Some(l) = lay("<body style='margin:0;font:16px/20px sans-serif'><button id=b style='height:60px;padding:0;border:0'>OK</button><input id=i type=submit value=Go style='height:60px;padding:0;border:0'></body>", 400.0) else { return };
    let b = l.box_id("b");
    let i = l.box_id("i");
    eprintln!(
        "T3 button {:?} line {:?}; input {:?} line {:?}",
        b.border_box(),
        b.lines.first().map(|l| l.rect),
        i.border_box(),
        i.lines.first().map(|l| l.rect)
    );
}
