//! Find-in-page state of a tab: query, matches and the current match.

use xbro_engine::Rect;

use super::line_edit::LineEdit;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FindState {
    /// Whether the find bar is shown.
    pub open: bool,
    pub query: LineEdit,
    /// Match rects in document coordinates, in document order.
    pub matches: Vec<Rect>,
    /// Index of the highlighted match.
    pub current: Option<usize>,
}

impl FindState {
    /// Shows the bar with the previous query selected.
    pub fn open(&mut self) {
        self.open = true;
        self.query.select_all();
    }

    /// Hides the bar and its highlights (the query is kept for next time).
    pub fn close(&mut self) {
        self.open = false;
        self.matches.clear();
        self.current = None;
    }

    /// Whether highlights should be computed (bar open with a query).
    pub fn active(&self) -> bool {
        self.open && !self.query.text().is_empty()
    }

    /// Replaces the matches after the query (or the page) changed. The
    /// current match becomes the first one starting at or below `from_y`
    /// (the top of the view), wrapping to the first match. Returns its rect.
    pub fn set_matches(&mut self, matches: Vec<Rect>, from_y: f32) -> Option<Rect> {
        self.matches = matches;
        self.current = if self.matches.is_empty() {
            None
        } else {
            Some(self.matches.iter().position(|r| r.y >= from_y).unwrap_or(0))
        };
        self.current_rect()
    }

    /// Replaces the matches after a relayout, keeping the current index.
    pub fn refresh_matches(&mut self, matches: Vec<Rect>) {
        self.matches = matches;
        self.current = match self.current {
            _ if self.matches.is_empty() => None,
            Some(i) => Some(i.min(self.matches.len() - 1)),
            None => Some(0),
        };
    }

    /// Moves to the next match (wrapping) and returns its rect.
    pub fn next(&mut self) -> Option<Rect> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(self.current.map_or(0, |i| (i + 1) % n));
        self.current_rect()
    }

    /// Moves to the previous match (wrapping) and returns its rect.
    pub fn prev(&mut self) -> Option<Rect> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(self.current.map_or(n - 1, |i| (i + n - 1) % n));
        self.current_rect()
    }

    pub fn current_rect(&self) -> Option<Rect> {
        self.current.and_then(|i| self.matches.get(i)).copied()
    }

    /// "3/12" style counter ("0/0" without matches, empty without a query).
    pub fn counter(&self) -> String {
        if self.query.text().is_empty() {
            return String::new();
        }
        match self.current {
            Some(i) if !self.matches.is_empty() => format!("{}/{}", i + 1, self.matches.len()),
            _ => "0/0".to_string(),
        }
    }

    /// True when a non-empty query has no match (the field turns red).
    pub fn not_found(&self) -> bool {
        !self.query.text().is_empty() && self.matches.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(ys: &[f32]) -> Vec<Rect> {
        ys.iter().map(|&y| Rect::new(0.0, y, 30.0, 16.0)).collect()
    }

    #[test]
    fn cycles_through_matches() {
        let mut f = FindState::default();
        f.open();
        assert!(f.open);
        assert_eq!(f.counter(), "");
        f.query.insert("ab");
        assert!(f.active());
        // The first match below the top of the view becomes current.
        let r = f.set_matches(rects(&[10.0, 400.0, 900.0]), 300.0);
        assert_eq!(r.unwrap().y, 400.0);
        assert_eq!(f.counter(), "2/3");
        assert_eq!(f.next().unwrap().y, 900.0);
        assert_eq!(f.next().unwrap().y, 10.0, "wraps around");
        assert_eq!(f.prev().unwrap().y, 900.0, "wraps backwards");
        assert_eq!(f.counter(), "3/3");
        // Matches above the view only: wraps to the first.
        assert_eq!(f.set_matches(rects(&[10.0, 20.0]), 500.0).unwrap().y, 10.0);
        // A relayout keeps the index when possible.
        f.next();
        f.refresh_matches(rects(&[11.0, 21.0]));
        assert_eq!(f.current_rect().unwrap().y, 21.0);
        f.refresh_matches(rects(&[11.0]));
        assert_eq!(f.current, Some(0));
    }

    #[test]
    fn no_matches_and_closing() {
        let mut f = FindState::default();
        f.open();
        f.query.insert("zzz");
        assert_eq!(f.set_matches(Vec::new(), 0.0), None);
        assert_eq!(f.counter(), "0/0");
        assert!(f.not_found());
        assert_eq!(f.next(), None);
        assert_eq!(f.prev(), None);
        f.set_matches(rects(&[5.0]), 0.0);
        f.close();
        assert!(!f.open && f.matches.is_empty() && f.current.is_none());
        assert_eq!(f.query.text(), "zzz", "query kept for the next search");
        f.open();
        assert_eq!(f.query.selected_text(), "zzz");
    }
}
