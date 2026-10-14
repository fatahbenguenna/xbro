//! Insertion modes around the body: initial, before html, before head, in
//! head, in head noscript, after head, text, after body, the frameset modes
//! and the "after after" modes.

use super::tags::Tag;
use super::{is_all_whitespace, is_html_whitespace, split_leading_whitespace, Mode, TreeBuilder};
use crate::tokenizer::{TextMode, Token};

/// Splits off the leading whitespace of a text token. Returns the
/// whitespace and, when non-empty, the remaining text as a token.
fn split_text(text: &str) -> (&str, Option<Token>) {
    let (ws, rest) = split_leading_whitespace(text);
    let rest = (!rest.is_empty()).then(|| Token::Text(rest.to_string()));
    (ws, rest)
}

impl TreeBuilder {
    pub(super) fn initial(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                let (_, rest) = split_text(&text);
                let rest = rest?;
                self.doc.quirks_mode = true;
                self.mode = Mode::BeforeHtml;
                Some(rest)
            }
            Token::Comment(data) => {
                let root = self.doc.root();
                self.append_comment_to(root, data);
                None
            }
            Token::Doctype { name, force_quirks } => {
                let root = self.doc.root();
                let doctype = self.doc.create_doctype(name);
                self.doc.append_child(root, doctype);
                self.doc.quirks_mode = force_quirks;
                self.mode = Mode::BeforeHtml;
                None
            }
            other => {
                // No doctype at all: quirks mode.
                self.doc.quirks_mode = true;
                self.mode = Mode::BeforeHtml;
                Some(other)
            }
        }
    }

    fn create_html_element(&mut self, attrs: Vec<xbro_dom::Attribute>) {
        let root = self.doc.root();
        let html = self.create_detached("html", attrs);
        self.append_moved(root, html);
        self.push_existing(html, Tag::Html);
        self.mode = Mode::BeforeHead;
    }

    pub(super) fn before_html(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Doctype { .. } => None,
            Token::Comment(data) => {
                let root = self.doc.root();
                self.append_comment_to(root, data);
                None
            }
            Token::Text(text) => {
                let (_, rest) = split_text(&text);
                let rest = rest?;
                self.create_html_element(Vec::new());
                Some(rest)
            }
            Token::StartTag { name, attrs, .. } if name == "html" => {
                self.create_html_element(attrs);
                None
            }
            Token::EndTag { name } if !matches!(name.as_str(), "head" | "body" | "html" | "br") => {
                None
            }
            other => {
                self.create_html_element(Vec::new());
                Some(other)
            }
        }
    }

    pub(super) fn before_head(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                let (_, rest) = split_text(&text);
                let rest = rest?;
                self.insert_implied_head();
                Some(rest)
            }
            Token::Comment(data) => {
                self.insert_comment(data);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag { ref name, .. } if name == "html" => self.in_body(token),
            Token::StartTag { name, attrs, .. } if name == "head" => {
                let head = self.insert_html(&name, attrs);
                self.head = Some(head);
                self.mode = Mode::InHead;
                None
            }
            Token::EndTag { name } if !matches!(name.as_str(), "head" | "body" | "html" | "br") => {
                None
            }
            other => {
                self.insert_implied_head();
                Some(other)
            }
        }
    }

    fn insert_implied_head(&mut self) {
        let head = self.insert_html("head", Vec::new());
        self.head = Some(head);
        self.mode = Mode::InHead;
    }

    pub(super) fn in_head(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                let (ws, rest) = split_text(&text);
                self.insert_text(ws);
                let rest = rest?;
                self.in_head_anything_else(rest)
            }
            Token::Comment(data) => {
                self.insert_comment(data);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => match Tag::from_name(&name) {
                Tag::Html => self.in_body(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                Tag::Base | Tag::Basefont | Tag::Bgsound | Tag::Link | Tag::Meta => {
                    self.insert_html_void(&name, attrs);
                    None
                }
                Tag::Title => {
                    self.parse_text_element(&name, attrs, TextMode::RcData);
                    None
                }
                Tag::Noscript => {
                    // Scripting is disabled: parse the content as markup.
                    self.insert_html(&name, attrs);
                    self.mode = Mode::InHeadNoscript;
                    None
                }
                Tag::Noframes | Tag::Style => {
                    self.parse_text_element(&name, attrs, TextMode::RawText);
                    None
                }
                Tag::Script => {
                    self.parse_text_element(&name, attrs, TextMode::ScriptData);
                    None
                }
                Tag::Template => {
                    self.insert_html(&name, attrs);
                    self.push_marker();
                    self.frameset_ok = false;
                    self.mode = Mode::InTemplate;
                    self.template_modes.push(Mode::InTemplate);
                    None
                }
                Tag::Head => None,
                _ => self.in_head_anything_else(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
            },
            Token::EndTag { name } => match Tag::from_name(&name) {
                Tag::Head => {
                    if self.current_is(Tag::Head) {
                        self.pop();
                    }
                    self.mode = Mode::AfterHead;
                    None
                }
                Tag::Body | Tag::Html | Tag::Br => {
                    self.in_head_anything_else(Token::EndTag { name })
                }
                Tag::Template => {
                    self.end_template();
                    None
                }
                _ => None,
            },
            Token::Eof => self.in_head_anything_else(Token::Eof),
        }
    }

    fn in_head_anything_else(&mut self, token: Token) -> Option<Token> {
        if self.current_is(Tag::Head) {
            self.pop();
        }
        self.mode = Mode::AfterHead;
        Some(token)
    }

    /// The `</template>` end tag rules (shared by several modes).
    pub(super) fn end_template(&mut self) {
        if !self.has_open(Tag::Template) {
            return;
        }
        self.generate_implied_end_tags_thoroughly();
        self.pop_until(Tag::Template);
        self.clear_active_to_last_marker();
        self.template_modes.pop();
        self.reset_insertion_mode();
    }

    pub(super) fn in_head_noscript(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Doctype { .. } => None,
            Token::StartTag { ref name, .. } if name == "html" => self.in_body(token),
            Token::EndTag { ref name } if name == "noscript" => {
                if self.current_is(Tag::Noscript) {
                    self.pop();
                }
                self.mode = Mode::InHead;
                None
            }
            Token::Text(text) => {
                let (ws, rest) = split_text(&text);
                self.insert_text(ws);
                let rest = rest?;
                self.in_head_noscript_anything_else(rest)
            }
            Token::Comment(_) => self.in_head(token),
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style"
                ) =>
            {
                self.in_head(token)
            }
            Token::StartTag { ref name, .. } if name == "head" || name == "noscript" => None,
            Token::EndTag { ref name } if name != "br" => None,
            other => self.in_head_noscript_anything_else(other),
        }
    }

    fn in_head_noscript_anything_else(&mut self, token: Token) -> Option<Token> {
        if self.current_is(Tag::Noscript) {
            self.pop();
        }
        self.mode = Mode::InHead;
        Some(token)
    }

    pub(super) fn after_head(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                let (ws, rest) = split_text(&text);
                self.insert_text(ws);
                let rest = rest?;
                self.after_head_anything_else(rest)
            }
            Token::Comment(data) => {
                self.insert_comment(data);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => match Tag::from_name(&name) {
                Tag::Html => self.in_body(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                Tag::Body => {
                    self.insert_html(&name, attrs);
                    self.frameset_ok = false;
                    self.mode = Mode::InBody;
                    None
                }
                Tag::Frameset => {
                    self.insert_html(&name, attrs);
                    self.mode = Mode::InFrameset;
                    None
                }
                Tag::Base
                | Tag::Basefont
                | Tag::Bgsound
                | Tag::Link
                | Tag::Meta
                | Tag::Noframes
                | Tag::Script
                | Tag::Style
                | Tag::Template
                | Tag::Title => {
                    // Parse error: process in the head element.
                    let token = Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    };
                    let Some(head) = self.head else {
                        return self.in_head(token);
                    };
                    self.push_existing(head, Tag::Head);
                    let result = self.in_head(token);
                    self.remove_node_from_stack(head);
                    result
                }
                Tag::Head => None,
                _ => self.after_head_anything_else(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
            },
            Token::EndTag { name } => match Tag::from_name(&name) {
                Tag::Template => self.in_head(Token::EndTag { name }),
                Tag::Body | Tag::Html | Tag::Br => {
                    self.after_head_anything_else(Token::EndTag { name })
                }
                _ => None,
            },
            Token::Eof => self.after_head_anything_else(Token::Eof),
        }
    }

    fn after_head_anything_else(&mut self, token: Token) -> Option<Token> {
        self.insert_html("body", Vec::new());
        self.mode = Mode::InBody;
        Some(token)
    }

    /// The "text" insertion mode (raw text and RCDATA elements).
    pub(super) fn text(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                self.insert_text(&text);
                None
            }
            Token::EndTag { .. } => {
                self.pop();
                self.mode = self.original_mode;
                None
            }
            other => {
                // EOF (or anything unexpected): close the element and reprocess.
                self.pop();
                self.mode = self.original_mode;
                Some(other)
            }
        }
    }

    pub(super) fn after_body(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(ref text) if is_all_whitespace(text) => self.in_body(token),
            Token::Comment(data) => {
                let html = self.open.first().map_or(self.doc.root(), |e| e.node);
                self.append_comment_to(html, data);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag { ref name, .. } if name == "html" => self.in_body(token),
            Token::EndTag { ref name } if name == "html" => {
                self.mode = Mode::AfterAfterBody;
                None
            }
            Token::Eof => {
                self.stopped = true;
                None
            }
            other => {
                self.mode = Mode::InBody;
                Some(other)
            }
        }
    }

    pub(super) fn in_frameset(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                let ws: String = text.chars().filter(|&c| is_html_whitespace(c)).collect();
                self.insert_text(&ws);
                None
            }
            Token::Comment(data) => {
                self.insert_comment(data);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => match Tag::from_name(&name) {
                Tag::Html => self.in_body(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                Tag::Frameset => {
                    self.insert_html(&name, attrs);
                    None
                }
                Tag::Frame => {
                    self.insert_html_void(&name, attrs);
                    None
                }
                Tag::Noframes => self.in_head(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                _ => None,
            },
            Token::EndTag { name } => {
                if name == "frameset" && self.open.len() > 1 && self.current_is(Tag::Frameset) {
                    self.pop();
                    if !self.current_is(Tag::Frameset) {
                        self.mode = Mode::AfterFrameset;
                    }
                }
                None
            }
            Token::Eof => {
                self.stopped = true;
                None
            }
        }
    }

    pub(super) fn after_frameset(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                let ws: String = text.chars().filter(|&c| is_html_whitespace(c)).collect();
                self.insert_text(&ws);
                None
            }
            Token::Comment(data) => {
                self.insert_comment(data);
                None
            }
            Token::StartTag { ref name, .. } if name == "html" => self.in_body(token),
            Token::StartTag { ref name, .. } if name == "noframes" => self.in_head(token),
            Token::EndTag { ref name } if name == "html" => {
                self.mode = Mode::AfterAfterFrameset;
                None
            }
            Token::Eof => {
                self.stopped = true;
                None
            }
            _ => None,
        }
    }

    pub(super) fn after_after_body(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Comment(data) => {
                let root = self.doc.root();
                self.append_comment_to(root, data);
                None
            }
            Token::Doctype { .. } => self.in_body(token),
            Token::Text(ref text) if is_all_whitespace(text) => self.in_body(token),
            Token::StartTag { ref name, .. } if name == "html" => self.in_body(token),
            Token::Eof => {
                self.stopped = true;
                None
            }
            other => {
                self.mode = Mode::InBody;
                Some(other)
            }
        }
    }

    pub(super) fn after_after_frameset(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Comment(data) => {
                let root = self.doc.root();
                self.append_comment_to(root, data);
                None
            }
            Token::Doctype { .. } => self.in_body(token),
            Token::Text(text) => {
                let ws: String = text.chars().filter(|&c| is_html_whitespace(c)).collect();
                if ws.is_empty() {
                    None
                } else {
                    self.in_body(Token::Text(ws))
                }
            }
            Token::StartTag { ref name, .. } if name == "html" => self.in_body(token),
            Token::StartTag { ref name, .. } if name == "noframes" => self.in_head(token),
            Token::Eof => {
                self.stopped = true;
                None
            }
            _ => None,
        }
    }
}
