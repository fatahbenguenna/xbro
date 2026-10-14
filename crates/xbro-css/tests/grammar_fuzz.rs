//! Grammar-aware fuzzing: random but well-formed-looking selectors, media
//! queries and colors (the token-soup tests rarely get past the first
//! production), with hostile numbers: huge, negative, NaN-ish, `calc()`.
//! Nothing may panic, results must stay in range, and parsing stays fast.

mod support;

use std::time::{Duration, Instant};

use support::{Dom, Rng};
use xbro_css::{parse_selector_list, parse_stylesheet, Color, MediaContext, MediaQueryList};

const NUMBERS: &[&str] = &[
    "0",
    "-0",
    "1",
    "-1",
    "0.5",
    ".5",
    "+3",
    "1e3",
    "-1e3",
    "1e-7",
    "255",
    "256",
    "-255",
    "2147483647",
    "-2147483648",
    "4294967296",
    "1e38",
    "3.4e38",
    "1e39",
    "-1e39",
    "1e308",
    "1e400",
    "99999999999999999999",
    "NaN",
    "infinity",
    "-infinity",
    "calc(1/0)",
    "calc(infinity)",
    "calc(-infinity)",
    "calc(NaN)",
    "calc(0/0)",
    "calc(1e38 * 1e38)",
    "calc(50% + 1)",
    "none",
    "",
];

fn number(rng: &mut Rng) -> String {
    let n = rng.pick(NUMBERS).to_string();
    match rng.below(5) {
        0 => format!("{n}%"),
        1 => format!(
            "{n}{}",
            rng.pick(&["deg", "rad", "grad", "turn", "px", "e"])
        ),
        _ => n,
    }
}

fn color(rng: &mut Rng, depth: usize) -> String {
    let f = *rng.pick(&[
        "rgb",
        "rgba",
        "hsl",
        "hsla",
        "hwb",
        "lab",
        "lch",
        "oklab",
        "oklch",
        "color",
        "color-mix",
        "light-dark",
        "RGB",
        "named",
        "hex",
        "device-cmyk",
    ]);
    match f {
        "named" => rng
            .pick(&[
                "red",
                "transparent",
                "currentcolor",
                "Canvas",
                "nope",
                "ReD",
                "rebeccapurple",
            ])
            .to_string(),
        "hex" => {
            let len = rng.below(10);
            let digits: String = (0..len)
                .map(|_| *rng.pick(&['0', '9', 'a', 'F', 'g', 'é']))
                .collect();
            format!("#{digits}")
        }
        "color-mix" if depth < 4 => format!(
            "color-mix(in {}, {} {}, {} {})",
            rng.pick(&["srgb", "oklab", "hsl longer hue", "lch", "bogus", "xyz"]),
            color(rng, depth + 1),
            number(rng),
            color(rng, depth + 1),
            number(rng)
        ),
        "light-dark" if depth < 4 => {
            format!(
                "light-dark({}, {})",
                color(rng, depth + 1),
                color(rng, depth + 1)
            )
        }
        "color" => format!(
            "color({} {} {} {}{})",
            rng.pick(&[
                "srgb",
                "display-p3",
                "xyz",
                "rec2020",
                "a98-rgb",
                "prophoto-rgb",
                "nope"
            ]),
            number(rng),
            number(rng),
            number(rng),
            if rng.chance(50) {
                format!(" / {}", number(rng))
            } else {
                String::new()
            }
        ),
        _ => {
            let from = if depth < 3 && rng.chance(15) {
                format!("from {} ", color(rng, depth + 1))
            } else {
                String::new()
            };
            let n = rng.below(5);
            let args: Vec<String> = (0..n).map(|_| number(rng)).collect();
            let sep = if rng.chance(40) { ", " } else { " " };
            let alpha = if rng.chance(30) {
                format!(" / {}", number(rng))
            } else {
                String::new()
            };
            format!("{f}({from}{}{alpha})", args.join(sep))
        }
    }
}

fn length(rng: &mut Rng) -> String {
    format!(
        "{}{}",
        rng.pick(NUMBERS),
        rng.pick(&["px", "em", "rem", "vw", "dpi", "dppx", "x", "", "%", "cm", "q"])
    )
}

fn media_query(rng: &mut Rng, depth: usize) -> String {
    let feature = |rng: &mut Rng| -> String {
        let name = *rng.pick(&[
            "width",
            "min-width",
            "max-width",
            "height",
            "min-height",
            "max-height",
            "aspect-ratio",
            "min-aspect-ratio",
            "resolution",
            "min-resolution",
            "-webkit-min-device-pixel-ratio",
            "orientation",
            "prefers-color-scheme",
            "hover",
            "color",
            "unknown-feature",
        ]);
        match rng.below(5) {
            0 => format!("({name})"),
            1 => format!(
                "({} {} {name})",
                length(rng),
                rng.pick(&["<", "<=", ">", ">=", "=", "=>"])
            ),
            2 => format!("({} < {name} <= {})", length(rng), length(rng)),
            3 => format!("({name}: {}/{})", rng.pick(NUMBERS), rng.pick(NUMBERS)),
            _ => format!(
                "({name}: {})",
                if rng.chance(30) {
                    rng.pick(&["portrait", "landscape", "dark", "light", "none", "hover"])
                        .to_string()
                } else {
                    length(rng)
                }
            ),
        }
    };
    let mut q = String::new();
    if rng.chance(20) {
        q.push_str(rng.pick(&["not ", "only ", "NOT "]));
    }
    if rng.chance(40) {
        q.push_str(rng.pick(&["screen", "print", "all", "speech", "tv", "bogus"]));
        q.push_str(" and ");
    }
    q.push_str(&feature(rng));
    for _ in 0..rng.below(3) {
        q.push_str(rng.pick(&[" and ", " or ", " , ", " "]));
        if depth < 4 && rng.chance(25) {
            q.push_str(&format!("(not {})", media_query(rng, depth + 1)));
        } else {
            q.push_str(&feature(rng));
        }
    }
    q
}

fn nth(rng: &mut Rng) -> String {
    let a = *rng.pick(&[
        "",
        "-",
        "+",
        "2",
        "-2",
        "2147483647",
        "-2147483648",
        "99999999999",
        "0",
    ]);
    let b = *rng.pick(&[
        "",
        "+1",
        "-1",
        "+ 3",
        "- 2147483648",
        "+2147483647",
        "+99999999999",
    ]);
    match rng.below(4) {
        0 => rng.pick(&["odd", "even", "n", "-n", "3", "-1"]).to_string(),
        1 => format!("{a}n{b}"),
        2 => format!("{a}n{b} of {}", compound(rng, 2)),
        _ => rng.pick(NUMBERS).to_string(),
    }
}

fn compound(rng: &mut Rng, depth: usize) -> String {
    let mut s = rng
        .pick(&["", "", "a", "b", "c", "*", "svg|a", "*|*"])
        .to_string();
    for _ in 0..rng.below(4) {
        let simple = match rng.below(if depth < 4 { 14 } else { 9 }) {
            0 | 1 => ".x".to_string(),
            2 => "#id".to_string(),
            3 => format!(
                "[{}]",
                rng.pick(&["x", "x=y", "x~=y i", "x|=\"\"", "x^=y s", "x$=", "x*=日"])
            ),
            4 => format!(":nth-child({})", nth(rng)),
            5 => format!(":nth-last-of-type({})", nth(rng)),
            6 => rng
                .pick(&[
                    ":first-child",
                    ":empty",
                    ":root",
                    ":hover",
                    ":lang(en)",
                    ":dir(ltr)",
                    ":nope",
                ])
                .to_string(),
            7 => rng
                .pick(&["::before", "::after", "::marker", ":after", "::nope"])
                .to_string(),
            8 => ".y".to_string(),
            9 => format!(":not({})", list(rng, depth + 1)),
            10 => format!(":is({})", list(rng, depth + 1)),
            11 => format!(":where({})", list(rng, depth + 1)),
            12 => format!(
                ":has({}{})",
                rng.pick(&["", "> ", "+ ", "~ "]),
                complex(rng, depth + 1)
            ),
            _ => "&".to_string(),
        };
        s.push_str(&simple);
    }
    if s.is_empty() {
        s.push('*');
    }
    s
}

fn complex(rng: &mut Rng, depth: usize) -> String {
    let mut s = compound(rng, depth);
    for _ in 0..rng.below(4) {
        s.push_str(rng.pick(&[" ", " > ", " + ", " ~ ", " >> ", " || "]));
        s.push_str(&compound(rng, depth));
    }
    s
}

fn list(rng: &mut Rng, depth: usize) -> String {
    let n = 1 + rng.below(3);
    (0..n)
        .map(|_| complex(rng, depth))
        .collect::<Vec<_>>()
        .join(", ")
}

fn random_dom(rng: &mut Rng) -> Dom {
    let mut dom = Dom::default();
    dom.add(None, "a.x");
    for _ in 1..2 + rng.below(30) {
        let len = dom.nodes.len();
        let parent = len - 1 - rng.below(len.min(4));
        let tag = *rng.pick(&["a", "b", "c"]);
        let class = *rng.pick(&["", ".x", ".y", ".x.y", "#id"]);
        dom.add(Some(parent), &format!("{tag}{class}"));
    }
    dom
}

#[test]
fn random_colors_parse_without_panicking() {
    let mut rng = Rng::new(0xc010);
    let mut parsed = 0;
    for _ in 0..20_000 {
        let text = color(&mut rng, 0);
        if Color::parse_str(&text).is_some() {
            parsed += 1;
        }
        let _ = Color::parse_legacy(&text);
    }
    // The generator mostly produces valid colors.
    assert!(parsed > 1_000, "{parsed} colors parsed");
}

#[test]
fn random_media_queries_evaluate_without_panicking() {
    let contexts = [
        MediaContext::default(),
        MediaContext {
            width: 0.0,
            height: 0.0,
            resolution: 0.0,
            dark: true,
        },
        MediaContext {
            width: f32::NAN,
            height: f32::INFINITY,
            resolution: f32::NAN,
            dark: false,
        },
        MediaContext {
            width: 1e9,
            height: -5.0,
            resolution: 3.0,
            dark: false,
        },
    ];
    let mut rng = Rng::new(0x3ed1a);
    let mut matched = [0usize; 2];
    for _ in 0..20_000 {
        let n = 1 + rng.below(3);
        let text = (0..n)
            .map(|_| media_query(&mut rng, 0))
            .collect::<Vec<_>>()
            .join(", ");
        let list = MediaQueryList::parse_str(&text);
        for ctx in &contexts {
            matched[usize::from(list.matches(ctx))] += 1;
        }
    }
    assert!(matched[0] > 1000 && matched[1] > 1000, "{matched:?}");
}

#[test]
fn random_selectors_parse_and_match_without_panicking() {
    let mut rng = Rng::new(0x5e1ec7);
    let mut parsed = 0;
    for _ in 0..4_000 {
        let text = list(&mut rng, 0);
        let Some(selectors) = parse_selector_list(&text) else {
            continue;
        };
        parsed += 1;
        let dom = random_dom(&mut rng);
        for selector in &selectors {
            let _ = selector.specificity();
            for index in 0..dom.nodes.len() {
                let _ = selector.matches(&dom.el(index));
            }
        }
    }
    assert!(parsed > 500, "{parsed} selector lists parsed");
}

#[test]
fn random_rules_with_grammar_parts_parse_quickly() {
    let mut rng = Rng::new(0x5ee7);
    let mut css = String::new();
    for _ in 0..3_000 {
        match rng.below(4) {
            0 => css.push_str(&format!(
                "@media {} {{ {} {{ color: {} }} }}\n",
                media_query(&mut rng, 0),
                list(&mut rng, 1),
                color(&mut rng, 0)
            )),
            1 => css.push_str(&format!(
                "{} {{ background: {}; & {} {{ border-color: {} }} }}\n",
                list(&mut rng, 1),
                color(&mut rng, 0),
                compound(&mut rng, 2),
                color(&mut rng, 0)
            )),
            2 => css.push_str(&format!(
                "@supports selector({}) {{ a {{ b: c }} }}\n",
                complex(&mut rng, 1)
            )),
            _ => css.push_str(&format!(
                "{} {{ width: calc({} * 1px) }}\n",
                list(&mut rng, 0),
                rng.pick(NUMBERS)
            )),
        }
    }
    let start = Instant::now();
    let sheet = parse_stylesheet(&css);
    assert!(!sheet.rules.is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}
