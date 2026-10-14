//! Keyboard and mouse handling for [`Browser`]: decoded key presses and
//! pointer events in device pixels come in, browser actions come out.

use std::time::{Duration, Instant};

use xbro_engine::{Cursor, Page, Point, Url};

use super::browser::{Browser, Drag, Focus, PAGE_OVERLAP, SCROLL_STEP};
use super::chrome::{ChromeElement, ChromeLayout, Hit, UI_TEXT_SIZE};
use super::input::{
    edit_action, field_command, global_command, page_command, Command, FieldCommand, KeyInput,
    Mods, PageCommand, ScrollKey,
};
use super::line_edit::{Clipboard, EditAction, EditOutcome};
use super::paint::caret_at;
use super::tabs::NavKind;
use super::viewport::{self, Axis};

/// Clipboard text prepared for a page text field: line breaks are kept
/// (as `\n`) only in a textarea, other control characters are dropped.
fn paste_text(text: &str, multiline: bool) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars()
        .filter_map(|c| match c {
            '\n' if multiline => Some('\n'),
            '\n' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// The URL of the focused element when it is a link (`<a href>`,
/// `<area href>`), as [`xbro_engine::Page::link_at`] gives it for a pointer.
fn focused_link(page: &Page) -> Option<Url> {
    let node = page.focused()?;
    let doc = page.document();
    if !matches!(doc.tag_name(node), Some("a" | "area")) {
        return None;
    }
    xbro_net::resolve(page.base_url(), doc.attr(node, "href")?)
        .filter(|url| url.scheme() != "javascript")
}

/// CSS px scrolled per wheel notch.
pub const WHEEL_LINE: f32 = 48.0;
const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(500);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum WheelDelta {
    /// Notches (positive = up / left, as winit reports them).
    Lines(f32, f32),
    /// Device pixels (touchpads).
    Pixels(f32, f32),
}

impl Browser {
    // ---- keyboard -----------------------------------------------------------

    /// Handles a key press.
    pub fn key(&mut self, k: &KeyInput, clipboard: &mut dyn Clipboard) {
        if let Some(cmd) = global_command(k) {
            self.command(cmd);
            return;
        }
        match self.focus {
            Focus::Address => {
                if let Some(action) = edit_action(k) {
                    self.edit_address(&action, clipboard);
                }
            }
            Focus::Find => {
                if let Some(action) = edit_action(k) {
                    self.edit_find(&action, clipboard);
                }
            }
            Focus::Page => self.page_key(k, clipboard),
        }
    }

    /// Runs a browser shortcut.
    pub fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Back => self.back(),
            Command::Forward => self.forward(),
            Command::Reload => self.reload(),
            Command::Home => self.home(),
            Command::NewTab => self.new_tab_with_address(),
            Command::CloseTab => self.close_tab(self.tabs.active_index()),
            Command::NextTab => self.next_tab(),
            Command::PrevTab => self.prev_tab(),
            Command::SelectTab(i) => self.select_tab(i),
            Command::SelectLastTab => self.last_tab(),
            Command::FocusAddress => self.focus_address(),
            Command::Find => self.open_find(),
            Command::FindNext | Command::FindPrev => {
                if self.active().is_some_and(|t| t.find.open) {
                    self.find_step(cmd == Command::FindPrev);
                } else {
                    self.open_find();
                }
            }
            Command::ZoomIn => self.set_zoom(viewport::zoom_in(self.zoom())),
            Command::ZoomOut => self.set_zoom(viewport::zoom_out(self.zoom())),
            Command::ZoomReset => self.set_zoom(1.0),
        }
    }

    fn edit_address(&mut self, action: &EditAction, clipboard: &mut dyn Clipboard) {
        match self.address.edit.apply(action, clipboard) {
            EditOutcome::Submit { .. } => {
                let url = self.address.submit();
                self.focus = Focus::Page;
                self.open_url(url);
            }
            EditOutcome::Cancel => self.focus_page(),
            EditOutcome::Changed | EditOutcome::Moved => {}
        }
        self.update_field_scrolls();
        self.invalidate();
    }

    fn edit_find(&mut self, action: &EditAction, clipboard: &mut dyn Clipboard) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        match tab.find.query.apply(action, clipboard) {
            EditOutcome::Changed => self.run_find(),
            EditOutcome::Submit { shift } => self.find_step(shift),
            EditOutcome::Cancel => self.close_find(),
            EditOutcome::Moved => {}
        }
        self.update_field_scrolls();
        self.invalidate();
    }

    fn page_key(&mut self, k: &KeyInput, clipboard: &mut dyn Clipboard) {
        let text_focus = self
            .active()
            .and_then(|t| t.page.as_ref())
            .is_some_and(|p| p.has_text_focus());
        if text_focus {
            if let Some(cmd) = field_command(k) {
                self.field_key(cmd, clipboard);
            }
            return;
        }
        let Some(cmd) = page_command(k) else {
            return;
        };
        match cmd {
            PageCommand::Scroll(ScrollKey::Step { dx, dy }) => {
                self.scroll_by(f32::from(dx) * SCROLL_STEP, f32::from(dy) * SCROLL_STEP);
            }
            PageCommand::Scroll(ScrollKey::Page(dir)) => self.scroll_page(f32::from(dir)),
            PageCommand::Space { shift } => {
                if self.control_focused() {
                    self.activate_focused();
                } else {
                    self.scroll_page(if shift { -1.0 } else { 1.0 });
                }
            }
            PageCommand::Scroll(ScrollKey::Top) => {
                let x = self.active().map_or(0.0, |t| t.scroll.x);
                self.scroll_to(Point::new(x, 0.0));
            }
            PageCommand::Scroll(ScrollKey::Bottom) => {
                let x = self.active().map_or(0.0, |t| t.scroll.x);
                self.scroll_to(Point::new(x, f32::MAX));
            }
            PageCommand::FocusNext { backwards } => self.focus_next(backwards),
            PageCommand::Activate => self.activate_focused(),
            PageCommand::Escape => self.escape(),
        }
    }

    /// Scrolls one viewport (minus an overlap) down (`dir` = 1) or up (−1).
    fn scroll_page(&mut self, dir: f32) {
        let amount = (self.viewport_size().height - PAGE_OVERLAP).max(SCROLL_STEP);
        self.scroll_by(0.0, dir * amount);
    }

    /// Enter on the page: activates the focused link or control. A link
    /// to a fragment of the page is recorded in the history, as when clicked.
    fn activate_focused(&mut self) {
        let Some(page) = self.tabs.active_mut().and_then(|t| t.page.as_mut()) else {
            return;
        };
        let link = focused_link(page);
        let action = page.activate_focused();
        self.apply_click(action, link);
    }

    /// Whether the page focus is on a form control that Space activates.
    fn control_focused(&self) -> bool {
        let Some(page) = self.active().and_then(|t| t.page.as_ref()) else {
            return false;
        };
        page.focused()
            .and_then(|node| page.document().tag_name(node))
            .is_some_and(|tag| matches!(tag, "input" | "button" | "select" | "summary"))
    }

    /// Escape on the page: stops loading, else drops the page focus, else
    /// closes the find bar.
    fn escape(&mut self) {
        if self.active().is_some_and(|t| t.is_loading()) {
            self.stop();
            return;
        }
        let blurred = self
            .tabs
            .active_mut()
            .and_then(|t| t.page.as_mut())
            .is_some_and(|p| p.blur());
        if blurred {
            self.invalidate_page();
        } else if self.active().is_some_and(|t| t.find.open) {
            self.close_find();
        }
    }

    /// A key for a focused text field of the page.
    fn field_key(&mut self, cmd: FieldCommand, clipboard: &mut dyn Clipboard) {
        let Some(page) = self.tabs.active_mut().and_then(|t| t.page.as_mut()) else {
            return;
        };
        match cmd {
            FieldCommand::Insert(text) => page.insert_text(&text),
            FieldCommand::Paste => {
                let multiline = page
                    .focused()
                    .and_then(|n| page.document().tag_name(n))
                    .is_some_and(|tag| tag == "textarea");
                let Some(text) = clipboard.get_text() else {
                    return;
                };
                page.insert_text(&paste_text(&text, multiline));
            }
            FieldCommand::Backspace => page.backspace(),
            FieldCommand::Submit => {
                if let Some(request) = page.submit_focused() {
                    self.navigate(request, NavKind::New);
                    return;
                }
            }
            FieldCommand::Escape => {
                self.escape();
                return;
            }
            FieldCommand::FocusNext { backwards } => {
                self.focus_next(backwards);
                return;
            }
        }
        self.refresh_find();
        self.invalidate_page();
    }

    // ---- mouse ------------------------------------------------------------

    /// The pointer moved to (`x`, `y`) device px.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.mouse = Some((x, y));
        let dpi = self.metrics.dpi;
        match self.drag {
            Some(Drag::Scrollbar { axis, grab }) => {
                let Some(bar) = self.scrollbar(axis) else {
                    return;
                };
                let Some(scroll) = self.active().map(|t| t.scroll) else {
                    return;
                };
                let target = match axis {
                    Axis::Vertical => Point::new(scroll.x, bar.scroll_for_thumb(y / dpi - grab)),
                    Axis::Horizontal => Point::new(bar.scroll_for_thumb(x / dpi - grab), scroll.y),
                };
                self.scroll_to(target);
            }
            Some(Drag::SelectAddress) => {
                let layout = self.layout();
                let pos = self.caret_in_field(
                    layout.address_text(),
                    self.address.edit.text(),
                    self.address.scroll_x,
                    x,
                );
                self.address.edit.set_caret(pos, true);
                self.update_field_scrolls();
                self.invalidate();
            }
            Some(Drag::SelectFind) => {
                let layout = self.layout();
                let (Some(find), Some(tab)) = (layout.find, self.active()) else {
                    return;
                };
                let pos = self.caret_in_field(
                    find.input_text(),
                    tab.find.query.text(),
                    self.find_scroll_x,
                    x,
                );
                if let Some(tab) = self.tabs.active_mut() {
                    tab.find.query.set_caret(pos, true);
                }
                self.update_field_scrolls();
                self.invalidate();
            }
            None => self.refresh_hover(),
        }
    }

    /// The pointer left the window.
    pub fn mouse_left(&mut self) {
        self.mouse = None;
        if self.drag.is_none() {
            self.refresh_hover();
        }
    }

    pub fn mouse_button(&mut self, button: MouseButton, pressed: bool, mods: Mods) {
        let Some((x, y)) = self.mouse else {
            // The pointer left the window during a drag: the release still
            // ends it (no button is activated).
            if button == MouseButton::Left && !pressed {
                self.left_up(Hit::Chrome);
            }
            return;
        };
        let layout = self.layout();
        let dpi = self.metrics.dpi;
        let hit = layout.hit(Point::new(x / dpi, y / dpi));
        match (button, pressed) {
            (MouseButton::Left, true) => self.left_down(hit, &layout, x, y, mods),
            (MouseButton::Left, false) => self.left_up(hit),
            (MouseButton::Middle, true) => match hit {
                Hit::Element(ChromeElement::Tab(i) | ChromeElement::TabClose(i)) => {
                    self.close_tab(i)
                }
                Hit::Page => {
                    if let Some(url) = self.link_under(x, y) {
                        self.new_tab(url, true);
                    }
                }
                _ => {}
            },
            (MouseButton::Back, true) => self.back(),
            (MouseButton::Forward, true) => self.forward(),
            _ => {}
        }
    }

    pub fn wheel(&mut self, delta: WheelDelta, mods: Mods) {
        if let Some((x, y)) = self.mouse {
            if !self.page_area().contains(x, y) {
                return;
            }
        }
        if mods.primary() {
            // Ctrl+wheel zooms (one step per notch; touchpad deltas are ignored).
            if let WheelDelta::Lines(_, dy) = delta {
                if dy > 0.0 {
                    self.set_zoom(viewport::zoom_in(self.zoom()));
                } else if dy < 0.0 {
                    self.set_zoom(viewport::zoom_out(self.zoom()));
                }
            }
            return;
        }
        let (mut dx, mut dy) = match delta {
            WheelDelta::Lines(x, y) => (-x * WHEEL_LINE, -y * WHEEL_LINE),
            WheelDelta::Pixels(x, y) => {
                let scale = self.page_area().scale;
                (-x / scale, -y / scale)
            }
        };
        if mods.shift && dx == 0.0 {
            std::mem::swap(&mut dx, &mut dy);
        }
        self.scroll_by(dx, dy);
    }

    fn left_down(&mut self, hit: Hit, layout: &ChromeLayout, x: f32, y: f32, mods: Mods) {
        let clicks = self.count_click(x, y);
        match hit {
            Hit::Element(ChromeElement::Tab(i)) => self.select_tab(i),
            Hit::Element(ChromeElement::Address) => {
                if self.focus != Focus::Address {
                    self.focus_address();
                    return;
                }
                let pos = self.caret_in_field(
                    layout.address_text(),
                    self.address.edit.text(),
                    self.address.scroll_x,
                    x,
                );
                let edit = &mut self.address.edit;
                match clicks {
                    1 => {
                        edit.set_caret(pos, mods.shift);
                        self.drag = Some(Drag::SelectAddress);
                    }
                    2 => edit.select_word_at(pos),
                    _ => edit.select_all(),
                }
                self.update_field_scrolls();
                self.invalidate();
            }
            Hit::Element(ChromeElement::FindInput) => {
                let Some(find) = layout.find else {
                    return;
                };
                self.address.blur();
                self.focus = Focus::Find;
                let Some(text) = self.active().map(|t| t.find.query.text().to_string()) else {
                    return;
                };
                let pos = self.caret_in_field(find.input_text(), &text, self.find_scroll_x, x);
                if let Some(tab) = self.tabs.active_mut() {
                    let edit = &mut tab.find.query;
                    match clicks {
                        1 => {
                            edit.set_caret(pos, mods.shift);
                            self.drag = Some(Drag::SelectFind);
                        }
                        2 => edit.select_word_at(pos),
                        _ => edit.select_all(),
                    }
                }
                self.update_field_scrolls();
                self.invalidate();
            }
            Hit::Element(element) => {
                self.pressed = Some(element);
                self.invalidate();
            }
            Hit::Chrome => {}
            Hit::Page => self.page_down(x, y, mods),
        }
    }

    fn left_up(&mut self, hit: Hit) {
        self.drag = None;
        if let Some(pressed) = self.pressed.take() {
            self.invalidate();
            if hit == Hit::Element(pressed) {
                self.activate(pressed);
            }
        }
        self.refresh_hover();
    }

    /// A chrome button was clicked.
    pub fn activate(&mut self, element: ChromeElement) {
        match element {
            ChromeElement::Back => self.back(),
            ChromeElement::Forward => self.forward(),
            ChromeElement::Reload => {
                if self.active().is_some_and(|t| t.is_loading()) {
                    self.stop();
                } else {
                    self.reload();
                }
            }
            ChromeElement::Home => self.home(),
            ChromeElement::NewTab => self.new_tab_with_address(),
            ChromeElement::TabClose(i) => self.close_tab(i),
            ChromeElement::FindPrev => self.find_step(true),
            ChromeElement::FindNext => self.find_step(false),
            ChromeElement::FindClose => self.close_find(),
            ChromeElement::Tab(i) => self.select_tab(i),
            ChromeElement::Address => self.focus_address(),
            ChromeElement::FindInput => self.open_find(),
        }
    }

    /// Left press in the page area: scrollbars, then page content.
    fn page_down(&mut self, x: f32, y: f32, mods: Mods) {
        let dpi = self.metrics.dpi;
        let p = Point::new(x / dpi, y / dpi);
        for axis in [Axis::Vertical, Axis::Horizontal] {
            let Some(bar) = self.scrollbar(axis) else {
                continue;
            };
            if !bar.track.contains(p) {
                continue;
            }
            let along = match axis {
                Axis::Vertical => p.y,
                Axis::Horizontal => p.x,
            };
            if bar.thumb.contains(p) {
                self.drag = Some(Drag::Scrollbar {
                    axis,
                    grab: along - bar.thumb_start(),
                });
            } else {
                // Click in the track: one page towards the click.
                let dir = if along < bar.thumb_start() { -1.0 } else { 1.0 };
                let view = self.viewport_size();
                match axis {
                    Axis::Vertical => {
                        self.scroll_by(0.0, dir * (view.height - PAGE_OVERLAP).max(SCROLL_STEP))
                    }
                    Axis::Horizontal => {
                        self.scroll_by(dir * (view.width - PAGE_OVERLAP).max(SCROLL_STEP), 0.0)
                    }
                };
            }
            self.invalidate();
            return;
        }

        self.focus_page();
        let area = self.page_area();
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let scroll = tab.scroll;
        let Some(page) = tab.page.as_mut() else {
            return;
        };
        let doc = area.to_document(x, y, scroll);
        let link = page.link_at(doc, scroll);
        if mods.primary() {
            if let Some(url) = link {
                self.new_tab(url, true);
                return;
            }
        }
        let action = page.click(doc, scroll);
        // The focus may have moved even when nothing else happened.
        self.invalidate_page();
        self.apply_click(action, link);
    }

    /// Link under a window point of the page area.
    fn link_under(&self, x: f32, y: f32) -> Option<Url> {
        let area = self.page_area();
        let tab = self.active()?;
        let page = tab.page.as_ref()?;
        page.link_at(area.to_document(x, y, tab.scroll), tab.scroll)
    }

    /// Counts successive clicks at the same place (double/triple clicks).
    fn count_click(&mut self, x: f32, y: f32) -> u32 {
        let now = Instant::now();
        let slop = 5.0 * self.metrics.dpi;
        let count = match self.last_click {
            Some((t, (px, py), n))
                if now.duration_since(t) < DOUBLE_CLICK_TIME
                    && (px - x).abs() < slop
                    && (py - y).abs() < slop =>
            {
                n + 1
            }
            _ => 1,
        };
        self.last_click = Some((now, (x, y), count));
        count
    }

    /// Caret position for a window x (device px) in a field whose text
    /// area is `inner` (logical px), scrolled by `scroll_x`.
    fn caret_in_field(&self, inner: xbro_engine::Rect, text: &str, scroll_x: f32, x: f32) -> usize {
        let dpi = self.metrics.dpi;
        let stops = self.fonts.caret_stops(text, UI_TEXT_SIZE, dpi);
        caret_at(&stops, x / dpi - inner.x + scroll_x)
    }

    /// Recomputes what is under the pointer: hovered chrome element, page
    /// `:hover`, hovered link (status bubble) and cursor.
    pub(super) fn refresh_hover(&mut self) {
        let dpi = self.metrics.dpi;
        let layout = self.layout();
        let mut element = None;
        let mut cursor = Cursor::Default;
        let mut page_point = None;
        let mut on_scrollbar = None;
        if let Some((x, y)) = self.mouse {
            let p = Point::new(x / dpi, y / dpi);
            match layout.hit(p) {
                Hit::Element(e) => {
                    element = Some(e);
                    cursor = match e {
                        ChromeElement::Address | ChromeElement::FindInput => Cursor::Text,
                        ChromeElement::Tab(_) => Cursor::Default,
                        _ => Cursor::Pointer,
                    };
                }
                Hit::Chrome => {}
                Hit::Page => {
                    on_scrollbar = [Axis::Vertical, Axis::Horizontal]
                        .into_iter()
                        .find(|&a| self.scrollbar(a).is_some_and(|b| b.track.contains(p)));
                    if on_scrollbar.is_none() && self.drag.is_none() {
                        page_point = Some((x, y));
                    }
                }
            }
        }
        if element != self.hover || on_scrollbar != self.scrollbar_hover {
            self.hover = element;
            self.scrollbar_hover = on_scrollbar;
            self.invalidate();
        }
        let area = self.page_area();
        let mut link = None;
        let mut restyled = false;
        if let Some(tab) = self.tabs.active_mut() {
            let scroll = tab.scroll;
            if let Some(page) = tab.page.as_mut() {
                let doc = page_point.map(|(x, y)| area.to_document(x, y, scroll));
                restyled = page.set_hover(doc.map(|d| (d, scroll)));
                if let Some(d) = doc {
                    link = page.link_at(d, scroll);
                    cursor = page.cursor_at(d, scroll);
                }
            }
        }
        if restyled {
            self.refresh_find();
            self.invalidate_page();
        }
        if link != self.hovered_link {
            self.hovered_link = link;
            self.invalidate();
        }
        self.cursor = cursor;
    }
}

#[cfg(test)]
mod tests {
    use super::super::browser::tests::{browser, finish, url};
    use super::*;
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

    #[test]
    fn pasted_text_is_cleaned_for_fields() {
        assert_eq!(paste_text("a\r\nb\tc\u{7}", false), "a b c");
        assert_eq!(paste_text("a\r\nb\rc", true), "a\nb\nc");
    }

    #[derive(Default)]
    struct Memory(Option<String>);

    impl Clipboard for Memory {
        fn get_text(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn set_text(&mut self, text: &str) {
            self.0 = Some(text.to_string());
        }
    }

    const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
        logo: false,
    };

    fn press(b: &mut Browser, key: Key, text: Option<&str>, mods: Mods) {
        let k = KeyInput {
            key: &key,
            code: PhysicalKey::Code(KeyCode::KeyA),
            text,
            mods,
        };
        b.key(&k, &mut Memory::default());
    }

    fn type_text(b: &mut Browser, s: &str) {
        for c in s.chars() {
            let t = c.to_string();
            press(b, Key::Character(t.as_str().into()), Some(&t), NONE);
        }
    }

    #[test]
    fn typing_in_the_address_bar_navigates() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 3);
        let ctrl = Mods { ctrl: true, ..NONE };
        press(&mut b, Key::Character("l".into()), Some("l"), ctrl);
        assert_eq!(b.focus, Focus::Address);
        assert_eq!(b.address.edit.selected_text(), "https://a.test/");
        type_text(&mut b, "about:help");
        assert_eq!(b.address.edit.text(), "about:help");
        press(&mut b, Key::Named(NamedKey::Enter), Some("\r"), NONE);
        assert_eq!(b.focus, Focus::Page);
        assert_eq!(
            jobs.borrow().last().unwrap().request.url().as_str(),
            "about:help"
        );

        // Escape reverts and gives the focus back to the page.
        press(&mut b, Key::Named(NamedKey::F6), None, NONE);
        type_text(&mut b, "zzz");
        press(&mut b, Key::Named(NamedKey::Escape), None, NONE);
        assert_eq!(b.focus, Focus::Page);
        assert!(!b.address.focused);
    }

    #[test]
    fn focusing_the_address_bar_shows_the_start_of_a_long_url() {
        let (mut b, jobs) = browser();
        let long = format!("https://a.test/{}", "segment/".repeat(60));
        b.open_initial(url(&long));
        finish(&mut b, &jobs, "A", 3);
        if !b.fonts.system.has_fonts() {
            return;
        }
        let ctrl = Mods { ctrl: true, ..NONE };
        press(&mut b, Key::Character("l".into()), Some("l"), ctrl);
        assert_eq!(b.address.edit.selected_text(), long);
        assert_eq!(b.address.scroll_x, 0.0, "scheme and domain stay visible");
        // Moving the caret to the end scrolls to it.
        press(&mut b, Key::Named(NamedKey::End), None, NONE);
        assert!(b.address.scroll_x > 0.0);
    }

    #[test]
    fn clicks_in_the_address_bar_place_the_caret() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 3);
        let field = b.layout().address;
        // First click: focus with everything selected.
        b.mouse_move(field.x + 30.0, field.y + 10.0);
        b.mouse_button(MouseButton::Left, true, NONE);
        b.mouse_button(MouseButton::Left, false, NONE);
        assert_eq!(b.focus, Focus::Address);
        assert_eq!(b.address.edit.selected_text(), "https://a.test/");
        if !b.fonts.system.has_fonts() {
            return;
        }
        // Later click: caret at the clicked character boundary.
        let inner = b.layout().address_text();
        let stops = b.fonts.caret_stops("https://a.test/", UI_TEXT_SIZE, 1.0);
        let x = inner.x + crate::gui::paint::caret_x(&stops, 8) + 0.4;
        b.last_click = None;
        b.mouse_move(x, field.y + 10.0);
        b.mouse_button(MouseButton::Left, true, NONE);
        assert_eq!(b.address.edit.caret(), 8);
        // Dragging extends the selection.
        let end = inner.x + crate::gui::paint::caret_x(&stops, 14);
        b.mouse_move(end, field.y + 10.0);
        b.mouse_button(MouseButton::Left, false, NONE);
        assert_eq!(b.address.edit.selected_text(), "a.test");
    }

    #[test]
    fn page_keys_scroll_and_backspace_is_not_back() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 3);
        b.open_url(url("https://b.test/"));
        finish(&mut b, &jobs, "B", 60);
        press(&mut b, Key::Named(NamedKey::ArrowDown), None, NONE);
        assert_eq!(b.active().unwrap().scroll.y, SCROLL_STEP);
        press(&mut b, Key::Named(NamedKey::PageDown), None, NONE);
        assert_eq!(
            b.active().unwrap().scroll.y,
            SCROLL_STEP + 526.0 - PAGE_OVERLAP
        );
        press(&mut b, Key::Named(NamedKey::End), None, NONE);
        let tab = b.active().unwrap();
        let page = tab.page.as_ref().unwrap();
        assert_eq!(
            tab.scroll.y,
            page.document_size().height - page.viewport().height
        );
        press(&mut b, Key::Named(NamedKey::Home), None, NONE);
        assert_eq!(b.active().unwrap().scroll.y, 0.0);
        let pending = jobs.borrow().len();
        press(&mut b, Key::Named(NamedKey::Backspace), None, NONE);
        assert_eq!(jobs.borrow().len(), pending);
        let alt = Mods { alt: true, ..NONE };
        press(&mut b, Key::Named(NamedKey::ArrowLeft), None, alt);
        assert_eq!(jobs.borrow().len(), pending + 1, "Alt+Left goes back");
    }

    #[test]
    fn space_toggles_a_focused_checkbox_and_scrolls_otherwise() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        let job = jobs.borrow_mut().pop().unwrap();
        let html = "<input type=checkbox id=c><div style='height:3000px'></div>";
        let page = super::super::browser::tests::engine().load_html(
            html,
            job.request.url().clone(),
            job.viewport,
        );
        b.on_loaded(job.tab_id, job.nav_id, page);
        let checked = |b: &Browser| {
            let page = b.active().unwrap().page.as_ref().unwrap();
            let doc = page.document();
            let node = doc.get_element_by_id("c").unwrap();
            doc.attr(node, "checked").is_some()
        };
        press(&mut b, Key::Named(NamedKey::Space), Some(" "), NONE);
        assert!(!checked(&b));
        assert!(
            b.active().unwrap().scroll.y > 0.0,
            "nothing focused: scrolls"
        );
        press(&mut b, Key::Named(NamedKey::Home), None, NONE);
        press(&mut b, Key::Named(NamedKey::Tab), Some("\t"), NONE);
        press(&mut b, Key::Named(NamedKey::Space), Some(" "), NONE);
        assert!(checked(&b));
        assert_eq!(b.active().unwrap().scroll.y, 0.0);
    }

    #[test]
    fn enter_on_a_focused_fragment_link_is_recorded_like_a_click() {
        // Regression: Enter scrolled to the target, but the address bar kept
        // the old URL and Back did not return to where the user was.
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/doc"));
        finish(&mut b, &jobs, "Doc", 40);
        // The first focusable element is `<a id=top href=#p20>`.
        press(&mut b, Key::Named(NamedKey::Tab), Some("\t"), NONE);
        press(&mut b, Key::Named(NamedKey::Enter), Some("\r"), NONE);
        assert!(jobs.borrow().is_empty(), "no load");
        let tab = b.active().unwrap();
        assert_eq!(tab.url().unwrap().as_str(), "https://a.test/doc#p20");
        let p20 = tab.page.as_ref().unwrap().fragment_offset("p20").unwrap();
        assert_eq!(tab.scroll.y, p20);
        let alt = Mods { alt: true, ..NONE };
        press(&mut b, Key::Named(NamedKey::ArrowLeft), None, alt);
        let tab = b.active().unwrap();
        assert_eq!(tab.url().unwrap().as_str(), "https://a.test/doc");
        assert_eq!(tab.scroll.y, 0.0);
    }

    #[test]
    fn a_release_outside_the_window_ends_a_scrollbar_drag() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 60);
        let bar = b.scrollbar(Axis::Vertical).unwrap();
        b.mouse_move(bar.thumb.x + 4.0, bar.thumb.y + 4.0);
        b.mouse_button(MouseButton::Left, true, NONE);
        b.mouse_move(bar.thumb.x + 4.0, bar.thumb.y + 100.0);
        let dragged = b.active().unwrap().scroll.y;
        assert!(dragged > 0.0);
        // The pointer leaves, then the button is released out there.
        b.mouse_left();
        b.mouse_button(MouseButton::Left, false, NONE);
        assert!(b.drag.is_none());
        // Moving back over the page no longer drags.
        b.mouse_move(400.0, 500.0);
        assert_eq!(b.active().unwrap().scroll.y, dragged);
    }

    #[test]
    fn find_bar_keys() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 20);
        let ctrl = Mods { ctrl: true, ..NONE };
        press(&mut b, Key::Character("f".into()), Some("f"), ctrl);
        assert_eq!(b.focus, Focus::Find);
        type_text(&mut b, "para 1");
        // "para 1", "para 10".."para 19"
        assert_eq!(b.active().unwrap().find.counter(), "1/11");
        press(&mut b, Key::Named(NamedKey::Enter), Some("\r"), NONE);
        assert_eq!(b.active().unwrap().find.counter(), "2/11");
        let shift = Mods {
            shift: true,
            ..NONE
        };
        press(&mut b, Key::Named(NamedKey::Enter), Some("\r"), shift);
        press(&mut b, Key::Named(NamedKey::Enter), Some("\r"), shift);
        assert_eq!(b.active().unwrap().find.counter(), "11/11");
        press(&mut b, Key::Named(NamedKey::Escape), None, NONE);
        assert!(!b.active().unwrap().find.open);
        assert_eq!(b.focus, Focus::Page);
    }

    #[test]
    fn mouse_on_chrome_and_page() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 60);
        // Click the "+" button: a new tab with the address bar focused.
        let layout = b.layout();
        let plus = layout.new_tab;
        b.mouse_move(plus.x + 5.0, plus.y + 5.0);
        assert_eq!(b.cursor, Cursor::Pointer);
        b.mouse_button(MouseButton::Left, true, NONE);
        b.mouse_button(MouseButton::Left, false, NONE);
        assert_eq!(b.tabs.len(), 2);
        assert_eq!(b.focus, Focus::Address);
        // Clicking the first tab selects it; middle click closes the second.
        b.mouse_move(20.0, 20.0);
        b.mouse_button(MouseButton::Left, true, NONE);
        b.mouse_button(MouseButton::Left, false, NONE);
        assert_eq!(b.tabs.active_index(), 0);
        let second = b.layout().tabs[1].rect;
        b.mouse_move(second.x + 10.0, 20.0);
        b.mouse_button(MouseButton::Middle, true, NONE);
        assert_eq!(b.tabs.len(), 1);

        // Wheel over the page scrolls; over the toolbar it does not.
        b.mouse_move(400.0, 300.0);
        b.wheel(WheelDelta::Lines(0.0, -2.0), NONE);
        assert_eq!(b.active().unwrap().scroll.y, 2.0 * WHEEL_LINE);
        b.wheel(WheelDelta::Pixels(0.0, 30.0), NONE);
        assert_eq!(b.active().unwrap().scroll.y, 2.0 * WHEEL_LINE - 30.0);
        b.mouse_move(400.0, 50.0);
        b.wheel(WheelDelta::Lines(0.0, -2.0), NONE);
        assert_eq!(b.active().unwrap().scroll.y, 2.0 * WHEEL_LINE - 30.0);
        assert_eq!(b.cursor, Cursor::Text, "over the address bar");

        // Dragging the scrollbar thumb to the bottom scrolls to the end.
        let bar = b.scrollbar(Axis::Vertical).unwrap();
        b.mouse_move(bar.thumb.x + 4.0, bar.thumb.y + 4.0);
        b.mouse_button(MouseButton::Left, true, NONE);
        b.mouse_move(bar.thumb.x + 4.0, 10_000.0);
        b.mouse_button(MouseButton::Left, false, NONE);
        let tab = b.active().unwrap();
        let page = tab.page.as_ref().unwrap();
        assert_eq!(
            tab.scroll.y,
            page.document_size().height - page.viewport().height
        );
    }

    #[test]
    fn clicking_links_navigates_or_opens_tabs() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        let job = jobs.borrow_mut().pop().unwrap();
        let html = "<style>a{display:block;height:50px}</style><a href=/next>next</a>";
        let page = super::super::browser::tests::engine().load_html(
            html,
            job.request.url().clone(),
            job.viewport,
        );
        b.on_loaded(job.tab_id, job.nav_id, page);
        // The link fills the top 50 CSS px of the page (below 74 px of chrome).
        b.mouse_move(20.0, 74.0 + 20.0);
        assert_eq!(b.cursor, Cursor::Pointer);
        assert_eq!(b.status_text().as_deref(), Some("https://a.test/next"));
        let ctrl = Mods { ctrl: true, ..NONE };
        b.mouse_button(MouseButton::Left, true, ctrl);
        b.mouse_button(MouseButton::Left, false, ctrl);
        assert_eq!(b.tabs.len(), 2, "Ctrl+click opens a background tab");
        assert_eq!(b.tabs.active_index(), 0);
        b.mouse_button(MouseButton::Left, true, NONE);
        b.mouse_button(MouseButton::Left, false, NONE);
        let job = jobs.borrow().last().cloned().unwrap();
        assert_eq!(job.request.url().as_str(), "https://a.test/next");
        assert_eq!(job.tab_id, b.active().unwrap().id);
    }
}
