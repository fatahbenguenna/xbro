//! Markup declarations: comments, bogus comments, CDATA sections and DOCTYPEs
//! (including the quirks-mode decision based on the doctype identifiers).

use super::scanner::Scanner;
use super::{is_tag_whitespace, Token};

impl<'s> Scanner<'s> {
    /// Markup declaration open state, at `<!`.
    pub(super) fn markup_declaration(&mut self, cdata_allowed: bool) -> Option<Token> {
        let after = self.pos + 2;
        if self.starts_with_at(after, b"--") {
            Some(self.comment())
        } else if self.starts_with_ci_at(after, b"doctype") {
            Some(self.doctype())
        } else if cdata_allowed && self.starts_with_at(after, b"[CDATA[") {
            self.cdata()
        } else {
            Some(self.bogus_comment(after))
        }
    }

    /// Comment states, at `<!--`. Handles `<!-->`, `<!--->`, `--!>` and EOF.
    fn comment(&mut self) -> Token {
        let start = self.pos + 4;
        // Abrupt closing of empty comments.
        if self.byte_at(start) == Some(b'>') {
            self.pos = start + 1;
            return Token::Comment(String::new());
        }
        if self.starts_with_at(start, b"->") {
            self.pos = start + 2;
            return Token::Comment(String::new());
        }
        let mut i = start;
        loop {
            match self.find_seq(i, b"--") {
                Some(p) => match self.byte_at(p + 2) {
                    Some(b'>') => {
                        self.pos = p + 3;
                        return Token::Comment(self.slice(start, p).to_string());
                    }
                    Some(b'!') if self.byte_at(p + 3) == Some(b'>') => {
                        self.pos = p + 4;
                        return Token::Comment(self.slice(start, p).to_string());
                    }
                    _ => i = p + 1,
                },
                None => {
                    // EOF in comment: emit what we have, minus a dangling
                    // "--!", "--" or "-" (those were never appended).
                    let data = self.rest_from(start);
                    let data = data
                        .strip_suffix("--!")
                        .or_else(|| data.strip_suffix("--"))
                        .or_else(|| data.strip_suffix('-'))
                        .unwrap_or(data);
                    self.pos = self.len();
                    return Token::Comment(data.to_string());
                }
            }
        }
    }

    /// Bogus comment state: data from `start` up to the next `>`.
    pub(super) fn bogus_comment(&mut self, start: usize) -> Token {
        match self.find_byte(start, b'>') {
            Some(end) => {
                self.pos = end + 1;
                Token::Comment(self.slice(start, end).to_string())
            }
            None => {
                self.pos = self.len();
                Token::Comment(self.rest_from(start).to_string())
            }
        }
    }

    /// CDATA section, at `<![CDATA[` (foreign content only). Returns `None`
    /// for an empty section.
    fn cdata(&mut self) -> Option<Token> {
        let start = self.pos + 9;
        let text = match self.find_seq(start, b"]]>") {
            Some(end) => {
                self.pos = end + 3;
                self.slice(start, end)
            }
            None => {
                self.pos = self.len();
                self.rest_from(start)
            }
        };
        (!text.is_empty()).then(|| Token::Text(text.to_string()))
    }

    /// DOCTYPE states, at `<!DOCTYPE`.
    fn doctype(&mut self) -> Token {
        self.pos += 9;
        let mut doctype = DoctypeParts::default();
        self.doctype_body(&mut doctype);
        let force_quirks = doctype.forces_quirks();
        Token::Doctype {
            name: doctype.name.unwrap_or_default(),
            force_quirks,
        }
    }

    fn doctype_body(&mut self, d: &mut DoctypeParts) {
        // DOCTYPE state and before DOCTYPE name state.
        self.skip_whitespace();
        match self.peek() {
            None => {
                d.force_quirks = true;
                return;
            }
            Some(b'>') => {
                self.pos += 1;
                d.force_quirks = true;
                return;
            }
            Some(_) => {
                let start = self.pos;
                while self
                    .peek()
                    .is_some_and(|c| !is_tag_whitespace(c) && c != b'>')
                {
                    self.pos += 1;
                }
                d.name = Some(self.slice(start, self.pos).to_ascii_lowercase());
            }
        }
        // After DOCTYPE name state.
        self.skip_whitespace();
        match self.peek() {
            None => d.force_quirks = true,
            Some(b'>') => self.pos += 1,
            Some(_) if self.starts_with_ci_at(self.pos, b"public") => {
                self.pos += 6;
                self.doctype_identifiers(d, true);
            }
            Some(_) if self.starts_with_ci_at(self.pos, b"system") => {
                self.pos += 6;
                self.doctype_identifiers(d, false);
            }
            Some(_) => {
                d.force_quirks = true;
                self.bogus_doctype();
            }
        }
    }

    /// After the PUBLIC or SYSTEM keyword: identifiers and the closing `>`.
    fn doctype_identifiers(&mut self, d: &mut DoctypeParts, public: bool) {
        self.skip_whitespace();
        match self.peek() {
            None => {
                d.force_quirks = true;
                return;
            }
            Some(b'"' | b'\'') => {}
            Some(b'>') => {
                self.pos += 1;
                d.force_quirks = true;
                return;
            }
            Some(_) => {
                d.force_quirks = true;
                self.bogus_doctype();
                return;
            }
        }
        let Some(id) = self.doctype_quoted(d) else {
            return;
        };
        if public {
            d.public_id = Some(id);
            // After DOCTYPE public identifier: optional system identifier.
            self.skip_whitespace();
            match self.peek() {
                None => {
                    d.force_quirks = true;
                    return;
                }
                Some(b'>') => {
                    self.pos += 1;
                    return;
                }
                Some(b'"' | b'\'') => match self.doctype_quoted(d) {
                    Some(system) => d.system_id = Some(system),
                    None => return,
                },
                Some(_) => {
                    d.force_quirks = true;
                    self.bogus_doctype();
                    return;
                }
            }
        } else {
            d.system_id = Some(id);
        }
        // After DOCTYPE system identifier.
        self.skip_whitespace();
        match self.peek() {
            None => d.force_quirks = true,
            Some(b'>') => self.pos += 1,
            // Garbage after the system identifier does not force quirks.
            Some(_) => self.bogus_doctype(),
        }
    }

    /// A quoted doctype identifier at the opening quote. Returns `None` when
    /// the doctype ended (abruptly with `>` or at EOF); quirks are forced then.
    fn doctype_quoted(&mut self, d: &mut DoctypeParts) -> Option<String> {
        let quote = self.peek()?;
        let start = self.pos + 1;
        let mut i = start;
        loop {
            match self.byte_at(i) {
                None => {
                    self.pos = self.len();
                    d.force_quirks = true;
                    store_partial(d, self.slice(start, i));
                    return None;
                }
                Some(b'>') => {
                    self.pos = i + 1;
                    d.force_quirks = true;
                    store_partial(d, self.slice(start, i));
                    return None;
                }
                Some(c) if c == quote => {
                    self.pos = i + 1;
                    return Some(self.slice(start, i).to_string());
                }
                Some(_) => i += 1,
            }
        }
    }

    /// Bogus DOCTYPE state: skip to the next `>`.
    fn bogus_doctype(&mut self) {
        self.pos = self.find_byte(self.pos, b'>').map_or(self.len(), |p| p + 1);
    }
}

/// Records an identifier that was cut short (only matters for quirks checks,
/// which are forced anyway in that case).
fn store_partial(d: &mut DoctypeParts, id: &str) {
    if d.public_id.is_none() {
        d.public_id = Some(id.to_string());
    } else {
        d.system_id = Some(id.to_string());
    }
}

#[derive(Default)]
struct DoctypeParts {
    name: Option<String>,
    public_id: Option<String>,
    system_id: Option<String>,
    force_quirks: bool,
}

impl DoctypeParts {
    /// The WHATWG "initial" insertion mode quirks checks. Limited-quirks
    /// doctypes are treated as no-quirks.
    fn forces_quirks(&self) -> bool {
        if self.force_quirks {
            return true;
        }
        if self.name.as_deref() != Some("html") {
            return true;
        }
        let public = self.public_id.as_deref().map(str::to_ascii_lowercase);
        let system = self.system_id.as_deref().map(str::to_ascii_lowercase);
        if let Some(system) = &system {
            if system == "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd" {
                return true;
            }
        }
        let Some(public) = public else {
            return false;
        };
        if QUIRKS_PUBLIC_IDS.contains(&public.as_str()) {
            return true;
        }
        if QUIRKS_PUBLIC_PREFIXES
            .iter()
            .any(|prefix| public.starts_with(prefix))
        {
            return true;
        }
        system.is_none()
            && QUIRKS_PUBLIC_PREFIXES_WITHOUT_SYSTEM
                .iter()
                .any(|prefix| public.starts_with(prefix))
    }
}

/// Public identifiers that trigger quirks mode when matched exactly (lowercase).
const QUIRKS_PUBLIC_IDS: &[&str] = &[
    "-//w3o//dtd w3 html strict 3.0//en//",
    "-/w3c/dtd html 4.0 transitional/en",
    "html",
];

/// Public identifier prefixes that trigger quirks mode (lowercase).
const QUIRKS_PUBLIC_PREFIXES: &[&str] = &[
    "+//silmaril//dtd html pro v0r11 19970101//",
    "-//as//dtd html 3.0 aswedit + extensions//",
    "-//advasoft ltd//dtd html 3.0 aswedit + extensions//",
    "-//ietf//dtd html 2.0 level 1//",
    "-//ietf//dtd html 2.0 level 2//",
    "-//ietf//dtd html 2.0 strict level 1//",
    "-//ietf//dtd html 2.0 strict level 2//",
    "-//ietf//dtd html 2.0 strict//",
    "-//ietf//dtd html 2.0//",
    "-//ietf//dtd html 2.1e//",
    "-//ietf//dtd html 3.0//",
    "-//ietf//dtd html 3.2 final//",
    "-//ietf//dtd html 3.2//",
    "-//ietf//dtd html 3//",
    "-//ietf//dtd html level 0//",
    "-//ietf//dtd html level 1//",
    "-//ietf//dtd html level 2//",
    "-//ietf//dtd html level 3//",
    "-//ietf//dtd html strict level 0//",
    "-//ietf//dtd html strict level 1//",
    "-//ietf//dtd html strict level 2//",
    "-//ietf//dtd html strict level 3//",
    "-//ietf//dtd html strict//",
    "-//ietf//dtd html//",
    "-//metrius//dtd metrius presentational//",
    "-//microsoft//dtd internet explorer 2.0 html strict//",
    "-//microsoft//dtd internet explorer 2.0 html//",
    "-//microsoft//dtd internet explorer 2.0 tables//",
    "-//microsoft//dtd internet explorer 3.0 html strict//",
    "-//microsoft//dtd internet explorer 3.0 html//",
    "-//microsoft//dtd internet explorer 3.0 tables//",
    "-//netscape comm. corp.//dtd html//",
    "-//netscape comm. corp.//dtd strict html//",
    "-//o'reilly and associates//dtd html 2.0//",
    "-//o'reilly and associates//dtd html extended 1.0//",
    "-//o'reilly and associates//dtd html extended relaxed 1.0//",
    "-//sq//dtd html 2.0 hotmetal + extensions//",
    "-//softquad software//dtd hotmetal pro 6.0::19990601::extensions to html 4.0//",
    "-//softquad//dtd hotmetal pro 4.0::19971010::extensions to html 4.0//",
    "-//spyglass//dtd html 2.0 extended//",
    "-//sun microsystems corp.//dtd hotjava html//",
    "-//sun microsystems corp.//dtd hotjava strict html//",
    "-//w3c//dtd html 3 1995-03-24//",
    "-//w3c//dtd html 3.2 draft//",
    "-//w3c//dtd html 3.2 final//",
    "-//w3c//dtd html 3.2//",
    "-//w3c//dtd html 3.2s draft//",
    "-//w3c//dtd html 4.0 frameset//",
    "-//w3c//dtd html 4.0 transitional//",
    "-//w3c//dtd html experimental 19960712//",
    "-//w3c//dtd html experimental 970421//",
    "-//w3c//dtd w3 html//",
    "-//w3o//dtd w3 html 3.0//",
    "-//webtechs//dtd mozilla html 2.0//",
    "-//webtechs//dtd mozilla html//",
];

/// Public identifier prefixes that trigger quirks mode only when the system
/// identifier is missing (lowercase).
const QUIRKS_PUBLIC_PREFIXES_WITHOUT_SYSTEM: &[&str] = &[
    "-//w3c//dtd html 4.01 frameset//",
    "-//w3c//dtd html 4.01 transitional//",
];
