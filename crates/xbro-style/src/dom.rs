//! Adapter between the DOM and selector matching.
//!
//! [`TreeInfo`] precomputes, in one pass over the document, everything the
//! selector matcher asks repeatedly: element siblings, first element child,
//! positions among siblings (for `:nth-*`) and the `:hover` / `:active` /
//! `:focus-within` chains. Every [`DomElement`] query is then O(1), which
//! keeps sibling combinators and `:nth-child()` linear on wide lists.

use std::collections::HashMap;

use xbro_css::selector::Element;
use xbro_dom::{Document, ElementData, NodeData, NodeId};

use crate::StyleContext;

const NONE: u32 = u32::MAX;

const HOVER: u8 = 1;
const ACTIVE: u8 = 2;
const FOCUS: u8 = 4;
const FOCUS_WITHIN: u8 = 8;

/// Per-node side tables used by selector matching.
pub(crate) struct TreeInfo<'a> {
    pub doc: &'a Document,
    prev: Vec<u32>,
    next: Vec<u32>,
    first_child: Vec<u32>,
    /// 1-based index among element siblings, from the start / from the end.
    index: Vec<u32>,
    index_rev: Vec<u32>,
    /// Same, counting only siblings with the same local name.
    type_index: Vec<u32>,
    type_index_rev: Vec<u32>,
    flags: Vec<u8>,
    root: Option<NodeId>,
    /// First `legend` element child of each `fieldset` that has one
    /// (`:disabled` excludes the contents of a disabled fieldset's first legend).
    first_legend: HashMap<usize, NodeId>,
}

fn to_u32(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(NONE - 1)
}

impl<'a> TreeInfo<'a> {
    pub fn new(doc: &'a Document, ctx: &StyleContext) -> Self {
        let n = doc.len();
        let mut info = TreeInfo {
            doc,
            prev: vec![NONE; n],
            next: vec![NONE; n],
            first_child: vec![NONE; n],
            index: vec![1; n],
            index_rev: vec![1; n],
            type_index: vec![1; n],
            type_index_rev: vec![1; n],
            flags: vec![0; n],
            root: doc.document_element(),
            first_legend: HashMap::new(),
        };
        let mut per_type: HashMap<&str, u32> = HashMap::new();
        let mut elements: Vec<NodeId> = Vec::new();
        for parent in doc.ids() {
            elements.clear();
            elements.extend(
                doc.children(parent)
                    .iter()
                    .copied()
                    .filter(|&c| c.index() < n && doc.node(c).is_element()),
            );
            let Some(first) = elements.first() else {
                continue;
            };
            if let Some(slot) = info.first_child.get_mut(parent.index()) {
                *slot = to_u32(first.index());
            }
            if doc.is_element_named(parent, "fieldset") {
                if let Some(&legend) = elements
                    .iter()
                    .find(|&&c| doc.is_element_named(c, "legend"))
                {
                    info.first_legend.insert(parent.index(), legend);
                }
            }
            let count = elements.len();
            per_type.clear();
            for (i, &e) in elements.iter().enumerate() {
                let idx = e.index();
                let name = doc.tag_name(e).unwrap_or("");
                let t = per_type.entry(name).or_insert(0);
                *t += 1;
                if let Some(slot) = info.type_index.get_mut(idx) {
                    *slot = *t;
                }
                if let Some(slot) = info.index.get_mut(idx) {
                    *slot = to_u32(i + 1);
                }
                if let Some(slot) = info.index_rev.get_mut(idx) {
                    *slot = to_u32(count - i);
                }
                if let Some(prev) = i.checked_sub(1).and_then(|p| elements.get(p)) {
                    if let Some(slot) = info.prev.get_mut(idx) {
                        *slot = to_u32(prev.index());
                    }
                }
                if let Some(next) = elements.get(i + 1) {
                    if let Some(slot) = info.next.get_mut(idx) {
                        *slot = to_u32(next.index());
                    }
                }
            }
            for &e in &elements {
                let name = doc.tag_name(e).unwrap_or("");
                let total = per_type.get(name).copied().unwrap_or(1);
                let idx = e.index();
                let from_start = info.type_index.get(idx).copied().unwrap_or(1);
                if let Some(slot) = info.type_index_rev.get_mut(idx) {
                    *slot = total.saturating_sub(from_start) + 1;
                }
            }
        }
        info.mark_chain(ctx.hover, HOVER, HOVER);
        info.mark_chain(ctx.active, ACTIVE, ACTIVE);
        info.mark_chain(ctx.focus, FOCUS | FOCUS_WITHIN, FOCUS_WITHIN);
        info
    }

    /// Sets `own` on `node` and `ancestors` on its ancestors.
    fn mark_chain(&mut self, node: Option<NodeId>, own: u8, ancestors: u8) {
        let Some(node) = node.filter(|n| n.index() < self.doc.len()) else {
            return;
        };
        if let Some(f) = self.flags.get_mut(node.index()) {
            *f |= own;
        }
        let mut current = self.doc.parent(node);
        let mut guard = 0usize;
        while let Some(p) = current {
            if guard > self.flags.len() {
                break; // corrupted parent links
            }
            guard += 1;
            if let Some(f) = self.flags.get_mut(p.index()) {
                *f |= ancestors;
            }
            current = self.doc.parent(p);
        }
    }

    pub fn element(&self, id: NodeId) -> Option<DomElement<'_>> {
        let data = self.doc.element(id)?;
        Some(DomElement {
            tree: self,
            id,
            data,
        })
    }

    fn node(&self, raw: u32) -> Option<DomElement<'_>> {
        if raw == NONE {
            return None;
        }
        self.element(NodeId(raw as usize))
    }

    pub fn is_root(&self, id: NodeId) -> bool {
        self.root == Some(id)
    }
}

/// An element as seen by the selector matcher.
#[derive(Clone, Copy)]
pub(crate) struct DomElement<'a> {
    tree: &'a TreeInfo<'a>,
    pub id: NodeId,
    pub data: &'a ElementData,
}

impl DomElement<'_> {
    fn flag(&self, f: u8) -> bool {
        self.tree
            .flags
            .get(self.id.index())
            .is_some_and(|v| v & f != 0)
    }

    fn is_form_control(&self) -> bool {
        matches!(
            self.data.name.as_str(),
            "button" | "input" | "select" | "textarea" | "optgroup" | "option" | "fieldset"
        )
    }

    /// Disabled by its context (HTML "actually disabled"): an `option` whose
    /// parent `optgroup` is disabled, or a `button` / `input` / `select` /
    /// `textarea` / `fieldset` that is a descendant of a disabled `fieldset`,
    /// outside that fieldset's first `legend` child. O(depth): the first
    /// legend of each fieldset is precomputed by [`TreeInfo`].
    fn disabled_by_ancestor(&self) -> bool {
        let doc = self.tree.doc;
        match self.data.name.as_str() {
            "option" => {
                return doc.parent_element(self.id).is_some_and(|p| {
                    doc.is_element_named(p, "optgroup") && doc.attr(p, "disabled").is_some()
                });
            }
            "button" | "input" | "select" | "textarea" | "fieldset" => {}
            _ => return false,
        }
        let mut child = self.id;
        let mut current = doc.parent_element(self.id);
        let mut guard = 0usize;
        while let Some(p) = current {
            guard += 1;
            if guard > 1024 {
                break;
            }
            if doc.is_element_named(p, "fieldset")
                && doc.attr(p, "disabled").is_some()
                && self.tree.first_legend.get(&p.index()) != Some(&child)
            {
                return true;
            }
            child = p;
            current = doc.parent_element(p);
        }
        false
    }
}

impl<'a> Element for DomElement<'a> {
    fn local_name(&self) -> &str {
        &self.data.name
    }

    fn id(&self) -> Option<&str> {
        self.data.id()
    }

    fn has_class(&self, class: &str) -> bool {
        self.data.has_class(class)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.data.attr(name)
    }

    fn parent_element(&self) -> Option<Self> {
        let p = self.tree.doc.parent(self.id)?;
        self.tree.element(p)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.tree.node(*self.tree.prev.get(self.id.index())?)
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.tree.node(*self.tree.next.get(self.id.index())?)
    }

    fn first_child_element(&self) -> Option<Self> {
        self.tree.node(*self.tree.first_child.get(self.id.index())?)
    }

    fn is_root(&self) -> bool {
        self.tree.is_root(self.id)
    }

    fn is_empty(&self) -> bool {
        let doc = self.tree.doc;
        !doc.children(self.id)
            .iter()
            .any(|&c| match &doc.node(c).data {
                NodeData::Element(_) => true,
                NodeData::Text(t) => !t.is_empty(),
                _ => false,
            })
    }

    fn is_hovered(&self) -> bool {
        self.flag(HOVER)
    }

    fn is_active(&self) -> bool {
        self.flag(ACTIVE)
    }

    fn is_focused(&self) -> bool {
        self.flag(FOCUS)
    }

    fn is_focus_within(&self) -> bool {
        self.flag(FOCUS_WITHIN)
    }

    fn is_checked(&self) -> bool {
        match self.data.name.as_str() {
            "input" => {
                self.data.attr("checked").is_some()
                    && self.data.attr("type").is_some_and(|t| {
                        t.eq_ignore_ascii_case("checkbox") || t.eq_ignore_ascii_case("radio")
                    })
            }
            "option" => self.data.attr("selected").is_some(),
            _ => false,
        }
    }

    fn is_disabled(&self) -> bool {
        self.is_form_control()
            && (self.data.attr("disabled").is_some() || self.disabled_by_ancestor())
    }

    fn nth_index(&self, of_type: bool, from_end: bool) -> usize {
        let table = match (of_type, from_end) {
            (false, false) => &self.tree.index,
            (false, true) => &self.tree.index_rev,
            (true, false) => &self.tree.type_index,
            (true, true) => &self.tree.type_index_rev,
        };
        table.get(self.id.index()).copied().unwrap_or(1) as usize
    }

    fn cache_key(&self) -> Option<usize> {
        Some(self.id.index())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_css::parse_selector_list;

    fn matches(doc: &Document, ctx: &StyleContext, selector: &str, id: &str) -> bool {
        let info = TreeInfo::new(doc, ctx);
        let node = doc.get_element_by_id(id).expect("element");
        let el = info.element(node).expect("element");
        parse_selector_list(selector)
            .expect("valid selector")
            .iter()
            .any(|s| s.matches(&el))
    }

    #[test]
    fn structural_pseudo_classes() {
        let doc = xbro_html::parse_document(
            "<ul><li id=a>1</li><li id=b class=x></li><p id=c></p><li id=d> </li></ul>",
        );
        let ctx = StyleContext::default();
        assert!(matches(&doc, &ctx, "li:first-child", "a"));
        assert!(matches(&doc, &ctx, "li:nth-child(2)", "b"));
        assert!(matches(&doc, &ctx, "li:last-child", "d"));
        assert!(matches(&doc, &ctx, "li:nth-of-type(3)", "d"));
        assert!(matches(&doc, &ctx, "li:nth-last-of-type(1)", "d"));
        assert!(matches(&doc, &ctx, "p:only-of-type", "c"));
        assert!(matches(&doc, &ctx, ".x + p", "c"));
        assert!(matches(&doc, &ctx, "#a ~ li", "d"));
        assert!(matches(&doc, &ctx, "li:empty", "b"));
        assert!(!matches(&doc, &ctx, "li:empty", "d"));
        assert!(!matches(&doc, &ctx, "li:empty", "a"));
        assert!(!matches(&doc, &ctx, "ul:has(> p)", "a"));
        assert!(matches(&doc, &ctx, ":root > body ul > li", "a"));
    }

    #[test]
    fn dynamic_states() {
        let doc = xbro_html::parse_document(
            "<div id=outer><a id=link href=x><span id=inner>t</span></a><input id=field></div>",
        );
        let ctx = StyleContext {
            hover: doc.get_element_by_id("inner"),
            focus: doc.get_element_by_id("field"),
            active: doc.get_element_by_id("link"),
            ..StyleContext::default()
        };
        assert!(matches(&doc, &ctx, "span:hover", "inner"));
        assert!(matches(&doc, &ctx, "a:hover", "link"));
        assert!(matches(&doc, &ctx, "div:hover", "outer"));
        assert!(matches(&doc, &ctx, "a:active", "link"));
        assert!(matches(&doc, &ctx, "div:active", "outer"));
        assert!(!matches(&doc, &ctx, "span:active", "inner"));
        assert!(matches(&doc, &ctx, "input:focus", "field"));
        assert!(matches(&doc, &ctx, "div:focus-within", "outer"));
        assert!(!matches(&doc, &ctx, "div:focus", "outer"));
        assert!(matches(&doc, &ctx, "a:any-link", "link"));
        // Out-of-range ids are ignored.
        let bogus = StyleContext {
            hover: Some(NodeId(1_000_000)),
            ..StyleContext::default()
        };
        assert!(!matches(&doc, &bogus, "div:hover", "outer"));
    }

    #[test]
    fn form_states() {
        let doc = xbro_html::parse_document(
            "<input id=c type=CHECKBOX checked><input id=t type=text checked>\
             <select><option id=o selected>a</option>\
             <optgroup disabled><option id=og>b</option></optgroup></select>\
             <fieldset disabled><legend><input id=inlegend></legend><input id=infs></fieldset>\
             <button id=btn disabled>x</button><div id=div disabled></div>",
        );
        let ctx = StyleContext::default();
        assert!(matches(&doc, &ctx, ":checked", "c"));
        assert!(!matches(&doc, &ctx, ":checked", "t"));
        assert!(matches(&doc, &ctx, ":checked", "o"));
        assert!(matches(&doc, &ctx, ":disabled", "og"));
        assert!(matches(&doc, &ctx, ":disabled", "infs"));
        assert!(!matches(&doc, &ctx, ":disabled", "inlegend"));
        assert!(matches(&doc, &ctx, ":disabled", "btn"));
        assert!(!matches(&doc, &ctx, ":disabled", "div"));
        assert!(matches(&doc, &ctx, ":enabled", "t"));
        assert!(!matches(&doc, &ctx, ":enabled", "btn"));
    }

    #[test]
    fn fieldset_disabling_follows_the_html_rules() {
        let doc = xbro_html::parse_document(
            "<fieldset disabled>\
               <legend><input id=in_first_legend></legend>\
               <legend><input id=in_second_legend></legend>\
               <select id=sel><option id=opt>a</option></select>\
               <fieldset id=inner><legend><input id=inner_legend></legend></fieldset>\
             </fieldset>",
        );
        let ctx = StyleContext::default();
        // Controls inside the first legend are enabled, others are not.
        assert!(!matches(&doc, &ctx, ":disabled", "in_first_legend"));
        assert!(matches(&doc, &ctx, ":disabled", "in_second_legend"));
        assert!(matches(&doc, &ctx, ":disabled", "sel"));
        assert!(matches(&doc, &ctx, ":disabled", "inner"));
        // An inner fieldset's legend does not escape the outer disabled fieldset.
        assert!(matches(&doc, &ctx, ":disabled", "inner_legend"));
        // Options are only disabled by their own attribute or their optgroup.
        assert!(!matches(&doc, &ctx, ":disabled", "opt"));
        assert!(matches(&doc, &ctx, ":enabled", "opt"));
    }

    #[test]
    fn disabled_fieldset_with_many_controls_is_linear() {
        let html = format!(
            "<fieldset disabled>{}<input id=last></fieldset>",
            "<input>".repeat(30_000)
        );
        let doc = xbro_html::parse_document(&html);
        let info = TreeInfo::new(&doc, &StyleContext::default());
        let sels = parse_selector_list("input:disabled").unwrap();
        let start = std::time::Instant::now();
        let count = doc
            .ids()
            .filter_map(|id| info.element(id))
            .filter(|el| sels.iter().any(|s| s.matches(el)))
            .count();
        assert_eq!(count, 30_001);
        // Quadratic scanning of the fieldset's children took seconds here.
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn wide_sibling_lists_are_fast() {
        let mut html = String::from("<ul>");
        for i in 0..20_000 {
            html.push_str(&format!("<li id=i{i}>x</li>"));
        }
        html.push_str("</ul>");
        let doc = xbro_html::parse_document(&html);
        let info = TreeInfo::new(&doc, &StyleContext::default());
        let sels = parse_selector_list("li:nth-child(2n+1) + li:nth-last-of-type(odd)").unwrap();
        let start = std::time::Instant::now();
        let mut count = 0;
        for id in doc.ids() {
            if let Some(el) = info.element(id) {
                if sels.iter().any(|s| s.matches(&el)) {
                    count += 1;
                }
            }
        }
        assert!(count > 0);
        assert!(start.elapsed().as_secs() < 5);
    }
}
