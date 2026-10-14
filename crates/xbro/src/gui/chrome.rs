//! Geometry of the browser chrome (tab strip, toolbar, find bar) in logical
//! px, and hit testing. Drawing lives in `render.rs`.

use xbro_engine::{Point, Rect};

pub const TAB_STRIP_HEIGHT: f32 = 34.0;
pub const TOOLBAR_HEIGHT: f32 = 40.0;
/// Height of everything above the page.
pub const CHROME_HEIGHT: f32 = TAB_STRIP_HEIGHT + TOOLBAR_HEIGHT;
pub const FIND_BAR_HEIGHT: f32 = 34.0;
/// Size of UI text (tab titles, fields, status), logical px.
pub const UI_TEXT_SIZE: f32 = 13.0;
/// Horizontal padding between the address field's edge and its text.
const ADDRESS_PADDING: f32 = 14.0;
/// Horizontal padding between the find field's edge and its text.
const FIND_PADDING: f32 = 10.0;

const STRIP_PADDING: f32 = 8.0;
const TAB_TOP: f32 = 6.0;
const MAX_TAB_WIDTH: f32 = 220.0;
const NEW_TAB_SIZE: f32 = 28.0;
/// Tabs narrower than this only show a close button when active.
const CLOSE_MIN_TAB_WIDTH: f32 = 60.0;
const CLOSE_SIZE: f32 = 18.0;
const BUTTON_SIZE: f32 = 32.0;
const BUTTON_GAP: f32 = 2.0;
const FIND_FIELD_MAX_WIDTH: f32 = 260.0;
const FIND_COUNTER_WIDTH: f32 = 70.0;
const FIND_BUTTON_SIZE: f32 = 26.0;
/// Size of the zoom level badge at the right end of the address field.
const ZOOM_BADGE_WIDTH: f32 = 50.0;
const ZOOM_BADGE_HEIGHT: f32 = 22.0;
/// Space between the badge and the field's edge, and the field's text.
const ZOOM_BADGE_GAP: f32 = 4.0;

/// An interactive element of the chrome.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChromeElement {
    Tab(usize),
    TabClose(usize),
    NewTab,
    Back,
    Forward,
    Reload,
    Home,
    Address,
    /// The zoom level shown in the address field (when not 100 %): resets it.
    ZoomReset,
    FindInput,
    FindPrev,
    FindNext,
    FindClose,
}

/// What lies under a point of the window.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Element(ChromeElement),
    /// Chrome background (tab strip, toolbar or find bar): inert.
    Chrome,
    Page,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TabSlot {
    pub rect: Rect,
    pub close: Option<Rect>,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FindBarLayout {
    pub bar: Rect,
    pub input: Rect,
    pub counter: Rect,
    pub prev: Rect,
    pub next: Rect,
    pub close: Rect,
}

/// Positions of every chrome element for a window size and tab count.
#[derive(Clone, Debug, PartialEq)]
pub struct ChromeLayout {
    pub width: f32,
    pub height: f32,
    pub tab_strip: Rect,
    pub tabs: Vec<TabSlot>,
    pub new_tab: Rect,
    pub toolbar: Rect,
    pub back: Rect,
    pub forward: Rect,
    pub reload: Rect,
    pub home: Rect,
    pub address: Rect,
    /// Zoom level badge inside the right end of the address field.
    pub zoom_badge: Option<Rect>,
    pub page: Rect,
    pub find: Option<FindBarLayout>,
}

/// The part of a text field where its text is drawn and hit-tested.
fn text_rect(field: Rect, padding: f32) -> Rect {
    Rect::new(
        field.x + padding,
        field.y,
        (field.width - 2.0 * padding).max(0.0),
        field.height,
    )
}

impl FindBarLayout {
    /// Text area of the find field.
    pub fn input_text(&self) -> Rect {
        text_rect(self.input, FIND_PADDING)
    }
}

impl ChromeLayout {
    /// Text area of the address field (left of the zoom badge).
    pub fn address_text(&self) -> Rect {
        let rect = text_rect(self.address, ADDRESS_PADDING);
        match self.zoom_badge {
            Some(badge) => Rect::new(
                rect.x,
                rect.y,
                (badge.x - ZOOM_BADGE_GAP - rect.x).clamp(0.0, rect.width),
                rect.height,
            ),
            None => rect,
        }
    }

    /// Shows the zoom badge in the address field (when the page is zoomed).
    pub fn with_zoom_badge(mut self, shown: bool) -> Self {
        let a = self.address;
        self.zoom_badge = (shown && a.width >= 2.0 * ZOOM_BADGE_WIDTH).then(|| {
            Rect::new(
                a.right() - ZOOM_BADGE_GAP - ZOOM_BADGE_WIDTH,
                a.y + (a.height - ZOOM_BADGE_HEIGHT) / 2.0,
                ZOOM_BADGE_WIDTH,
                ZOOM_BADGE_HEIGHT,
            )
        });
        self
    }

    /// Layout for a `width`×`height` (logical px) window with `tab_count`
    /// tabs (`active` always shows its close button).
    pub fn new(width: f32, height: f32, tab_count: usize, active: usize, find_open: bool) -> Self {
        let width = width.max(1.0);
        let height = height.max(1.0);

        // Tab strip: tabs share the room left of the "+" button.
        let room = (width - 2.0 * STRIP_PADDING - NEW_TAB_SIZE - 4.0).max(0.0);
        let tab_width = if tab_count == 0 {
            0.0
        } else {
            (room / tab_count as f32).min(MAX_TAB_WIDTH)
        };
        let tab_height = TAB_STRIP_HEIGHT - TAB_TOP;
        let tabs = (0..tab_count)
            .map(|i| {
                let rect = Rect::new(
                    STRIP_PADDING + i as f32 * tab_width,
                    TAB_TOP,
                    tab_width,
                    tab_height,
                );
                let close = (tab_width >= CLOSE_MIN_TAB_WIDTH
                    || (i == active && tab_width >= CLOSE_SIZE + 4.0))
                    .then(|| {
                        Rect::new(
                            rect.right() - 8.0 - CLOSE_SIZE,
                            rect.y + (rect.height - CLOSE_SIZE) / 2.0,
                            CLOSE_SIZE,
                            CLOSE_SIZE,
                        )
                    });
                TabSlot { rect, close }
            })
            .collect();
        let new_tab = Rect::new(
            STRIP_PADDING + tab_count as f32 * tab_width + 4.0,
            TAB_TOP + (tab_height - NEW_TAB_SIZE + 4.0) / 2.0,
            NEW_TAB_SIZE,
            NEW_TAB_SIZE - 4.0,
        );

        // Toolbar: four buttons, then the address field.
        let toolbar = Rect::new(0.0, TAB_STRIP_HEIGHT, width, TOOLBAR_HEIGHT);
        let button_y = TAB_STRIP_HEIGHT + (TOOLBAR_HEIGHT - BUTTON_SIZE) / 2.0;
        let button = |i: usize| {
            Rect::new(
                6.0 + i as f32 * (BUTTON_SIZE + BUTTON_GAP),
                button_y,
                BUTTON_SIZE,
                BUTTON_SIZE,
            )
        };
        let (back, forward, reload, home) = (button(0), button(1), button(2), button(3));
        let address_x = home.right() + 8.0;
        let address = Rect::new(
            address_x,
            TAB_STRIP_HEIGHT + 5.0,
            (width - 10.0 - address_x).max(0.0),
            TOOLBAR_HEIGHT - 10.0,
        );

        let find_height = if find_open { FIND_BAR_HEIGHT } else { 0.0 };
        let page_bottom = (height - find_height).max(CHROME_HEIGHT);
        let page = Rect::new(0.0, CHROME_HEIGHT, width, page_bottom - CHROME_HEIGHT);
        let find = find_open.then(|| {
            let bar = Rect::new(0.0, page_bottom, width, FIND_BAR_HEIGHT);
            let y = bar.y + (FIND_BAR_HEIGHT - FIND_BUTTON_SIZE) / 2.0;
            let close = Rect::new(
                width - 8.0 - FIND_BUTTON_SIZE,
                y,
                FIND_BUTTON_SIZE,
                FIND_BUTTON_SIZE,
            );
            // Field, counter, ▲, ▼ from the left; the field shrinks on narrow windows.
            let fixed = 8.0 + 6.0 + FIND_COUNTER_WIDTH + 2.0 * FIND_BUTTON_SIZE + 4.0 + 8.0;
            let input_width = (close.x - 8.0 - fixed).clamp(40.0, FIND_FIELD_MAX_WIDTH);
            let input = Rect::new(8.0, y, input_width, FIND_BUTTON_SIZE);
            let counter = Rect::new(input.right() + 6.0, y, FIND_COUNTER_WIDTH, FIND_BUTTON_SIZE);
            let prev = Rect::new(counter.right(), y, FIND_BUTTON_SIZE, FIND_BUTTON_SIZE);
            let next = Rect::new(prev.right() + 4.0, y, FIND_BUTTON_SIZE, FIND_BUTTON_SIZE);
            FindBarLayout {
                bar,
                input,
                counter,
                prev,
                next,
                close,
            }
        });

        Self {
            width,
            height,
            tab_strip: Rect::new(0.0, 0.0, width, TAB_STRIP_HEIGHT),
            tabs,
            new_tab,
            toolbar,
            back,
            forward,
            reload,
            home,
            address,
            zoom_badge: None,
            page,
            find,
        }
    }

    /// The element (or area) under a logical point.
    pub fn hit(&self, p: Point) -> Hit {
        use ChromeElement::*;
        let inside = |r: &Rect| p.x >= r.x && p.x < r.right() && p.y >= r.y && p.y < r.bottom();
        if p.y < TAB_STRIP_HEIGHT {
            for (i, slot) in self.tabs.iter().enumerate() {
                if slot.close.is_some_and(|c| inside(&c)) {
                    return Hit::Element(TabClose(i));
                }
                if inside(&slot.rect) {
                    return Hit::Element(Tab(i));
                }
            }
            if inside(&self.new_tab) {
                return Hit::Element(NewTab);
            }
            return Hit::Chrome;
        }
        if p.y < CHROME_HEIGHT {
            if self.zoom_badge.is_some_and(|r| inside(&r)) {
                return Hit::Element(ZoomReset);
            }
            let buttons = [
                (self.back, Back),
                (self.forward, Forward),
                (self.reload, Reload),
                (self.home, Home),
                (self.address, Address),
            ];
            return buttons
                .iter()
                .find(|(r, _)| inside(r))
                .map_or(Hit::Chrome, |&(_, e)| Hit::Element(e));
        }
        if let Some(find) = &self.find {
            if p.y >= find.bar.y {
                let parts = [
                    (find.input, FindInput),
                    (find.prev, FindPrev),
                    (find.next, FindNext),
                    (find.close, FindClose),
                ];
                return parts
                    .iter()
                    .find(|(r, _)| inside(r))
                    .map_or(Hit::Chrome, |&(_, e)| Hit::Element(e));
            }
        }
        if inside(&self.page) {
            Hit::Page
        } else {
            Hit::Chrome
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ChromeElement::*;

    fn hit(l: &ChromeLayout, x: f32, y: f32) -> Hit {
        l.hit(Point::new(x, y))
    }

    #[test]
    fn tabs_share_the_strip() {
        let l = ChromeLayout::new(1024.0, 768.0, 2, 0, false);
        assert_eq!(l.tabs.len(), 2);
        // Two tabs fit at the maximum width.
        assert_eq!(l.tabs[0].rect, Rect::new(8.0, 6.0, 220.0, 28.0));
        assert_eq!(l.tabs[1].rect.x, 228.0);
        assert_eq!(l.new_tab.x, 452.0);
        assert_eq!(hit(&l, 20.0, 20.0), Hit::Element(Tab(0)));
        assert_eq!(hit(&l, 300.0, 20.0), Hit::Element(Tab(1)));
        let close = l.tabs[1].close.unwrap();
        assert_eq!(close.right(), l.tabs[1].rect.right() - 8.0);
        assert_eq!(
            hit(&l, close.x + 5.0, close.y + 5.0),
            Hit::Element(TabClose(1))
        );
        assert_eq!(hit(&l, 460.0, 20.0), Hit::Element(NewTab));
        assert_eq!(hit(&l, 800.0, 20.0), Hit::Chrome);
        assert_eq!(hit(&l, 20.0, 2.0), Hit::Chrome, "above the tabs");

        // Many tabs shrink; narrow inactive tabs lose their close button.
        let l = ChromeLayout::new(800.0, 600.0, 40, 3, false);
        let w = l.tabs[0].rect.width;
        assert!((w - (800.0 - 16.0 - 32.0) / 40.0).abs() < 1e-3);
        assert!(l.tabs[0].close.is_none());
        assert!(l.tabs[3].close.is_none(), "too narrow even when active");
        assert!(l.new_tab.right() <= 800.0);
        let l = ChromeLayout::new(800.0, 600.0, 20, 3, false);
        assert!(l.tabs[0].close.is_none());
        assert!(l.tabs[3].close.is_some(), "the active tab keeps it");
        assert_eq!(
            hit(&l, l.tabs[19].rect.x + 1.0, 20.0),
            Hit::Element(Tab(19))
        );
    }

    #[test]
    fn toolbar_buttons_and_address_field() {
        let l = ChromeLayout::new(1024.0, 768.0, 1, 0, false);
        assert_eq!(l.back, Rect::new(6.0, 38.0, 32.0, 32.0));
        assert_eq!(hit(&l, 20.0, 50.0), Hit::Element(Back));
        assert_eq!(hit(&l, 55.0, 50.0), Hit::Element(Forward));
        assert_eq!(hit(&l, 90.0, 50.0), Hit::Element(Reload));
        assert_eq!(hit(&l, 125.0, 50.0), Hit::Element(Home));
        assert_eq!(l.address.x, l.home.right() + 8.0);
        assert_eq!(l.address.right(), 1014.0);
        assert_eq!(hit(&l, 500.0, 50.0), Hit::Element(Address));
        assert_eq!(hit(&l, 1020.0, 50.0), Hit::Chrome);
        assert_eq!(hit(&l, 500.0, 72.0), Hit::Chrome, "toolbar margin");
        assert_eq!(hit(&l, 500.0, 74.0), Hit::Page);
        assert_eq!(hit(&l, 500.0, 767.0), Hit::Page);
        assert_eq!(l.page, Rect::new(0.0, 74.0, 1024.0, 694.0));
    }

    #[test]
    fn zoom_badge_sits_in_the_address_field() {
        let plain = ChromeLayout::new(1024.0, 768.0, 1, 0, false);
        assert_eq!(plain.zoom_badge, None);
        assert_eq!(plain.clone().with_zoom_badge(false), plain);
        let l = plain.clone().with_zoom_badge(true);
        let badge = l.zoom_badge.unwrap();
        assert_eq!(badge.right(), l.address.right() - 4.0);
        assert!(badge.y > l.address.y && badge.bottom() < l.address.bottom());
        assert_eq!(
            hit(&l, badge.x + 5.0, badge.y + 5.0),
            Hit::Element(ZoomReset)
        );
        assert_eq!(hit(&l, badge.x - 5.0, badge.y + 5.0), Hit::Element(Address));
        // The text of the field stops before the badge.
        let text = l.address_text();
        assert_eq!(text.x, plain.address_text().x);
        assert_eq!(text.right(), badge.x - 4.0);
        // No room for it in a tiny field.
        let l = ChromeLayout::new(200.0, 300.0, 1, 0, false).with_zoom_badge(true);
        assert_eq!(l.zoom_badge, None);
        assert!(l.address_text().width >= 0.0);
    }

    #[test]
    fn find_bar_takes_the_bottom() {
        let l = ChromeLayout::new(1024.0, 768.0, 1, 0, true);
        let find = l.find.unwrap();
        assert_eq!(find.bar, Rect::new(0.0, 734.0, 1024.0, 34.0));
        assert_eq!(l.page.bottom(), 734.0);
        assert_eq!(hit(&l, 20.0, 750.0), Hit::Element(FindInput));
        assert_eq!(hit(&l, find.prev.x + 2.0, 750.0), Hit::Element(FindPrev));
        assert_eq!(hit(&l, find.next.x + 2.0, 750.0), Hit::Element(FindNext));
        assert_eq!(hit(&l, 1010.0, 750.0), Hit::Element(FindClose));
        assert_eq!(hit(&l, 700.0, 750.0), Hit::Chrome);
        assert_eq!(hit(&l, 700.0, 733.0), Hit::Page);
        assert!(find.next.right() < find.close.x);

        // On a narrow window the parts still do not overlap.
        let l = ChromeLayout::new(400.0, 300.0, 1, 0, true);
        let find = l.find.unwrap();
        assert!(find.input.width >= 40.0);
        assert!(find.next.right() <= find.close.x);
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        for (w, h, n) in [
            (0.0, 0.0, 0),
            (1.0, 1.0, 1),
            (50.0, 30.0, 500),
            (f32::NAN, 10.0, 3),
        ] {
            let l = ChromeLayout::new(w, h, n, 0, true);
            let _ = l.hit(Point::new(5.0, 5.0));
            let _ = l.hit(Point::new(5.0, 100.0));
            assert!(l.page.height >= 0.0);
        }
        let l = ChromeLayout::new(1024.0, 768.0, 0, 0, false);
        assert!(l.tabs.is_empty());
        assert_eq!(hit(&l, 20.0, 20.0), Hit::Element(NewTab));
    }
}
