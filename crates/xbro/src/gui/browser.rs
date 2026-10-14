//! The browser model behind the window: tabs, navigation and loading,
//! scrolling, zoom, find, focus and redraw bookkeeping. It knows nothing
//! about winit: the app feeds it decoded input (see `events.rs`) and draws
//! it (see `render.rs`); page loads are started through a [`Loader`]
//! callback and completed with [`Browser::on_loaded`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use xbro_engine::{
    ClickAction, Cursor, FindMatch, NavigationRequest, Page, Point, Rect, Size, Url,
};
use xbro_text::FontSystem;

use super::address_bar::{self, AddressBar};
use super::chrome::{ChromeElement, ChromeLayout, CHROME_HEIGHT, FIND_BAR_HEIGHT, UI_TEXT_SIZE};
use super::history::HistoryEntry;
use super::paint::{caret_x, UiFonts};
use super::tabs::{LoadState, NavKind, Refresh, Tab, TabId, TabList};
use super::viewport::{self, Axis, PageArea, Scrollbar};

/// The browser's name, at the end of the window title.
const APP_NAME: &str = "xbro";
/// Longest `<meta refresh>` delay that is honoured, in seconds.
const MAX_REFRESH_DELAY: f32 = 30.0;
/// Shortest delay before a page refreshes itself (avoids reload loops).
const MIN_SELF_REFRESH_DELAY: f32 = 1.0;
/// Longest title kept for a history entry (tab strip, window title).
const MAX_TITLE_CHARS: usize = 300;
/// Longest URL shown whole in the status bubble (bytes).
const MAX_STATUS_URL: usize = 2048;
/// Frame interval of the loading animations.
pub const ANIMATION_INTERVAL: Duration = Duration::from_millis(40);
/// Arrow-key scroll step, CSS px.
pub const SCROLL_STEP: f32 = 40.0;
/// Overlap kept between two pages when scrolling by a page, CSS px.
pub const PAGE_OVERLAP: f32 = 40.0;

/// A page load to start on a background thread.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadJob {
    pub tab_id: TabId,
    pub nav_id: u64,
    pub request: NavigationRequest,
    /// Layout viewport in CSS px.
    pub viewport: Size,
}

/// Starts a load; its page must come back through [`Browser::on_loaded`].
pub type Loader = Box<dyn FnMut(LoadJob)>;

/// Which part of the UI receives typed text.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Focus {
    Page,
    Address,
    Find,
}

/// Window size in device pixels and its scale factor.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Metrics {
    pub width: u32,
    pub height: u32,
    pub dpi: f32,
}

impl Metrics {
    pub fn logical_size(&self) -> Size {
        Size::new(self.width as f32 / self.dpi, self.height as f32 / self.dpi)
    }
}

/// An ongoing mouse drag.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Drag {
    /// Scrollbar thumb, grabbed `grab` logical px after its start.
    Scrollbar {
        axis: Axis,
        grab: f32,
    },
    /// Selecting text in the address bar or the find field.
    SelectAddress,
    SelectFind,
}

pub struct Browser {
    pub fonts: UiFonts,
    pub tabs: TabList,
    pub address: AddressBar,
    pub focus: Focus,
    pub metrics: Metrics,
    /// Horizontal scroll of the find field text (logical px).
    pub find_scroll_x: f32,
    loader: Loader,
    next_nav_id: u64,
    /// Last pointer position in device px (None outside the window).
    pub(super) mouse: Option<(f32, f32)>,
    /// Chrome element under the pointer.
    pub hover: Option<ChromeElement>,
    /// Scrollbar under the pointer (drawn thicker).
    pub scrollbar_hover: Option<Axis>,
    /// Chrome button held down.
    pub pressed: Option<ChromeElement>,
    pub(super) drag: Option<Drag>,
    /// Last left click (time, position, count) for double clicks.
    pub(super) last_click: Option<(Instant, (f32, f32), u32)>,
    /// Link under the pointer (shown in the status bubble).
    pub hovered_link: Option<Url>,
    pub cursor: Cursor,
    needs_redraw: bool,
    page_dirty: bool,
    /// Set when the last tab was closed.
    pub quit: bool,
    /// Origin of the animation clock.
    pub epoch: Instant,
    /// When the loading animation last asked for a frame.
    animation_frame: Instant,
}

impl Browser {
    pub fn new(fonts: Arc<FontSystem>, metrics: Metrics, loader: Loader) -> Self {
        Self {
            fonts: UiFonts::new(fonts),
            tabs: TabList::new(),
            address: AddressBar::default(),
            focus: Focus::Page,
            metrics: sanitize_metrics(metrics),
            find_scroll_x: 0.0,
            loader,
            next_nav_id: 1,
            mouse: None,
            hover: None,
            scrollbar_hover: None,
            pressed: None,
            drag: None,
            last_click: None,
            hovered_link: None,
            cursor: Cursor::Default,
            needs_redraw: true,
            page_dirty: true,
            quit: false,
            epoch: Instant::now(),
            animation_frame: Instant::now(),
        }
    }

    // ---- redraw bookkeeping ----------------------------------------------

    /// Something in the chrome or overlays changed.
    pub fn invalidate(&mut self) {
        self.needs_redraw = true;
    }

    /// The page pixels changed (content, scroll, zoom, size or tab).
    pub fn invalidate_page(&mut self) {
        self.needs_redraw = true;
        self.page_dirty = true;
    }

    /// Returns and clears the "redraw needed" flag.
    pub fn take_needs_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    /// Returns and clears the "page must be re-rasterised" flag.
    pub fn take_page_dirty(&mut self) -> bool {
        std::mem::take(&mut self.page_dirty)
    }

    // ---- geometry ---------------------------------------------------------

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.active()
    }

    fn find_open(&self) -> bool {
        self.active().is_some_and(|t| t.find.open)
    }

    /// Chrome layout for the current window and tabs.
    pub fn layout(&self) -> ChromeLayout {
        let size = self.metrics.logical_size();
        ChromeLayout::new(
            size.width,
            size.height,
            self.tabs.len(),
            self.tabs.active_index(),
            self.find_open(),
        )
        .with_zoom_badge((self.zoom() - 1.0).abs() > 1e-4)
    }

    /// Page area of the active tab.
    pub fn page_area(&self) -> PageArea {
        let (zoom, find) = self
            .active()
            .map_or((1.0, false), |t| (t.zoom, t.find.open));
        area_for(self.metrics, zoom, find)
    }

    /// Window title: the active tab's title, followed by the browser name
    /// (which is not repeated for the home page, itself titled "xbro").
    pub fn window_title(&self) -> String {
        let title = self.active().map(Tab::title).unwrap_or_default();
        let title = title.trim();
        if title.is_empty() || title == APP_NAME {
            APP_NAME.to_string()
        } else {
            format!("{title} — {APP_NAME}")
        }
    }

    /// Text of the status bubble: the hovered link, else the loading URL.
    pub fn status_text(&self) -> Option<String> {
        // Only the start of a huge (`data:`) URL can show in the bubble.
        let shown = |url: &Url| match url.as_str().get(..MAX_STATUS_URL) {
            Some(start) if url.as_str().len() > MAX_STATUS_URL => format!("{start}…"),
            _ => address_bar::readable_url(url),
        };
        if let Some(link) = &self.hovered_link {
            return Some(shown(link));
        }
        let load = self.active()?.loading.as_ref()?;
        Some(format!("Loading {}…", shown(load.request.url())))
    }

    /// Whether the status bubble is shown (cheaper than [`Self::status_text`]).
    pub fn has_status(&self) -> bool {
        self.hovered_link.is_some() || self.active().is_some_and(Tab::is_loading)
    }

    /// Progress (0..1) of the active tab's load for the progress line. The
    /// engine reports no progress, so this eases towards 90 % over time.
    pub fn load_progress(&self, now: Instant) -> Option<f32> {
        let load = self.active()?.loading.as_ref()?;
        let t = now.saturating_duration_since(load.started).as_secs_f32();
        Some(0.1 + 0.8 * (1.0 - (-t / 2.0).exp()))
    }

    // ---- window -------------------------------------------------------------

    /// The window was resized or moved to a screen with another scale factor.
    pub fn resize(&mut self, width: u32, height: u32, dpi: f32) {
        let metrics = sanitize_metrics(Metrics { width, height, dpi });
        if metrics == self.metrics {
            return;
        }
        self.metrics = metrics;
        self.sync_viewport();
        self.update_field_scrolls();
        self.invalidate_page();
    }

    /// Lays the active page out again if its viewport no longer matches the
    /// window (resize, zoom, find bar, tab switch); keeps scroll and find
    /// highlights consistent.
    pub fn sync_viewport(&mut self) {
        let area = self.page_area();
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(page) = tab.page.as_mut() else {
            return;
        };
        let before = page.viewport();
        page.set_viewport(area.viewport());
        if page.viewport() != before {
            if tab.find.active() {
                tab.find
                    .refresh_matches(page.find_matches(tab.find.query.text()));
            }
            self.page_dirty = true;
        }
        tab.scroll = viewport::clamp_scroll(tab.scroll, page.document_size(), page.viewport());
    }

    // ---- navigation -----------------------------------------------------

    /// Opens the first page of the window.
    pub fn open_initial(&mut self, url: Url) {
        let home = url.as_str() == "about:home";
        self.tabs.open();
        self.navigate(NavigationRequest::Get(url), NavKind::New);
        if home {
            self.focus_address();
        }
    }

    /// Loads `url` in the active tab on behalf of the user (address bar,
    /// home button): the address bar shows `url` while it loads.
    pub fn open_url(&mut self, url: Url) {
        let index = self.tabs.active_index();
        self.start_navigation(index, NavigationRequest::Get(url), NavKind::New, true);
    }

    /// Starts a navigation in the active tab.
    pub fn navigate(&mut self, request: NavigationRequest, kind: NavKind) {
        let index = self.tabs.active_index();
        self.navigate_tab(index, request, kind);
    }

    /// Starts a navigation in the tab at `index`. New entries come from the
    /// page (links, forms, refresh); history moves and reloads come from the
    /// browser UI (see [`LoadState::from_ui`]).
    pub fn navigate_tab(&mut self, index: usize, request: NavigationRequest, kind: NavKind) {
        let from_ui = matches!(kind, NavKind::History(_) | NavKind::Reload);
        self.start_navigation(index, request, kind, from_ui);
    }

    fn start_navigation(
        &mut self,
        index: usize,
        request: NavigationRequest,
        kind: NavKind,
        from_ui: bool,
    ) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let area = area_for(self.metrics, tab.zoom, tab.find.open);
        let tab_id = tab.id;
        let new_entry = matches!(kind, NavKind::New | NavKind::Replace);
        if new_entry && self.same_document_navigation(index, &request, kind == NavKind::Replace) {
            return;
        }
        let nav_id = self.next_nav_id;
        self.next_nav_id += 1;
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.refresh = None;
        tab.loading = Some(LoadState {
            nav_id,
            request: request.clone(),
            started: Instant::now(),
            kind,
            from_ui,
        });
        (self.loader)(LoadJob {
            tab_id,
            nav_id,
            request,
            viewport: area.viewport(),
        });
        self.invalidate();
    }

    /// A link to a fragment of the current document: scrolls instead of
    /// reloading and records a history entry (in place of the current one
    /// when `replace` is set). Returns whether it applied.
    fn same_document_navigation(
        &mut self,
        index: usize,
        request: &NavigationRequest,
        replace: bool,
    ) -> bool {
        let NavigationRequest::Get(url) = request else {
            return false;
        };
        let Some(fragment) = url.fragment() else {
            return false;
        };
        let Some(tab) = self.tabs.get(index) else {
            return false;
        };
        let (Some(page), Some(current)) = (tab.page.as_ref(), tab.url()) else {
            return false;
        };
        // The fragment must belong to the page on screen (and to its history
        // entry). A load in progress elsewhere is abandoned: the user chose
        // to stay on this page.
        if !same_document(url, current) || !same_document(url, page.url()) {
            return false;
        }
        let y = page.fragment_offset(fragment);
        let title = tab
            .history
            .current()
            .map(|e| e.title.clone())
            .unwrap_or_default();
        let is_active = index == self.tabs.active_index();
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        tab.loading = None;
        if let Some(entry) = tab.history.current_mut() {
            entry.scroll_y = tab.scroll.y;
        }
        if tab.history.current().map(|e| &e.request) != Some(request) {
            let entry = HistoryEntry::new(request.clone(), title);
            if replace {
                tab.history.replace(entry);
            } else {
                tab.history.push(entry);
            }
        }
        if let (Some(y), Some(page)) = (y, tab.page.as_ref()) {
            tab.scroll = viewport::clamp_scroll(
                Point::new(tab.scroll.x, y),
                page.document_size(),
                page.viewport(),
            );
        }
        if is_active {
            self.invalidate_page();
            self.refresh_hover();
        } else {
            self.invalidate();
        }
        true
    }

    pub fn back(&mut self) {
        if let Some(i) = self.active().and_then(Tab::back_index) {
            self.go_history(i);
        }
    }

    pub fn forward(&mut self) {
        if let Some(i) = self.active().and_then(Tab::forward_index) {
            self.go_history(i);
        }
    }

    /// Goes to history entry `index` of the active tab: an entry of the
    /// same document is just scrolled to, others are loaded again.
    pub fn go_history(&mut self, index: usize) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(entry) = tab.history.get(index).cloned() else {
            return;
        };
        // Both entries must be GETs of the shown document (a POST result at
        // the same URL is a different document).
        let current = tab.history.current().map(|e| &e.request);
        let instant = match (&entry.request, tab.page.as_ref(), current) {
            (NavigationRequest::Get(url), Some(page), Some(NavigationRequest::Get(current))) => {
                same_document(url, page.url()) && same_document(url, current)
            }
            _ => false,
        };
        if instant {
            if let Some(current) = tab.history.current_mut() {
                current.scroll_y = tab.scroll.y;
            }
            tab.history.go_to(index);
            tab.loading = None;
            if let Some(page) = tab.page.as_ref() {
                tab.scroll = viewport::clamp_scroll(
                    Point::new(0.0, entry.scroll_y),
                    page.document_size(),
                    page.viewport(),
                );
            }
            self.invalidate_page();
            self.refresh_hover();
            return;
        }
        self.navigate(entry.request, NavKind::History(index));
    }

    /// Loads the current entry again (or restarts the pending load).
    pub fn reload(&mut self) {
        let Some(tab) = self.active() else {
            return;
        };
        if let Some(entry) = tab.history.current() {
            let request = entry.request.clone();
            self.navigate(request, NavKind::Reload);
        } else if let Some(load) = &tab.loading {
            let (request, kind) = (load.request.clone(), load.kind);
            self.navigate(request, kind);
        }
    }

    /// Abandons the active tab's load (its result will be ignored).
    pub fn stop(&mut self) {
        if let Some(tab) = self.tabs.active_mut() {
            if tab.loading.take().is_some() {
                self.invalidate();
            }
        }
    }

    pub fn home(&mut self) {
        self.open_url(home_url());
    }

    /// A page finished loading on a background thread. Results for closed
    /// tabs or superseded navigations are dropped.
    pub fn on_loaded(&mut self, tab_id: TabId, nav_id: u64, page: Page) {
        let Some(index) = self.tabs.index_of(tab_id) else {
            return;
        };
        let is_active = index == self.tabs.active_index();
        let Some(tab) = self.tabs.by_id_mut(tab_id) else {
            return;
        };
        if tab.loading.as_ref().map(|l| l.nav_id) != Some(nav_id) {
            return;
        }
        let Some(load) = tab.loading.take() else {
            return;
        };
        let area = area_for(self.metrics, tab.zoom, tab.find.open);
        commit(tab, load, page, area.viewport(), Instant::now());
        if is_active {
            if self.focus == Focus::Find {
                self.update_field_scrolls();
            }
            self.hovered_link = None;
            self.invalidate_page();
            self.refresh_hover();
        } else {
            self.invalidate();
        }
    }

    /// Fires due `<meta refresh>` timers and paces the loading animation.
    /// Returns whether a refresh fired.
    pub fn tick(&mut self, now: Instant) -> bool {
        let due: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.loading.is_none() && t.refresh.as_ref().is_some_and(|r| r.at <= now))
            .map(|(i, _)| i)
            .collect();
        let fired = !due.is_empty();
        for index in due {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            // A timer fires once: the page it leads to sets its own.
            let Some(refresh) = tab.refresh.take() else {
                continue;
            };
            let kind = refresh_kind(&refresh.url, tab.page.as_ref().map(Page::url));
            self.navigate_tab(index, NavigationRequest::Get(refresh.url), kind);
        }
        if self.tabs.iter().any(Tab::is_loading)
            && now.saturating_duration_since(self.animation_frame) >= ANIMATION_INTERVAL
        {
            self.animation_frame = now;
            self.invalidate();
        }
        fired
    }

    /// When the event loop should wake up next: the next frame of the
    /// loading animations, or the next refresh timer. The animation clock
    /// does not move with `now`, so a stream of other events cannot keep
    /// postponing the next frame.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let refresh = self
            .tabs
            .iter()
            .filter(|t| t.loading.is_none())
            .filter_map(|t| t.refresh.as_ref().map(|r| r.at))
            .min();
        let animation = self
            .tabs
            .iter()
            .any(Tab::is_loading)
            .then(|| (self.animation_frame + ANIMATION_INTERVAL).max(now));
        match (refresh, animation) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    // ---- tabs -------------------------------------------------------------

    /// Opens `url` in a new tab, in the foreground or behind the active tab.
    pub fn new_tab(&mut self, url: Url, background: bool) {
        let id = if background {
            self.tabs.open_background()
        } else {
            self.tabs.open()
        };
        if let Some(index) = self.tabs.index_of(id) {
            self.navigate_tab(index, NavigationRequest::Get(url), NavKind::New);
        }
        if background {
            self.invalidate();
        } else {
            self.tab_switched();
        }
    }

    /// Ctrl+T: a new tab on the home page, with the address bar focused.
    pub fn new_tab_with_address(&mut self) {
        self.new_tab(home_url(), false);
        self.focus_address();
    }

    pub fn close_tab(&mut self, index: usize) {
        let was_active = index == self.tabs.active_index();
        if self.tabs.close(index).is_none() {
            return;
        }
        if self.tabs.is_empty() {
            self.quit = true;
            return;
        }
        if was_active {
            self.tab_switched();
        } else {
            self.invalidate();
        }
    }

    pub fn select_tab(&mut self, index: usize) {
        if self.tabs.select(index) {
            self.tab_switched();
        }
    }

    pub fn next_tab(&mut self) {
        if self.tabs.select_next() {
            self.tab_switched();
        }
    }

    pub fn prev_tab(&mut self) {
        if self.tabs.select_prev() {
            self.tab_switched();
        }
    }

    pub fn last_tab(&mut self) {
        if self.tabs.select_last() {
            self.tab_switched();
        }
    }

    /// Resets per-tab UI state after the active tab changed.
    fn tab_switched(&mut self) {
        self.address.blur();
        self.focus = Focus::Page;
        self.drag = None;
        self.pressed = None;
        self.hovered_link = None;
        self.sync_viewport();
        self.invalidate_page();
        self.refresh_hover();
    }

    // ---- focus --------------------------------------------------------------

    /// Focuses the address bar with its text selected.
    pub fn focus_address(&mut self) {
        let text = address_bar::display_text(self.active().and_then(Tab::display_url));
        // The field starts scrolled to the left, as other browsers do: the
        // scheme and domain of a long URL stay visible (the caret, at the
        // end of the selection, is revealed once it moves).
        self.address.focus(&text);
        self.focus = Focus::Address;
        self.invalidate();
    }

    /// Gives the keyboard back to the page (the address bar shows the URL again).
    pub fn focus_page(&mut self) {
        if self.focus != Focus::Page {
            self.address.blur();
            self.focus = Focus::Page;
            self.invalidate();
        }
    }

    // ---- scrolling and zoom -------------------------------------------------

    /// Scrolls the active page to `p` (clamped). Returns whether it moved.
    pub fn scroll_to(&mut self, p: Point) -> bool {
        let Some(tab) = self.tabs.active_mut() else {
            return false;
        };
        let Some(page) = tab.page.as_ref() else {
            return false;
        };
        let clamped = viewport::clamp_scroll(p, page.document_size(), page.viewport());
        if clamped == tab.scroll {
            return false;
        }
        tab.scroll = clamped;
        self.invalidate_page();
        self.refresh_hover();
        true
    }

    pub fn scroll_by(&mut self, dx: f32, dy: f32) -> bool {
        let Some(scroll) = self.active().map(|t| t.scroll) else {
            return false;
        };
        self.scroll_to(Point::new(scroll.x + dx, scroll.y + dy))
    }

    /// Height of the active page's viewport in CSS px.
    pub fn viewport_size(&self) -> Size {
        self.page_area().viewport()
    }

    /// Scrolls a find match into view (centred): matches in fixed content
    /// are always in view, the page does not move for them.
    fn reveal_match(&mut self, m: FindMatch) {
        if !m.fixed {
            self.reveal(m.rect, true);
        }
    }

    /// Scrolls so that a document rect is visible.
    pub fn reveal(&mut self, rect: Rect, center: bool) {
        let Some(tab) = self.active() else {
            return;
        };
        let Some(page) = tab.page.as_ref() else {
            return;
        };
        let target = viewport::scroll_into_view(
            rect,
            tab.scroll,
            page.viewport(),
            page.document_size(),
            center,
        );
        self.scroll_to(target);
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let zoom = zoom.clamp(viewport::ZOOM_STEPS[0], viewport::ZOOM_STEPS[11]);
        if (tab.zoom - zoom).abs() < 1e-4 {
            return;
        }
        tab.zoom = zoom;
        self.sync_viewport();
        // The zoom badge narrows the address field's text.
        self.update_field_scrolls();
        self.invalidate_page();
        self.refresh_hover();
    }

    pub fn zoom(&self) -> f32 {
        self.active().map_or(1.0, |t| t.zoom)
    }

    /// The scrollbar of the active page on `axis`, in logical px.
    pub fn scrollbar(&self, axis: Axis) -> Option<Scrollbar> {
        let tab = self.active()?;
        let page = tab.page.as_ref()?;
        Scrollbar::new(
            axis,
            self.layout().page,
            page.document_size(),
            page.viewport(),
            tab.scroll,
        )
    }

    // ---- find -------------------------------------------------------------

    /// Ctrl+F: shows the find bar with its field focused.
    pub fn open_find(&mut self) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let was_open = tab.find.open;
        tab.find.open();
        self.address.blur();
        self.focus = Focus::Find;
        if !was_open {
            self.sync_viewport();
            self.run_find();
        }
        self.update_field_scrolls();
        self.invalidate_page();
    }

    pub fn close_find(&mut self) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        tab.find.close();
        if self.focus == Focus::Find {
            self.focus = Focus::Page;
        }
        self.sync_viewport();
        self.invalidate_page();
    }

    /// Searches the active page for the find query and shows the first
    /// match from the top of the view.
    pub fn run_find(&mut self) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let matches = match (&tab.page, tab.find.query.text()) {
            (Some(page), query) if !query.is_empty() => page.find_matches(query),
            _ => Vec::new(),
        };
        let current = tab.find.set_matches(matches, tab.scroll);
        self.invalidate();
        if let Some(m) = current {
            self.reveal_match(m);
        }
    }

    /// Enter / Shift+Enter in the find bar.
    pub fn find_step(&mut self, backwards: bool) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let current = if backwards {
            tab.find.prev()
        } else {
            tab.find.next()
        };
        self.invalidate();
        if let Some(m) = current {
            self.reveal_match(m);
        }
    }

    /// Re-runs the search after the page changed (layout may have moved matches).
    pub(super) fn refresh_find(&mut self) {
        if let Some(tab) = self.tabs.active_mut() {
            if let (true, Some(page)) = (tab.find.active(), tab.page.as_ref()) {
                tab.find
                    .refresh_matches(page.find_matches(tab.find.query.text()));
            }
        }
    }

    // ---- page interaction -----------------------------------------------

    /// Carries out the result of a click (or Enter) on page content.
    /// `link` is the link under the click, used to record fragment
    /// navigations in the history.
    pub fn apply_click(&mut self, action: ClickAction, link: Option<Url>) {
        match action {
            ClickAction::None => {}
            ClickAction::Navigate(request) => self.navigate(request, NavKind::New),
            ClickAction::ScrollTo(y) => {
                let index = self.tabs.active_index();
                let recorded = link.filter(|l| l.fragment().is_some()).is_some_and(|l| {
                    self.same_document_navigation(index, &NavigationRequest::Get(l), false)
                });
                if !recorded {
                    let x = self.active().map_or(0.0, |t| t.scroll.x);
                    self.scroll_to(Point::new(x, y));
                }
            }
            ClickAction::Repaint => {
                self.refresh_find();
                self.invalidate_page();
            }
        }
    }

    /// Tab / Shift+Tab: moves the page focus and scrolls it into view.
    pub fn focus_next(&mut self, backwards: bool) {
        let rect = self
            .tabs
            .active_mut()
            .and_then(|t| t.page.as_mut())
            .and_then(|p| p.focus_next(backwards));
        self.invalidate_page();
        if let Some(rect) = rect {
            self.reveal(rect, false);
        }
    }

    // ---- text fields ------------------------------------------------------

    /// Keeps the carets of the address bar and find field visible.
    pub fn update_field_scrolls(&mut self) {
        let layout = self.layout();
        let dpi = self.metrics.dpi;
        if self.address.focused {
            let inner = layout.address_text();
            let stops = self
                .fonts
                .caret_stops(self.address.edit.text(), UI_TEXT_SIZE, dpi);
            let caret = caret_x(&stops, self.address.edit.caret());
            let width = stops.last().map_or(0.0, |s| s.1);
            self.address.scroll_x =
                address_bar::scroll_to_caret(self.address.scroll_x, caret, width, inner.width);
        }
        if let (Some(find), Some(tab)) = (layout.find, self.tabs.active()) {
            let inner = find.input_text();
            let stops = self
                .fonts
                .caret_stops(tab.find.query.text(), UI_TEXT_SIZE, dpi);
            let caret = caret_x(&stops, tab.find.query.caret());
            let width = stops.last().map_or(0.0, |s| s.1);
            self.find_scroll_x =
                address_bar::scroll_to_caret(self.find_scroll_x, caret, width, inner.width);
        }
    }
}

pub fn home_url() -> Url {
    xbro_net::parse_user_input("about:home")
}

fn sanitize_metrics(m: Metrics) -> Metrics {
    let dpi = if m.dpi.is_finite() && m.dpi > 0.0 {
        m.dpi.clamp(0.25, 8.0)
    } else {
        1.0
    };
    Metrics {
        width: m.width.max(1),
        height: m.height.max(1),
        dpi,
    }
}

/// Page area of a tab with `zoom` and the find bar open or not.
fn area_for(metrics: Metrics, zoom: f32, find_open: bool) -> PageArea {
    let dpi = metrics.dpi;
    let top = (CHROME_HEIGHT * dpi).round() as u32;
    let bottom = if find_open {
        (FIND_BAR_HEIGHT * dpi).round() as u32
    } else {
        0
    };
    PageArea::new(metrics.width, metrics.height, top, bottom, dpi, zoom)
}

/// How a due `<meta refresh>` to `url` navigates a tab showing `page_url`.
/// A fragment of the document is scrolled to (reloading would start the
/// timer again, every second, forever), the document itself is reloaded
/// (auto-refreshing pages), and any other page replaces the current history
/// entry, so that Back does not return to a page that redirects again.
fn refresh_kind(url: &Url, page_url: Option<&Url>) -> NavKind {
    match page_url {
        Some(page_url) if url.fragment().is_none() && same_document(url, page_url) => {
            NavKind::Reload
        }
        _ => NavKind::Replace,
    }
}

/// Whether two URLs name the same document (they differ at most by fragment).
pub fn same_document(a: &Url, b: &Url) -> bool {
    let strip = |u: &Url| {
        let s = u.as_str();
        s.split_once('#').map_or(s, |(base, _)| base).to_string()
    };
    strip(a) == strip(b)
}

/// The request recorded in the history for a load: after a redirect, a GET
/// of the final URL (keeping the requested fragment).
fn history_request(request: &NavigationRequest, final_url: &Url) -> NavigationRequest {
    if same_document(request.url(), final_url) {
        return request.clone();
    }
    let mut url = final_url.clone();
    if url.fragment().is_none() {
        url.set_fragment(request.url().fragment());
    }
    NavigationRequest::Get(url)
}

/// Puts a freshly loaded page into its tab and updates the history.
fn commit(tab: &mut Tab, load: LoadState, mut page: Page, viewport: Size, now: Instant) {
    page.set_viewport(viewport);
    // Remember where the page being left was scrolled to.
    if tab.page.is_some() {
        if let Some(entry) = tab.history.current_mut() {
            entry.scroll_y = tab.scroll.y;
        }
    }
    // Titles are drawn every frame: keep them reasonably short.
    let title: String = page.title().chars().take(MAX_TITLE_CHARS).collect();
    let request = history_request(&load.request, page.url());
    let fragment = load
        .request
        .url()
        .fragment()
        .or_else(|| page.url().fragment())
        .filter(|f| !f.is_empty())
        .map(str::to_string);
    let fragment_y = || {
        fragment
            .as_deref()
            .and_then(|f| page.fragment_offset(f))
            .unwrap_or(0.0)
    };

    let kind = if tab.history.is_empty() {
        NavKind::New
    } else {
        load.kind
    };
    let target_y = match kind {
        NavKind::New | NavKind::Replace => {
            let y = fragment_y();
            let mut entry = HistoryEntry::new(request, title);
            entry.scroll_y = y;
            // Loading the current URL again replaces its entry.
            let same = tab
                .history
                .current()
                .is_some_and(|c| c.request == entry.request);
            if kind == NavKind::Replace || same {
                tab.history.replace(entry);
            } else {
                tab.history.push(entry);
            }
            y
        }
        NavKind::Reload | NavKind::History(_) => {
            if let NavKind::History(i) = kind {
                tab.history.go_to(i);
            }
            match tab.history.current_mut() {
                Some(entry) => {
                    entry.title = title;
                    entry.request = request;
                    entry.scroll_y
                }
                None => 0.0,
            }
        }
    };
    tab.scroll = viewport::clamp_scroll(
        Point::new(0.0, target_y),
        page.document_size(),
        page.viewport(),
    );
    tab.refresh = page.meta_refresh().and_then(|(delay, url)| {
        if !(delay.is_finite() && (0.0..=MAX_REFRESH_DELAY).contains(&delay)) {
            return None;
        }
        let delay = if same_document(&url, page.url()) {
            delay.max(MIN_SELF_REFRESH_DELAY)
        } else {
            delay
        };
        Some(Refresh {
            at: now + Duration::from_secs_f32(delay),
            url,
        })
    });
    if tab.find.active() {
        let matches = page.find_matches(tab.find.query.text());
        tab.find.set_matches(matches, tab.scroll);
    } else {
        tab.find.matches.clear();
        tab.find.current = None;
    }
    tab.page = Some(page);
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::OnceLock;
    use xbro_engine::Engine;

    pub fn engine() -> Engine {
        static ENGINE: OnceLock<Engine> = OnceLock::new();
        ENGINE.get_or_init(Engine::new).clone()
    }

    /// A browser whose loads are recorded instead of started.
    pub fn browser() -> (Browser, Rc<RefCell<Vec<LoadJob>>>) {
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let sink = jobs.clone();
        let metrics = Metrics {
            width: 800,
            height: 600,
            dpi: 1.0,
        };
        let browser = Browser::new(
            engine().fonts,
            metrics,
            Box::new(move |job| sink.borrow_mut().push(job)),
        );
        (browser, jobs)
    }

    pub fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// A tall test page: `n` paragraphs, each with an id.
    pub fn html(title: &str, n: usize) -> String {
        let mut s = format!(
            "<title>{title}</title><style>p{{height:100px;margin:0}}</style>\
             <a id=top href=#p20>jump</a>"
        );
        for i in 0..n {
            s.push_str(&format!("<p id=p{i}>para {i} needle</p>"));
        }
        s
    }

    /// Completes the most recent recorded load with an HTML page.
    pub fn finish(b: &mut Browser, jobs: &Rc<RefCell<Vec<LoadJob>>>, title: &str, n: usize) {
        let job = jobs.borrow_mut().pop().expect("a pending load");
        let page = engine().load_html(&html(title, n), job.request.url().clone(), job.viewport);
        b.on_loaded(job.tab_id, job.nav_id, page);
    }

    fn titles(b: &Browser) -> Vec<String> {
        b.active()
            .unwrap()
            .history
            .entries()
            .iter()
            .map(|e| e.title.clone())
            .collect()
    }

    #[test]
    fn navigation_builds_history_and_restores_scroll() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        assert!(b.active().unwrap().is_loading());
        assert_eq!(jobs.borrow()[0].viewport, Size::new(800.0, 526.0));
        finish(&mut b, &jobs, "A", 50);
        assert_eq!(b.window_title(), "A — xbro");
        assert!(!b.active().unwrap().is_loading());
        assert!(b.scroll_by(0.0, 700.0));

        b.open_url(url("https://b.test/"));
        // The old page stays until the new one arrives.
        assert_eq!(b.active().unwrap().page.as_ref().unwrap().title(), "A");
        finish(&mut b, &jobs, "B", 50);
        assert_eq!(titles(&b), ["A", "B"]);
        assert_eq!(b.active().unwrap().scroll.y, 0.0);
        b.scroll_by(0.0, 300.0);

        b.back();
        assert_eq!(
            jobs.borrow().last().unwrap().request.url().as_str(),
            "https://a.test/"
        );
        finish(&mut b, &jobs, "A", 50);
        let tab = b.active().unwrap();
        assert_eq!(tab.history.index(), 0);
        assert_eq!(tab.scroll.y, 700.0, "scroll restored");
        assert_eq!(tab.forward_index(), Some(1));

        b.forward();
        finish(&mut b, &jobs, "B", 50);
        assert_eq!(b.active().unwrap().scroll.y, 300.0);

        // Reload keeps the position and does not add an entry.
        b.scroll_by(0.0, 100.0);
        b.reload();
        finish(&mut b, &jobs, "B2", 50);
        assert_eq!(titles(&b), ["A", "B2"]);
        assert_eq!(b.active().unwrap().scroll.y, 400.0);

        // A new navigation from the middle of the history truncates it.
        b.back();
        finish(&mut b, &jobs, "A", 50);
        b.open_url(url("https://c.test/"));
        finish(&mut b, &jobs, "C", 5);
        assert_eq!(titles(&b), ["A", "C"]);
        assert_eq!(b.active().unwrap().forward_index(), None);
    }

    #[test]
    fn the_address_bar_shows_urls_loaded_from_the_ui_at_once() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 3);
        let shown = |b: &Browser| address_bar::display_text(b.active().unwrap().display_url());
        // Typed in the address bar: shown while it loads (and when the bar
        // is focused again), not replaced by the page still on screen.
        b.open_url(url("https://typed.test/caf%C3%A9"));
        assert_eq!(shown(&b), "https://typed.test/café");
        b.focus_address();
        assert_eq!(b.address.edit.text(), "https://typed.test/café");
        b.focus_page();
        assert_eq!(
            b.status_text().as_deref(),
            Some("Loading https://typed.test/café…")
        );
        // Stopped: the bar shows the page on screen again.
        b.stop();
        assert_eq!(shown(&b), "https://a.test/");
        // A link keeps showing the current page until the new one arrives.
        let link = NavigationRequest::Get(url("https://link.test/"));
        b.apply_click(ClickAction::Navigate(link), None);
        assert_eq!(shown(&b), "https://a.test/");
        finish(&mut b, &jobs, "Link", 3);
        assert_eq!(shown(&b), "https://link.test/");
        // Back and forward show the entry they go to.
        b.back();
        assert_eq!(shown(&b), "https://a.test/");
        finish(&mut b, &jobs, "A", 3);
        b.forward();
        assert_eq!(shown(&b), "https://link.test/");
    }

    #[test]
    fn the_status_bubble_shows_the_start_of_huge_urls() {
        let (mut b, jobs) = browser();
        assert!(!b.has_status());
        b.open_initial(url("https://a.test/"));
        assert!(b.has_status(), "loading");
        finish(&mut b, &jobs, "A", 3);
        assert!(!b.has_status());
        assert_eq!(b.status_text(), None);
        let huge = url(&format!("data:text/plain,{}", "x".repeat(1_000_000)));
        b.hovered_link = Some(huge.clone());
        assert!(b.has_status());
        let text = b.status_text().unwrap();
        assert!(
            text.len() <= MAX_STATUS_URL + '…'.len_utf8(),
            "{}",
            text.len()
        );
        assert!(text.starts_with("data:text/plain,xxx") && text.ends_with('…'));
        b.hovered_link = Some(url("https://a.test/caf%C3%A9"));
        assert_eq!(b.status_text().as_deref(), Some("https://a.test/café"));
    }

    #[test]
    fn stale_and_stopped_loads_are_ignored() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 3);
        b.open_url(url("https://slow.test/"));
        let slow = jobs.borrow_mut().pop().unwrap();
        b.open_url(url("https://fast.test/"));
        finish(&mut b, &jobs, "Fast", 3);
        // The superseded load arrives late: dropped.
        let page = engine().load_html(
            "<title>Slow</title>",
            slow.request.url().clone(),
            slow.viewport,
        );
        b.on_loaded(slow.tab_id, slow.nav_id, page);
        assert_eq!(b.window_title(), "Fast — xbro");

        b.open_url(url("https://stopped.test/"));
        b.stop();
        assert!(!b.active().unwrap().is_loading());
        finish(&mut b, &jobs, "Stopped", 3);
        assert_eq!(b.window_title(), "Fast — xbro");

        // Results for a closed tab are dropped too.
        b.new_tab(url("https://bg.test/"), true);
        let bg = jobs.borrow_mut().pop().unwrap();
        b.close_tab(1);
        let page = engine().load_html("<title>BG</title>", bg.request.url().clone(), bg.viewport);
        b.on_loaded(bg.tab_id, bg.nav_id, page);
        assert_eq!(b.tabs.len(), 1);
    }

    #[test]
    fn tabs_open_close_and_quit() {
        let (mut b, jobs) = browser();
        b.open_initial(home_url());
        assert_eq!(b.focus, Focus::Address, "home page focuses the address bar");
        assert_eq!(b.address.edit.text(), "");
        finish(&mut b, &jobs, "Home", 1);
        b.new_tab(url("https://x.test/"), true);
        assert_eq!(b.tabs.len(), 2);
        assert_eq!(b.tabs.active_index(), 0, "background tab");
        b.new_tab_with_address();
        assert_eq!(b.tabs.active_index(), 2);
        assert_eq!(b.focus, Focus::Address);
        b.next_tab();
        assert_eq!(b.tabs.active_index(), 0);
        assert_eq!(b.focus, Focus::Page);
        b.prev_tab();
        b.last_tab();
        assert_eq!(b.tabs.active_index(), 2);
        b.close_tab(2);
        b.close_tab(0);
        assert!(!b.quit);
        b.close_tab(0);
        assert!(b.quit, "closing the last tab exits");
    }

    #[test]
    fn fragments_scroll_and_same_document_links_skip_loading() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/doc#p10"));
        finish(&mut b, &jobs, "Doc", 50);
        let tab = b.active().unwrap();
        let p10 = tab.page.as_ref().unwrap().fragment_offset("p10").unwrap();
        assert!(p10 > 0.0);
        assert_eq!(tab.scroll.y, p10);
        // A same-document fragment link scrolls without a load and is recorded.
        let pending = jobs.borrow().len();
        b.open_url(url("https://a.test/doc#p20"));
        assert_eq!(jobs.borrow().len(), pending, "no load");
        let tab = b.active().unwrap();
        assert_eq!(tab.history.len(), 2);
        assert!(tab.scroll.y > p10);
        assert_eq!(tab.url().unwrap().fragment(), Some("p20"));
        b.back();
        assert_eq!(
            jobs.borrow().len(),
            pending,
            "back within the document: no load"
        );
        assert_eq!(b.active().unwrap().scroll.y, p10);
    }

    #[test]
    fn a_fragment_link_during_a_load_scrolls_and_cancels_it() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/doc"));
        finish(&mut b, &jobs, "Doc", 40);
        b.open_url(url("https://slow.test/"));
        let slow = jobs.borrow_mut().pop().unwrap();
        // The user changes their mind and follows a link of the shown page.
        b.open_url(url("https://a.test/doc#p20"));
        assert!(jobs.borrow().is_empty(), "no reload of the shown page");
        let tab = b.active().unwrap();
        assert!(!tab.is_loading(), "the pending load is abandoned");
        assert_eq!(tab.url().unwrap().fragment(), Some("p20"));
        let p20 = tab.page.as_ref().unwrap().fragment_offset("p20").unwrap();
        assert_eq!(tab.scroll.y, p20);
        let page = engine().load_html(
            "<title>Slow</title>",
            slow.request.url().clone(),
            slow.viewport,
        );
        b.on_loaded(slow.tab_id, slow.nav_id, page);
        assert_eq!(b.window_title(), "Doc — xbro", "its late result is dropped");
        assert_eq!(titles(&b), ["Doc", "Doc"]);
    }

    #[test]
    fn zoom_resize_and_scroll_clamping() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 30);
        assert_eq!(b.viewport_size(), Size::new(800.0, 526.0));
        b.set_zoom(viewport::zoom_in(b.zoom()));
        assert_eq!(b.zoom(), 1.1);
        let vp = b.active().unwrap().page.as_ref().unwrap().viewport();
        assert!((vp.width - 800.0 / 1.1).abs() < 0.01, "{vp:?}");
        b.set_zoom(1.0);
        b.scroll_to(Point::new(0.0, 1e9));
        let doc = b.active().unwrap().page.as_ref().unwrap().document_size();
        assert_eq!(b.active().unwrap().scroll.y, doc.height - 526.0);
        // Growing the window clamps the scroll again.
        b.resize(1600, 1200, 2.0);
        assert_eq!(b.viewport_size(), Size::new(800.0, 526.0));
        b.resize(800, 2000, 1.0);
        let tab = b.active().unwrap();
        let page = tab.page.as_ref().unwrap();
        assert_eq!(
            tab.scroll.y,
            (page.document_size().height - page.viewport().height).max(0.0)
        );
    }

    #[test]
    fn find_highlights_and_steps() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        finish(&mut b, &jobs, "A", 40);
        b.open_find();
        assert_eq!(b.focus, Focus::Find);
        assert_eq!(
            b.viewport_size().height,
            526.0 - 34.0,
            "find bar shrinks the page"
        );
        b.tabs.active_mut().unwrap().find.query.insert("needle");
        b.run_find();
        let tab = b.active().unwrap();
        assert_eq!(tab.find.matches.len(), 40);
        assert_eq!(tab.find.counter(), "1/40");
        b.find_step(true);
        let tab = b.active().unwrap();
        assert_eq!(tab.find.counter(), "40/40");
        assert!(tab.scroll.y > 3000.0, "last match scrolled into view");
        b.close_find();
        assert_eq!(b.focus, Focus::Page);
        assert_eq!(b.viewport_size().height, 526.0);
        assert!(b.active().unwrap().find.matches.is_empty());
    }

    /// A tall page whose fixed header contains the find query.
    pub fn fixed_header_page() -> String {
        format!(
            "<div id=bar style='position:fixed;top:0;left:0;right:0;height:40px;\
             background:white'>fixed needle</div>{}",
            html("Fixed", 40)
        )
    }

    #[test]
    fn find_matches_in_fixed_content_do_not_scroll() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        respond(&mut b, &jobs, &fixed_header_page());
        b.scroll_to(Point::new(0.0, 1500.0));
        b.open_find();
        b.tabs.active_mut().unwrap().find.query.insert("needle");
        b.run_find();
        let tab = b.active().unwrap();
        assert_eq!(tab.find.matches.len(), 41);
        // The header match comes first in document order and is in view.
        let current = tab.find.current_match().unwrap();
        assert!(current.fixed, "{current:?}");
        assert_eq!(tab.find.counter(), "1/41");
        assert_eq!(tab.scroll.y, 1500.0, "no jump to the top");
        // The next match is in the flow, at the top of the document.
        b.find_step(false);
        let tab = b.active().unwrap();
        assert!(!tab.find.current_match().unwrap().fixed);
        assert_eq!(tab.scroll.y, 0.0);
        b.scroll_to(Point::new(0.0, 2000.0));
        // Stepping back to the header match leaves the scroll alone.
        b.find_step(true);
        let tab = b.active().unwrap();
        assert!(tab.find.current_match().unwrap().fixed);
        assert_eq!(tab.scroll.y, 2000.0);
    }

    #[test]
    fn meta_refresh_navigates_after_the_delay() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/"));
        let job = jobs.borrow_mut().pop().unwrap();
        let html = "<meta http-equiv=refresh content='2; url=https://b.test/'>";
        let page = engine().load_html(html, job.request.url().clone(), job.viewport);
        b.on_loaded(job.tab_id, job.nav_id, page);
        let now = Instant::now();
        let at = b.active().unwrap().refresh.as_ref().unwrap().at;
        assert!(at > now + Duration::from_secs(1));
        assert_eq!(b.next_deadline(now), Some(at));
        assert!(!b.tick(now));
        assert!(b.tick(at));
        assert_eq!(
            jobs.borrow().last().unwrap().request.url().as_str(),
            "https://b.test/"
        );
        // While loading, the deadline is the next animation frame.
        assert_eq!(b.next_deadline(at), Some(at + ANIMATION_INTERVAL));
        // The redirect replaces the redirecting page in the history.
        finish(&mut b, &jobs, "B", 3);
        let tab = b.active().unwrap();
        assert_eq!(titles(&b), ["B"]);
        assert_eq!(tab.back_index(), None);
        assert!(tab.refresh.is_none());
    }

    /// Loads `html` as the response to the most recent recorded load.
    fn respond(b: &mut Browser, jobs: &Rc<RefCell<Vec<LoadJob>>>, html: &str) {
        let job = jobs.borrow_mut().pop().expect("a pending load");
        let page = engine().load_html(html, job.request.url().clone(), job.viewport);
        b.on_loaded(job.tab_id, job.nav_id, page);
    }

    #[test]
    fn meta_refresh_to_a_fragment_scrolls_once_without_reloading() {
        // Regression: the timer was not cleared by the same-document
        // navigation and the next firing reloaded the page, which then
        // refreshed again: one reload per second, forever.
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/start"));
        finish(&mut b, &jobs, "Start", 1);
        b.open_url(url("https://a.test/doc"));
        let page = format!(
            "<meta http-equiv=refresh content='0; url=#p20'>{}",
            html("Doc", 40)
        );
        respond(&mut b, &jobs, &page);
        let at = b.active().unwrap().refresh.as_ref().unwrap().at;
        assert!(b.tick(at));
        assert!(jobs.borrow().is_empty(), "no load for a fragment");
        let tab = b.active().unwrap();
        assert!(tab.refresh.is_none(), "a timer fires once");
        assert_eq!(tab.url().unwrap().fragment(), Some("p20"));
        assert_eq!(titles(&b), ["Start", "Doc"], "the entry is replaced");
        let p20 = tab.page.as_ref().unwrap().fragment_offset("p20").unwrap();
        assert_eq!(tab.scroll.y, p20);
        assert_eq!(b.next_deadline(at), None, "nothing left to wake up for");
        assert!(!b.tick(at + Duration::from_secs(5)));
        assert!(jobs.borrow().is_empty());

        // Reloading the "#p20" entry: the refresh scrolls again, no reload loop.
        b.reload();
        let page = format!(
            "<meta http-equiv=refresh content='0; url=#p20'>{}",
            html("Doc", 40)
        );
        respond(&mut b, &jobs, &page);
        let at = b.active().unwrap().refresh.as_ref().unwrap().at;
        assert!(b.tick(at));
        assert!(jobs.borrow().is_empty());
        assert!(b.active().unwrap().refresh.is_none());
    }

    #[test]
    fn meta_refresh_without_a_target_reloads_the_page() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/start"));
        finish(&mut b, &jobs, "Start", 1);
        b.open_url(url("https://a.test/live"));
        respond(
            &mut b,
            &jobs,
            "<title>Live</title><meta http-equiv=refresh content=5>",
        );
        let at = b.active().unwrap().refresh.as_ref().unwrap().at;
        assert!(b.tick(at));
        let job = jobs.borrow().last().cloned().expect("a reload");
        assert_eq!(job.request.url().as_str(), "https://a.test/live");
        respond(
            &mut b,
            &jobs,
            "<title>Live 2</title><meta http-equiv=refresh content=5>",
        );
        assert_eq!(titles(&b), ["Start", "Live 2"], "a reload adds no entry");
        assert!(b.active().unwrap().refresh.is_some(), "and refreshes again");
    }

    #[test]
    fn input_events_do_not_postpone_the_loading_animation() {
        let (mut b, _jobs) = browser();
        b.open_initial(url("https://a.test/"));
        let t0 = Instant::now();
        b.tick(t0);
        b.take_needs_redraw();
        let deadline = b.next_deadline(t0).unwrap();
        // Events keep arriving before the deadline: it does not move.
        let soon = t0 + ANIMATION_INTERVAL / 2;
        assert_eq!(b.next_deadline(soon), Some(deadline));
        b.tick(soon);
        assert!(!b.take_needs_redraw(), "too early for a frame");
        // Once it is reached (even without a timeout wake-up), a frame is drawn.
        b.tick(deadline);
        assert!(b.take_needs_redraw());
        assert_eq!(
            b.next_deadline(deadline),
            Some(deadline + ANIMATION_INTERVAL)
        );
        // A late check asks for a frame right away, not in the past.
        let late = deadline + ANIMATION_INTERVAL * 10;
        assert_eq!(b.next_deadline(late), Some(late));
    }

    #[test]
    fn window_title_names_the_page_and_the_browser_once() {
        let (mut b, jobs) = browser();
        assert_eq!(b.window_title(), "xbro");
        b.open_initial(home_url());
        respond(&mut b, &jobs, "<title>xbro</title>");
        assert_eq!(b.window_title(), "xbro");
        b.open_url(url("https://a.test/"));
        respond(&mut b, &jobs, "<title> Page </title>");
        assert_eq!(b.window_title(), "Page — xbro");
    }

    #[test]
    fn back_from_a_post_result_at_the_same_url_reloads() {
        let (mut b, jobs) = browser();
        b.open_initial(url("https://a.test/form"));
        finish(&mut b, &jobs, "Form", 3);
        let post = NavigationRequest::Post {
            url: url("https://a.test/form"),
            fields: vec![("q".into(), "x".into())],
        };
        b.navigate(post, NavKind::New);
        finish(&mut b, &jobs, "Result", 3);
        assert_eq!(b.active().unwrap().history.len(), 2);
        b.back();
        let job = jobs.borrow().last().cloned().expect("a load, not a scroll");
        assert_eq!(
            job.request,
            NavigationRequest::Get(url("https://a.test/form"))
        );
    }

    #[test]
    fn redirects_are_recorded_with_the_final_url() {
        let req = NavigationRequest::Post {
            url: url("https://a.test/form"),
            fields: vec![],
        };
        let recorded = history_request(&req, &url("https://a.test/done"));
        assert_eq!(recorded, NavigationRequest::Get(url("https://a.test/done")));
        assert_eq!(history_request(&req, &url("https://a.test/form")), req);
        let get = NavigationRequest::Get(url("http://a.test/x#frag"));
        assert_eq!(
            history_request(&get, &url("https://a.test/x")),
            NavigationRequest::Get(url("https://a.test/x#frag"))
        );
        assert!(same_document(
            &url("https://a.test/x#a"),
            &url("https://a.test/x")
        ));
        assert!(!same_document(
            &url("https://a.test/x"),
            &url("https://a.test/y")
        ));
    }
}
