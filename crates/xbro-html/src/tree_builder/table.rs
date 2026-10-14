//! Table insertion modes (in table, in table text, in caption, in column
//! group, in table body, in row, in cell), the select modes and the template
//! mode.

use super::stack::Scope;
use super::tags::Tag;
use super::{is_all_whitespace, split_leading_whitespace, Mode, TreeBuilder};
use crate::tokenizer::Token;

/// Tag name of a start or end tag token, interned.
fn tag_of(token: &Token) -> Tag {
    match token {
        Token::StartTag { name, .. } | Token::EndTag { name } => Tag::from_name(name),
        _ => Tag::Other,
    }
}

fn is_start(token: &Token) -> bool {
    matches!(token, Token::StartTag { .. })
}

fn is_end(token: &Token) -> bool {
    matches!(token, Token::EndTag { .. })
}

impl TreeBuilder {
    pub(super) fn in_table(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        match token {
            Token::Text(text) => {
                let table_context = self.current().is_some_and(|e| {
                    e.ns == super::tags::Ns::Html
                        && matches!(
                            e.tag,
                            Tag::Table
                                | Tag::Tbody
                                | Tag::Template
                                | Tag::Tfoot
                                | Tag::Thead
                                | Tag::Tr
                        )
                });
                if table_context {
                    self.in_table_text(text);
                    None
                } else {
                    self.in_table_anything_else(Token::Text(text))
                }
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
            } => match tag {
                Tag::Caption => {
                    self.clear_stack_back_to_table_context();
                    self.push_marker();
                    self.insert_html(&name, attrs);
                    self.mode = Mode::InCaption;
                    None
                }
                Tag::Colgroup => {
                    self.clear_stack_back_to_table_context();
                    self.insert_html(&name, attrs);
                    self.mode = Mode::InColumnGroup;
                    None
                }
                Tag::Col => {
                    self.clear_stack_back_to_table_context();
                    self.insert_html("colgroup", Vec::new());
                    self.mode = Mode::InColumnGroup;
                    Some(Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    })
                }
                Tag::Tbody | Tag::Tfoot | Tag::Thead => {
                    self.clear_stack_back_to_table_context();
                    self.insert_html(&name, attrs);
                    self.mode = Mode::InTableBody;
                    None
                }
                Tag::Td | Tag::Th | Tag::Tr => {
                    self.clear_stack_back_to_table_context();
                    self.insert_html("tbody", Vec::new());
                    self.mode = Mode::InTableBody;
                    Some(Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    })
                }
                Tag::Table => {
                    // Parse error: a nested <table> closes the current one.
                    if !self.in_scope(Tag::Table, Scope::Table) {
                        return None;
                    }
                    self.pop_until(Tag::Table);
                    self.reset_insertion_mode();
                    Some(Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    })
                }
                Tag::Style | Tag::Script | Tag::Template => self.in_head(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                Tag::Input
                    if attrs
                        .iter()
                        .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden")) =>
                {
                    self.insert_html_void(&name, attrs);
                    None
                }
                Tag::Form => {
                    if self.has_open(Tag::Template) || self.form.is_some() {
                        return None;
                    }
                    let form = self.insert_html_void(&name, attrs);
                    self.form = Some(form);
                    None
                }
                _ => self.in_table_anything_else(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
            },
            Token::EndTag { name } => match tag {
                Tag::Table => {
                    if self.in_scope(Tag::Table, Scope::Table) {
                        self.pop_until(Tag::Table);
                        self.reset_insertion_mode();
                    }
                    None
                }
                Tag::Body
                | Tag::Caption
                | Tag::Col
                | Tag::Colgroup
                | Tag::Html
                | Tag::Tbody
                | Tag::Td
                | Tag::Tfoot
                | Tag::Th
                | Tag::Thead
                | Tag::Tr => None,
                Tag::Template => self.in_head(Token::EndTag { name }),
                _ => self.in_table_anything_else(Token::EndTag { name }),
            },
            Token::Eof => self.in_body(Token::Eof),
        }
    }

    /// "Anything else" in table: in body rules with foster parenting.
    fn in_table_anything_else(&mut self, token: Token) -> Option<Token> {
        self.foster_parenting = true;
        let result = self.in_body(token);
        self.foster_parenting = false;
        result
    }

    /// The "in table text" mode. Text tokens are maximal runs, so the
    /// pending character tokens of the spec are exactly one token here.
    fn in_table_text(&mut self, text: String) {
        if is_all_whitespace(&text) {
            self.insert_text(&text);
        } else {
            // Parse error: foster-parent the text (in body rules).
            self.in_table_anything_else(Token::Text(text));
        }
    }

    /// Closes the caption; returns false when there is none in table scope.
    fn close_caption(&mut self) -> bool {
        if !self.in_scope(Tag::Caption, Scope::Table) {
            return false;
        }
        self.generate_implied_end_tags(None);
        self.pop_until(Tag::Caption);
        self.clear_active_to_last_marker();
        self.mode = Mode::InTable;
        true
    }

    pub(super) fn in_caption(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        if is_end(&token) && tag == Tag::Caption {
            self.close_caption();
            return None;
        }
        let reprocess_after_close = (is_start(&token)
            && matches!(
                tag,
                Tag::Caption
                    | Tag::Col
                    | Tag::Colgroup
                    | Tag::Tbody
                    | Tag::Td
                    | Tag::Tfoot
                    | Tag::Th
                    | Tag::Thead
                    | Tag::Tr
            ))
            || (is_end(&token) && tag == Tag::Table);
        if reprocess_after_close {
            return self.close_caption().then_some(token);
        }
        if is_end(&token)
            && matches!(
                tag,
                Tag::Body
                    | Tag::Col
                    | Tag::Colgroup
                    | Tag::Html
                    | Tag::Tbody
                    | Tag::Td
                    | Tag::Tfoot
                    | Tag::Th
                    | Tag::Thead
                    | Tag::Tr
            )
        {
            return None;
        }
        self.in_body(token)
    }

    pub(super) fn in_column_group(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        match token {
            Token::Text(text) => {
                let (ws, rest) = split_leading_whitespace(&text);
                self.insert_text(ws);
                if rest.is_empty() {
                    return None;
                }
                let rest = Token::Text(rest.to_string());
                self.in_column_group_anything_else(rest)
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
            } => match tag {
                Tag::Html => self.in_body(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                Tag::Col => {
                    self.insert_html_void(&name, attrs);
                    None
                }
                Tag::Template => self.in_head(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                _ => self.in_column_group_anything_else(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
            },
            Token::EndTag { name } => match tag {
                Tag::Colgroup => {
                    if self.current_is(Tag::Colgroup) {
                        self.pop();
                        self.mode = Mode::InTable;
                    }
                    None
                }
                Tag::Col => None,
                Tag::Template => self.in_head(Token::EndTag { name }),
                _ => self.in_column_group_anything_else(Token::EndTag { name }),
            },
            Token::Eof => self.in_body(Token::Eof),
        }
    }

    fn in_column_group_anything_else(&mut self, token: Token) -> Option<Token> {
        if !self.current_is(Tag::Colgroup) {
            return None;
        }
        self.pop();
        self.mode = Mode::InTable;
        Some(token)
    }

    pub(super) fn in_table_body(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        if is_start(&token) {
            match tag {
                Tag::Tr => {
                    self.clear_stack_back_to_table_body_context();
                    if let Token::StartTag { name, attrs, .. } = token {
                        self.insert_html(&name, attrs);
                    }
                    self.mode = Mode::InRow;
                    return None;
                }
                Tag::Th | Tag::Td => {
                    self.clear_stack_back_to_table_body_context();
                    self.insert_html("tr", Vec::new());
                    self.mode = Mode::InRow;
                    return Some(token);
                }
                Tag::Caption | Tag::Col | Tag::Colgroup | Tag::Tbody | Tag::Tfoot | Tag::Thead => {
                    return self.close_table_body().then_some(token);
                }
                _ => {}
            }
        } else if is_end(&token) {
            match tag {
                Tag::Tbody | Tag::Tfoot | Tag::Thead => {
                    if self.in_scope(tag, Scope::Table) {
                        self.clear_stack_back_to_table_body_context();
                        self.pop();
                        self.mode = Mode::InTable;
                    }
                    return None;
                }
                Tag::Table => return self.close_table_body().then_some(token),
                Tag::Body
                | Tag::Caption
                | Tag::Col
                | Tag::Colgroup
                | Tag::Html
                | Tag::Td
                | Tag::Th
                | Tag::Tr => return None,
                _ => {}
            }
        }
        self.in_table(token)
    }

    /// Closes the current tbody/thead/tfoot; false when none is in table scope.
    fn close_table_body(&mut self) -> bool {
        let any = self.in_scope(Tag::Tbody, Scope::Table)
            || self.in_scope(Tag::Thead, Scope::Table)
            || self.in_scope(Tag::Tfoot, Scope::Table);
        if !any {
            return false;
        }
        self.clear_stack_back_to_table_body_context();
        self.pop();
        self.mode = Mode::InTable;
        true
    }

    /// Closes the current row; false when there is no tr in table scope.
    fn close_row(&mut self) -> bool {
        if !self.in_scope(Tag::Tr, Scope::Table) {
            return false;
        }
        self.clear_stack_back_to_table_row_context();
        self.pop();
        self.mode = Mode::InTableBody;
        true
    }

    pub(super) fn in_row(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        if is_start(&token) {
            match tag {
                Tag::Th | Tag::Td => {
                    self.clear_stack_back_to_table_row_context();
                    if let Token::StartTag { name, attrs, .. } = token {
                        self.insert_html(&name, attrs);
                    }
                    self.mode = Mode::InCell;
                    self.push_marker();
                    return None;
                }
                Tag::Caption
                | Tag::Col
                | Tag::Colgroup
                | Tag::Tbody
                | Tag::Tfoot
                | Tag::Thead
                | Tag::Tr => return self.close_row().then_some(token),
                _ => {}
            }
        } else if is_end(&token) {
            match tag {
                Tag::Tr => {
                    self.close_row();
                    return None;
                }
                Tag::Table => return self.close_row().then_some(token),
                Tag::Tbody | Tag::Tfoot | Tag::Thead => {
                    if !self.in_scope(tag, Scope::Table) {
                        return None;
                    }
                    return self.close_row().then_some(token);
                }
                Tag::Body
                | Tag::Caption
                | Tag::Col
                | Tag::Colgroup
                | Tag::Html
                | Tag::Td
                | Tag::Th => {
                    return None;
                }
                _ => {}
            }
        }
        self.in_table(token)
    }

    /// "Close the cell".
    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None);
        self.pop_until_any(|t| t == Tag::Td || t == Tag::Th);
        self.clear_active_to_last_marker();
        self.mode = Mode::InRow;
    }

    pub(super) fn in_cell(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        if is_end(&token) {
            match tag {
                Tag::Td | Tag::Th => {
                    if self.in_scope(tag, Scope::Table) {
                        self.generate_implied_end_tags(None);
                        self.pop_until(tag);
                        self.clear_active_to_last_marker();
                        self.mode = Mode::InRow;
                    }
                    return None;
                }
                Tag::Body | Tag::Caption | Tag::Col | Tag::Colgroup | Tag::Html => return None,
                Tag::Table | Tag::Tbody | Tag::Tfoot | Tag::Thead | Tag::Tr => {
                    if !self.in_scope(tag, Scope::Table) {
                        return None;
                    }
                    self.close_cell();
                    return Some(token);
                }
                _ => {}
            }
        } else if is_start(&token)
            && matches!(
                tag,
                Tag::Caption
                    | Tag::Col
                    | Tag::Colgroup
                    | Tag::Tbody
                    | Tag::Td
                    | Tag::Tfoot
                    | Tag::Th
                    | Tag::Thead
                    | Tag::Tr
            )
        {
            if !self.in_scope(Tag::Td, Scope::Table) && !self.in_scope(Tag::Th, Scope::Table) {
                return None;
            }
            self.close_cell();
            return Some(token);
        }
        self.in_body(token)
    }

    /// Pops up to the select element and resets the insertion mode; false
    /// when there is no select in select scope.
    fn close_select(&mut self) -> bool {
        if !self.in_scope(Tag::Select, Scope::Select) {
            return false;
        }
        self.pop_until(Tag::Select);
        self.reset_insertion_mode();
        true
    }

    pub(super) fn in_select(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        match token {
            Token::Text(text) => {
                self.insert_text(&text);
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
            } => match tag {
                Tag::Html => self.in_body(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                Tag::Option => {
                    if self.current_is(Tag::Option) {
                        self.pop();
                    }
                    self.insert_html(&name, attrs);
                    None
                }
                Tag::Optgroup => {
                    if self.current_is(Tag::Option) {
                        self.pop();
                    }
                    if self.current_is(Tag::Optgroup) {
                        self.pop();
                    }
                    self.insert_html(&name, attrs);
                    None
                }
                Tag::Hr => {
                    if self.current_is(Tag::Option) {
                        self.pop();
                    }
                    if self.current_is(Tag::Optgroup) {
                        self.pop();
                    }
                    self.insert_html_void(&name, attrs);
                    None
                }
                Tag::Select => {
                    // Parse error: a nested <select> closes the current one.
                    self.close_select();
                    None
                }
                Tag::Input | Tag::Keygen | Tag::Textarea => {
                    let token = Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    };
                    self.close_select().then_some(token)
                }
                Tag::Script | Tag::Template => self.in_head(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                }),
                _ => None,
            },
            Token::EndTag { name } => match tag {
                Tag::Optgroup => {
                    let len = self.open.len();
                    let below_is_optgroup = len >= 2
                        && self
                            .open
                            .get(len - 2)
                            .is_some_and(|e| e.is_html(Tag::Optgroup));
                    if self.current_is(Tag::Option) && below_is_optgroup {
                        self.pop();
                    }
                    if self.current_is(Tag::Optgroup) {
                        self.pop();
                    }
                    None
                }
                Tag::Option => {
                    if self.current_is(Tag::Option) {
                        self.pop();
                    }
                    None
                }
                Tag::Select => {
                    self.close_select();
                    None
                }
                Tag::Template => self.in_head(Token::EndTag { name }),
                _ => None,
            },
            Token::Eof => self.in_body(Token::Eof),
        }
    }

    pub(super) fn in_select_in_table(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        let table_tag = matches!(
            tag,
            Tag::Caption
                | Tag::Table
                | Tag::Tbody
                | Tag::Tfoot
                | Tag::Thead
                | Tag::Tr
                | Tag::Td
                | Tag::Th
        );
        if table_tag && is_start(&token) {
            self.pop_until(Tag::Select);
            self.reset_insertion_mode();
            return Some(token);
        }
        if table_tag && is_end(&token) {
            if !self.in_scope(tag, Scope::Table) {
                return None;
            }
            self.pop_until(Tag::Select);
            self.reset_insertion_mode();
            return Some(token);
        }
        self.in_select(token)
    }

    pub(super) fn in_template(&mut self, token: Token) -> Option<Token> {
        let tag = tag_of(&token);
        match token {
            Token::Text(_) | Token::Comment(_) | Token::Doctype { .. } => self.in_body(token),
            Token::StartTag { .. } => {
                let new_mode = match tag {
                    Tag::Base
                    | Tag::Basefont
                    | Tag::Bgsound
                    | Tag::Link
                    | Tag::Meta
                    | Tag::Noframes
                    | Tag::Script
                    | Tag::Style
                    | Tag::Template
                    | Tag::Title => return self.in_head(token),
                    Tag::Caption | Tag::Colgroup | Tag::Tbody | Tag::Tfoot | Tag::Thead => {
                        Mode::InTable
                    }
                    Tag::Col => Mode::InColumnGroup,
                    Tag::Tr => Mode::InTableBody,
                    Tag::Td | Tag::Th => Mode::InRow,
                    _ => Mode::InBody,
                };
                self.template_modes.pop();
                self.template_modes.push(new_mode);
                self.mode = new_mode;
                Some(token)
            }
            Token::EndTag { .. } => {
                if tag == Tag::Template {
                    self.in_head(token)
                } else {
                    None
                }
            }
            Token::Eof => {
                if !self.has_open(Tag::Template) {
                    self.stopped = true;
                    return None;
                }
                self.pop_until(Tag::Template);
                self.clear_active_to_last_marker();
                self.template_modes.pop();
                self.reset_insertion_mode();
                Some(Token::Eof)
            }
        }
    }
}
