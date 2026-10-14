//! Find-in-page state of a tab: query, matches and the current match.

use xbro_engine::{FindMatch, Point};

use super::line_edit::LineEdit;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FindState {
    /// Whether the find bar is shown.
    pub open: bool,
    pub query: LineEdit,
    /// Matches in document order: rects in document coordinates, or in
    /// viewport coordinates for matches in `position: fixed` content
    /// (always at the same place on screen, see [`FindMatch`]).
    pub matches: Vec<FindMatch>,
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
    /// current match becomes the first one starting at or below the top of
    /// the view (the page is scrolled to `scroll`; fixed matches are always
    /// in view), wrapping to the first match. Returns it.
    pub fn set_matches(&mut self, matches: Vec<FindMatch>, scroll: Point) -> Option<FindMatch> {
        self.matches = matches;
        self.current = if self.matches.is_empty() {
            None
        } else {
            let below = |m: &FindMatch| m.fixed || m.rect.y >= scroll.y;
            Some(self.matches.iter().position(below).unwrap_or(0))
        };
        self.current_match()
    }

    /// Replaces the matches after a relayout, keeping the current index.
    pub fn refresh_matches(&mut self, matches: Vec<FindMatch>) {
        self.matches = matches;
        self.current = match self.current {
            _ if self.matches.is_empty() => None,
            Some(i) => Some(i.min(self.matches.len() - 1)),
            None => Some(0),
        };
    }

    /// Moves to the next match (wrapping) and returns it.
    pub fn next(&mut self) -> Option<FindMatch> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(self.current.map_or(0, |i| (i + 1) % n));
        self.current_match()
    }

    /// Moves to the previous match (wrapping) and returns it.
    pub fn prev(&mut self) -> Option<FindMatch> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(self.current.map_or(n - 1, |i| (i + n - 1) % n));
        self.current_match()
    }

    pub fn current_match(&self) -> Option<FindMatch> {
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
    use xbro_engine::Rect;

    fn rects(ys: &[f32]) -> Vec<FindMatch> {
        ys.iter()
            .map(|&y| FindMatch {
                rect: Rect::new(0.0, y, 30.0, 16.0),
                fixed: false,
            })
            .collect()
    }

    fn at(y: f32) -> Point {
        Point::new(0.0, y)
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
        let r = f.set_matches(rects(&[10.0, 400.0, 900.0]), at(300.0));
        assert_eq!(r.unwrap().rect.y, 400.0);
        assert_eq!(f.counter(), "2/3");
        assert_eq!(f.next().unwrap().rect.y, 900.0);
        assert_eq!(f.next().unwrap().rect.y, 10.0, "wraps around");
        assert_eq!(f.prev().unwrap().rect.y, 900.0, "wraps backwards");
        assert_eq!(f.counter(), "3/3");
        // Matches above the view only: wraps to the first.
        let first = f.set_matches(rects(&[10.0, 20.0]), at(500.0));
        assert_eq!(first.unwrap().rect.y, 10.0);
        // A relayout keeps the index when possible.
        f.next();
        f.refresh_matches(rects(&[11.0, 21.0]));
        assert_eq!(f.current_match().unwrap().rect.y, 21.0);
        f.refresh_matches(rects(&[11.0]));
        assert_eq!(f.current, Some(0));
    }

    #[test]
    fn no_matches_and_closing() {
        let mut f = FindState::default();
        f.open();
        f.query.insert("zzz");
        assert_eq!(f.set_matches(Vec::new(), Point::ZERO), None);
        assert_eq!(f.counter(), "0/0");
        assert!(f.not_found());
        assert_eq!(f.next(), None);
        assert_eq!(f.prev(), None);
        f.set_matches(rects(&[5.0]), Point::ZERO);
        f.close();
        assert!(!f.open && f.matches.is_empty() && f.current.is_none());
        assert_eq!(f.query.text(), "zzz", "query kept for the next search");
        f.open();
        assert_eq!(f.query.selected_text(), "zzz");
    }

    #[test]
    fn fixed_matches_count_as_in_view() {
        let mut f = FindState::default();
        f.open();
        f.query.insert("ab");
        // A fixed header match (viewport y 5) comes first in document
        // order: it is on screen whatever the scroll.
        let mut matches = rects(&[100.0, 2000.0]);
        matches.insert(
            0,
            FindMatch {
                rect: Rect::new(0.0, 5.0, 30.0, 16.0),
                fixed: true,
            },
        );
        let current = f.set_matches(matches.clone(), at(1500.0)).unwrap();
        assert!(current.fixed);
        assert_eq!(f.counter(), "1/3");
        assert_eq!(f.next().unwrap().rect.y, 100.0);
        // Without the fixed match, the first one below the view.
        f.set_matches(matches[1..].to_vec(), at(1500.0));
        assert_eq!(f.current_match().unwrap().rect.y, 2000.0);
    }
}
