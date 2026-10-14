//! Form controls state and submission (GET and POST urlencoded).
//!
//! Control state lives in the DOM, where layout reads it: the `value`
//! attribute of text inputs, the `checked` attribute of checkboxes and
//! radios, the `selected` attribute of options and the text of textareas.
//! The values present when the page was loaded are kept in [`FormDefaults`]
//! for `<input type=reset>`.

use std::collections::HashMap;

use xbro_dom::{Document, NodeData, NodeId};
use xbro_net::{encode_form, resolve, Url};

use crate::NavigationRequest;

/// Input types edited as text (everything that is not a button, a box, a
/// file, a range, a color picker or hidden).
fn is_textual_type(ty: &str) -> bool {
    !matches!(
        ty,
        "checkbox"
            | "radio"
            | "submit"
            | "reset"
            | "button"
            | "image"
            | "file"
            | "hidden"
            | "range"
            | "color"
    )
}

/// Lowercased, trimmed `type` of an `<input>` (default `text`).
pub(crate) fn input_type(doc: &Document, node: NodeId) -> String {
    let ty = doc
        .attr(node, "type")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if ty.is_empty() {
        "text".to_string()
    } else {
        ty
    }
}

/// `<input>` of a textual type.
pub(crate) fn is_text_input(doc: &Document, node: NodeId) -> bool {
    doc.is_element_named(node, "input") && is_textual_type(&input_type(doc, node))
}

/// Text input or textarea.
pub(crate) fn is_text_field(doc: &Document, node: NodeId) -> bool {
    is_text_input(doc, node) || doc.is_element_named(node, "textarea")
}

/// `<button>` type: `submit` (default, also for invalid values), `reset` or `button`.
pub(crate) fn button_type(doc: &Document, node: NodeId) -> &'static str {
    match doc
        .attr(node, "type")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "reset" => "reset",
        "button" => "button",
        _ => "submit",
    }
}

/// Submit buttons: `<input type=submit|image>` and `<button type=submit>`.
pub(crate) fn is_submit_button(doc: &Document, node: NodeId) -> bool {
    match doc.tag_name(node) {
        Some("input") => matches!(input_type(doc, node).as_str(), "submit" | "image"),
        Some("button") => button_type(doc, node) == "submit",
        _ => false,
    }
}

/// Form-associated elements that take part in submission.
fn is_listed_control(doc: &Document, node: NodeId) -> bool {
    matches!(
        doc.tag_name(node),
        Some("input" | "select" | "textarea" | "button")
    )
}

/// `disabled` attribute on the control, or inside a disabled `<fieldset>`
/// (except in that fieldset's first `<legend>`).
pub(crate) fn is_disabled(doc: &Document, node: NodeId) -> bool {
    if doc.attr(node, "disabled").is_some() {
        return true;
    }
    let mut child = node;
    for ancestor in doc.ancestors(node) {
        if doc.is_element_named(ancestor, "fieldset")
            && doc.attr(ancestor, "disabled").is_some()
            && !is_first_legend(doc, child)
        {
            return true;
        }
        child = ancestor;
    }
    false
}

/// True when `node` is a `<legend>` with no `<legend>` element sibling
/// before it (the first legend child of its parent). Only legends pay for
/// the sibling walk, which stops at the first earlier legend, so checking
/// every control of a huge disabled fieldset stays linear.
fn is_first_legend(doc: &Document, node: NodeId) -> bool {
    if !doc.is_element_named(node, "legend") {
        return false;
    }
    let mut sibling = doc.prev_sibling_element(node);
    while let Some(s) = sibling {
        if doc.is_element_named(s, "legend") {
            return false;
        }
        sibling = doc.prev_sibling_element(s);
    }
    true
}

/// `readonly` text fields keep focus but cannot be edited.
pub(crate) fn is_readonly(doc: &Document, node: NodeId) -> bool {
    doc.attr(node, "readonly").is_some()
}

/// The form a control belongs to: the element named by its `form`
/// attribute (when it is a `<form>`), else its nearest `<form>` ancestor.
pub(crate) fn form_owner(doc: &Document, control: NodeId) -> Option<NodeId> {
    Owners::new(doc).owner(control)
}

/// Form owner lookups for many controls: the `id` index needed by `form`
/// attributes is built once (a lookup by id scans the whole document, so
/// thousands of `<input form=…>` made each submission quadratic).
pub(crate) struct Owners<'a> {
    doc: &'a Document,
    ids: Option<HashMap<&'a str, NodeId>>,
}

impl<'a> Owners<'a> {
    pub(crate) fn new(doc: &'a Document) -> Self {
        Self { doc, ids: None }
    }

    /// See [`form_owner`].
    pub(crate) fn owner(&mut self, control: NodeId) -> Option<NodeId> {
        let doc = self.doc;
        if let Some(id) = doc.attr(control, "form") {
            let id = id.trim();
            if !id.is_empty() {
                let ids = self.ids.get_or_insert_with(|| {
                    let mut ids = HashMap::new();
                    for n in doc.descendants(doc.root()) {
                        if let Some(id) = doc.element(n).and_then(|e| e.id()) {
                            ids.entry(id).or_insert(n);
                        }
                    }
                    ids
                });
                return ids
                    .get(id)
                    .copied()
                    .filter(|&f| doc.is_element_named(f, "form"));
            }
        }
        doc.ancestors(control)
            .find(|&a| doc.is_element_named(a, "form"))
    }
}

/// Whitespace-collapsed text of an element.
fn collapsed_text(doc: &Document, node: NodeId) -> String {
    doc.text_content(node)
        .split_ascii_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `<option>` elements of a `<select>` (inside optgroups too, at most 10 000).
pub(crate) fn select_options(doc: &Document, select: NodeId) -> Vec<NodeId> {
    doc.descendants(select)
        .filter(|&n| doc.is_element_named(n, "option"))
        .take(10_000)
        .collect()
}

/// Value of an `<option>`: its `value` attribute, else its collapsed text.
fn option_value(doc: &Document, option: NodeId) -> String {
    match doc.attr(option, "value") {
        Some(v) => v.to_string(),
        None => collapsed_text(doc, option),
    }
}

fn option_disabled(doc: &Document, option: NodeId) -> bool {
    doc.attr(option, "disabled").is_some()
        || doc.parent(option).is_some_and(|p| {
            doc.is_element_named(p, "optgroup") && doc.attr(p, "disabled").is_some()
        })
}

/// Selected options of a `<select>`: those with `selected` (the last one for
/// a single select), else the first enabled option of a single select.
pub(crate) fn selected_options(doc: &Document, select: NodeId) -> Vec<NodeId> {
    let options = select_options(doc, select);
    let selected: Vec<NodeId> = options
        .iter()
        .copied()
        .filter(|&o| doc.attr(o, "selected").is_some())
        .collect();
    if doc.attr(select, "multiple").is_some() {
        return selected;
    }
    if let Some(&last) = selected.last() {
        return vec![last];
    }
    options
        .into_iter()
        .find(|&o| !option_disabled(doc, o))
        .into_iter()
        .collect()
}

/// Text value of a textarea (its text content).
pub(crate) fn textarea_value(doc: &Document, textarea: NodeId) -> String {
    doc.text_content(textarea)
}

/// Converts lone `\n` and `\r` to `\r\n` (form submission newline normalization).
fn crlf(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\r\n");
            }
            '\n' => out.push_str("\r\n"),
            c => out.push(c),
        }
    }
    out
}

/// The button that submits a form, and for image buttons the click position.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Submitter {
    pub node: NodeId,
    /// Click position relative to an image button (`name.x` / `name.y`).
    pub coords: Option<(i32, i32)>,
}

/// The form data set (HTML "constructing the entry list"): name/value pairs
/// of the successful controls of `form`, in tree order.
pub(crate) fn form_data(
    doc: &Document,
    form: NodeId,
    submitter: Option<Submitter>,
) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut owners = Owners::new(doc);
    for node in doc.descendants(doc.root()) {
        if !is_listed_control(doc, node) || owners.owner(node) != Some(form) {
            continue;
        }
        if is_disabled(doc, node)
            || doc
                .ancestors(node)
                .any(|a| doc.is_element_named(a, "datalist"))
        {
            continue;
        }
        let is_submitter = submitter.is_some_and(|s| s.node == node);
        let name = doc.attr(node, "name").unwrap_or("").to_string();
        match doc.tag_name(node) {
            Some("input") => {
                let ty = input_type(doc, node);
                if ty == "image" {
                    if is_submitter {
                        let (x, y) = submitter.and_then(|s| s.coords).unwrap_or((0, 0));
                        let prefix = if name.is_empty() {
                            String::new()
                        } else {
                            format!("{name}.")
                        };
                        fields.push((format!("{prefix}x"), x.to_string()));
                        fields.push((format!("{prefix}y"), y.to_string()));
                    }
                    continue;
                }
                if name.is_empty() {
                    continue;
                }
                let value = doc.attr(node, "value");
                match ty.as_str() {
                    "checkbox" | "radio" => {
                        if doc.attr(node, "checked").is_some() {
                            fields.push((name, value.unwrap_or("on").to_string()));
                        }
                    }
                    "submit" => {
                        if is_submitter {
                            fields.push((name, value.unwrap_or("Submit").to_string()));
                        }
                    }
                    "button" | "reset" | "file" => {}
                    "hidden" if name.eq_ignore_ascii_case("_charset_") => {
                        fields.push((name, "UTF-8".to_string()));
                    }
                    _ => {
                        let v = value.unwrap_or("");
                        let v = if ty == "hidden" {
                            v.to_string()
                        } else {
                            v.replace(['\r', '\n'], "")
                        };
                        fields.push((name, v));
                    }
                }
            }
            Some("button") => {
                if is_submitter && !name.is_empty() {
                    fields.push((name, doc.attr(node, "value").unwrap_or("").to_string()));
                }
            }
            Some("select") => {
                if name.is_empty() {
                    continue;
                }
                for option in selected_options(doc, node) {
                    if !option_disabled(doc, option) {
                        fields.push((name.clone(), option_value(doc, option)));
                    }
                }
            }
            Some("textarea") if !name.is_empty() => {
                fields.push((name, crlf(&textarea_value(doc, node))));
            }
            _ => {}
        }
    }
    fields
}

/// Submits `form` (HTML "form submission algorithm", without validation):
/// `action`/`method` (overridable by the submitter's `formaction` /
/// `formmethod`), GET → the action URL with its query replaced by the
/// encoded fields, POST → [`NavigationRequest::Post`]. `None` for
/// `javascript:` actions and `method=dialog`.
pub(crate) fn submit(
    doc: &Document,
    form: NodeId,
    submitter: Option<Submitter>,
    base: &Url,
    document_url: &Url,
) -> Option<NavigationRequest> {
    let sub_attr = |name: &str| {
        submitter
            .and_then(|s| doc.attr(s.node, name))
            .filter(|_| submitter.is_some_and(|s| is_submit_button(doc, s.node)))
    };
    let action = sub_attr("formaction")
        .or_else(|| doc.attr(form, "action"))
        .map(str::trim)
        .unwrap_or("");
    let mut url = if action.is_empty() {
        document_url.clone()
    } else {
        resolve(base, action)?
    };
    if url.scheme() == "javascript" {
        return None;
    }
    let method = sub_attr("formmethod")
        .or_else(|| doc.attr(form, "method"))
        .unwrap_or("get")
        .trim()
        .to_ascii_lowercase();
    if method == "dialog" {
        return None;
    }
    let fields = form_data(doc, form, submitter);
    url.set_fragment(None);
    if method == "post" {
        Some(NavigationRequest::Post { url, fields })
    } else {
        url.set_query(Some(&encode_form(&fields)));
        Some(NavigationRequest::Get(url))
    }
}

/// Text inputs that block implicit submission when a form has no submit button.
fn blocks_implicit_submission(doc: &Document, node: NodeId) -> bool {
    doc.is_element_named(node, "input")
        && matches!(
            input_type(doc, node).as_str(),
            "text"
                | "search"
                | "email"
                | "url"
                | "tel"
                | "password"
                | "date"
                | "month"
                | "week"
                | "time"
                | "datetime-local"
                | "number"
        )
}

/// Enter in a text field (HTML "implicit submission"): the form's default
/// button submits it; without a submit button, the form is submitted only if
/// it has a single field blocking implicit submission.
pub(crate) fn implicit_submission(
    doc: &Document,
    field: NodeId,
    base: &Url,
    document_url: &Url,
) -> Option<NavigationRequest> {
    let mut owners = Owners::new(doc);
    let form = owners.owner(field)?;
    let mut default_button = None;
    let mut blocking = 0usize;
    for node in doc.descendants(doc.root()) {
        if !is_listed_control(doc, node) || owners.owner(node) != Some(form) {
            continue;
        }
        if default_button.is_none() && is_submit_button(doc, node) {
            default_button = Some(node);
        }
        if blocks_implicit_submission(doc, node) {
            blocking += 1;
        }
    }
    match default_button {
        Some(button) if is_disabled(doc, button) => None,
        Some(button) => submit(
            doc,
            form,
            Some(Submitter {
                node: button,
                coords: None,
            }),
            base,
            document_url,
        ),
        None if blocking <= 1 => submit(doc, form, None, base, document_url),
        None => None,
    }
}

/// Load-time state of a control, restored by a reset button.
#[derive(Clone, Debug, PartialEq)]
enum ControlDefault {
    /// `<input>`: `value` attribute and `checked`.
    Input {
        value: Option<String>,
        checked: bool,
    },
    /// `<option>`: `selected`.
    Option { selected: bool },
    /// `<textarea>`: text content.
    TextArea(String),
}

/// Snapshot of the initial state of every form control of a document.
#[derive(Clone, Debug, Default)]
pub(crate) struct FormDefaults {
    controls: HashMap<NodeId, ControlDefault>,
}

impl FormDefaults {
    pub(crate) fn snapshot(doc: &Document) -> Self {
        let mut controls = HashMap::new();
        for node in doc.descendants(doc.root()) {
            let state = match doc.tag_name(node) {
                Some("input") => ControlDefault::Input {
                    value: doc.attr(node, "value").map(str::to_string),
                    checked: doc.attr(node, "checked").is_some(),
                },
                Some("option") => ControlDefault::Option {
                    selected: doc.attr(node, "selected").is_some(),
                },
                Some("textarea") => ControlDefault::TextArea(textarea_value(doc, node)),
                _ => continue,
            };
            controls.insert(node, state);
        }
        Self { controls }
    }
}

/// Sets or removes a boolean attribute; returns true if it changed.
fn set_flag(doc: &mut Document, node: NodeId, name: &str, on: bool) -> bool {
    if doc.attr(node, name).is_some() == on {
        return false;
    }
    if let Some(el) = doc.element_mut(node) {
        if on {
            el.set_attr(name, "");
        } else {
            el.remove_attr(name);
        }
    }
    true
}

/// Restores the load-time state of the controls of `form`. Returns true if
/// anything changed.
pub(crate) fn reset(doc: &mut Document, form: NodeId, defaults: &FormDefaults) -> bool {
    let mut owners = Owners::new(doc);
    let nodes: Vec<NodeId> = doc
        .descendants(doc.root())
        .filter(|&n| {
            if !defaults.controls.contains_key(&n) {
                return false;
            }
            let owner_node = if doc.is_element_named(n, "option") {
                doc.ancestors(n)
                    .find(|&a| doc.is_element_named(a, "select"))
            } else {
                Some(n)
            };
            owner_node.is_some_and(|o| owners.owner(o) == Some(form))
        })
        .collect();
    let mut changed = false;
    for node in nodes {
        match defaults.controls.get(&node) {
            Some(ControlDefault::Input { value, checked }) => {
                changed |= set_flag(doc, node, "checked", *checked);
                if doc.attr(node, "value") != value.as_deref() {
                    if let Some(el) = doc.element_mut(node) {
                        match value {
                            Some(v) => el.set_attr("value", v.clone()),
                            None => el.remove_attr("value"),
                        }
                    }
                    changed = true;
                }
            }
            Some(ControlDefault::Option { selected }) => {
                changed |= set_flag(doc, node, "selected", *selected);
            }
            Some(ControlDefault::TextArea(text)) if textarea_value(doc, node) != *text => {
                set_textarea_text(doc, node, text);
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// Toggles a checkbox.
pub(crate) fn toggle_checkbox(doc: &mut Document, node: NodeId) {
    let on = doc.attr(node, "checked").is_none();
    set_flag(doc, node, "checked", on);
}

/// Radios of the same group as `radio` (same name, same form owner).
fn radio_group(doc: &Document, radio: NodeId) -> Vec<NodeId> {
    let Some(name) = doc.attr(radio, "name").filter(|n| !n.is_empty()) else {
        return Vec::new();
    };
    let mut owners = Owners::new(doc);
    let owner = owners.owner(radio);
    doc.descendants(doc.root())
        .filter(|&n| {
            n != radio
                && doc.is_element_named(n, "input")
                && input_type(doc, n) == "radio"
                && doc.attr(n, "name") == Some(name)
                && owners.owner(n) == owner
        })
        .collect()
}

/// Checks a radio button and unchecks the others of its group. Returns true
/// if anything changed.
pub(crate) fn check_radio(doc: &mut Document, radio: NodeId) -> bool {
    let mut changed = set_flag(doc, radio, "checked", true);
    for other in radio_group(doc, radio) {
        changed |= set_flag(doc, other, "checked", false);
    }
    changed
}

/// Selects the next enabled option of a `<select>` (wrapping around).
/// Returns true if the selection changed.
pub(crate) fn cycle_select(doc: &mut Document, select: NodeId) -> bool {
    let options = select_options(doc, select);
    if options.is_empty() {
        return false;
    }
    let current = selected_options(doc, select).last().copied();
    let start = current
        .and_then(|c| options.iter().position(|&o| o == c))
        .map_or(0, |p| p + 1);
    let next = (0..options.len())
        .filter_map(|k| options.get((start + k) % options.len()).copied())
        .find(|&o| !option_disabled(doc, o));
    let Some(next) = next else {
        return false;
    };
    if Some(next) == current && doc.attr(next, "selected").is_some() {
        return false;
    }
    for &o in &options {
        set_flag(doc, o, "selected", o == next);
    }
    true
}

/// Replaces the text of a textarea (first text child reused, others removed).
fn set_textarea_text(doc: &mut Document, textarea: NodeId, text: &str) {
    let texts: Vec<NodeId> = doc
        .children(textarea)
        .iter()
        .copied()
        .filter(|&c| doc.node(c).is_text())
        .collect();
    match texts.split_first() {
        Some((&first, rest)) => {
            if let NodeData::Text(t) = &mut doc.node_mut(first).data {
                *t = text.to_string();
            }
            for &r in rest {
                doc.detach(r);
            }
        }
        None => {
            if !text.is_empty() {
                let t = doc.create_text(text);
                doc.append_child(textarea, t);
            }
        }
    }
}

/// Maximum value length accepted while typing (`maxlength`, capped).
fn max_length(doc: &Document, node: NodeId) -> usize {
    doc.attr(node, "maxlength")
        .and_then(|m| m.trim().parse::<usize>().ok())
        .unwrap_or(usize::MAX)
        .min(1_000_000)
}

/// Appends typed text to a text field (honouring `maxlength`). Line breaks
/// are kept in textareas only; other control characters are dropped.
/// Returns `(changed, created_node)`: whether the value changed and whether
/// a new DOM node had to be created (a first text node in a textarea).
pub(crate) fn insert_text(doc: &mut Document, field: NodeId, text: &str) -> (bool, bool) {
    let textarea = doc.is_element_named(field, "textarea");
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let filtered: String = normalized
        .chars()
        .filter(|&c| !c.is_control() || (textarea && (c == '\n' || c == '\t')))
        .collect();
    if filtered.is_empty() {
        return (false, false);
    }
    let limit = max_length(doc, field);
    if textarea {
        let current = textarea_value(doc, field);
        let room = limit.saturating_sub(current.chars().count());
        let add: String = filtered.chars().take(room).collect();
        if add.is_empty() {
            return (false, false);
        }
        let last_text = doc
            .children(field)
            .iter()
            .rev()
            .copied()
            .find(|&c| doc.node(c).is_text());
        match last_text {
            Some(t) => {
                if let NodeData::Text(s) = &mut doc.node_mut(t).data {
                    s.push_str(&add);
                }
                (true, false)
            }
            None => {
                let t = doc.create_text(add);
                doc.append_child(field, t);
                (true, true)
            }
        }
    } else {
        let mut value = doc.attr(field, "value").unwrap_or("").to_string();
        let room = limit.saturating_sub(value.chars().count());
        let add: String = filtered.chars().take(room).collect();
        if add.is_empty() {
            return (false, false);
        }
        value.push_str(&add);
        if let Some(el) = doc.element_mut(field) {
            el.set_attr("value", value);
        }
        (true, false)
    }
}

/// Deletes the last character of a text field's value. Returns true if it changed.
pub(crate) fn backspace(doc: &mut Document, field: NodeId) -> bool {
    if doc.is_element_named(field, "textarea") {
        let texts: Vec<NodeId> = doc
            .children(field)
            .iter()
            .rev()
            .copied()
            .filter(|&c| doc.node(c).is_text())
            .collect();
        for t in texts {
            if let NodeData::Text(s) = &mut doc.node_mut(t).data {
                if s.pop().is_some() {
                    return true;
                }
            }
        }
        false
    } else {
        let Some(mut value) = doc.attr(field, "value").map(str::to_string) else {
            return false;
        };
        if value.pop().is_none() {
            return false;
        }
        if let Some(el) = doc.element_mut(field) {
            el.set_attr("value", value);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(html: &str) -> Document {
        xbro_html::parse_document(html)
    }

    fn by_id(doc: &Document, id: &str) -> NodeId {
        doc.get_element_by_id(id).unwrap()
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn successful_controls() {
        let d = doc("<form id=f>\
            <input name=t value='a b'><input name=empty>\
            <input type=checkbox name=c1 checked><input type=checkbox name=c2 value=x>\
            <input type=radio name=r value=1><input type=radio name=r value=2 checked>\
            <input type=hidden name=h value=hid><input type=file name=file>\
            <input type=reset name=rs><input type=button name=b value=v>\
            <input name=dis value=d disabled>\
            <fieldset disabled><legend><input name=inlegend value=l></legend><input name=infs></fieldset>\
            <select name=s><option>One</option><option value=2 selected>Two</option></select>\
            <select name=s2><option disabled>No</option><option> Fi  rst </option></select>\
            <select name=m multiple><option selected>a</option><option selected value=b>B</option></select>\
            <textarea name=ta>l1\nl2</textarea>\
            <input type=submit name=go value=Go id=go><button name=btn value=bv id=btn>B</button>\
            <input type=image name=img id=img>\
            </form><input name=outside form=f value=o><input name=other value=no>");
        let f = by_id(&d, "f");
        let fields = form_data(&d, f, None);
        let expected: Vec<(String, String)> = [
            ("t", "a b"),
            ("empty", ""),
            ("c1", "on"),
            ("r", "2"),
            ("h", "hid"),
            ("inlegend", "l"),
            ("s", "2"),
            ("s2", "Fi rst"),
            ("m", "a"),
            ("m", "b"),
            ("ta", "l1\r\nl2"),
            ("outside", "o"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(fields, expected);
        let go = Submitter {
            node: by_id(&d, "go"),
            coords: None,
        };
        assert!(form_data(&d, f, Some(go)).contains(&("go".into(), "Go".into())));
        let btn = Submitter {
            node: by_id(&d, "btn"),
            coords: None,
        };
        assert!(form_data(&d, f, Some(btn)).contains(&("btn".into(), "bv".into())));
        let img = Submitter {
            node: by_id(&d, "img"),
            coords: Some((3, 4)),
        };
        let with_img = form_data(&d, f, Some(img));
        assert!(with_img.contains(&("img.x".into(), "3".into())));
        assert!(with_img.contains(&("img.y".into(), "4".into())));
    }

    #[test]
    fn get_and_post_submission() {
        let d = doc(
            "<form id=g action='search?old=1#frag'><input name=q value='a&b é'></form>\
                     <form id=p method=POST action=/post><input name=x value=1></form>\
                     <form id=e><input name=y value=2></form>\
                     <form id=j action='javascript:void(0)'></form>",
        );
        let base = url("https://e.org/dir/page.html?z=1#top");
        let r = submit(&d, by_id(&d, "g"), None, &base, &base).unwrap();
        assert_eq!(
            r,
            NavigationRequest::Get(url("https://e.org/dir/search?q=a%26b+%C3%A9"))
        );
        let r = submit(&d, by_id(&d, "p"), None, &base, &base).unwrap();
        assert_eq!(
            r,
            NavigationRequest::Post {
                url: url("https://e.org/post"),
                fields: vec![("x".into(), "1".into())]
            }
        );
        let r = submit(&d, by_id(&d, "e"), None, &base, &base).unwrap();
        assert_eq!(r.url().as_str(), "https://e.org/dir/page.html?y=2");
        assert_eq!(submit(&d, by_id(&d, "j"), None, &base, &base), None);
    }

    #[test]
    fn implicit_submission_rules() {
        let d = doc("<form id=a action=a><input id=a1 name=q><input type=submit disabled></form>\
                     <form id=b action=b><input id=b1 name=q></form>\
                     <form id=c action=c><input id=c1 name=q><input name=r></form>\
                     <form id=d action=d><input id=d1 name=q><input name=r><button>Go</button></form>");
        let base = url("https://e.org/");
        assert_eq!(implicit_submission(&d, by_id(&d, "a1"), &base, &base), None);
        assert_eq!(
            implicit_submission(&d, by_id(&d, "b1"), &base, &base)
                .unwrap()
                .url()
                .as_str(),
            "https://e.org/b?q="
        );
        assert_eq!(implicit_submission(&d, by_id(&d, "c1"), &base, &base), None);
        assert!(implicit_submission(&d, by_id(&d, "d1"), &base, &base).is_some());
    }

    #[test]
    fn control_state_changes_and_reset() {
        let mut d = doc("<form id=f><input type=checkbox id=c>\
            <input type=radio name=r id=r1 checked><input type=radio name=r id=r2>\
            <select id=s><option>a</option><option disabled>b</option><option>c</option></select>\
            <input id=t value=start><textarea id=ta>hello</textarea></form>\
            <input type=radio name=r id=r3 checked>");
        let defaults = FormDefaults::snapshot(&d);
        let f = by_id(&d, "f");
        let c = by_id(&d, "c");
        toggle_checkbox(&mut d, c);
        assert!(d.attr(c, "checked").is_some());
        let (r1, r2, r3) = (by_id(&d, "r1"), by_id(&d, "r2"), by_id(&d, "r3"));
        assert!(check_radio(&mut d, r2));
        assert!(d.attr(r1, "checked").is_none());
        assert!(
            d.attr(r3, "checked").is_some(),
            "other form owner untouched"
        );
        assert!(!check_radio(&mut d, r2));
        let s = by_id(&d, "s");
        assert!(cycle_select(&mut d, s));
        let selected = |d: &Document| collapsed_text(d, selected_options(d, s)[0]);
        assert_eq!(selected(&d), "c", "disabled option skipped");
        assert!(cycle_select(&mut d, s));
        assert_eq!(selected(&d), "a", "wraps around");
        let t = by_id(&d, "t");
        assert_eq!(insert_text(&mut d, t, "!\n\u{7}x"), (true, false));
        assert_eq!(d.attr(t, "value"), Some("start!x"));
        assert!(backspace(&mut d, t));
        assert_eq!(d.attr(t, "value"), Some("start!"));
        let ta = by_id(&d, "ta");
        assert_eq!(insert_text(&mut d, ta, "\r\nworld"), (true, false));
        assert_eq!(textarea_value(&d, ta), "hello\nworld");
        assert!(reset(&mut d, f, &defaults));
        assert!(d.attr(c, "checked").is_none());
        assert!(d.attr(r1, "checked").is_some());
        assert!(d.attr(r2, "checked").is_none());
        assert_eq!(d.attr(t, "value"), Some("start"));
        assert_eq!(textarea_value(&d, ta), "hello");
        assert!(!reset(&mut d, f, &defaults));
    }

    #[test]
    fn empty_textarea_and_maxlength() {
        let mut d = doc("<textarea id=ta maxlength=3></textarea><input id=i maxlength=2>");
        let ta = by_id(&d, "ta");
        assert_eq!(insert_text(&mut d, ta, "abcdef"), (true, true));
        assert_eq!(textarea_value(&d, ta), "abc");
        assert_eq!(insert_text(&mut d, ta, "z"), (false, false));
        assert!(backspace(&mut d, ta));
        assert_eq!(textarea_value(&d, ta), "ab");
        let i = by_id(&d, "i");
        insert_text(&mut d, i, "xyz");
        assert_eq!(d.attr(i, "value"), Some("xy"));
        assert!(!backspace(&mut doc("<input id=e>"), NodeId(0)));
    }

    #[test]
    fn disabled_fieldsets_and_owners() {
        let d = doc(
            "<form id=f1><fieldset disabled id=fs><legend><button id=b1>x</button></legend>\
                     <button id=b2>y</button></fieldset></form><button id=b3 form=nope>z</button>",
        );
        assert!(!is_disabled(&d, by_id(&d, "b1")));
        assert!(is_disabled(&d, by_id(&d, "b2")));
        // Only the first legend child is exempt, wherever it is.
        let d2 = doc("<fieldset disabled><p>x</p><legend><input id=l1></legend>\
                      <legend><input id=l2></legend><div><legend><input id=l3></legend></div>\
                      </fieldset>");
        assert!(!is_disabled(&d2, by_id(&d2, "l1")));
        assert!(is_disabled(&d2, by_id(&d2, "l2")));
        assert!(is_disabled(&d2, by_id(&d2, "l3")));
        assert_eq!(form_owner(&d, by_id(&d, "b2")), Some(by_id(&d, "f1")));
        assert_eq!(form_owner(&d, by_id(&d, "b3")), None);
    }

    #[test]
    fn fuzz_form_documents() {
        const PIECES: &[&str] = &[
            "<form id=f action='?a=1#x' method=post>",
            "<form action=javascript:x>",
            "<form method=dialog>",
            "<form action='%zz' method=GET>",
            "<form>",
            "</form>",
            "<input name=q value='a&b=c d'>",
            "<input type=checkbox name=c checked>",
            "<input type=radio name=r value=1>",
            "<input type=radio name=r checked>",
            "<input type=submit name=s value=Go>",
            "<input type=image name=i>",
            "<input type=image>",
            "<input type=reset>",
            "<input type=hidden name=_charset_>",
            "<input name='\u{e9}\u{65e5}' value='\u{1F600}\r\n'>",
            "<input maxlength=2 name=m>",
            "<input maxlength=-1 name=m2>",
            "<input type=password name=p>",
            "<button name=b value=v>",
            "<button type=reset>",
            "<button type=button>",
            "</button>",
            "<select name=s>",
            "<select name=m multiple>",
            "<option>o",
            "<option selected value=2>",
            "<option disabled selected>",
            "<optgroup disabled>",
            "</select>",
            "<textarea name=t>a\nb",
            "</textarea>",
            "<fieldset disabled>",
            "<legend>",
            "</legend>",
            "</fieldset>",
            "<datalist><input name=dl></datalist>",
            "<input form=f name=outside>",
            "<button form=f formaction=other.html formmethod=get>",
            "<input disabled name=d>",
            "<label>",
            "</label>",
            "<table><tr><td>",
            "</table>",
            "<div>",
            "</div>",
            "text",
        ];
        let mut state = 0xfeed_f00d_dead_beefu64;
        let mut below = |n: usize| {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            (state.wrapping_mul(0x2545_F491_4F6C_DD1D) % n as u64) as usize
        };
        let page = url("https://e.org/dir/page.html");
        let (mut gets, mut posts) = (0, 0);
        for _ in 0..600 {
            let html: String = (0..below(40))
                .map(|_| PIECES[below(PIECES.len())])
                .collect();
            let mut d = doc(&html);
            let defaults = FormDefaults::snapshot(&d);
            let nodes: Vec<NodeId> = d.descendants(d.root()).collect();
            for &node in &nodes {
                if !d.is_element_named(node, "form") {
                    continue;
                }
                let submitters = std::iter::once(None).chain(
                    nodes
                        .iter()
                        .filter(|&&n| is_submit_button(&d, n))
                        .map(|&n| {
                            Some(Submitter {
                                node: n,
                                coords: Some((below(100) as i32 - 50, 7)),
                            })
                        }),
                );
                for submitter in submitters.collect::<Vec<_>>() {
                    match submit(&d, node, submitter, &page, &page) {
                        Some(NavigationRequest::Get(u)) => {
                            gets += 1;
                            assert_eq!(u.fragment(), None, "{html:?}");
                            assert_ne!(u.scheme(), "javascript");
                        }
                        Some(NavigationRequest::Post { url, fields }) => {
                            posts += 1;
                            assert_eq!(url.fragment(), None);
                            for (name, _) in &fields {
                                assert!(!name.is_empty(), "{html:?}: {fields:?}");
                            }
                        }
                        None => {}
                    }
                }
            }
            // Edit every control, then reset every form.
            for &node in &nodes {
                match below(5) {
                    0 => toggle_checkbox(&mut d, node),
                    1 => {
                        let _ = check_radio(&mut d, node);
                    }
                    2 => {
                        let _ = cycle_select(&mut d, node);
                    }
                    3 => {
                        let _ = insert_text(&mut d, node, "x\ny\u{1F600}");
                        let _ = implicit_submission(&d, node, &page, &page);
                    }
                    _ => {
                        let _ = backspace(&mut d, node);
                    }
                }
            }
            for &node in &nodes {
                if d.is_element_named(node, "form") {
                    let _ = reset(&mut d, node, &defaults);
                }
            }
        }
        assert!(gets > 100 && posts > 100, "{gets} GET, {posts} POST");
    }
}
