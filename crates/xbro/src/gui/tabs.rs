//! Tabs: one page with its history, scroll position, zoom and find state,
//! and the ordered list of tabs with the active one.

use std::time::Instant;

use xbro_engine::{NavigationRequest, Page, Point, Url};

use super::find::FindState;
use super::history::History;

pub type TabId = u64;

/// How a navigation affects the history once its page has loaded.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NavKind {
    /// A new entry (link, address bar, form): forward entries are dropped.
    New,
    /// The new page takes the place of the current entry (`<meta refresh>`
    /// redirects, so that Back does not lead to the redirecting page).
    Replace,
    /// The current entry is loaded again (scroll position kept).
    Reload,
    /// Back/forward to this history index (its scroll position is restored).
    History(usize),
}

/// A navigation in flight.
#[derive(Clone, Debug)]
pub struct LoadState {
    pub nav_id: u64,
    pub request: NavigationRequest,
    pub started: Instant,
    pub kind: NavKind,
    /// Started from the browser UI (address bar, home button, back and
    /// forward): the address bar shows its URL while it loads, as in other
    /// browsers. Loads started by the page (links, forms, `<meta refresh>`)
    /// keep showing the current page's URL until they commit.
    pub from_ui: bool,
}

/// A pending `<meta http-equiv=refresh>`.
#[derive(Clone, Debug, PartialEq)]
pub struct Refresh {
    pub at: Instant,
    pub url: Url,
}

pub struct Tab {
    pub id: TabId,
    pub page: Option<Page>,
    pub history: History,
    pub loading: Option<LoadState>,
    /// Scroll position in CSS px.
    pub scroll: Point,
    pub zoom: f32,
    pub find: FindState,
    pub refresh: Option<Refresh>,
}

impl Tab {
    pub fn new(id: TabId) -> Self {
        Self {
            id,
            page: None,
            history: History::default(),
            loading: None,
            scroll: Point::ZERO,
            zoom: 1.0,
            find: FindState::default(),
            refresh: None,
        }
    }

    /// Title for the tab strip and the window.
    pub fn title(&self) -> String {
        if let Some(entry) = self.history.current() {
            if !entry.title.is_empty() {
                return entry.title.clone();
            }
            return entry.url().to_string();
        }
        match &self.loading {
            Some(load) => load.request.url().to_string(),
            None => "New Tab".to_string(),
        }
    }

    /// URL of the current history entry (what the address bar shows).
    pub fn url(&self) -> Option<&Url> {
        self.history.current().map(|e| e.url())
    }

    /// URL shown in the address bar: the one being loaded when the load
    /// was started from the browser UI (see [`LoadState::from_ui`]) or when
    /// nothing was committed yet, else the committed URL.
    pub fn display_url(&self) -> Option<&Url> {
        match &self.loading {
            Some(load) if load.from_ui => Some(load.request.url()),
            load => self
                .url()
                .or_else(|| load.as_ref().map(|l| l.request.url())),
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// History index that back/forward start from: the pending history
    /// navigation's target, else the current entry.
    pub fn history_position(&self) -> usize {
        match &self.loading {
            Some(LoadState {
                kind: NavKind::History(i),
                ..
            }) => *i,
            _ => self.history.index(),
        }
    }

    pub fn back_index(&self) -> Option<usize> {
        self.history.back_from(self.history_position())
    }

    pub fn forward_index(&self) -> Option<usize> {
        self.history.forward_from(self.history_position())
    }
}

/// The tabs of a window, in strip order, with the active one.
pub struct TabList {
    tabs: Vec<Tab>,
    active: usize,
    next_id: TabId,
    /// Background tabs opened in a row from the active tab (they are placed
    /// one after the other, to its right).
    opened_from_active: usize,
}

impl Default for TabList {
    fn default() -> Self {
        Self::new()
    }
}

impl TabList {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            opened_from_active: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn get(&self, index: usize) -> Option<&Tab> {
        self.tabs.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Tab> {
        self.tabs.get_mut(index)
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn by_id_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Appends a new tab at the end and activates it.
    pub fn open(&mut self) -> TabId {
        let id = self.allocate_id();
        self.tabs.push(Tab::new(id));
        self.select(self.tabs.len() - 1);
        id
    }

    /// Opens a tab in the background, to the right of the active tab (after
    /// the ones already opened from it).
    pub fn open_background(&mut self) -> TabId {
        let id = self.allocate_id();
        let at = if self.tabs.is_empty() {
            0
        } else {
            (self.active + 1 + self.opened_from_active).min(self.tabs.len())
        };
        self.tabs.insert(at, Tab::new(id));
        self.opened_from_active += 1;
        id
    }

    /// Closes the tab at `index`. The tab to its right becomes active if it
    /// was the active one (the one to its left for the last tab). Returns
    /// the closed tab.
    pub fn close(&mut self, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        if index < self.active || (index == self.active && self.active == self.tabs.len()) {
            self.active = self.active.saturating_sub(1);
        }
        self.opened_from_active = 0;
        Some(tab)
    }

    /// Activates the tab at `index` (ignored when out of range).
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        if index != self.active {
            self.opened_from_active = 0;
        }
        let changed = index != self.active;
        self.active = index;
        changed
    }

    /// Activates the next tab, wrapping around.
    pub fn select_next(&mut self) -> bool {
        if self.tabs.is_empty() {
            return false;
        }
        self.select((self.active + 1) % self.tabs.len())
    }

    /// Activates the previous tab, wrapping around.
    pub fn select_prev(&mut self) -> bool {
        if self.tabs.is_empty() {
            return false;
        }
        self.select((self.active + self.tabs.len() - 1) % self.tabs.len())
    }

    pub fn select_last(&mut self) -> bool {
        if self.tabs.is_empty() {
            return false;
        }
        self.select(self.tabs.len() - 1)
    }

    fn allocate_id(&mut self) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::history::HistoryEntry;

    fn ids(list: &TabList) -> Vec<TabId> {
        list.iter().map(|t| t.id).collect()
    }

    #[test]
    fn open_select_and_cycle() {
        let mut list = TabList::new();
        assert!(list.active().is_none());
        assert!(!list.select_next());
        let a = list.open();
        let b = list.open();
        let c = list.open();
        assert_eq!(ids(&list), [a, b, c]);
        assert!(a != b && b != c);
        assert_eq!(list.active().unwrap().id, c);
        assert!(list.select_next());
        assert_eq!(list.active_index(), 0, "wraps to the first tab");
        assert!(list.select_prev());
        assert_eq!(list.active_index(), 2, "wraps to the last tab");
        assert!(list.select(1));
        assert!(!list.select(1), "already active");
        assert!(!list.select(9), "out of range");
        assert_eq!(list.active().unwrap().id, b);
        list.select(0);
        assert!(list.select_last());
        assert_eq!(list.active_index(), 2);
        assert_eq!(list.index_of(b), Some(1));
        assert_eq!(list.index_of(999), None);
    }

    #[test]
    fn closing_tabs_moves_the_active_tab() {
        let mut list = TabList::new();
        let a = list.open();
        let b = list.open();
        let c = list.open();
        let d = list.open();
        // Closing the active tab activates its right neighbour.
        list.select(1);
        assert_eq!(list.close(1).unwrap().id, b);
        assert_eq!(list.active().unwrap().id, c);
        // Closing a tab before the active one keeps the same tab active.
        assert_eq!(list.close(0).unwrap().id, a);
        assert_eq!(list.active().unwrap().id, c);
        // Closing the active last tab activates its left neighbour.
        list.select(1);
        assert_eq!(list.close(1).unwrap().id, d);
        assert_eq!(list.active().unwrap().id, c);
        assert!(list.close(5).is_none());
        assert!(list.close(0).is_some());
        assert!(list.is_empty());
        assert!(list.active().is_none());
        // Ids are never reused.
        assert!(list.open() > d);
    }

    #[test]
    fn background_tabs_open_to_the_right_in_order() {
        let mut list = TabList::new();
        let a = list.open();
        let z = list.open();
        list.select(0);
        let b1 = list.open_background();
        let b2 = list.open_background();
        assert_eq!(ids(&list), [a, b1, b2, z]);
        assert_eq!(list.active().unwrap().id, a, "stays in the background");
        // After switching tabs, a new run starts next to the new active tab.
        list.select(3);
        let b3 = list.open_background();
        assert_eq!(ids(&list), [a, b1, b2, z, b3]);
    }

    #[test]
    fn titles_and_history_positions() {
        let mut tab = Tab::new(1);
        assert_eq!(tab.title(), "New Tab");
        assert_eq!(tab.display_url(), None);
        let url = Url::parse("https://example.com/").unwrap();
        tab.loading = Some(LoadState {
            nav_id: 1,
            request: NavigationRequest::Get(url.clone()),
            started: Instant::now(),
            kind: NavKind::New,
            from_ui: false,
        });
        assert_eq!(tab.title(), "https://example.com/");
        assert_eq!(tab.display_url(), Some(&url));
        tab.loading = None;
        for t in ["one", "two", "three"] {
            tab.history.push(HistoryEntry::new(
                NavigationRequest::Get(url.clone()),
                t.to_string(),
            ));
        }
        assert_eq!(tab.title(), "three");
        assert_eq!(tab.back_index(), Some(1));
        assert_eq!(tab.forward_index(), None);
        // While going back to entry 1, "back" continues from there.
        tab.loading = Some(LoadState {
            nav_id: 2,
            request: NavigationRequest::Get(url),
            started: Instant::now(),
            kind: NavKind::History(1),
            from_ui: true,
        });
        assert_eq!(tab.back_index(), Some(0));
        assert_eq!(tab.forward_index(), Some(2));
    }
}
