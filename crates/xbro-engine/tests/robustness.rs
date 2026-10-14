//! Hostile and random inputs: nothing may panic.

mod common;

use common::*;
use xbro_engine::{Point, Size};

/// Small deterministic PRNG (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn f32(&mut self, max: f32) -> f32 {
        (self.next() % 1_000_000) as f32 / 1_000_000.0 * max
    }
}

const PIECES: &[&str] = &[
    "<div>",
    "</div>",
    "<p>",
    "text ",
    "日本語 ",
    "<a href='#x'>",
    "<a href=javascript:x>",
    "</a>",
    "<form action=?a#b method=post>",
    "</form>",
    "<input name=q value='v'>",
    "<input type=checkbox name=c checked>",
    "<input type=radio name=r>",
    "<input type=image src=data:,>",
    "<input type=submit name=s>",
    "<input type=reset>",
    "<button>",
    "</button>",
    "<select name=s><option>1<option selected>2</select>",
    "<textarea name=t>",
    "</textarea>",
    "<label for=q>",
    "</label>",
    "<label>",
    "<details><summary>s</summary>",
    "</details>",
    "<img src='data:image/gif;base64,R0lGODlhAQABAAAAACw=' srcset=',, 2x,'>",
    "<img usemap=#m src=x.png><map name=m><area shape=poly coords='1,2,3' href=a></map>",
    "<picture><source srcset='a.webp 2x' media='(min-width: 0'>",
    "</picture>",
    "<svg viewBox='0 0 1 1'><use href='#nope'/><foreignObject><p>x</p></foreignObject>",
    "<svg><circle r=1 fill='url(#g)'/><linearGradient id=g/></svg>",
    "</svg>",
    "<style>a:hover{color:red} :is(p:hover) {} @import 'nope.css' print; p:focus-within{}</style>",
    "<style media='(max-width: 100px)'>p{display:none}</style>",
    "<link rel=stylesheet href='data:text/css,p%7Bcolor:blue%7D' media=screen>",
    "<base href=/base/>",
    "<meta http-equiv=refresh content='0;url=\"'>",
    "<table><tr><td>",
    "</td></tr></table>",
    "<ul><li>",
    "</li></ul>",
    "<span id=x>",
    "</span>",
    "<h1 id=top>",
    "</h1>",
    "<a name=n></a>",
    "<br>",
    "<hr>",
    "<div style='position:fixed;top:0'>fixed <input></div>",
    "<div style='background:url(\"bg.png\") ; list-style-image:url(li.png)'>",
    "<object data=x.svg type=image/svg+xml></object>",
    "<video poster=p.jpg></video>",
    "<input type=password value=secret>",
    "<p tabindex=0>",
    "<div tabindex=-3>",
    "&amp;&lt;&#x0;",
    "\u{0}",
    "<!-- c -->",
    "<template><img src=t.png></template>",
];

/// Loads with a `file:` base URL so relative subresources never hit the
/// network (DNS lookups of a fake host can stall for seconds).
fn load_page(html: &str) -> xbro_engine::Page {
    engine().load_html(html, fixture_url("robustness/page.html"), VIEWPORT)
}

fn random_html(rng: &mut Rng) -> String {
    let mut html = String::new();
    for _ in 0..rng.below(200) + 20 {
        html.push_str(PIECES[rng.below(PIECES.len())]);
    }
    html
}

#[test]
fn random_documents_and_interactions() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..40 {
        let html = random_html(&mut rng);
        let mut page = load_page(&html);
        let size = page.document_size();
        for _ in 0..30 {
            let p = Point::new(rng.f32(size.width), rng.f32(size.height.min(3000.0)));
            let s = Point::new(0.0, rng.f32(500.0));
            match rng.below(12) {
                0 => {
                    let _ = page.click(p, s);
                }
                1 => {
                    let _ = page.set_hover(Some((p, s)));
                }
                2 => page.insert_text("abc\n\u{7}é"),
                3 => page.backspace(),
                4 => {
                    let _ = page.submit_focused();
                }
                5 => {
                    let _ = page.focus_next(rng.below(2) == 0);
                }
                6 => {
                    let _ = page.activate_focused();
                }
                7 => {
                    let _ = page.blur();
                }
                8 => {
                    let _ = page.find(["text", "日", "a b", "x"][rng.below(4)]);
                }
                9 => {
                    let _ = (page.link_at(p, s), page.cursor_at(p, s));
                }
                10 => {
                    let _ = page.fragment_offset(["x", "n", "top", "%zz", "日"][rng.below(5)]);
                }
                _ => page.set_viewport(Size::new(200.0 + rng.f32(1200.0), 300.0 + rng.f32(800.0))),
            }
        }
        let _ = (page.title(), page.meta_refresh());
        if round % 10 == 0 {
            let _ = render(&page, 300, 200);
        }
    }
}

#[test]
fn deep_and_wide_documents() {
    let deep = "<div><span><a href=#>".repeat(400) + "deep text";
    let mut page = load_page(&deep);
    let _ = page.click(Point::new(5.0, 5.0), Point::ZERO);
    let _ = page.find("deep");
    let _ = page.focus_next(false);
    let wide = "<input type=radio name=r>".repeat(3000) + &"<img src=s.gif>".repeat(3000);
    let mut page = load_page(&wide);
    let _ = page.focus_next(false);
    let _ = page.activate_focused();
    let svg_bomb = "<svg>".to_string() + &"<g>".repeat(5000) + "<rect width=1 height=1/>";
    let _ = load_page(&svg_bomb);
    let long_attr = format!("<img srcset='{}'>", "a.png 1x, ".repeat(20_000));
    let _ = load_page(&long_attr);
}
