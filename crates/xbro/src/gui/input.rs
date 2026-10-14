//! Keyboard decoding: key presses → browser commands, page commands, text
//! field edits. Pure functions of the key, the modifiers and the typed text.

use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

use super::line_edit::EditAction;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
}

impl Mods {
    pub fn from_state(state: ModifiersState) -> Self {
        Self {
            ctrl: state.control_key(),
            shift: state.shift_key(),
            alt: state.alt_key(),
            logo: state.super_key(),
        }
    }

    /// The platform's shortcut modifier (Cmd on macOS, Ctrl elsewhere).
    pub fn primary(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.logo
        } else {
            self.ctrl
        }
    }
}

/// A key press, as delivered by the window system.
#[derive(Copy, Clone, Debug)]
pub struct KeyInput<'a> {
    pub key: &'a Key,
    pub code: PhysicalKey,
    /// Text produced by the key (without Ctrl applied).
    pub text: Option<&'a str>,
    pub mods: Mods,
}

impl<'a> KeyInput<'a> {
    /// Printable text typed by this press: `None` for shortcuts (Ctrl, Alt
    /// or Super held; Ctrl+Alt is AltGr on Windows and still types) and
    /// control characters.
    pub fn typed_text(&self) -> Option<&'a str> {
        let m = self.mods;
        if m.logo || (m.ctrl != m.alt) {
            return None;
        }
        let text = self.text?;
        (!text.is_empty() && !text.chars().any(char::is_control)).then_some(text)
    }

    fn named(&self) -> Option<NamedKey> {
        match self.key {
            Key::Named(k) => Some(*k),
            _ => None,
        }
    }

    /// The lowercased character of a character key.
    fn character(&self) -> Option<String> {
        match self.key {
            Key::Character(s) => Some(s.to_lowercase()),
            _ => None,
        }
    }

    fn key_code(&self) -> Option<KeyCode> {
        match self.code {
            PhysicalKey::Code(c) => Some(c),
            PhysicalKey::Unidentified(_) => None,
        }
    }
}

/// Browser-level shortcuts, available whatever has the focus.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Back,
    Forward,
    Reload,
    Home,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    /// Ctrl+1…8 (0-based index).
    SelectTab(usize),
    SelectLastTab,
    FocusAddress,
    Find,
    FindNext,
    FindPrev,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

/// Decodes a browser shortcut.
pub fn global_command(k: &KeyInput) -> Option<Command> {
    let m = k.mods;
    let primary = m.primary();
    if let Some(named) = k.named() {
        let cmd = match named {
            NamedKey::F5 | NamedKey::BrowserRefresh => Some(Command::Reload),
            NamedKey::F6 => Some(Command::FocusAddress),
            NamedKey::F3 if m.shift => Some(Command::FindPrev),
            NamedKey::F3 => Some(Command::FindNext),
            NamedKey::BrowserBack | NamedKey::GoBack => Some(Command::Back),
            NamedKey::BrowserForward => Some(Command::Forward),
            NamedKey::BrowserHome => Some(Command::Home),
            NamedKey::ArrowLeft if m.alt && !m.ctrl && !m.shift => Some(Command::Back),
            NamedKey::ArrowRight if m.alt && !m.ctrl && !m.shift => Some(Command::Forward),
            NamedKey::Home if m.alt && !m.ctrl => Some(Command::Home),
            NamedKey::Tab if m.ctrl && m.shift => Some(Command::PrevTab),
            NamedKey::Tab if m.ctrl => Some(Command::NextTab),
            NamedKey::PageDown if m.ctrl && !m.shift => Some(Command::NextTab),
            NamedKey::PageUp if m.ctrl && !m.shift => Some(Command::PrevTab),
            _ => None,
        };
        if cmd.is_some() {
            return cmd;
        }
    }
    if m.alt && !m.ctrl && !m.logo && k.character().as_deref() == Some("d") {
        return Some(Command::FocusAddress);
    }
    if !primary || m.alt {
        return None;
    }
    let ch = k.character();
    if let Some(cmd) = ch.as_deref().and_then(|c| char_command(c, m.shift)) {
        return Some(cmd);
    }
    // Physical keys as a fallback for layouts without Latin letters (and
    // digits that need Shift, as on AZERTY), but never for a Latin
    // character that maps elsewhere (Ctrl+Z on AZERTY is not Ctrl+W).
    if ch
        .as_deref()
        .is_some_and(|c| c.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return None;
    }
    let code = k.key_code()?;
    let digit = |c: KeyCode| -> Option<usize> {
        use KeyCode::*;
        Some(match c {
            Digit1 | Numpad1 => 1,
            Digit2 | Numpad2 => 2,
            Digit3 | Numpad3 => 3,
            Digit4 | Numpad4 => 4,
            Digit5 | Numpad5 => 5,
            Digit6 | Numpad6 => 6,
            Digit7 | Numpad7 => 7,
            Digit8 | Numpad8 => 8,
            Digit9 | Numpad9 => 9,
            _ => return None,
        })
    };
    if let Some(d) = digit(code) {
        return Some(if d == 9 {
            Command::SelectLastTab
        } else {
            Command::SelectTab(d - 1)
        });
    }
    match code {
        KeyCode::Digit0 | KeyCode::Numpad0 => Some(Command::ZoomReset),
        KeyCode::Equal | KeyCode::NumpadAdd => Some(Command::ZoomIn),
        KeyCode::Minus | KeyCode::NumpadSubtract => Some(Command::ZoomOut),
        KeyCode::KeyT if !m.shift => Some(Command::NewTab),
        KeyCode::KeyW => Some(Command::CloseTab),
        KeyCode::KeyL => Some(Command::FocusAddress),
        KeyCode::KeyF => Some(Command::Find),
        KeyCode::KeyR => Some(Command::Reload),
        KeyCode::KeyG if m.shift => Some(Command::FindPrev),
        KeyCode::KeyG => Some(Command::FindNext),
        KeyCode::BracketLeft => Some(Command::Back),
        KeyCode::BracketRight => Some(Command::Forward),
        _ => None,
    }
}

/// Shortcut for a (lowercased) character typed with the primary modifier.
fn char_command(c: &str, shift: bool) -> Option<Command> {
    Some(match c {
        "t" if !shift => Command::NewTab,
        "w" => Command::CloseTab,
        "l" => Command::FocusAddress,
        "f" => Command::Find,
        "r" => Command::Reload,
        "g" if shift => Command::FindPrev,
        "g" => Command::FindNext,
        "=" | "+" => Command::ZoomIn,
        "-" | "_" => Command::ZoomOut,
        "0" => Command::ZoomReset,
        "[" => Command::Back,
        "]" => Command::Forward,
        "9" => Command::SelectLastTab,
        _ => {
            let d = c.parse::<usize>().ok().filter(|d| (1..=8).contains(d))?;
            Command::SelectTab(d - 1)
        }
    })
}

/// Scrolling requested from the keyboard.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollKey {
    /// Arrow keys: a step on each axis (−1, 0 or 1).
    Step {
        dx: i8,
        dy: i8,
    },
    /// One viewport (minus some overlap) down (1) or up (−1).
    Page(i8),
    Top,
    Bottom,
}

/// Keys handled by the page when nothing else has the focus.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PageCommand {
    Scroll(ScrollKey),
    /// Space: activates a focused control (checkbox, button…), else
    /// scrolls a page down (up with Shift).
    Space {
        shift: bool,
    },
    FocusNext {
        backwards: bool,
    },
    Activate,
    Escape,
}

/// Decodes a key for the page when no text field (browser or page) has the focus.
pub fn page_command(k: &KeyInput) -> Option<PageCommand> {
    let m = k.mods;
    if m.alt || m.logo {
        return None;
    }
    let named = k.named()?;
    let step = |dx, dy| Some(PageCommand::Scroll(ScrollKey::Step { dx, dy }));
    if m.ctrl {
        return match named {
            NamedKey::Home => Some(PageCommand::Scroll(ScrollKey::Top)),
            NamedKey::End => Some(PageCommand::Scroll(ScrollKey::Bottom)),
            _ => None,
        };
    }
    match named {
        NamedKey::ArrowDown => step(0, 1),
        NamedKey::ArrowUp => step(0, -1),
        NamedKey::ArrowLeft => step(-1, 0),
        NamedKey::ArrowRight => step(1, 0),
        NamedKey::PageDown => Some(PageCommand::Scroll(ScrollKey::Page(1))),
        NamedKey::PageUp => Some(PageCommand::Scroll(ScrollKey::Page(-1))),
        NamedKey::Space => Some(PageCommand::Space { shift: m.shift }),
        NamedKey::Home => Some(PageCommand::Scroll(ScrollKey::Top)),
        NamedKey::End => Some(PageCommand::Scroll(ScrollKey::Bottom)),
        NamedKey::Tab => Some(PageCommand::FocusNext { backwards: m.shift }),
        NamedKey::Enter => Some(PageCommand::Activate),
        NamedKey::Escape => Some(PageCommand::Escape),
        _ => None,
    }
}

/// Keys for a focused text field of the page (the engine only supports
/// appending and deleting at the end).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldCommand {
    Insert(String),
    /// Ctrl+V / Shift+Insert: insert the clipboard text.
    Paste,
    Backspace,
    Submit,
    Escape,
    FocusNext {
        backwards: bool,
    },
}

pub fn field_command(k: &KeyInput) -> Option<FieldCommand> {
    let m = k.mods;
    if let Some(named) = k.named() {
        if !m.ctrl && !m.alt && !m.logo {
            match named {
                NamedKey::Backspace => return Some(FieldCommand::Backspace),
                NamedKey::Enter => return Some(FieldCommand::Submit),
                NamedKey::Escape => return Some(FieldCommand::Escape),
                NamedKey::Tab => return Some(FieldCommand::FocusNext { backwards: m.shift }),
                NamedKey::Insert if m.shift => return Some(FieldCommand::Paste),
                _ => {}
            }
        }
    }
    let paste_key = k.character().as_deref() == Some("v")
        || (k.key_code() == Some(KeyCode::KeyV) && !is_latin(k));
    if m.primary() && !m.alt && paste_key {
        return Some(FieldCommand::Paste);
    }
    k.typed_text().map(|t| FieldCommand::Insert(t.to_string()))
}

/// Decodes an editing key for the address bar or the find field.
pub fn edit_action(k: &KeyInput) -> Option<EditAction> {
    let m = k.mods;
    let primary = m.primary();
    // Word-wise motion: Ctrl (Alt on macOS).
    let word = if cfg!(target_os = "macos") {
        m.alt
    } else {
        m.ctrl
    };
    if let Some(named) = k.named() {
        let action = match named {
            NamedKey::Backspace if word => Some(EditAction::DeleteWordBack),
            NamedKey::Backspace => Some(EditAction::Backspace),
            NamedKey::Delete if m.shift => Some(EditAction::Cut),
            NamedKey::Delete => Some(EditAction::Delete),
            NamedKey::Insert if m.shift => Some(EditAction::Paste),
            NamedKey::Insert if m.ctrl => Some(EditAction::Copy),
            NamedKey::ArrowLeft => Some(EditAction::Left {
                extend: m.shift,
                word,
            }),
            NamedKey::ArrowRight => Some(EditAction::Right {
                extend: m.shift,
                word,
            }),
            NamedKey::Home => Some(EditAction::Home { extend: m.shift }),
            NamedKey::End => Some(EditAction::End { extend: m.shift }),
            NamedKey::Enter => Some(EditAction::Submit { shift: m.shift }),
            NamedKey::Escape => Some(EditAction::Cancel),
            _ => None,
        };
        if action.is_some() {
            return action;
        }
    }
    if primary && !m.alt {
        let action = match k.character().as_deref() {
            Some("a") => Some(EditAction::SelectAll),
            Some("c") => Some(EditAction::Copy),
            Some("x") => Some(EditAction::Cut),
            Some("v") => Some(EditAction::Paste),
            _ => match k.key_code() {
                // Non-Latin layouts: fall back to the key position.
                Some(KeyCode::KeyA) if !is_latin(k) => Some(EditAction::SelectAll),
                Some(KeyCode::KeyC) if !is_latin(k) => Some(EditAction::Copy),
                Some(KeyCode::KeyX) if !is_latin(k) => Some(EditAction::Cut),
                Some(KeyCode::KeyV) if !is_latin(k) => Some(EditAction::Paste),
                _ => None,
            },
        };
        if action.is_some() {
            return action;
        }
    }
    k.typed_text().map(|t| EditAction::Insert(t.to_string()))
}

fn is_latin(k: &KeyInput) -> bool {
    k.character().is_some_and(|c| c.is_ascii())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
        logo: false,
    };
    const CTRL: Mods = Mods { ctrl: true, ..NONE };
    const SHIFT: Mods = Mods {
        shift: true,
        ..NONE
    };
    const ALT: Mods = Mods { alt: true, ..NONE };
    const CTRL_SHIFT: Mods = Mods {
        ctrl: true,
        shift: true,
        ..NONE
    };

    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    fn named(k: NamedKey) -> Key {
        Key::Named(k)
    }

    fn input<'a>(key: &'a Key, code: KeyCode, text: Option<&'a str>, mods: Mods) -> KeyInput<'a> {
        KeyInput {
            key,
            code: PhysicalKey::Code(code),
            text,
            mods,
        }
    }

    fn global(key: Key, code: KeyCode, mods: Mods) -> Option<Command> {
        global_command(&input(&key, code, None, mods))
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn browser_shortcuts() {
        use Command::*;
        assert_eq!(global(ch("t"), KeyCode::KeyT, CTRL), Some(NewTab));
        assert_eq!(global(ch("T"), KeyCode::KeyT, CTRL_SHIFT), None);
        assert_eq!(global(ch("w"), KeyCode::KeyW, CTRL), Some(CloseTab));
        assert_eq!(global(ch("l"), KeyCode::KeyL, CTRL), Some(FocusAddress));
        assert_eq!(global(ch("d"), KeyCode::KeyD, ALT), Some(FocusAddress));
        assert_eq!(
            global(named(NamedKey::F6), KeyCode::F6, NONE),
            Some(FocusAddress)
        );
        assert_eq!(global(ch("f"), KeyCode::KeyF, CTRL), Some(Find));
        assert_eq!(global(ch("r"), KeyCode::KeyR, CTRL), Some(Reload));
        assert_eq!(global(named(NamedKey::F5), KeyCode::F5, NONE), Some(Reload));
        assert_eq!(
            global(named(NamedKey::Tab), KeyCode::Tab, CTRL),
            Some(NextTab)
        );
        assert_eq!(
            global(named(NamedKey::Tab), KeyCode::Tab, CTRL_SHIFT),
            Some(PrevTab)
        );
        assert_eq!(
            global(named(NamedKey::PageDown), KeyCode::PageDown, CTRL),
            Some(NextTab)
        );
        assert_eq!(
            global(named(NamedKey::PageUp), KeyCode::PageUp, CTRL),
            Some(PrevTab)
        );
        assert_eq!(global(ch("1"), KeyCode::Digit1, CTRL), Some(SelectTab(0)));
        assert_eq!(global(ch("8"), KeyCode::Digit8, CTRL), Some(SelectTab(7)));
        assert_eq!(global(ch("9"), KeyCode::Digit9, CTRL), Some(SelectLastTab));
        // AZERTY: Ctrl+& is the "1" key.
        assert_eq!(global(ch("&"), KeyCode::Digit1, CTRL), Some(SelectTab(0)));
        assert_eq!(global(ch("="), KeyCode::Equal, CTRL), Some(ZoomIn));
        assert_eq!(global(ch("+"), KeyCode::Equal, CTRL_SHIFT), Some(ZoomIn));
        assert_eq!(global(ch("-"), KeyCode::Minus, CTRL), Some(ZoomOut));
        assert_eq!(global(ch("0"), KeyCode::Digit0, CTRL), Some(ZoomReset));
        assert_eq!(
            global(named(NamedKey::ArrowLeft), KeyCode::ArrowLeft, ALT),
            Some(Back)
        );
        assert_eq!(
            global(named(NamedKey::ArrowRight), KeyCode::ArrowRight, ALT),
            Some(Forward)
        );
        assert_eq!(global(ch("["), KeyCode::BracketLeft, CTRL), Some(Back));
        assert_eq!(global(ch("]"), KeyCode::BracketRight, CTRL), Some(Forward));
        assert_eq!(global(ch("g"), KeyCode::KeyG, CTRL), Some(FindNext));
        assert_eq!(
            global(named(NamedKey::F3), KeyCode::F3, SHIFT),
            Some(FindPrev)
        );
        // Not shortcuts.
        assert_eq!(
            global(named(NamedKey::Backspace), KeyCode::Backspace, NONE),
            None
        );
        assert_eq!(global(ch("t"), KeyCode::KeyT, NONE), None);
        assert_eq!(
            global(named(NamedKey::ArrowLeft), KeyCode::ArrowLeft, CTRL),
            None
        );
        // AZERTY Ctrl+Z (physical W) must not close the tab; Cyrillic Ctrl+е (physical T) opens one.
        assert_eq!(global(ch("z"), KeyCode::KeyW, CTRL), None);
        assert_eq!(global(ch("е"), KeyCode::KeyT, CTRL), Some(NewTab));
    }

    #[test]
    fn page_keys() {
        let page = |key: Key, mods: Mods| page_command(&input(&key, KeyCode::Space, None, mods));
        use PageCommand::*;
        assert_eq!(
            page(named(NamedKey::ArrowDown), NONE),
            Some(Scroll(ScrollKey::Step { dx: 0, dy: 1 }))
        );
        assert_eq!(
            page(named(NamedKey::ArrowLeft), NONE),
            Some(Scroll(ScrollKey::Step { dx: -1, dy: 0 }))
        );
        assert_eq!(
            page(named(NamedKey::Space), NONE),
            Some(Space { shift: false })
        );
        assert_eq!(
            page(named(NamedKey::Space), SHIFT),
            Some(Space { shift: true })
        );
        assert_eq!(
            page(named(NamedKey::PageUp), NONE),
            Some(Scroll(ScrollKey::Page(-1)))
        );
        assert_eq!(
            page(named(NamedKey::End), NONE),
            Some(Scroll(ScrollKey::Bottom))
        );
        assert_eq!(
            page(named(NamedKey::Home), CTRL),
            Some(Scroll(ScrollKey::Top))
        );
        assert_eq!(
            page(named(NamedKey::Tab), SHIFT),
            Some(FocusNext { backwards: true })
        );
        assert_eq!(page(named(NamedKey::Enter), NONE), Some(Activate));
        assert_eq!(page(named(NamedKey::Escape), NONE), Some(Escape));
        assert_eq!(
            page(named(NamedKey::Backspace), NONE),
            None,
            "Backspace is not Back"
        );
        assert_eq!(page(named(NamedKey::ArrowLeft), ALT), None);
        assert_eq!(page(ch("a"), NONE), None);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn editing_keys() {
        let edit = |key: Key, text: Option<&str>, mods: Mods| {
            edit_action(&input(&key, KeyCode::KeyA, text, mods))
        };
        assert_eq!(
            edit(ch("a"), Some("a"), NONE),
            Some(EditAction::Insert("a".into()))
        );
        assert_eq!(
            edit(ch("A"), Some("A"), SHIFT),
            Some(EditAction::Insert("A".into()))
        );
        assert_eq!(edit(ch("a"), Some("a"), CTRL), Some(EditAction::SelectAll));
        assert_eq!(edit(ch("v"), Some("v"), CTRL), Some(EditAction::Paste));
        assert_eq!(edit(ch("c"), Some("c"), CTRL), Some(EditAction::Copy));
        assert_eq!(edit(ch("x"), Some("x"), CTRL), Some(EditAction::Cut));
        // Ctrl+other letters type nothing (they may be browser shortcuts).
        assert_eq!(edit(ch("t"), Some("t"), CTRL), None);
        // AltGr reported as Ctrl+Alt types its character.
        let altgr = Mods {
            ctrl: true,
            alt: true,
            ..NONE
        };
        assert_eq!(
            edit(ch("@"), Some("@"), altgr),
            Some(EditAction::Insert("@".into()))
        );
        assert_eq!(
            edit(named(NamedKey::Enter), Some("\r"), NONE),
            Some(EditAction::Submit { shift: false })
        );
        assert_eq!(
            edit(named(NamedKey::Escape), Some("\u{1b}"), NONE),
            Some(EditAction::Cancel)
        );
        assert_eq!(
            edit(named(NamedKey::Backspace), None, NONE),
            Some(EditAction::Backspace)
        );
        assert_eq!(
            edit(named(NamedKey::Backspace), None, CTRL),
            Some(EditAction::DeleteWordBack)
        );
        assert_eq!(
            edit(named(NamedKey::ArrowLeft), None, CTRL_SHIFT),
            Some(EditAction::Left {
                extend: true,
                word: true
            })
        );
        assert_eq!(
            edit(named(NamedKey::End), None, SHIFT),
            Some(EditAction::End { extend: true })
        );
        assert_eq!(
            edit(named(NamedKey::Space), Some(" "), NONE),
            Some(EditAction::Insert(" ".into()))
        );
        assert_eq!(edit(named(NamedKey::F1), None, NONE), None);
    }

    #[test]
    fn text_field_keys() {
        let field = |key: Key, text: Option<&str>, mods: Mods| {
            field_command(&input(&key, KeyCode::KeyA, text, mods))
        };
        assert_eq!(
            field(ch("é"), Some("é"), NONE),
            Some(FieldCommand::Insert("é".into()))
        );
        assert_eq!(
            field(named(NamedKey::Backspace), None, NONE),
            Some(FieldCommand::Backspace)
        );
        assert_eq!(
            field(named(NamedKey::Enter), Some("\r"), NONE),
            Some(FieldCommand::Submit)
        );
        assert_eq!(
            field(named(NamedKey::Escape), None, NONE),
            Some(FieldCommand::Escape)
        );
        assert_eq!(
            field(named(NamedKey::Tab), Some("\t"), SHIFT),
            Some(FieldCommand::FocusNext { backwards: true })
        );
        assert_eq!(field(ch("w"), Some("w"), CTRL), None);
        assert_eq!(field(ch("v"), Some("v"), CTRL), Some(FieldCommand::Paste));
        assert_eq!(
            field(named(NamedKey::Insert), None, SHIFT),
            Some(FieldCommand::Paste)
        );
        assert_eq!(field(named(NamedKey::ArrowLeft), None, NONE), None);
    }
}
