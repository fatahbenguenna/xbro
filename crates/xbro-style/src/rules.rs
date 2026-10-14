//! Rule collection and the rule index.
//!
//! Stylesheets are flattened (`@media` / `@supports` evaluated, groups
//! applied unconditionally, `@import` ignored), declarations are parsed once
//! per rule, and every selector is filed in a bucket keyed by its subject
//! compound's id, first class, tag, first attribute name, or in the
//! universal bucket. Styling an
//! element then only tests the rules of its own buckets, after a quick
//! ancestor Bloom filter check.

use std::borrow::Cow;
use std::collections::HashMap;

use xbro_css::selector::Element;
use xbro_css::{
    Combinator, Compound, CssRule, MediaContext, PseudoClass, PseudoElement, Selector, Specificity,
    StyleRule, Stylesheet,
};

use crate::bloom::{hash, AncestorFilter, KIND_CLASS, KIND_ID, KIND_TAG};
use crate::dom::DomElement;
use crate::properties::DeclBlock;

/// Maximum nesting of `@media` / `@supports` / group rules followed.
const MAX_RULE_DEPTH: usize = 64;
/// Maximum number of bucket keys extracted from an `:is()` subject.
const MAX_IS_KEYS: usize = 16;

/// One selector of a rule.
pub(crate) struct RuleEntry<'s> {
    pub selector: &'s Selector,
    pub specificity: Specificity,
    /// Source order across all sheets of the rule set.
    pub order: u32,
    /// Index of the rule's declarations in [`RuleSet::blocks`].
    pub block: u32,
    /// Hashes of tags/ids/classes required on ancestors (0 = unused).
    pub ancestor_hashes: [u32; 4],
}

/// Rules bucketed by the key of their subject compound.
#[derive(Default)]
pub(crate) struct SelectorMap<'s> {
    pub entries: Vec<RuleEntry<'s>>,
    by_id: HashMap<&'s str, Vec<u32>>,
    by_class: HashMap<&'s str, Vec<u32>>,
    by_tag: HashMap<&'s str, Vec<u32>>,
    by_attr: HashMap<&'s str, Vec<u32>>,
    universal: Vec<u32>,
}

enum Key<'s> {
    Id(&'s str),
    Class(&'s str),
    Tag(&'s str),
    Attr(&'s str),
}

/// Bucket keys of a compound; false when it can match any element.
fn compound_keys<'s>(c: &'s Compound, depth: usize, out: &mut Vec<Key<'s>>) -> bool {
    if let Some(id) = c.ids.first() {
        out.push(Key::Id(id));
        return true;
    }
    if let Some(class) = c.classes.first() {
        out.push(Key::Class(class));
        return true;
    }
    if let Some(tag) = &c.tag {
        out.push(Key::Tag(tag));
        return true;
    }
    if let Some(attr) = c.attrs.first() {
        out.push(Key::Attr(&attr.name));
        return true;
    }
    if depth < 3 {
        for pc in &c.pseudo_classes {
            if let PseudoClass::Is(list) | PseudoClass::Where(list) = pc {
                let mut keys = Vec::new();
                if !list.is_empty()
                    && list
                        .iter()
                        .all(|s| compound_keys(&s.subject, depth + 1, &mut keys))
                    && keys.len() <= MAX_IS_KEYS
                {
                    out.extend(keys);
                    return true;
                }
            }
        }
    }
    false
}

/// Hashes that must be present among the ancestors for `selector` to match:
/// every compound linked by a child/descendant combinator is an ancestor
/// of the subject (siblings of ancestors share their ancestors).
fn ancestor_hashes(selector: &Selector) -> [u32; 4] {
    let mut out = [0u32; 4];
    let mut n = 0;
    for (combinator, compound) in &selector.ancestors {
        if !matches!(combinator, Combinator::Child | Combinator::Descendant) {
            continue;
        }
        let hashes = compound
            .ids
            .iter()
            .map(|i| hash(KIND_ID, i))
            .chain(compound.classes.iter().map(|c| hash(KIND_CLASS, c)))
            .chain(compound.tag.iter().map(|t| hash(KIND_TAG, t)));
        for h in hashes {
            match out.get_mut(n) {
                Some(slot) => {
                    *slot = h;
                    n += 1;
                }
                None => return out,
            }
        }
    }
    out
}

impl<'s> SelectorMap<'s> {
    fn insert(&mut self, entry: RuleEntry<'s>) {
        let index = u32::try_from(self.entries.len()).unwrap_or(u32::MAX);
        let mut keys = Vec::new();
        if compound_keys(&entry.selector.subject, 0, &mut keys) {
            for key in keys {
                let bucket = match key {
                    Key::Id(k) => self.by_id.entry(k),
                    Key::Class(k) => self.by_class.entry(k),
                    Key::Tag(k) => self.by_tag.entry(k),
                    Key::Attr(k) => self.by_attr.entry(k),
                };
                bucket.or_default().push(index);
            }
        } else {
            self.universal.push(index);
        }
        self.entries.push(entry);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Calls `f` for every entry whose selector matches `el` (each entry at
    /// most once; `seen` / `stamp` deduplicate entries found via several buckets).
    pub fn for_each_match(
        &self,
        el: &DomElement<'_>,
        filter: &AncestorFilter,
        seen: &mut [u32],
        stamp: u32,
        mut f: impl FnMut(&RuleEntry<'s>),
    ) {
        let mut visit = |list: &[u32]| {
            for &i in list {
                let Some(slot) = seen.get_mut(i as usize) else {
                    continue;
                };
                if *slot == stamp {
                    continue;
                }
                *slot = stamp;
                let Some(entry) = self.entries.get(i as usize) else {
                    continue;
                };
                if entry
                    .ancestor_hashes
                    .iter()
                    .any(|&h| h != 0 && !filter.may_contain(h))
                {
                    continue;
                }
                if entry.selector.matches(el) {
                    f(entry);
                }
            }
        };
        if let Some(id) = el.id() {
            if let Some(list) = self.by_id.get(id) {
                visit(list);
            }
        }
        if !self.by_class.is_empty() {
            for class in el.data.classes() {
                if let Some(list) = self.by_class.get(class) {
                    visit(list);
                }
            }
        }
        let name = el.local_name();
        let name: Cow<'_, str> = if name.bytes().any(|b| b.is_ascii_uppercase()) {
            Cow::Owned(name.to_ascii_lowercase())
        } else {
            Cow::Borrowed(name)
        };
        if let Some(list) = self.by_tag.get(name.as_ref()) {
            visit(list);
        }
        if !self.by_attr.is_empty() {
            for attr in &el.data.attrs {
                let list = match self.by_attr.get(attr.name.as_str()) {
                    Some(list) => Some(list),
                    None if attr.name.bytes().any(|b| b.is_ascii_uppercase()) => {
                        self.by_attr.get(attr.name.to_ascii_lowercase().as_str())
                    }
                    None => None,
                };
                if let Some(list) = list {
                    visit(list);
                }
            }
        }
        visit(&self.universal);
    }
}

/// The parsed rules of one origin.
#[derive(Default)]
pub(crate) struct RuleSet<'s> {
    pub blocks: Vec<DeclBlock>,
    pub main: SelectorMap<'s>,
    pub before: SelectorMap<'s>,
    pub after: SelectorMap<'s>,
}

fn flatten<'s>(
    rules: &'s [CssRule],
    media: &MediaContext,
    depth: usize,
    out: &mut Vec<&'s StyleRule>,
) {
    if depth > MAX_RULE_DEPTH {
        return;
    }
    for rule in rules {
        match rule {
            CssRule::Style(style) => out.push(style),
            CssRule::Media { query, rules } => {
                if query.matches(media) {
                    flatten(rules, media, depth + 1, out);
                }
            }
            CssRule::Supports { supported, rules } => {
                if *supported {
                    flatten(rules, media, depth + 1, out);
                }
            }
            CssRule::Group { rules } => flatten(rules, media, depth + 1, out),
            CssRule::Import { .. } => {}
        }
    }
}

impl<'s> RuleSet<'s> {
    /// Builds the rule set of `sheets` (in order); `order_base` offsets the
    /// source order so several sets of the same origin can be combined.
    pub fn new(
        sheets: &'s [Stylesheet],
        media: &MediaContext,
        quirks: bool,
        order_base: u32,
    ) -> Self {
        let mut style_rules = Vec::new();
        for sheet in sheets {
            flatten(&sheet.rules, media, 0, &mut style_rules);
        }
        let mut set = RuleSet::default();
        let mut order = order_base;
        for rule in style_rules {
            let block = DeclBlock::parse(&rule.declarations, quirks);
            if block.is_empty() {
                continue;
            }
            let block_index = u32::try_from(set.blocks.len()).unwrap_or(u32::MAX);
            set.blocks.push(block);
            for selector in &rule.selectors {
                let entry = RuleEntry {
                    selector,
                    specificity: selector.specificity(),
                    order,
                    block: block_index,
                    ancestor_hashes: ancestor_hashes(selector),
                };
                match selector.pseudo_element {
                    None => set.main.insert(entry),
                    Some(PseudoElement::Before) => set.before.insert(entry),
                    Some(PseudoElement::After) => set.after.insert(entry),
                    Some(PseudoElement::Unsupported) => {}
                }
            }
            order = order.saturating_add(1);
        }
        set
    }

    /// Number of source rules with at least one declaration.
    #[cfg(test)]
    pub fn rule_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn block(&self, index: u32) -> Option<&DeclBlock> {
        self.blocks.get(index as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::TreeInfo;
    use crate::StyleContext;

    fn matching_orders(css: &str, html: &str, id: &str) -> Vec<u32> {
        let sheets = vec![xbro_css::parse_stylesheet(css)];
        let set = RuleSet::new(&sheets, &MediaContext::default(), false, 0);
        let doc = xbro_html::parse_document(html);
        let info = TreeInfo::new(&doc, &StyleContext::default());
        let node = doc.get_element_by_id(id).unwrap();
        let el = info.element(node).unwrap();
        // Fill the ancestor filter like the cascade does.
        let mut filter = AncestorFilter::new();
        for a in doc.ancestors(node) {
            if let Some(e) = doc.element(a) {
                filter.push(e);
            }
        }
        let mut seen = vec![0; set.main.entries.len()];
        let mut out = Vec::new();
        set.main
            .for_each_match(&el, &filter, &mut seen, 1, |e| out.push(e.order));
        out.sort();
        out
    }

    #[test]
    fn buckets_and_matching() {
        let css = "#x { color: red } .a { color: red } p { color: red } * { color: red } \
                   .b .a { color: red } div > p.a { color: red } :is(p, span) { color: red } \
                   :is(.zz, p) .q { color: red } span { color: red } [title] { color: red } \
                   [data-x] { color: red } [DATA-Y=b] { color: red }";
        let html = "<div class=b><p id=x class='a a' data-x=1 data-y=b>t</p></div>";
        assert_eq!(
            matching_orders(css, html, "x"),
            vec![0, 1, 2, 3, 4, 5, 6, 10, 11]
        );
    }

    #[test]
    fn media_supports_and_groups() {
        let css = "@media (max-width: 100px) { p { color: red } } \
                   @media (min-width: 100px) { p { color: blue } } \
                   @supports (display: grid) { p { color: green } } \
                   @supports (frobnicate: 1) { p { color: red } } \
                   @layer base { p { color: black } } \
                   p { }";
        // The empty rule has no declarations and is skipped; the order of
        // kept rules counts only non-empty rules.
        assert_eq!(matching_orders(css, "<p id=p></p>", "p"), vec![0, 1, 2]);
    }

    #[test]
    fn pseudo_element_buckets() {
        let sheets = vec![xbro_css::parse_stylesheet(
            "p::before { content: 'a' } p::after { content: 'b' } p::first-line { color: red } p { color: red }",
        )];
        let set = RuleSet::new(&sheets, &MediaContext::default(), false, 0);
        assert_eq!(set.before.entries.len(), 1);
        assert_eq!(set.after.entries.len(), 1);
        assert_eq!(set.main.entries.len(), 1);
        assert_eq!(set.rule_count(), 4);
    }

    #[test]
    fn ancestor_hash_extraction() {
        let s = &xbro_css::parse_selector_list("div#a.b > .c + .d span").unwrap()[0];
        let hashes = ancestor_hashes(s);
        // `.d` (descendant) and `div#a.b` (child)... but `.c` is a sibling.
        assert!(hashes.contains(&hash(KIND_CLASS, "d")));
        assert!(!hashes.contains(&hash(KIND_CLASS, "c")));
        assert!(hashes.contains(&hash(KIND_ID, "a")));
    }
}
