//! Interpretation of the generic rules of `rules` into [`CssRule`]s:
//! supported at-rules, `@import` placement, and CSS Nesting flattening.

use std::borrow::Cow;

use super::rules::{
    consume_block_contents, consume_rule_list, AtRule, BlockItem, QualifiedRule, TopItem,
};
use super::{supports, ComponentValue as CV, CssRule, Declaration, StyleRule, Stylesheet};
use crate::media::MediaQueryList;
use crate::selector::{self, Selector};
use crate::tokenizer::Token;
use crate::values::trim;

/// Recursion guard for nested rules (the component value tree is already
/// bounded by [`super::MAX_NESTING_DEPTH`]; this is a second safety net).
const MAX_RULE_DEPTH: usize = 64;

/// Parses a whole stylesheet from its component values.
pub(crate) fn parse_stylesheet(values: Vec<CV>) -> Stylesheet {
    let mut builder = Builder {
        nesting_budget: NESTING_BUDGET,
    };
    let mut rules = Vec::new();
    // `@import` is only valid before any other rule except `@charset` and
    // `@layer` statements.
    let mut imports_allowed = true;
    for item in consume_rule_list(values) {
        match item {
            TopItem::Qualified(rule) => {
                let before = rules.len();
                builder.style_rule(rule, None, 0, &mut rules);
                if rules.len() > before {
                    imports_allowed = false;
                }
            }
            TopItem::At(rule) => {
                let name = rule.name.to_ascii_lowercase();
                match name.as_str() {
                    "import" => {
                        if imports_allowed {
                            if let Some(import) = parse_import(&rule.prelude) {
                                rules.push(import);
                            }
                        }
                    }
                    "charset" => {}
                    "layer" if rule.block.is_none() => {}
                    _ => {
                        // Unknown (hence invalid) at-rules are dropped by the
                        // parser and do not count as preceding rules.
                        if is_known_at_rule(&name) {
                            imports_allowed = false;
                        }
                        builder.group_at_rule(rule, None, 0, &mut rules);
                    }
                }
            }
        }
    }
    Stylesheet { rules }
}

/// At-rules browsers recognize at the top level of a stylesheet (lowercase
/// name), whether or not xbro applies them. They make a later `@import`
/// invalid; unknown at-rules (`@-ms-viewport`, typos...) do not.
fn is_known_at_rule(name: &str) -> bool {
    let name = name
        .strip_prefix("-webkit-")
        .or_else(|| name.strip_prefix("-moz-"))
        .unwrap_or(name);
    matches!(
        name,
        "media"
            | "supports"
            | "layer"
            | "container"
            | "scope"
            | "document"
            | "starting-style"
            | "font-face"
            | "keyframes"
            | "page"
            | "namespace"
            | "property"
            | "counter-style"
            | "font-feature-values"
            | "font-palette-values"
            | "view-transition"
            | "position-try"
    )
}

/// `@import <url> [layer | layer()]? [supports()]? <media-query-list>?`
fn parse_import(prelude: &[CV]) -> Option<CssRule> {
    let items = trim(prelude);
    let (first, rest) = items.split_first()?;
    let url = match first {
        CV::Token(Token::String(s) | Token::Url(s)) => s.clone(),
        CV::Function { name, args }
            if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("src") =>
        {
            match trim(args) {
                [CV::Token(Token::String(s)), ..] => s.clone(),
                _ => return None,
            }
        }
        _ => return None,
    };
    let mut rest = trim(rest);
    if rest
        .first()
        .is_some_and(|v| v.is_ident("layer") || v.is_function("layer"))
    {
        rest = trim(&rest[1..]);
    }
    if let Some(CV::Function { name, args }) = rest.first() {
        if name.eq_ignore_ascii_case("supports") {
            if !supports::evaluate_import_supports(args) {
                return None;
            }
            rest = trim(&rest[1..]);
        }
    }
    Some(CssRule::Import {
        url,
        media: MediaQueryList::new(rest.to_vec()),
    })
}

/// Maximum total weight (see [`selector::list_weight`]) of the selector
/// lists created by CSS Nesting in one stylesheet. Nested rules copy their
/// parent's selectors, so without a bound a small sheet could expand into a
/// huge one; rules beyond the budget are dropped.
const NESTING_BUDGET: usize = 200_000;

/// The rule that nested rules and declarations are resolved against.
///
/// Built once per style rule that has nested content: resolving each nested
/// rule must not cost time proportional to the parent's selector list (a
/// parent with thousands of selectors and thousands of nested rules would
/// be quadratic).
struct Parent<'a> {
    /// The parent rule's selectors; declaration groups are emitted with them.
    selectors: &'a [Selector],
    /// [`selector::list_weight`] of `selectors`, paid by each declaration group.
    weight: usize,
    /// The selectors `&` can represent (those without a pseudo-element).
    nesting: Cow<'a, [Selector]>,
    /// [`selector::list_weight`] of `nesting`.
    nesting_weight: usize,
}

impl<'a> Parent<'a> {
    fn new(selectors: &'a [Selector]) -> Self {
        let nesting = selector::nesting_parent(selectors);
        let nesting_weight = selector::list_weight(&nesting);
        let weight = if nesting.len() == selectors.len() {
            nesting_weight
        } else {
            selector::list_weight(selectors)
        };
        Parent {
            selectors,
            weight,
            nesting,
            nesting_weight,
        }
    }
}

/// Interprets generic rules into [`CssRule`]s.
struct Builder {
    /// Remaining weight of selectors copied by CSS Nesting.
    nesting_budget: usize,
}

impl Builder {
    /// Pays `weight` from the nesting budget; false when it is exhausted.
    fn spend(&mut self, weight: usize) -> bool {
        if weight > self.nesting_budget {
            self.nesting_budget = 0;
            return false;
        }
        self.nesting_budget -= weight;
        true
    }

    /// Interprets a qualified rule as a style rule (with nesting relative to
    /// `parent` when it is nested) and appends the resulting rules to `out`.
    fn style_rule(
        &mut self,
        rule: QualifiedRule,
        parent: Option<&Parent<'_>>,
        depth: usize,
        out: &mut Vec<CssRule>,
    ) {
        if depth > MAX_RULE_DEPTH {
            return;
        }
        let selectors = match parent {
            None => selector::parse_selector_list_from(&rule.prelude),
            Some(parent) => {
                if self.nesting_budget == 0 {
                    return;
                }
                selector::parse_nested_with(&rule.prelude, &parent.nesting, parent.nesting_weight)
                    .filter(|list| self.spend(selector::list_weight(list)))
            }
        };
        let Some(selectors) = selectors else {
            return;
        };
        let items = consume_block_contents(rule.block);
        if items.iter().all(|item| matches!(item, BlockItem::Decl(_))) {
            let declarations = items
                .into_iter()
                .filter_map(|item| match item {
                    BlockItem::Decl(d) => Some(d),
                    _ => None,
                })
                .collect();
            out.push(CssRule::Style(StyleRule {
                selectors,
                declarations,
            }));
            return;
        }
        // The declarations of a top-level rule are not copies made by
        // nesting; each extra group they form follows a nested rule that paid.
        let this = Parent::new(&selectors);
        self.nested_body(items, &this, depth, parent.is_some(), out);
    }

    /// Emits the contents of a style rule's block (or of a conditional rule
    /// nested in a style rule): declarations become style rules with the
    /// parent selectors, nested rules are flattened after them. Declarations
    /// that follow a nested rule form a new style rule with the parent
    /// selectors, placed after that nested rule (so source order is preserved
    /// for the cascade). Empty declaration groups produce no rule. With
    /// `charge`, each group pays the parent's weight from the nesting budget.
    fn nested_body(
        &mut self,
        items: Vec<BlockItem>,
        parent: &Parent<'_>,
        depth: usize,
        charge: bool,
        out: &mut Vec<CssRule>,
    ) {
        let mut declarations: Vec<Declaration> = Vec::new();
        let flush =
            |builder: &mut Builder, declarations: &mut Vec<Declaration>, out: &mut Vec<CssRule>| {
                if declarations.is_empty() {
                    return;
                }
                let declarations = std::mem::take(declarations);
                if !charge || builder.spend(parent.weight) {
                    out.push(CssRule::Style(StyleRule {
                        selectors: parent.selectors.to_vec(),
                        declarations,
                    }));
                }
            };
        for item in items {
            let mut nested = Vec::new();
            match item {
                BlockItem::Decl(d) => {
                    declarations.push(d);
                    continue;
                }
                BlockItem::Qualified(rule) => {
                    self.style_rule(rule, Some(parent), depth + 1, &mut nested)
                }
                BlockItem::At(rule) => {
                    self.group_at_rule(rule, Some(parent), depth + 1, &mut nested)
                }
            }
            if !nested.is_empty() {
                flush(self, &mut declarations, out);
                out.append(&mut nested);
            }
        }
        flush(self, &mut declarations, out);
    }

    /// Conditional and grouping at-rules (`@media`, `@supports`, `@layer`,
    /// `@container`, `@scope`, `@document`). Everything else is dropped.
    fn group_at_rule(
        &mut self,
        rule: AtRule,
        parent: Option<&Parent<'_>>,
        depth: usize,
        out: &mut Vec<CssRule>,
    ) {
        if depth > MAX_RULE_DEPTH {
            return;
        }
        let Some(block) = rule.block else {
            return;
        };
        let name = rule.name.to_ascii_lowercase();
        enum Kind {
            Media,
            Supports,
            Group,
        }
        let kind = match name.as_str() {
            "media" => Kind::Media,
            "supports" => Kind::Supports,
            "layer" | "container" | "scope" | "document" | "-moz-document" => Kind::Group,
            // `@starting-style` only applies to transitions; unknown at-rules
            // (`@font-face`, `@keyframes`, `@page`, ...) are not applied either.
            _ => return,
        };
        let rules = self.group_body(block, parent, depth);
        if parent.is_some() && rules.is_empty() {
            return;
        }
        out.push(match kind {
            Kind::Media => CssRule::Media {
                query: MediaQueryList::new(trim(&rule.prelude).to_vec()),
                rules,
            },
            Kind::Supports => CssRule::Supports {
                supported: supports::evaluate_supports(&rule.prelude),
                rules,
            },
            Kind::Group => CssRule::Group { rules },
        });
    }

    /// The rules inside a grouping at-rule's block.
    fn group_body(
        &mut self,
        block: Vec<CV>,
        parent: Option<&Parent<'_>>,
        depth: usize,
    ) -> Vec<CssRule> {
        let items = consume_block_contents(block);
        let mut rules = Vec::new();
        match parent {
            Some(parent) => self.nested_body(items, parent, depth + 1, true, &mut rules),
            None => {
                for item in items {
                    match item {
                        // Declarations are not allowed directly in a top-level group.
                        BlockItem::Decl(_) => {}
                        BlockItem::Qualified(rule) => {
                            self.style_rule(rule, None, depth + 1, &mut rules)
                        }
                        BlockItem::At(rule) => {
                            self.group_at_rule(rule, None, depth + 1, &mut rules)
                        }
                    }
                }
            }
        }
        rules
    }
}
