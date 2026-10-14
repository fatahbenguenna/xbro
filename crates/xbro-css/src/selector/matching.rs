//! Right-to-left selector matching.
//!
//! Complex selectors are matched from the subject leftwards with
//! backtracking over descendant / subsequent-sibling combinators. The
//! failure statuses of Servo's matcher prune the backtracking: when the
//! left part of a selector runs out of ancestors, trying further ancestors
//! for a combinator on its right cannot succeed either, which keeps
//! selectors like `a a a a b` linear in the tree depth.
//!
//! Selector lists nested in `:is()` / `:where()` / `:not()` (which CSS
//! Nesting produces at every level of a rule with several parent selectors)
//! would make that backtracking exponential in the nesting depth: each
//! candidate ancestor re-evaluates the whole inner list, which itself walks
//! the ancestors again. Within one top-level match, the results of such
//! lists are therefore memoized per element. Elements have no identity in
//! the [`Element`] trait, but every element reached from the subject by
//! parent / previous-sibling steps is identified by a [`Pos`]: the number of
//! parent steps, then the number of previous-sibling steps after the last
//! parent step (siblings share their parent, so that pair is unique). A
//! global step budget ([`MATCH_BUDGET`]) bounds pathological cases that
//! remain (the selector is then treated as not matching).
//!
//! `:has()` walks the descendants or following siblings of the anchor
//! (bounded by [`HAS_BUDGET`] elements per evaluation) and matches each
//! candidate right-to-left, with positions tracked relative to the anchor.

use std::collections::HashMap;

use super::predicates::{matches_attribute, matches_simple_pseudo_class};
use super::{Combinator, Compound, Element, PseudoClass, Selector};

/// Maximum number of elements a single `:has()` evaluation visits.
const HAS_BUDGET: u32 = 100_000;

/// Maximum number of compound matching steps of one top-level
/// [`Selector::matches`] call, all nested lists included. Exhausting it
/// makes the selector not match; ordinary selectors stay several orders of
/// magnitude below it.
const MATCH_BUDGET: u32 = 1_000_000;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum MatchResult {
    Matched,
    NotMatchedAndRestartFromClosestLaterSibling,
    NotMatchedAndRestartFromClosestDescendant,
    NotMatchedGlobally,
}

use MatchResult::*;

/// Position of an element relative to the subject of a top-level match:
/// `up` parent steps, then `left` previous-sibling steps.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct Pos {
    up: u32,
    left: u32,
}

impl Pos {
    const SUBJECT: Pos = Pos { up: 0, left: 0 };

    fn parent(self) -> Pos {
        Pos {
            up: self.up.saturating_add(1),
            left: 0,
        }
    }

    fn prev_sibling(self) -> Pos {
        Pos {
            up: self.up,
            left: self.left.saturating_add(1),
        }
    }
}

/// State of one top-level match.
struct Ctx {
    /// Remaining matching steps.
    budget: u32,
    /// Set when the budget ran out: the whole match then fails.
    exhausted: bool,
    /// Results of nested selector lists: (list address, position) → matched.
    memo: Option<HashMap<(usize, Pos), bool>>,
}

impl Ctx {
    fn new() -> Self {
        Ctx {
            budget: MATCH_BUDGET,
            exhausted: false,
            memo: None,
        }
    }

    /// Pays for one step; false once the budget is exhausted.
    fn step(&mut self) -> bool {
        if self.budget == 0 {
            self.exhausted = true;
            return false;
        }
        self.budget -= 1;
        true
    }

    /// Returns the memoized result for `key`, or computes and records it.
    /// Results computed after the budget ran out are not recorded.
    fn memoized(
        &mut self,
        key: Option<(usize, Pos)>,
        compute: impl FnOnce(&mut Ctx) -> bool,
    ) -> bool {
        let Some(key) = key else {
            return compute(self);
        };
        if let Some(&cached) = self.memo.as_ref().and_then(|memo| memo.get(&key)) {
            return cached;
        }
        let result = compute(self);
        if !self.exhausted {
            self.memo
                .get_or_insert_with(HashMap::new)
                .insert(key, result);
        }
        result
    }
}

/// Position of a candidate relative to a `:has()` anchor.
#[derive(Copy, Clone, Debug)]
struct Relative {
    /// The relative selector's leading combinator.
    leading: Combinator,
    /// Depth below the scope's top level: for descendant scopes the anchor's
    /// children are at depth 1; for sibling scopes the anchor's following
    /// siblings are at depth 0 and their children at depth 1.
    depth: u32,
    /// Sibling offset from the anchor (1 = next sibling), meaningful at depth 0.
    offset: u32,
    /// Offset of the top-level sibling whose subtree the candidate is in.
    top_offset: u32,
}

impl Relative {
    fn sibling_scope(&self) -> bool {
        matches!(
            self.leading,
            Combinator::NextSibling | Combinator::SubsequentSibling
        )
    }

    /// Position of the parent, or `None` when it leaves the scope.
    fn up(self) -> Option<Relative> {
        let depth = self.depth.checked_sub(1)?;
        if depth == 0 && !self.sibling_scope() {
            return None;
        }
        Some(Relative {
            depth,
            offset: if depth == 0 { self.top_offset } else { 0 },
            ..self
        })
    }

    /// Position of the previous sibling, or `None` when it leaves the scope.
    fn left(self) -> Option<Relative> {
        if self.depth == 0 {
            if self.offset <= 1 {
                return None;
            }
            return Some(Relative {
                offset: self.offset - 1,
                ..self
            });
        }
        Some(self)
    }

    /// Is the leftmost compound's element related to the anchor by the
    /// leading combinator?
    fn anchored(&self) -> bool {
        match self.leading {
            Combinator::Descendant => self.depth >= 1,
            Combinator::Child => self.depth == 1,
            Combinator::NextSibling => self.depth == 0 && self.offset == 1,
            Combinator::SubsequentSibling => self.depth == 0 && self.offset >= 1,
        }
    }
}

/// Where a candidate element is: relative to a `:has()` anchor, and/or
/// relative to the subject of the top-level match (for memoization).
#[derive(Copy, Clone, Debug)]
struct Place {
    relative: Option<Relative>,
    pos: Option<Pos>,
}

pub(crate) fn matches_selector<E: Element>(selector: &Selector, element: &E) -> bool {
    let mut ctx = Ctx::new();
    let place = Place {
        relative: None,
        pos: Some(Pos::SUBJECT),
    };
    match_from(selector, 0, element, place, &mut ctx) == Matched && !ctx.exhausted
}

/// The compound at `index` (0 = subject, then leftwards).
fn compound_at(selector: &Selector, index: usize) -> Option<&Compound> {
    if index == 0 {
        Some(&selector.subject)
    } else {
        selector.ancestors.get(index - 1).map(|(_, c)| c)
    }
}

/// Matches the compounds from `index` leftwards, with `element` as the
/// candidate for compound `index`.
fn match_from<E: Element>(
    selector: &Selector,
    index: usize,
    element: &E,
    place: Place,
    ctx: &mut Ctx,
) -> MatchResult {
    if !ctx.step() {
        return NotMatchedGlobally;
    }
    let Some(compound) = compound_at(selector, index) else {
        return NotMatchedGlobally;
    };
    if !matches_compound(compound, element, place.pos, ctx) {
        if ctx.exhausted {
            return NotMatchedGlobally;
        }
        return NotMatchedAndRestartFromClosestLaterSibling;
    }
    let Some(&(combinator, _)) = selector.ancestors.get(index) else {
        // Leftmost compound.
        if place.relative.is_some_and(|r| !r.anchored()) {
            return NotMatchedAndRestartFromClosestLaterSibling;
        }
        return Matched;
    };
    let candidate_not_found = match combinator {
        Combinator::NextSibling | Combinator::SubsequentSibling => {
            NotMatchedAndRestartFromClosestDescendant
        }
        Combinator::Child | Combinator::Descendant => NotMatchedGlobally,
    };
    let mut next = step(element, combinator, place);
    loop {
        let Some((candidate, candidate_place)) = next else {
            return candidate_not_found;
        };
        let result = match_from(selector, index + 1, &candidate, candidate_place, ctx);
        match (result, combinator) {
            (Matched, _) | (NotMatchedGlobally, _) | (_, Combinator::NextSibling) => return result,
            (_, Combinator::Child) => return NotMatchedAndRestartFromClosestDescendant,
            (NotMatchedAndRestartFromClosestDescendant, Combinator::SubsequentSibling) => {
                return result
            }
            _ => {}
        }
        next = step(&candidate, combinator, candidate_place);
    }
}

/// The next candidate for `combinator` (parent or previous sibling).
fn step<E: Element>(element: &E, combinator: Combinator, place: Place) -> Option<(E, Place)> {
    match combinator {
        Combinator::Descendant | Combinator::Child => {
            let relative = match place.relative {
                Some(r) => Some(r.up()?),
                None => None,
            };
            let place = Place {
                relative,
                pos: place.pos.map(Pos::parent),
            };
            Some((element.parent_element()?, place))
        }
        Combinator::NextSibling | Combinator::SubsequentSibling => {
            let relative = match place.relative {
                Some(r) => Some(r.left()?),
                None => None,
            };
            let place = Place {
                relative,
                pos: place.pos.map(Pos::prev_sibling),
            };
            Some((element.prev_sibling_element()?, place))
        }
    }
}

fn matches_compound<E: Element>(
    compound: &Compound,
    element: &E,
    pos: Option<Pos>,
    ctx: &mut Ctx,
) -> bool {
    if let Some(tag) = &compound.tag {
        if !element.local_name().eq_ignore_ascii_case(tag) {
            return false;
        }
    }
    if !compound.ids.is_empty() {
        let Some(id) = element.id() else {
            return false;
        };
        if compound.ids.iter().any(|wanted| wanted != id) {
            return false;
        }
    }
    compound.classes.iter().all(|c| element.has_class(c))
        && compound.attrs.iter().all(|a| matches_attribute(a, element))
        && compound
            .pseudo_classes
            .iter()
            .all(|pc| matches_pseudo_class(pc, element, pos, ctx))
}

fn matches_pseudo_class<E: Element>(
    pc: &PseudoClass,
    element: &E,
    pos: Option<Pos>,
    ctx: &mut Ctx,
) -> bool {
    match pc {
        PseudoClass::Not(list) => !matches_list(list, element, pos, ctx),
        PseudoClass::Is(list) | PseudoClass::Where(list) => matches_list(list, element, pos, ctx),
        PseudoClass::Has(list) => {
            // A `:has()` evaluation walks a whole subtree: memoized like
            // the other lists away from the subject.
            let key = memo_key(list, pos, || true);
            ctx.memoized(key, |ctx| {
                list.iter().any(|(combinator, selector)| {
                    matches_relative(element, *combinator, selector, ctx)
                })
            })
        }
        simple => matches_simple_pseudo_class(simple, element),
    }
}

/// The memoization key of a nested list at `pos`: the list's address and
/// the position. `None` at the subject (evaluated once per match), outside
/// the subject-relative frame (inside `:has()`), or when not `worth` it.
fn memo_key<T>(list: &[T], pos: Option<Pos>, worth: impl FnOnce() -> bool) -> Option<(usize, Pos)> {
    pos.filter(|p| *p != Pos::SUBJECT && !list.is_empty() && worth())
        .map(|p| (list.as_ptr() as usize, p))
}

/// Is evaluating `list` expensive enough to be worth memoizing (does it
/// walk other elements or contain nested lists)?
fn worth_memoizing(list: &[Selector]) -> bool {
    list.iter().any(|selector| {
        !selector.ancestors.is_empty()
            || std::iter::once(&selector.subject)
                .chain(selector.ancestors.iter().map(|(_, c)| c))
                .flat_map(|c| c.pseudo_classes.iter())
                .any(|pc| {
                    matches!(
                        pc,
                        PseudoClass::Not(_)
                            | PseudoClass::Is(_)
                            | PseudoClass::Where(_)
                            | PseudoClass::Has(_)
                    )
                })
    })
}

/// Does `element` match any selector of `list` (the argument of `:is()`,
/// `:where()` or `:not()`)? The inner selectors are absolute: their own
/// combinators walk the whole document.
fn matches_list<E: Element>(
    list: &[Selector],
    element: &E,
    pos: Option<Pos>,
    ctx: &mut Ctx,
) -> bool {
    // The subject is evaluated once per match; other positions may be
    // reached repeatedly through backtracking.
    let key = memo_key(list, pos, || worth_memoizing(list));
    let place = Place {
        relative: None,
        pos,
    };
    ctx.memoized(key, |ctx| {
        list.iter()
            .any(|selector| match_from(selector, 0, element, place, ctx) == Matched)
    })
}

/// Does `anchor` have a descendant / sibling matching the relative selector
/// `leading selector`?
fn matches_relative<E: Element>(
    anchor: &E,
    leading: Combinator,
    selector: &Selector,
    ctx: &mut Ctx,
) -> bool {
    let needs_subtree = selector
        .ancestors
        .iter()
        .any(|(c, _)| matches!(c, Combinator::Descendant | Combinator::Child));
    let mut budget = HAS_BUDGET;
    let at = |relative: Relative| Place {
        relative: Some(relative),
        pos: None,
    };
    match leading {
        Combinator::Descendant | Combinator::Child => {
            if leading == Combinator::Descendant || needs_subtree {
                walk_subtree(anchor, leading, 0, selector, &mut budget, ctx)
            } else {
                let mut child = anchor.first_child_element();
                while let Some(element) = child {
                    if budget == 0 || ctx.exhausted {
                        return false;
                    }
                    budget -= 1;
                    let relative = Relative {
                        leading,
                        depth: 1,
                        offset: 0,
                        top_offset: 0,
                    };
                    if match_from(selector, 0, &element, at(relative), ctx) == Matched {
                        return true;
                    }
                    child = element.next_sibling_element();
                }
                false
            }
        }
        Combinator::NextSibling | Combinator::SubsequentSibling => {
            let has_sibling_combinator = selector
                .ancestors
                .iter()
                .any(|(c, _)| matches!(c, Combinator::NextSibling | Combinator::SubsequentSibling));
            let only_next = leading == Combinator::NextSibling && !has_sibling_combinator;
            let mut offset = 1u32;
            let mut sibling = anchor.next_sibling_element();
            while let Some(element) = sibling {
                if budget == 0 || ctx.exhausted {
                    return false;
                }
                budget -= 1;
                let relative = Relative {
                    leading,
                    depth: 0,
                    offset,
                    top_offset: offset,
                };
                if match_from(selector, 0, &element, at(relative), ctx) == Matched {
                    return true;
                }
                if needs_subtree
                    && walk_subtree(&element, leading, offset, selector, &mut budget, ctx)
                {
                    return true;
                }
                if only_next {
                    break;
                }
                offset = offset.saturating_add(1);
                sibling = element.next_sibling_element();
            }
            false
        }
    }
}

/// Tries every descendant of `root` (pre-order, iteratively) as the subject.
fn walk_subtree<E: Element>(
    root: &E,
    leading: Combinator,
    top_offset: u32,
    selector: &Selector,
    budget: &mut u32,
    ctx: &mut Ctx,
) -> bool {
    let mut current = root.first_child_element();
    let mut depth = 1u32;
    while let Some(element) = current {
        if *budget == 0 || ctx.exhausted {
            return false;
        }
        *budget -= 1;
        let place = Place {
            relative: Some(Relative {
                leading,
                depth,
                offset: 0,
                top_offset,
            }),
            pos: None,
        };
        if match_from(selector, 0, &element, place, ctx) == Matched {
            return true;
        }
        if let Some(child) = element.first_child_element() {
            current = Some(child);
            depth += 1;
            continue;
        }
        let mut node = element;
        current = loop {
            if let Some(next) = node.next_sibling_element() {
                break Some(next);
            }
            if depth <= 1 {
                break None;
            }
            match node.parent_element() {
                Some(parent) => {
                    node = parent;
                    depth -= 1;
                }
                None => break None,
            }
        };
    }
    false
}
