//! The "in body" insertion mode and the adoption agency algorithm.

use xbro_dom::Attribute;

use super::stack::Scope;
use super::tags::{Ns, Tag};
use super::{is_all_whitespace, Formatting, Mode, OpenElement, TreeBuilder};
use crate::tokenizer::{TextMode, Token};

impl TreeBuilder {
    pub(super) fn in_body(&mut self, token: Token) -> Option<Token> {
        match token {
            Token::Text(text) => {
                self.reconstruct_active_formatting();
                self.insert_text(&text);
                if !is_all_whitespace(&text) {
                    self.frameset_ok = false;
                }
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
            } => self.in_body_start_tag(name, attrs, self_closing),
            Token::EndTag { name } => self.in_body_end_tag(name),
            Token::Eof => {
                if !self.template_modes.is_empty() {
                    return self.in_template(Token::Eof);
                }
                self.stopped = true;
                None
            }
        }
    }

    /// Adds the attributes of a stray `<html>`/`<body>` start tag that the
    /// existing element does not have yet.
    ///
    /// The names already present are kept in a set per merged element
    /// (built on its first merge), so repeated stray tags cost O(their
    /// attributes) instead of O(attributes already on the element) each.
    fn merge_attributes(&mut self, index: usize, attrs: Vec<Attribute>) {
        if attrs.is_empty() {
            return;
        }
        let Some(node) = self.open.get(index).map(|e| e.node) else {
            return;
        };
        let Some(element) = self.doc.element_mut(node) else {
            return;
        };
        let names = self.merged_attr_names.entry(node).or_insert_with(|| {
            element
                .attrs
                .iter()
                .map(|a| a.name.to_ascii_lowercase())
                .collect()
        });
        for attr in attrs {
            if names.insert(attr.name.to_ascii_lowercase()) {
                element.attrs.push(attr);
            }
        }
    }

    fn in_body_start_tag(
        &mut self,
        name: String,
        attrs: Vec<Attribute>,
        self_closing: bool,
    ) -> Option<Token> {
        let tag = Tag::from_name(&name);
        match tag {
            Tag::Html => {
                if !self.has_open(Tag::Template) {
                    self.merge_attributes(0, attrs);
                }
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
                return self.in_head(Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                });
            }
            Tag::Body => {
                let second_is_body = self.open.get(1).is_some_and(|e| e.is_html(Tag::Body));
                if second_is_body && !self.has_open(Tag::Template) {
                    self.frameset_ok = false;
                    self.merge_attributes(1, attrs);
                }
            }
            Tag::Frameset => {
                let body = self
                    .open
                    .get(1)
                    .filter(|e| e.is_html(Tag::Body))
                    .map(|e| e.node);
                if let (Some(body), true) = (body, self.frameset_ok) {
                    self.doc.detach(body);
                    self.truncate_open(1);
                    self.insert_html(&name, attrs);
                    self.mode = Mode::InFrameset;
                }
            }
            Tag::Address
            | Tag::Article
            | Tag::Aside
            | Tag::Blockquote
            | Tag::Center
            | Tag::Details
            | Tag::Dialog
            | Tag::Dir
            | Tag::Div
            | Tag::Dl
            | Tag::Fieldset
            | Tag::Figcaption
            | Tag::Figure
            | Tag::Footer
            | Tag::Header
            | Tag::Hgroup
            | Tag::Main
            | Tag::Menu
            | Tag::Nav
            | Tag::Ol
            | Tag::P
            | Tag::Search
            | Tag::Section
            | Tag::Summary
            | Tag::Ul => {
                self.close_p_in_button_scope();
                self.insert_html(&name, attrs);
            }
            Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6 => {
                self.close_p_in_button_scope();
                if self
                    .current()
                    .is_some_and(|e| e.ns == Ns::Html && e.tag.is_heading())
                {
                    self.pop();
                }
                self.insert_html(&name, attrs);
            }
            Tag::Pre | Tag::Listing => {
                self.close_p_in_button_scope();
                self.insert_html(&name, attrs);
                self.skip_newline = true;
                self.frameset_ok = false;
            }
            Tag::Form => {
                let has_template = self.has_open(Tag::Template);
                if self.form.is_some() && !has_template {
                    return None;
                }
                self.close_p_in_button_scope();
                let form = self.insert_html(&name, attrs);
                if !has_template {
                    self.form = Some(form);
                }
            }
            Tag::Li => {
                self.frameset_ok = false;
                self.close_list_item(|t| t == Tag::Li);
                self.close_p_in_button_scope();
                self.insert_html(&name, attrs);
            }
            Tag::Dd | Tag::Dt => {
                self.frameset_ok = false;
                self.close_list_item(|t| t == Tag::Dd || t == Tag::Dt);
                self.close_p_in_button_scope();
                self.insert_html(&name, attrs);
            }
            Tag::Plaintext => {
                self.close_p_in_button_scope();
                self.insert_html(&name, attrs);
                self.request_text_mode(TextMode::PlainText, "plaintext");
            }
            Tag::Button => {
                if self.in_scope(Tag::Button, Scope::Default) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(Tag::Button);
                }
                self.reconstruct_active_formatting();
                self.insert_html(&name, attrs);
                self.frameset_ok = false;
            }
            Tag::A => {
                if let Some((_, existing)) = self.formatting_element_for(Tag::A) {
                    // Parse error: an <a> inside an <a> closes the first one.
                    if !self.adoption_agency(Tag::A) {
                        self.any_other_end_tag(Tag::A, "a");
                    }
                    self.remove_formatting_node(existing);
                    self.remove_node_from_stack(existing);
                }
                self.reconstruct_active_formatting();
                let node = self.insert_html(&name, attrs);
                self.push_formatting(node, tag);
            }
            Tag::B
            | Tag::Big
            | Tag::Code
            | Tag::Em
            | Tag::Font
            | Tag::I
            | Tag::S
            | Tag::Small
            | Tag::Strike
            | Tag::Strong
            | Tag::Tt
            | Tag::U => {
                self.reconstruct_active_formatting();
                let node = self.insert_html(&name, attrs);
                self.push_formatting(node, tag);
            }
            Tag::Nobr => {
                self.reconstruct_active_formatting();
                if self.in_scope(Tag::Nobr, Scope::Default) {
                    if !self.adoption_agency(Tag::Nobr) {
                        self.any_other_end_tag(Tag::Nobr, "nobr");
                    }
                    self.reconstruct_active_formatting();
                }
                let node = self.insert_html(&name, attrs);
                self.push_formatting(node, tag);
            }
            Tag::Applet | Tag::Marquee | Tag::Object => {
                self.reconstruct_active_formatting();
                self.insert_html(&name, attrs);
                self.push_marker();
                self.frameset_ok = false;
            }
            Tag::Table => {
                if !self.doc.quirks_mode {
                    self.close_p_in_button_scope();
                }
                self.insert_html(&name, attrs);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            Tag::Area | Tag::Br | Tag::Embed | Tag::Img | Tag::Keygen | Tag::Wbr => {
                self.reconstruct_active_formatting();
                self.insert_html_void(&name, attrs);
                self.frameset_ok = false;
            }
            Tag::Input => {
                let hidden = attrs
                    .iter()
                    .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
                self.reconstruct_active_formatting();
                self.insert_html_void(&name, attrs);
                if !hidden {
                    self.frameset_ok = false;
                }
            }
            Tag::Param | Tag::Source | Tag::Track => {
                self.insert_html_void(&name, attrs);
            }
            Tag::Hr => {
                self.close_p_in_button_scope();
                self.insert_html_void(&name, attrs);
                self.frameset_ok = false;
            }
            Tag::Image => {
                // Parse error: <image> is treated as <img>.
                return Some(Token::StartTag {
                    name: "img".to_string(),
                    attrs,
                    self_closing,
                });
            }
            Tag::Textarea => {
                self.insert_html(&name, attrs);
                self.skip_newline = true;
                self.request_text_mode(TextMode::RcData, "textarea");
                self.original_mode = self.mode;
                self.frameset_ok = false;
                self.mode = Mode::Text;
            }
            Tag::Xmp => {
                self.close_p_in_button_scope();
                self.reconstruct_active_formatting();
                self.frameset_ok = false;
                self.parse_text_element(&name, attrs, TextMode::RawText);
            }
            Tag::Iframe => {
                self.frameset_ok = false;
                self.parse_text_element(&name, attrs, TextMode::RawText);
            }
            Tag::Noembed => {
                self.parse_text_element(&name, attrs, TextMode::RawText);
            }
            Tag::Select => {
                self.reconstruct_active_formatting();
                self.insert_html(&name, attrs);
                self.frameset_ok = false;
                self.mode = match self.mode {
                    Mode::InTable
                    | Mode::InCaption
                    | Mode::InTableBody
                    | Mode::InRow
                    | Mode::InCell => Mode::InSelectInTable,
                    _ => Mode::InSelect,
                };
            }
            Tag::Optgroup | Tag::Option => {
                if self.current_is(Tag::Option) {
                    self.pop();
                }
                self.reconstruct_active_formatting();
                self.insert_html(&name, attrs);
            }
            Tag::Rb | Tag::Rtc => {
                if self.in_scope(Tag::Ruby, Scope::Default) {
                    self.generate_implied_end_tags(None);
                }
                self.insert_html(&name, attrs);
            }
            Tag::Rp | Tag::Rt => {
                if self.in_scope(Tag::Ruby, Scope::Default) {
                    self.generate_implied_end_tags(Some(Tag::Rtc));
                }
                self.insert_html(&name, attrs);
            }
            Tag::Math => {
                self.reconstruct_active_formatting();
                self.insert_foreign(&name, attrs, Ns::MathMl, self_closing);
            }
            Tag::Svg => {
                self.reconstruct_active_formatting();
                self.insert_foreign(&name, attrs, Ns::Svg, self_closing);
            }
            Tag::Caption
            | Tag::Col
            | Tag::Colgroup
            | Tag::Frame
            | Tag::Head
            | Tag::Tbody
            | Tag::Td
            | Tag::Tfoot
            | Tag::Th
            | Tag::Thead
            | Tag::Tr => {
                // Parse error: ignored.
            }
            _ => {
                self.reconstruct_active_formatting();
                self.insert_html(&name, attrs);
            }
        }
        None
    }

    /// The `<li>`/`<dd>`/`<dt>` loop: closes an open list item unless a
    /// special element (other than address, div and p) comes first.
    fn close_list_item(&mut self, is_item: impl Fn(Tag) -> bool) {
        let candidates = [Tag::Li, Tag::Dd, Tag::Dt];
        if !candidates.iter().any(|&t| is_item(t) && self.has_open(t)) {
            return;
        }
        let mut found = None;
        for e in self.open.iter().rev() {
            if e.ns == Ns::Html && is_item(e.tag) {
                found = Some(e.tag);
                break;
            }
            if e.is_special()
                && !(e.ns == Ns::Html && matches!(e.tag, Tag::Address | Tag::Div | Tag::P))
            {
                break;
            }
        }
        if let Some(item) = found {
            self.generate_implied_end_tags(Some(item));
            self.pop_until(item);
        }
    }

    fn in_body_end_tag(&mut self, name: String) -> Option<Token> {
        let tag = Tag::from_name(&name);
        match tag {
            Tag::Template => return self.in_head(Token::EndTag { name }),
            Tag::Body => {
                if self.in_scope(Tag::Body, Scope::Default) {
                    self.mode = Mode::AfterBody;
                }
            }
            Tag::Html => {
                if self.in_scope(Tag::Body, Scope::Default) {
                    self.mode = Mode::AfterBody;
                    return Some(Token::EndTag { name });
                }
            }
            Tag::Address
            | Tag::Article
            | Tag::Aside
            | Tag::Blockquote
            | Tag::Button
            | Tag::Center
            | Tag::Details
            | Tag::Dialog
            | Tag::Dir
            | Tag::Div
            | Tag::Dl
            | Tag::Fieldset
            | Tag::Figcaption
            | Tag::Figure
            | Tag::Footer
            | Tag::Header
            | Tag::Hgroup
            | Tag::Listing
            | Tag::Main
            | Tag::Menu
            | Tag::Nav
            | Tag::Ol
            | Tag::Pre
            | Tag::Search
            | Tag::Section
            | Tag::Summary
            | Tag::Ul => {
                if self.in_scope(tag, Scope::Default) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(tag);
                }
            }
            Tag::Form => {
                if self.has_open(Tag::Template) {
                    if self.in_scope(Tag::Form, Scope::Default) {
                        self.generate_implied_end_tags(None);
                        self.pop_until(Tag::Form);
                    }
                } else {
                    let node = self.form.take();
                    if let Some(node) = node {
                        if self.node_in_scope(node, Scope::Default) {
                            self.generate_implied_end_tags(None);
                            self.remove_node_from_stack(node);
                        }
                    }
                }
            }
            Tag::P => {
                if !self.in_scope(Tag::P, Scope::Button) {
                    // Parse error: `</p>` without an open p creates an empty one.
                    self.insert_html("p", Vec::new());
                }
                self.close_p();
            }
            Tag::Li => {
                if self.in_scope(Tag::Li, Scope::ListItem) {
                    self.generate_implied_end_tags(Some(Tag::Li));
                    self.pop_until(Tag::Li);
                }
            }
            Tag::Dd | Tag::Dt => {
                if self.in_scope(tag, Scope::Default) {
                    self.generate_implied_end_tags(Some(tag));
                    self.pop_until(tag);
                }
            }
            Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6 => {
                let headings = [Tag::H1, Tag::H2, Tag::H3, Tag::H4, Tag::H5, Tag::H6];
                let any_open = headings.iter().any(|&h| self.has_open(h));
                if any_open && self.in_scope_any(Scope::Default, Tag::is_heading) {
                    self.generate_implied_end_tags(None);
                    self.pop_until_any(Tag::is_heading);
                }
            }
            Tag::A
            | Tag::B
            | Tag::Big
            | Tag::Code
            | Tag::Em
            | Tag::Font
            | Tag::I
            | Tag::Nobr
            | Tag::S
            | Tag::Small
            | Tag::Strike
            | Tag::Strong
            | Tag::Tt
            | Tag::U => {
                if !self.adoption_agency(tag) {
                    self.any_other_end_tag(tag, &name);
                }
            }
            Tag::Applet | Tag::Marquee | Tag::Object => {
                if self.in_scope(tag, Scope::Default) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(tag);
                    self.clear_active_to_last_marker();
                }
            }
            Tag::Br => {
                // Parse error: `</br>` is treated as `<br>`.
                self.reconstruct_active_formatting();
                self.insert_html_void("br", Vec::new());
                self.frameset_ok = false;
            }
            _ => self.any_other_end_tag(tag, &name),
        }
        None
    }

    /// "Any other end tag" in body.
    pub(super) fn any_other_end_tag(&mut self, tag: Tag, name: &str) {
        // No such element is open (for unknown names: no unknown HTML
        // element at all): the walk could only end on a special element,
        // i.e. the token is ignored.
        if !self.has_open(tag) {
            return;
        }
        let mut target = None;
        for (i, e) in self.open.iter().enumerate().rev() {
            let matches = e.ns == Ns::Html
                && e.tag == tag
                && (tag != Tag::Other || self.doc.tag_name(e.node) == Some(name));
            if matches {
                target = Some(i);
                break;
            }
            if e.is_special() {
                return;
            }
        }
        let Some(index) = target else {
            return;
        };
        // Generate implied end tags, except for elements with this name.
        while let Some(cur) = self.open.last() {
            let same = self.doc.tag_name(cur.node) == Some(name);
            if self.open.len() - 1 > index
                && cur.ns == Ns::Html
                && cur.tag.has_implied_end_tag()
                && !same
            {
                if self.pop().is_none() {
                    break;
                }
            } else {
                break;
            }
        }
        self.truncate_open(index);
    }

    /// The adoption agency algorithm. Returns `false` when the caller must
    /// fall back to "any other end tag".
    pub(super) fn adoption_agency(&mut self, tag: Tag) -> bool {
        // Step 2: the current node is the subject and not a formatting entry.
        if let Some(cur) = self.current() {
            if cur.is_html(tag) && self.formatting_index_of(cur.node).is_none() {
                self.pop();
                return true;
            }
        }
        for _ in 0..8 {
            let Some((fe_afe_index, fe)) = self.formatting_element_for(tag) else {
                return false;
            };
            let Some(fe_index) = self.position_of(fe) else {
                // Not on the stack: parse error, drop it from the list.
                self.active.remove(fe_afe_index);
                return true;
            };
            if !self.node_in_scope(fe, Scope::Default) {
                return true;
            }
            let furthest = self
                .open
                .iter()
                .enumerate()
                .skip(fe_index + 1)
                .find(|(_, e)| e.is_special())
                .map(|(i, e)| (i, e.node));
            let Some((mut fb_index, furthest_block)) = furthest else {
                // No furthest block: pop up to and including the formatting element.
                self.truncate_open(fe_index);
                self.remove_formatting_node(fe);
                return true;
            };
            let Some(common_ancestor) = fe_index
                .checked_sub(1)
                .and_then(|i| self.open.get(i))
                .map(|e| e.node)
            else {
                return true;
            };
            let mut bookmark = fe_afe_index;
            let mut node_index = fb_index;
            let mut last_node = furthest_block;
            let mut inner = 0usize;
            loop {
                inner += 1;
                node_index = match node_index.checked_sub(1) {
                    Some(i) => i,
                    None => break,
                };
                let Some(entry) = self.open.get(node_index).copied() else {
                    break;
                };
                if entry.node == fe {
                    break;
                }
                let mut afe_index = self.formatting_index_of(entry.node);
                if inner > 3 {
                    if let Some(i) = afe_index {
                        self.active.remove(i);
                        if i < bookmark {
                            bookmark -= 1;
                        }
                        afe_index = None;
                    }
                }
                let Some(afe_index) = afe_index else {
                    self.remove_open_at(node_index);
                    fb_index -= 1;
                    continue;
                };
                // Replace the node with a fresh clone.
                let Some(element) = self.doc.element(entry.node).cloned() else {
                    break;
                };
                let clone = self.create_detached(&element.name, element.attrs);
                self.active[afe_index] = Formatting::Element(clone, entry.tag);
                self.replace_open_at(
                    node_index,
                    OpenElement {
                        node: clone,
                        ..entry
                    },
                );
                if last_node == furthest_block {
                    bookmark = afe_index + 1;
                }
                self.append_moved(clone, last_node);
                last_node = clone;
            }
            // Insert the last node at the appropriate place for the common ancestor.
            let pos = self.appropriate_place(Some(common_ancestor));
            self.move_node(last_node, pos);
            // Refresh the recorded depths along the chain of clones down to
            // the furthest block (at most a few elements long).
            let mut chain_node = last_node;
            let mut chain_depth = self.depth_of(last_node);
            for _ in 0..16 {
                if chain_node == furthest_block {
                    break;
                }
                let Some(&child) = self.doc.children(chain_node).last() else {
                    break;
                };
                chain_depth += 1;
                self.set_node_depth(child, chain_depth);
                chain_node = child;
            }
            // Wrap the furthest block's children in a clone of the formatting element.
            let Some(element) = self.doc.element(fe).cloned() else {
                return true;
            };
            let new_element = self.create_detached(&element.name, element.attrs);
            let children = std::mem::take(&mut self.doc.node_mut(furthest_block).children);
            for &child in &children {
                self.doc.node_mut(child).parent = Some(new_element);
            }
            self.doc.node_mut(new_element).children = children;
            self.append_moved(furthest_block, new_element);
            // Fix up the list of active formatting elements.
            if let Some(i) = self.formatting_index_of(fe) {
                self.active.remove(i);
                if i < bookmark {
                    bookmark -= 1;
                }
            }
            let bookmark = bookmark.min(self.active.len());
            self.active
                .insert(bookmark, Formatting::Element(new_element, tag));
            // Fix up the stack of open elements.
            self.remove_node_from_stack(fe);
            let fb_now = self
                .position_of(furthest_block)
                .unwrap_or(fb_index.saturating_sub(1));
            self.insert_open_at(
                fb_now + 1,
                OpenElement {
                    node: new_element,
                    tag,
                    ns: Ns::Html,
                    html_integration_point: false,
                },
            );
        }
        true
    }
}
