//! The system clipboard (created on first use; failures are silent).

use super::line_edit::Clipboard;

#[derive(Default)]
pub struct SystemClipboard {
    inner: Option<arboard::Clipboard>,
    unavailable: bool,
}

impl SystemClipboard {
    fn get(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.inner.is_none() && !self.unavailable {
            match arboard::Clipboard::new() {
                Ok(c) => self.inner = Some(c),
                Err(e) => {
                    eprintln!("xbro: clipboard unavailable: {e}");
                    self.unavailable = true;
                }
            }
        }
        self.inner.as_mut()
    }
}

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.get()?.get_text().ok()
    }

    fn set_text(&mut self, text: &str) {
        if let Some(clipboard) = self.get() {
            let _ = clipboard.set_text(text);
        }
    }
}
