//! Per-tab session history (back/forward list).

use xbro_engine::{NavigationRequest, Url};

/// Most entries kept per tab; the oldest are dropped beyond that.
pub const MAX_ENTRIES: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    /// What to load to come back to this entry.
    pub request: NavigationRequest,
    pub title: String,
    /// Vertical scroll position (CSS px) when the entry was left.
    pub scroll_y: f32,
}

impl HistoryEntry {
    pub fn new(request: NavigationRequest, title: String) -> Self {
        Self {
            request,
            title,
            scroll_y: 0.0,
        }
    }

    pub fn url(&self) -> &Url {
        self.request.url()
    }
}

/// A list of entries with a current position. Empty until the first page commits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    entries: Vec<HistoryEntry>,
    index: usize,
}

impl History {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Index of the current entry (0 when empty).
    pub fn index(&self) -> usize {
        self.index
    }

    #[cfg(test)]
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    pub fn current(&self) -> Option<&HistoryEntry> {
        self.entries.get(self.index)
    }

    pub fn current_mut(&mut self) -> Option<&mut HistoryEntry> {
        self.entries.get_mut(self.index)
    }

    pub fn get(&self, index: usize) -> Option<&HistoryEntry> {
        self.entries.get(index)
    }

    /// Adds an entry after the current one, dropping the forward entries,
    /// and makes it current.
    pub fn push(&mut self, entry: HistoryEntry) {
        if !self.entries.is_empty() {
            self.entries.truncate(self.index + 1);
        }
        self.entries.push(entry);
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..excess);
        }
        self.index = self.entries.len() - 1;
    }

    /// Replaces the current entry (or adds the first one); the other
    /// entries are kept.
    pub fn replace(&mut self, entry: HistoryEntry) {
        match self.current_mut() {
            Some(current) => *current = entry,
            None => self.push(entry),
        }
    }

    /// Makes `index` current (no-op when out of range). Returns whether it moved.
    pub fn go_to(&mut self, index: usize) -> bool {
        if index < self.entries.len() && index != self.index {
            self.index = index;
            true
        } else {
            false
        }
    }

    /// Index of the entry before `from`, if any.
    pub fn back_from(&self, from: usize) -> Option<usize> {
        (from > 0 && from <= self.entries.len()).then(|| from - 1)
    }

    /// Index of the entry after `from`, if any.
    pub fn forward_from(&self, from: usize) -> Option<usize> {
        (from + 1 < self.entries.len()).then_some(from + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> HistoryEntry {
        let url = Url::parse(&format!("https://example.com/{path}")).unwrap();
        HistoryEntry::new(NavigationRequest::Get(url), path.to_string())
    }

    fn titles(h: &History) -> Vec<&str> {
        h.entries().iter().map(|e| e.title.as_str()).collect()
    }

    #[test]
    fn navigate_back_forward_and_truncate() {
        let mut h = History::default();
        assert!(h.is_empty());
        assert_eq!(h.back_from(h.index()), None);
        assert_eq!(h.forward_from(h.index()), None);
        h.push(entry("a"));
        h.push(entry("b"));
        h.push(entry("c"));
        assert_eq!(h.index(), 2);
        assert_eq!(h.current().unwrap().title, "c");
        assert_eq!(h.forward_from(h.index()), None);

        let back = h.back_from(h.index()).unwrap();
        assert!(h.go_to(back));
        assert_eq!(h.current().unwrap().title, "b");
        assert_eq!(h.forward_from(h.index()), Some(2));
        assert!(h.go_to(0));
        assert_eq!(h.back_from(h.index()), None);
        assert!(!h.go_to(7), "out of range");

        // A new navigation from the middle drops the forward entries.
        h.go_to(1);
        h.push(entry("d"));
        assert_eq!(titles(&h), ["a", "b", "d"]);
        assert_eq!(h.index(), 2);
        assert_eq!(h.url().path(), "/d");
    }

    #[test]
    fn scroll_positions_are_kept_per_entry() {
        let mut h = History::default();
        h.push(entry("a"));
        h.current_mut().unwrap().scroll_y = 120.0;
        h.push(entry("b"));
        h.current_mut().unwrap().scroll_y = 40.0;
        h.go_to(0);
        assert_eq!(h.current().unwrap().scroll_y, 120.0);
        h.go_to(1);
        assert_eq!(h.current().unwrap().scroll_y, 40.0);
    }

    #[test]
    fn replace_swaps_the_current_entry_only() {
        let mut h = History::default();
        h.replace(entry("a"));
        assert_eq!(titles(&h), ["a"], "the first entry is added");
        h.push(entry("b"));
        h.push(entry("c"));
        h.go_to(1);
        h.replace(entry("r"));
        assert_eq!(titles(&h), ["a", "r", "c"], "forward entries are kept");
        assert_eq!(h.index(), 1);
    }

    #[test]
    fn length_is_bounded() {
        let mut h = History::default();
        for i in 0..MAX_ENTRIES + 10 {
            h.push(entry(&i.to_string()));
        }
        assert_eq!(h.len(), MAX_ENTRIES);
        assert_eq!(h.index(), MAX_ENTRIES - 1);
        assert_eq!(h.entries()[0].title, "10");
    }

    impl History {
        fn url(&self) -> &Url {
            self.current().unwrap().url()
        }
    }
}
