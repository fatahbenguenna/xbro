//! Which interactions can change styles: a scan of the author stylesheets
//! for `:hover`, `:focus*` and value-dependent selectors.
//!
//! For `:hover`, the scan keeps the static part (type, ids, classes,
//! attribute names) of every compound selector containing `:hover`, at any
//! nesting depth (`:is()`, `:not()`, `:where()`, `:has()`). When the hovered
//! element changes, only the elements entering or leaving the hover chain
//! (the hovered element and its ancestors) change their `:hover` state, and
//! a selector's result can only change if one of them matches the static
//! part of a compound containing `:hover`. Mouse moves over plain text thus
//! skip the restyle on most pages.

use xbro_css::{AttrSelector, Compound, CssRule, PseudoClass, Selector, Stylesheet};
use xbro_dom::{Document, NodeId};

/// Static requirements of a compound selector that contains `:hover`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HoverCompound {
    tag: Option<String>,
    ids: Vec<String>,
    classes: Vec<String>,
    attrs: Vec<String>,
}

impl HoverCompound {
    fn from_compound(c: &Compound) -> Self {
        Self {
            tag: c.tag.clone(),
            ids: c.ids.clone(),
            classes: c.classes.clone(),
            attrs: c
                .attrs
                .iter()
                .map(|a: &AttrSelector| a.name.clone())
                .collect(),
        }
    }

    /// Necessary condition for the compound to match `node`.
    fn may_match(&self, doc: &Document, node: NodeId) -> bool {
        let Some(el) = doc.element(node) else {
            return false;
        };
        self.tag
            .as_ref()
            .is_none_or(|t| el.name.eq_ignore_ascii_case(t))
            && self.ids.iter().all(|id| el.id() == Some(id.as_str()))
            && self.classes.iter().all(|c| el.has_class(c))
            && self.attrs.iter().all(|a| el.has_attr(a))
    }
}

/// Summary of the interaction-dependent selectors of a page.
#[derive(Clone, Debug, Default)]
pub(crate) struct DynamicRules {
    /// Compounds containing `:hover` (empty: hovering never changes styles).
    pub hover: Vec<HoverCompound>,
    /// Some selector uses `:focus`, `:focus-within` or `:focus-visible`.
    pub focus: bool,
    /// Some selector depends on a text field's value (`[value]`,
    /// `:placeholder-shown`), so typing requires a restyle.
    pub value: bool,
}

/// Upper bound on the hover compounds kept (beyond it every hover change restyles).
const MAX_HOVER_COMPOUNDS: usize = 4096;

impl DynamicRules {
    /// Scans every style rule of `sheets`, including rules inside `@media`
    /// (whatever the current viewport) and supported `@supports` blocks.
    pub(crate) fn scan(sheets: &[Stylesheet]) -> Self {
        let mut rules = DynamicRules::default();
        for sheet in sheets {
            rules.scan_rules(&sheet.rules, 0);
        }
        if rules.hover.len() > MAX_HOVER_COMPOUNDS {
            rules.hover = vec![HoverCompound::default()];
        }
        rules
    }

    fn scan_rules(&mut self, rules: &[CssRule], depth: usize) {
        if depth > 64 {
            return;
        }
        for rule in rules {
            match rule {
                CssRule::Style(style) => {
                    for selector in &style.selectors {
                        self.scan_selector(selector, 0);
                    }
                }
                CssRule::Media { rules, .. } | CssRule::Group { rules } => {
                    self.scan_rules(rules, depth + 1)
                }
                CssRule::Supports { supported, rules } => {
                    if *supported {
                        self.scan_rules(rules, depth + 1);
                    }
                }
                CssRule::Import { .. } => {}
            }
        }
    }

    fn scan_selector(&mut self, selector: &Selector, depth: usize) {
        if depth > 32 {
            return;
        }
        self.scan_compound(&selector.subject, depth);
        for (_, compound) in &selector.ancestors {
            self.scan_compound(compound, depth);
        }
    }

    fn scan_compound(&mut self, compound: &Compound, depth: usize) {
        if compound.attrs.iter().any(|a| a.name == "value") {
            self.value = true;
        }
        for pc in &compound.pseudo_classes {
            match pc {
                PseudoClass::Hover => {
                    let c = HoverCompound::from_compound(compound);
                    if !self.hover.contains(&c) {
                        self.hover.push(c);
                    }
                }
                PseudoClass::Focus | PseudoClass::FocusWithin | PseudoClass::FocusVisible => {
                    self.focus = true
                }
                PseudoClass::PlaceholderShown => self.value = true,
                PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Where(list) => {
                    for s in list {
                        self.scan_selector(s, depth + 1);
                    }
                }
                PseudoClass::Has(list) => {
                    for (_, s) in list {
                        self.scan_selector(s, depth + 1);
                    }
                }
                _ => {}
            }
        }
    }

    /// True when some stylesheet uses `:hover`.
    pub(crate) fn has_hover(&self) -> bool {
        !self.hover.is_empty()
    }

    /// True when moving the hover from `old` to `new` may change styles.
    pub(crate) fn hover_change_matters(
        &self,
        doc: &Document,
        old: Option<NodeId>,
        new: Option<NodeId>,
    ) -> bool {
        if self.hover.is_empty() || old == new {
            return false;
        }
        let chain = |n: Option<NodeId>| -> Vec<NodeId> {
            match n {
                Some(n) => std::iter::once(n)
                    .chain(doc.ancestors(n))
                    .filter(|&a| doc.node(a).is_element())
                    .collect(),
                None => Vec::new(),
            }
        };
        let old_chain = chain(old);
        let new_chain = chain(new);
        let changed = old_chain
            .iter()
            .filter(|n| !new_chain.contains(n))
            .chain(new_chain.iter().filter(|n| !old_chain.contains(n)));
        for &node in changed {
            if self.hover.iter().any(|c| c.may_match(doc, node)) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_css::parse_stylesheet;

    #[test]
    fn scans_nested_and_media_rules() {
        let sheets = vec![parse_stylesheet(
            "@media (max-width: 1px) { a.nav:hover { color: red } }\
             .x:is(.menu:hover .item) { color: blue }\
             input:focus { outline: 1px solid }\
             input:placeholder-shown { color: grey }",
        )];
        let rules = DynamicRules::scan(&sheets);
        assert!(rules.has_hover());
        assert_eq!(rules.hover.len(), 2);
        assert!(rules.focus);
        assert!(rules.value);
        let none = DynamicRules::scan(&[parse_stylesheet("a { color: red } p:first-child {}")]);
        assert!(!none.has_hover() && !none.focus && !none.value);
    }

    #[test]
    fn hover_changes_are_filtered() {
        let doc = xbro_html::parse_document(
            "<div class=menu><p id=p>text <a class=nav href=#>link</a></p></div><span id=s>x</span>",
        );
        let rules = DynamicRules::scan(&[parse_stylesheet("a.nav:hover { color: red }")]);
        let p = doc.get_element_by_id("p").unwrap();
        let s = doc.get_element_by_id("s").unwrap();
        let a = doc
            .descendants(doc.root())
            .find(|&n| doc.is_element_named(n, "a"))
            .unwrap();
        assert!(!rules.hover_change_matters(&doc, Some(p), Some(s)));
        assert!(rules.hover_change_matters(&doc, Some(p), Some(a)));
        assert!(rules.hover_change_matters(&doc, Some(a), None));
        assert!(!rules.hover_change_matters(&doc, Some(a), Some(a)));
        let menu = DynamicRules::scan(&[parse_stylesheet(".menu:hover .item { color: red }")]);
        assert!(menu.hover_change_matters(&doc, None, Some(p)));
        assert!(!menu.hover_change_matters(&doc, Some(p), Some(a)));
        let any = DynamicRules::scan(&[parse_stylesheet(":hover { color: red }")]);
        assert!(any.hover_change_matters(&doc, Some(p), Some(a)));
    }
}
