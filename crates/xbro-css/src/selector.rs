//! Selectors Level 4 (subset): parsing, specificity and matching.
//!
//! Parsing lives in `parse`, matching in `matching` (combinators, nested
//! lists, `:has()`) and `predicates` (per-element tests), and the `An+B`
//! microsyntax in `nth`. Selectors are bounded at parse time (at most
//! [`MAX_COMPOUNDS`] compounds per complex selector, functional pseudo-classes
//! nested at most [`MAX_SELECTOR_NESTING`] deep) so matching recursion is
//! bounded too.

mod matching;
mod nth;
mod parse;
mod predicates;

#[cfg(test)]
mod tests;

pub use matching::MatchCache;
pub use parse::{list_weight, parse_nested_selector_list, MAX_COMPOUNDS, MAX_SELECTOR_NESTING};
pub(crate) use parse::{nesting_parent, parse_nested_with};

use crate::parser::ComponentValue;

/// The DOM interface selector matching needs. Implemented by `xbro-style`
/// for DOM nodes and by test doubles in this crate's tests.
pub trait Element: Sized + Clone {
    /// Lowercase local name (`"div"`).
    fn local_name(&self) -> &str;
    fn id(&self) -> Option<&str>;
    fn has_class(&self, class: &str) -> bool;
    /// Attribute value by (lowercase) name.
    fn attr(&self, name: &str) -> Option<&str>;
    fn parent_element(&self) -> Option<Self>;
    fn prev_sibling_element(&self) -> Option<Self>;
    fn next_sibling_element(&self) -> Option<Self>;
    /// First element child (used, with `next_sibling_element`, to walk
    /// descendants for `:has()`).
    fn first_child_element(&self) -> Option<Self>;
    /// True for the document element (`<html>`).
    fn is_root(&self) -> bool;
    /// True when the element has no element children and no non-empty text (`:empty`).
    fn is_empty(&self) -> bool;
    /// `:link` / `:any-link` — `<a>`/`<area>` with an `href`.
    fn is_link(&self) -> bool {
        matches!(self.local_name(), "a" | "area") && self.attr("href").is_some()
    }
    fn is_visited(&self) -> bool {
        false
    }
    fn is_hovered(&self) -> bool {
        false
    }
    fn is_active(&self) -> bool {
        false
    }
    fn is_focused(&self) -> bool {
        false
    }
    /// `:focus-within` — the element or one of its descendants is focused.
    ///
    /// The default walks (at most 4096) descendants looking for a focused
    /// element; implementations that know the focused node should override it
    /// with an O(depth) ancestor check.
    fn is_focus_within(&self) -> bool {
        if self.is_focused() {
            return true;
        }
        let mut budget = 4096u32;
        let mut stack: Vec<Self> = self.first_child_element().into_iter().collect();
        while let Some(element) = stack.pop() {
            if budget == 0 {
                return false;
            }
            budget -= 1;
            if element.is_focused() {
                return true;
            }
            if let Some(next) = element.next_sibling_element() {
                stack.push(next);
            }
            if let Some(child) = element.first_child_element() {
                stack.push(child);
            }
        }
        false
    }
    /// `:target` — the element is the target of the document URL's fragment.
    fn is_target(&self) -> bool {
        false
    }
    /// `:checked` — checked checkbox/radio, selected option.
    fn is_checked(&self) -> bool {
        false
    }
    /// `:disabled` — form controls with the `disabled` attribute.
    fn is_disabled(&self) -> bool {
        false
    }
    /// 1-based position of the element among its element siblings, used by
    /// `:nth-child()` and friends: counted from the first sibling, or from
    /// the last one when `from_end` is set; with `of_type`, only siblings
    /// with the same local name count. An element without siblings is 1.
    ///
    /// The default walks the siblings with [`Element::prev_sibling_element`]
    /// / [`Element::next_sibling_element`], so it costs one call per sibling.
    /// DOMs where those calls are not O(1) (e.g. they search the parent's
    /// child list) should override it with a single scan of the child list,
    /// otherwise matching `:nth-child()` against N siblings is O(N³).
    fn nth_index(&self, of_type: bool, from_end: bool) -> usize {
        predicates::walk_nth_index(self, of_type, from_end)
    }
    /// A number identifying the element within its document, for
    /// [`MatchCache`]: distinct elements must have distinct keys. `None`
    /// (the default) disables caching for the element.
    fn cache_key(&self) -> Option<usize> {
        None
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Combinator {
    /// `a b`
    Descendant,
    /// `a > b`
    Child,
    /// `a + b`
    NextSibling,
    /// `a ~ b`
    SubsequentSibling,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AttrOperator {
    /// `[a]`
    Exists,
    /// `[a=v]`
    Equals,
    /// `[a~=v]`
    Includes,
    /// `[a|=v]`
    DashMatch,
    /// `[a^=v]`
    Prefix,
    /// `[a$=v]`
    Suffix,
    /// `[a*=v]`
    Substring,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrSelector {
    /// Lowercased attribute name.
    pub name: String,
    pub op: AttrOperator,
    pub value: String,
    /// `[a=v i]`; also true without a flag for attributes whose values HTML
    /// matches case-insensitively (`type`, `lang`, `dir`, `align`, ...), unless
    /// the `s` flag is given.
    pub case_insensitive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PseudoClass {
    Root,
    Empty,
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    /// `:nth-child(an+b [of S]?)` — `of S` is not supported (rule dropped).
    NthChild {
        a: i32,
        b: i32,
    },
    NthLastChild {
        a: i32,
        b: i32,
    },
    NthOfType {
        a: i32,
        b: i32,
    },
    NthLastOfType {
        a: i32,
        b: i32,
    },
    Link,
    AnyLink,
    Visited,
    Hover,
    Active,
    Focus,
    FocusWithin,
    FocusVisible,
    Checked,
    Disabled,
    Enabled,
    /// `:not(list)`
    Not(Vec<Selector>),
    /// `:is(list)` / `:matches(list)` / `:-webkit-any(list)`
    Is(Vec<Selector>),
    /// `:where(list)` — like `:is` but zero specificity.
    Where(Vec<Selector>),
    /// `:has(list)` — relative selectors: descendant (`:has(x)`), child
    /// (`:has(> x)`), next-sibling (`:has(+ x)`) and subsequent-sibling
    /// (`:has(~ x)`) forms.
    Has(Vec<(Combinator, Selector)>),
    /// Recognized but unimplemented pseudo-classes that never match
    /// (`:valid`, `:invalid`, `:fullscreen`, `:host`, `:-webkit-autofill`, ...).
    NeverMatches,
    /// `:target` (see [`Element::is_target`]).
    Target,
    /// `:lang(ranges)` — matched against the nearest `lang` attribute
    /// (prefix match on `-` boundaries, ASCII case-insensitive, `*` wildcard).
    Lang(Vec<String>),
    /// `:dir(ltr|rtl)` — matched against the nearest `dir` attribute
    /// (`auto` and no attribute count as `ltr`).
    Dir {
        rtl: bool,
    },
    /// `:defined` — always true (xbro has no custom element registry).
    Defined,
    /// `:required` — `input`/`select`/`textarea` with a `required` attribute.
    Required,
    /// `:optional` — `input`/`select`/`textarea` without `required`.
    Optional,
    /// `:read-only` — the negation of `:read-write`.
    ReadOnly,
    /// `:read-write` — editable text fields (not `readonly`/disabled) and
    /// `contenteditable` content.
    ReadWrite,
    /// `:placeholder-shown` — text field with a `placeholder` and no value.
    PlaceholderShown,
    /// `:indeterminate` — `<progress>` without a `value`.
    Indeterminate,
    /// `:default` — `option[selected]`, checkbox/radio with `checked`.
    Default,
    /// `:open` — `details`/`dialog` with an `open` attribute.
    Open,
    /// `:closed` — `details`/`dialog` without an `open` attribute.
    Closed,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Before,
    After,
    /// `::marker`, `::first-line`, `::first-letter`, `::placeholder`, `::selection`,
    /// `::-webkit-*`… — parsed so the rule is valid, but never generated by xbro.
    Unsupported,
}

/// A compound selector: `tag#id.class[attr]:pseudo`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Compound {
    /// Lowercased type selector; `None` means `*` or no type selector.
    pub tag: Option<String>,
    pub ids: Vec<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSelector>,
    pub pseudo_classes: Vec<PseudoClass>,
}

/// A complex selector such as `ul > li.item a:hover::before`.
///
/// Stored right-to-left: `subject` is the rightmost compound; `ancestors[0]`
/// is related to `subject` by `ancestors[0].0`, `ancestors[1]` to `ancestors[0]`, etc.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Selector {
    pub subject: Compound,
    pub ancestors: Vec<(Combinator, Compound)>,
    pub pseudo_element: Option<PseudoElement>,
}

/// (ids, classes/attributes/pseudo-classes, types/pseudo-elements). Ordered lexicographically.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Specificity(pub u32, pub u32, pub u32);

impl Specificity {
    fn add(self, other: Specificity) -> Specificity {
        Specificity(
            self.0.saturating_add(other.0),
            self.1.saturating_add(other.1),
            self.2.saturating_add(other.2),
        )
    }
}

fn max_specificity<'a>(selectors: impl Iterator<Item = &'a Selector>) -> Specificity {
    selectors
        .map(Selector::specificity)
        .max()
        .unwrap_or_default()
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl Compound {
    /// Specificity of this compound alone.
    pub fn specificity(&self) -> Specificity {
        let mut s = Specificity(
            count(self.ids.len()),
            count(self.classes.len()).saturating_add(count(self.attrs.len())),
            u32::from(self.tag.is_some()),
        );
        for pc in &self.pseudo_classes {
            s = s.add(match pc {
                PseudoClass::Where(_) => Specificity::default(),
                PseudoClass::Is(list) | PseudoClass::Not(list) => max_specificity(list.iter()),
                PseudoClass::Has(list) => max_specificity(list.iter().map(|(_, s)| s)),
                _ => Specificity(0, 1, 0),
            });
        }
        s
    }
}

impl Selector {
    /// Specificity per Selectors Level 4 (`:is/:not/:has` take their most
    /// specific argument, `:where` contributes zero).
    pub fn specificity(&self) -> Specificity {
        let mut s = self.subject.specificity();
        for (_, compound) in &self.ancestors {
            s = s.add(compound.specificity());
        }
        if self.pseudo_element.is_some() {
            s.2 = s.2.saturating_add(1);
        }
        s
    }

    /// Returns true when `element` matches this selector, ignoring
    /// `pseudo_element` (the caller decides which pseudo-element it is styling).
    ///
    /// Matching does a bounded amount of work: nested selector lists are
    /// memoized per element within the call, and a pathological selector
    /// that still exceeds a fixed step budget (about a million compound
    /// tests) is treated as not matching.
    pub fn matches<E: Element>(&self, element: &E) -> bool {
        matching::matches_selector(self, element, None)
    }

    /// [`Selector::matches`], reusing (and filling) `cache` for the
    /// results that do not depend on the subject: `:has()` evaluated on
    /// other elements, which would otherwise walk the same subtrees again
    /// for every element below them (`.page:has(.x) p` costs a walk of the
    /// whole page per paragraph).
    ///
    /// The cache is only valid while the document, the element states and
    /// the selectors stay unchanged (it is keyed by the address of the
    /// selector lists): use one per style pass.
    pub fn matches_with_cache<E: Element>(&self, element: &E, cache: &mut MatchCache) -> bool {
        matching::matches_selector(self, element, Some(cache))
    }
}

/// Parses a comma-separated selector list from component values.
/// Returns `None` if any selector in the list is invalid or uses an unknown
/// pseudo-class/element (the whole rule must then be dropped, per spec).
///
/// A top-level `&` (nesting selector outside of any style rule) matches the
/// root element, like `:scope`.
pub fn parse_selector_list_from(values: &[ComponentValue]) -> Option<Vec<Selector>> {
    parse::parse_top_level(values)
}

/// Convenience wrapper around [`parse_selector_list_from`] for a string.
pub fn parse_selector_list(s: &str) -> Option<Vec<Selector>> {
    parse_selector_list_from(&crate::parser::parse_component_values(s))
}
