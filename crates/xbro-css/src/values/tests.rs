use super::*;
use crate::parser::parse_component_values;

fn first(css: &str) -> ComponentValue {
    parse_component_values(css)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{css:?} is empty"))
}

#[test]
fn unit_names() {
    let table = [
        ("px", Some(LengthUnit::Px)),
        ("PX", Some(LengthUnit::Px)),
        ("em", Some(LengthUnit::Em)),
        ("rem", Some(LengthUnit::Rem)),
        ("ex", Some(LengthUnit::Ex)),
        ("ch", Some(LengthUnit::Ch)),
        ("pt", Some(LengthUnit::Pt)),
        ("pc", Some(LengthUnit::Pc)),
        ("in", Some(LengthUnit::In)),
        ("cm", Some(LengthUnit::Cm)),
        ("mm", Some(LengthUnit::Mm)),
        ("q", Some(LengthUnit::Q)),
        ("Q", Some(LengthUnit::Q)),
        ("vw", Some(LengthUnit::Vw)),
        ("vh", Some(LengthUnit::Vh)),
        ("vmin", Some(LengthUnit::Vmin)),
        ("vmax", Some(LengthUnit::Vmax)),
        ("%", Some(LengthUnit::Percent)),
        ("dvh", Some(LengthUnit::Vh)),
        ("svh", Some(LengthUnit::Vh)),
        ("lvh", Some(LengthUnit::Vh)),
        ("dvw", Some(LengthUnit::Vw)),
        ("svw", Some(LengthUnit::Vw)),
        ("lvw", Some(LengthUnit::Vw)),
        ("dvmin", Some(LengthUnit::Vmin)),
        ("svmin", Some(LengthUnit::Vmin)),
        ("lvmax", Some(LengthUnit::Vmax)),
        ("vi", Some(LengthUnit::Vw)),
        ("vb", Some(LengthUnit::Vh)),
        ("cqw", Some(LengthUnit::Vw)),
        ("cqi", Some(LengthUnit::Vw)),
        ("cqh", Some(LengthUnit::Vh)),
        ("cqb", Some(LengthUnit::Vh)),
        ("cqmin", Some(LengthUnit::Vmin)),
        ("cqmax", Some(LengthUnit::Vmax)),
        ("ic", Some(LengthUnit::Em)),
        ("rex", Some(LengthUnit::Ex)),
        ("rch", Some(LengthUnit::Ch)),
        ("x", None),
        ("dppx", None),
        ("dpi", None),
        ("deg", None),
        ("s", None),
        ("", None),
        ("pxx", None),
        ("verylongunitname", None),
    ];
    for (name, expected) in table {
        assert_eq!(LengthUnit::from_name(name), expected, "{name:?}");
    }
    assert!(LengthUnit::Cm.is_absolute());
    assert!(!LengthUnit::Em.is_absolute());
    assert_eq!(LengthUnit::In.absolute_px(), Some(96.0));
    assert_eq!(LengthUnit::Pt.absolute_px(), Some(96.0 / 72.0));
    assert_eq!(LengthUnit::Percent.absolute_px(), None);
}

#[test]
fn length_parse() {
    assert_eq!(
        Length::parse(&first("10px")),
        Some(Length {
            value: 10.0,
            unit: LengthUnit::Px
        })
    );
    assert_eq!(
        Length::parse(&first("-1.5EM")),
        Some(Length {
            value: -1.5,
            unit: LengthUnit::Em
        })
    );
    assert_eq!(
        Length::parse(&first("50%")),
        Some(Length {
            value: 50.0,
            unit: LengthUnit::Percent
        })
    );
    assert_eq!(
        Length::parse(&first("100dvh")),
        Some(Length {
            value: 100.0,
            unit: LengthUnit::Vh
        })
    );
    assert_eq!(Length::parse(&first("0")), Some(Length::px(0.0)));
    assert_eq!(Length::parse(&first("0.0")), Some(Length::px(0.0)));
    assert_eq!(Length::parse(&first("-0")), Some(Length::px(0.0)));
    assert_eq!(Length::parse(&first("5")), None);
    assert_eq!(Length::parse(&first("2x")), None);
    assert_eq!(Length::parse(&first("10deg")), None);
    assert_eq!(Length::parse(&first("auto")), None);
    assert_eq!(Length::parse(&first("calc(1px)")), None);
    assert_eq!(Length::parse(&first("\"10px\"")), None);
}

#[test]
fn url_helper() {
    assert_eq!(url(&first("url(a.png)")).as_deref(), Some("a.png"));
    assert_eq!(url(&first("URL( 'b c.png' )")).as_deref(), Some("b c.png"));
    assert_eq!(
        url(&first("url(\"d.png\" crossorigin)")).as_deref(),
        Some("d.png")
    );
    assert_eq!(url(&first("src(\"e.svg\")")).as_deref(), Some("e.svg"));
    assert_eq!(url(&first("url()")).as_deref(), Some(""));
    assert_eq!(url(&first("url(a b)")), None);
    assert_eq!(url(&first("url(\"x\"")).as_deref(), Some("x"));
    assert_eq!(url(&first("\"a.png\"")), None);
    assert_eq!(url(&first("image(a.png)")), None);
    assert_eq!(url(&first("src(var(--x))")), None);
}

#[test]
fn splitting_helpers() {
    let values = parse_component_values("  1px solid  rgb(0, 0, 0)  ");
    let trimmed = trim(&values);
    assert_eq!(trimmed.len(), 5);
    let parts = split_whitespace(&values);
    assert_eq!(parts.len(), 3);
    assert!(parts[2][0].is_function("rgb"));
    let values = parse_component_values("a , b c,, d");
    let parts = split_commas(&values);
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[1].len(), 3);
    assert!(parts[2].is_empty());
    assert!(trim(&parse_component_values("   ")).is_empty());
    assert!(trim(&[]).is_empty());
}

#[test]
fn token_helpers() {
    assert_eq!(number(&first("1.5")), Some(1.5));
    assert_eq!(number(&first("1px")), None);
    assert_eq!(string(&first("'s'")), Some("s"));
    assert_eq!(string(&first("s")), None);
    assert_eq!(ident(&first("auto")), Some("auto"));
    assert_eq!(dimension(&first("2EM")), Some((2.0, "em")));
    assert_eq!(dimension(&first("2")), None);
    assert_eq!(percentage(&first("25%")), Some(25.0));
    assert_eq!(percentage(&first("25")), None);
    assert_eq!(integer(&first("7")), Some(7));
    assert_eq!(integer(&first("-7")), Some(-7));
    assert_eq!(integer(&first("7.0")), None);
    assert_eq!(integer(&first("1e3")), None);
    assert_eq!(integer(&first("99999999999")), Some(i32::MAX));
}
