//! Randomised ("monkey") test of the browser model and renderer: long
//! sequences of keys, pointer events, resizes, page loads and timer ticks
//! must never panic and must keep the model consistent.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use xbro_engine::Url;

use super::browser::tests::engine;
use super::browser::{Browser, LoadJob, Metrics};
use super::events::{MouseButton, WheelDelta};
use super::input::{KeyInput, Mods};
use super::line_edit::Clipboard;
use super::render::Renderer;
use super::viewport;

/// A small deterministic pseudo-random generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

struct Memory(Option<String>);

impl Clipboard for Memory {
    fn get_text(&mut self) -> Option<String> {
        self.0.clone()
    }
    fn set_text(&mut self, text: &str) {
        self.0 = Some(text.to_string());
    }
}

/// Pages served to the browser: links, fragments, forms, refreshes,
/// fixed elements, tall and wide content, a page with no content at all.
fn page_for(url: &Url) -> String {
    let path = url.path();
    let mut html = format!("<title>{path}</title>");
    match path.len() % 6 {
        0 => html.push_str("<meta http-equiv=refresh content='0; url=#end'>"),
        1 => html.push_str("<meta http-equiv=refresh content='1; url=/other'>"),
        2 => html.push_str("<div style='width:3000px;height:20px'>wide</div>"),
        3 => return String::new(),
        _ => {}
    }
    html.push_str(
        "<style>p{height:90px;margin:0}</style>\
         <header style='position:fixed;top:0'>needle header</header>\
         <form action=/search><input name=q value=x><input type=checkbox name=c>\
         <select name=s><option>a<option>b</select><button>Go</button></form>\
         <a href=#p5>to p5</a> <a href=/next>next</a> <a href=/a/b/c>deep</a>\
         <textarea>t</textarea>",
    );
    for i in 0..30 {
        html.push_str(&format!(
            "<p id=p{i}>needle paragraph {i} <a href=#p{i}>self</a></p>"
        ));
    }
    html.push_str("<p id=end>end</p>");
    html
}

fn named_keys() -> Vec<Key> {
    use NamedKey::*;
    [
        ArrowUp, ArrowDown, ArrowLeft, ArrowRight, PageUp, PageDown, Home, End, Tab, Enter, Escape,
        Backspace, Delete, Space, F3, F5, F6, Insert,
    ]
    .into_iter()
    .map(Key::Named)
    .collect()
}

fn random_mods(rng: &mut Rng) -> Mods {
    Mods {
        ctrl: rng.chance(30),
        shift: rng.chance(25),
        alt: rng.chance(10),
        logo: rng.chance(3),
    }
}

/// Checks the invariants of the model.
fn check(b: &Browser) {
    if b.quit {
        return;
    }
    assert!(b.tabs.active_index() < b.tabs.len(), "active tab in range");
    for tab in b.tabs.iter() {
        assert!(tab.history.is_empty() || tab.history.index() < tab.history.len());
        assert!(tab.zoom >= viewport::ZOOM_STEPS[0] && tab.zoom <= viewport::ZOOM_STEPS[11]);
        if let Some(page) = &tab.page {
            let clamped = viewport::clamp_scroll(tab.scroll, page.document_size(), page.viewport());
            assert_eq!(tab.scroll, clamped, "scroll inside the document");
        }
        if let Some(i) = tab.find.current {
            assert!(i < tab.find.matches.len(), "current match in range");
        }
    }
    let text = b.address.edit.text();
    assert!(text.is_char_boundary(b.address.edit.caret()));
}

fn run(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let jobs: Rc<RefCell<Vec<LoadJob>>> = Rc::default();
    let sink = jobs.clone();
    let metrics = Metrics {
        width: 900,
        height: 700,
        dpi: 1.0,
    };
    let mut b = Browser::new(
        engine().fonts,
        metrics,
        Box::new(move |job| sink.borrow_mut().push(job)),
    );
    let mut clipboard = Memory(Some("pasted 文字 text\nwith a line".to_string()));
    let mut renderer = Renderer::default();
    let mut now = Instant::now();
    b.open_initial(Url::parse("https://t.test/start").unwrap());

    let named = named_keys();
    let chars = [
        "a", "d", "f", "g", "l", "r", "t", "w", "v", "c", "x", "z", "1", "5", "9", "0", "=", "-",
        "+", "[", "]", "é", "€", " ", "/", "#",
    ];
    for step in 0..steps {
        match rng.below(100) {
            // Complete (or drop) a pending load, in any order.
            0..=14 => {
                let n = jobs.borrow().len();
                if n > 0 {
                    let job = jobs.borrow_mut().remove(rng.below(n));
                    if rng.chance(90) {
                        let html = page_for(job.request.url());
                        let page =
                            engine().load_html(&html, job.request.url().clone(), job.viewport);
                        b.on_loaded(job.tab_id, job.nav_id, page);
                    }
                }
            }
            15..=39 => {
                let key = if rng.chance(50) {
                    rng.pick(&named).clone()
                } else {
                    Key::Character((*rng.pick(&chars)).into())
                };
                let text = match &key {
                    Key::Character(s) => Some(s.to_string()),
                    Key::Named(NamedKey::Enter) => Some("\r".to_string()),
                    Key::Named(NamedKey::Space) => Some(" ".to_string()),
                    _ => None,
                };
                let input = KeyInput {
                    key: &key,
                    code: PhysicalKey::Code(KeyCode::KeyA),
                    text: text.as_deref(),
                    mods: random_mods(&mut rng),
                };
                b.key(&input, &mut clipboard);
            }
            40..=64 => {
                let (w, h) = (b.metrics.width as f32, b.metrics.height as f32);
                let x = rng.below(w as usize + 40) as f32 - 20.0;
                let y = rng.below(h as usize + 40) as f32 - 20.0;
                b.mouse_move(x, y);
            }
            65..=79 => {
                let button = *rng.pick(&[
                    MouseButton::Left,
                    MouseButton::Left,
                    MouseButton::Middle,
                    MouseButton::Right,
                    MouseButton::Back,
                    MouseButton::Forward,
                ]);
                let mods = random_mods(&mut rng);
                b.mouse_button(button, true, mods);
                if rng.chance(80) {
                    b.mouse_button(button, false, mods);
                }
            }
            80..=86 => {
                let delta = if rng.chance(50) {
                    WheelDelta::Lines(0.0, rng.below(7) as f32 - 3.0)
                } else {
                    WheelDelta::Pixels(rng.below(200) as f32 - 100.0, rng.below(200) as f32 - 100.0)
                };
                b.wheel(delta, random_mods(&mut rng));
            }
            87..=89 => b.mouse_left(),
            90..=93 => {
                let dpi = *rng.pick(&[1.0, 1.0, 1.25, 1.5, 2.0, 3.0]);
                let w = *rng.pick(&[1, 50, 400, 801, 1280, 2000]);
                let h = *rng.pick(&[1, 30, 300, 599, 860, 1500]);
                b.resize(w, h, dpi);
            }
            94..=97 => {
                now += Duration::from_millis(rng.below(1500) as u64);
                b.tick(now);
                let _ = b.next_deadline(now);
            }
            _ => {
                b.take_needs_redraw();
                let _ = renderer.render(&mut b, now);
            }
        }
        if b.quit {
            // Closing the last tab quits: start again with a new window.
            b = {
                let sink = jobs.clone();
                jobs.borrow_mut().clear();
                let mut fresh = Browser::new(
                    engine().fonts,
                    b.metrics,
                    Box::new(move |job| sink.borrow_mut().push(job)),
                );
                fresh.open_initial(Url::parse("https://t.test/again").unwrap());
                fresh
            };
        }
        check(&b);
        if step % 97 == 0 {
            let _ = renderer.render(&mut b, now);
        }
    }
}

#[test]
fn random_input_never_panics_and_keeps_the_model_consistent() {
    for seed in [1, 2, 3, 0x5eed, 0xdead_beef] {
        run(seed, 1500);
    }
}
