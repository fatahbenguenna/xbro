//! A single-line text editor model (address bar, find field): text, caret
//! and selection, with no knowledge of fonts or windows.
//!
//! Positions are byte offsets into the UTF-8 text and always sit on char
//! boundaries.

use std::ops::Range;

/// Longest text accepted (pasting a huge string is truncated), in bytes.
pub const MAX_LEN: usize = 2 << 20;

/// Something that can hold text for copy/paste. Errors are swallowed by
/// implementations (a missing clipboard just does nothing).
pub trait Clipboard {
    fn get_text(&mut self) -> Option<String>;
    fn set_text(&mut self, text: &str);
}

/// An editing command, already decoded from a key press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditAction {
    Insert(String),
    Backspace,
    Delete,
    /// Delete the word before the caret (Ctrl+Backspace).
    DeleteWordBack,
    Left {
        extend: bool,
        word: bool,
    },
    Right {
        extend: bool,
        word: bool,
    },
    Home {
        extend: bool,
    },
    End {
        extend: bool,
    },
    SelectAll,
    Copy,
    Cut,
    Paste,
    /// Enter (`shift` held or not).
    Submit {
        shift: bool,
    },
    /// Escape.
    Cancel,
}

/// What applying an [`EditAction`] did.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EditOutcome {
    /// The text changed.
    Changed,
    /// Only the caret or selection moved (or nothing happened).
    Moved,
    Submit {
        shift: bool,
    },
    Cancel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEdit {
    text: String,
    caret: usize,
    /// The other end of the selection (equal to `caret` when nothing is selected).
    anchor: usize,
}

impl LineEdit {
    /// An editor holding `text` with the caret at its end.
    #[cfg(test)]
    pub fn new(text: &str) -> Self {
        let mut edit = Self::default();
        edit.set_text(text);
        edit
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected byte range (ordered), if not empty.
    pub fn selection(&self) -> Option<Range<usize>> {
        let (a, b) = (self.anchor.min(self.caret), self.anchor.max(self.caret));
        (a < b).then_some(a..b)
    }

    pub fn selected_text(&self) -> &str {
        self.selection()
            .and_then(|r| self.text.get(r))
            .unwrap_or_default()
    }

    /// Replaces the whole text (sanitized) and puts the caret at its end.
    pub fn set_text(&mut self, text: &str) {
        self.text = sanitize(text, MAX_LEN);
        self.caret = self.text.len();
        self.anchor = self.caret;
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Moves the caret to `pos` (snapped to a char boundary), extending the
    /// selection or collapsing it.
    pub fn set_caret(&mut self, pos: usize, extend: bool) {
        let pos = floor_boundary(&self.text, pos);
        self.caret = pos;
        if !extend {
            self.anchor = pos;
        }
    }

    /// Selects `range` (snapped to char boundaries), caret at its end.
    pub fn select(&mut self, range: Range<usize>) {
        self.anchor = floor_boundary(&self.text, range.start);
        self.caret = floor_boundary(&self.text, range.end);
    }

    /// Inserts `s` at the caret, replacing the selection. Control
    /// characters are dropped (line breaks and tabs from a paste included).
    pub fn insert(&mut self, s: &str) -> bool {
        let had_selection = self.delete_selection();
        let room = MAX_LEN.saturating_sub(self.text.len());
        let s = sanitize(s, room);
        if s.is_empty() {
            return had_selection;
        }
        self.text.insert_str(self.caret, &s);
        self.caret += s.len();
        self.anchor = self.caret;
        true
    }

    /// Deletes the selection, or the char before the caret.
    pub fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let Some(prev) = prev_boundary(&self.text, self.caret) else {
            return false;
        };
        self.text.replace_range(prev..self.caret, "");
        self.caret = prev;
        self.anchor = prev;
        true
    }

    /// Deletes the selection, or the char after the caret.
    pub fn delete(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let Some(next) = next_boundary(&self.text, self.caret) else {
            return false;
        };
        self.text.replace_range(self.caret..next, "");
        true
    }

    /// Deletes the selection, or back to the start of the previous word.
    pub fn delete_word_back(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let start = word_start_before(&self.text, self.caret);
        if start == self.caret {
            return false;
        }
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
        self.anchor = start;
        true
    }

    pub fn move_left(&mut self, extend: bool, word: bool) {
        if !extend {
            if let Some(sel) = self.selection() {
                self.set_caret(sel.start, false);
                return;
            }
        }
        let pos = if word {
            word_start_before(&self.text, self.caret)
        } else {
            prev_boundary(&self.text, self.caret).unwrap_or(0)
        };
        self.set_caret(pos, extend);
    }

    pub fn move_right(&mut self, extend: bool, word: bool) {
        if !extend {
            if let Some(sel) = self.selection() {
                self.set_caret(sel.end, false);
                return;
            }
        }
        let pos = if word {
            word_end_after(&self.text, self.caret)
        } else {
            next_boundary(&self.text, self.caret).unwrap_or(self.text.len())
        };
        self.set_caret(pos, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        self.set_caret(0, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        self.set_caret(self.text.len(), extend);
    }

    /// Selects the word around `pos` (double click).
    pub fn select_word_at(&mut self, pos: usize) {
        let pos = floor_boundary(&self.text, pos);
        let start = self.text[..pos]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_word_char(c))
            .last()
            .map_or(pos, |(i, _)| i);
        let end = self.text[pos..]
            .char_indices()
            .find(|&(_, c)| !is_word_char(c))
            .map_or(self.text.len(), |(i, _)| pos + i);
        self.select(start..end);
    }

    /// Applies an editing command; clipboard actions go through `clipboard`.
    pub fn apply(&mut self, action: &EditAction, clipboard: &mut dyn Clipboard) -> EditOutcome {
        let changed = match action {
            EditAction::Insert(s) => self.insert(s),
            EditAction::Backspace => self.backspace(),
            EditAction::Delete => self.delete(),
            EditAction::DeleteWordBack => self.delete_word_back(),
            EditAction::Left { extend, word } => {
                self.move_left(*extend, *word);
                false
            }
            EditAction::Right { extend, word } => {
                self.move_right(*extend, *word);
                false
            }
            EditAction::Home { extend } => {
                self.move_home(*extend);
                false
            }
            EditAction::End { extend } => {
                self.move_end(*extend);
                false
            }
            EditAction::SelectAll => {
                self.select_all();
                false
            }
            EditAction::Copy => {
                if self.selection().is_some() {
                    clipboard.set_text(self.selected_text());
                }
                false
            }
            EditAction::Cut => {
                if self.selection().is_some() {
                    clipboard.set_text(self.selected_text());
                    self.delete_selection()
                } else {
                    false
                }
            }
            EditAction::Paste => match clipboard.get_text() {
                Some(text) => self.insert(&text),
                None => false,
            },
            EditAction::Submit { shift } => return EditOutcome::Submit { shift: *shift },
            EditAction::Cancel => return EditOutcome::Cancel,
        };
        if changed {
            EditOutcome::Changed
        } else {
            EditOutcome::Moved
        }
    }

    fn delete_selection(&mut self) -> bool {
        let Some(sel) = self.selection() else {
            return false;
        };
        self.text.replace_range(sel.clone(), "");
        self.caret = sel.start;
        self.anchor = sel.start;
        true
    }
}

/// Drops control characters and truncates to `max` bytes (on a char boundary).
fn sanitize(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max));
    for c in s.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The largest char boundary `<= pos` (clamped to the text).
fn floor_boundary(text: &str, pos: usize) -> usize {
    let mut pos = pos.min(text.len());
    while !text.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn prev_boundary(text: &str, pos: usize) -> Option<usize> {
    text.get(..pos)?.char_indices().next_back().map(|(i, _)| i)
}

fn next_boundary(text: &str, pos: usize) -> Option<usize> {
    let c = text.get(pos..)?.chars().next()?;
    Some(pos + c.len_utf8())
}

/// Start of the word before `pos`: skips separators, then word chars.
fn word_start_before(text: &str, pos: usize) -> usize {
    let mut chars = text[..floor_boundary(text, pos)]
        .char_indices()
        .rev()
        .peekable();
    let mut start = floor_boundary(text, pos);
    while let Some(&(i, c)) = chars.peek() {
        if is_word_char(c) {
            break;
        }
        start = i;
        chars.next();
    }
    while let Some(&(i, c)) = chars.peek() {
        if !is_word_char(c) {
            break;
        }
        start = i;
        chars.next();
    }
    start
}

/// End of the word after `pos`: skips separators, then word chars.
fn word_end_after(text: &str, pos: usize) -> usize {
    let pos = floor_boundary(text, pos);
    let rest = &text[pos..];
    let mut iter = rest.char_indices().peekable();
    while iter.next_if(|&(_, c)| !is_word_char(c)).is_some() {}
    while iter.next_if(|&(_, c)| is_word_char(c)).is_some() {}
    pos + iter.peek().map_or(rest.len(), |&(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory clipboard.
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

    #[test]
    fn insert_and_delete_ascii() {
        let mut e = LineEdit::new("helo");
        e.move_left(false, false);
        assert!(e.insert("l"));
        assert_eq!(e.text(), "hello");
        assert_eq!(e.caret(), 4);
        e.move_end(false);
        assert!(e.backspace());
        assert_eq!(e.text(), "hell");
        e.move_home(false);
        assert!(e.delete());
        assert_eq!(e.text(), "ell");
        e.move_end(false);
        assert!(!e.delete());
        e.move_home(false);
        assert!(!e.backspace());
    }

    #[test]
    fn utf8_caret_moves_by_chars() {
        let mut e = LineEdit::new("héé😀");
        assert_eq!(e.caret(), e.text().len());
        e.move_left(false, false);
        assert_eq!(&e.text()[e.caret()..], "😀");
        assert!(e.backspace());
        assert_eq!(e.text(), "hé😀");
        e.move_right(false, false);
        assert_eq!(e.caret(), e.text().len());
        e.set_caret(2, false); // inside "é": snapped back to its start
        assert_eq!(e.caret(), 1);
        e.insert("ü");
        assert_eq!(e.text(), "hüé😀");
        e.move_home(false);
        e.move_right(true, false);
        e.move_right(true, false);
        assert_eq!(e.selected_text(), "hü");
    }

    #[test]
    fn selection_is_replaced_and_collapsed() {
        let mut e = LineEdit::new("example.com");
        e.select_all();
        assert_eq!(e.selected_text(), "example.com");
        e.insert("rust");
        assert_eq!(e.text(), "rust");
        assert!(e.selection().is_none());

        let mut e = LineEdit::new("abcdef");
        e.set_caret(1, false);
        e.set_caret(4, true);
        assert_eq!(e.selection(), Some(1..4));
        assert!(e.backspace());
        assert_eq!(e.text(), "aef");
        assert_eq!(e.caret(), 1);

        // Left/Right without shift collapse the selection to its ends.
        let mut e = LineEdit::new("abcdef");
        e.set_caret(2, false);
        e.set_caret(5, true);
        e.move_left(false, false);
        assert_eq!((e.caret(), e.selection()), (2, None));
        e.set_caret(5, true);
        e.move_right(false, false);
        assert_eq!((e.caret(), e.selection()), (5, None));
        // Shift+Home selects back to the start.
        e.move_home(true);
        assert_eq!(e.selected_text(), "abcde");
    }

    #[test]
    fn control_characters_are_dropped_and_length_is_capped() {
        let mut e = LineEdit::default();
        e.insert("a\nb\tc\u{7f}d\r");
        assert_eq!(e.text(), "abcd");
        let mut e = LineEdit::default();
        assert!(!e.insert("\n"));
        let long = "é".repeat(MAX_LEN);
        e.insert(&long);
        assert!(e.text().len() <= MAX_LEN);
        assert!(e.text().is_char_boundary(e.text().len()));
    }

    #[test]
    fn word_motion_and_deletion() {
        let mut e = LineEdit::new("https://www.example.com/path");
        e.move_left(false, true);
        assert_eq!(&e.text()[e.caret()..], "path");
        e.move_left(false, true);
        assert_eq!(&e.text()[e.caret()..], "com/path");
        e.move_right(false, true);
        assert_eq!(&e.text()[e.caret()..], "/path");
        e.move_end(false);
        assert!(e.delete_word_back());
        assert_eq!(e.text(), "https://www.example.com/");
        assert!(e.delete_word_back());
        assert_eq!(e.text(), "https://www.example.");
        e.select_word_at(14);
        assert_eq!(e.selected_text(), "example");
    }

    #[test]
    fn clipboard_actions() {
        let mut clip = Memory::default();
        let mut e = LineEdit::new("hello world");
        assert_eq!(e.apply(&EditAction::Copy, &mut clip), EditOutcome::Moved);
        assert_eq!(clip.0, None, "nothing selected, nothing copied");
        e.select_word_at(8);
        e.apply(&EditAction::Cut, &mut clip);
        assert_eq!(e.text(), "hello ");
        assert_eq!(clip.0.as_deref(), Some("world"));
        e.apply(&EditAction::Home { extend: false }, &mut clip);
        assert_eq!(e.apply(&EditAction::Paste, &mut clip), EditOutcome::Changed);
        assert_eq!(e.text(), "worldhello ");
        e.apply(&EditAction::SelectAll, &mut clip);
        e.apply(&EditAction::Copy, &mut clip);
        assert_eq!(clip.0.as_deref(), Some("worldhello "));
        assert_eq!(
            e.apply(&EditAction::Submit { shift: true }, &mut clip),
            EditOutcome::Submit { shift: true }
        );
        assert_eq!(e.apply(&EditAction::Cancel, &mut clip), EditOutcome::Cancel);
    }
}
