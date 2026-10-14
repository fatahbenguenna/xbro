//! Rule collection, matching, cascade and inheritance.
//!
//! The document is walked in tree order with an explicit stack (no
//! recursion). For each element the candidate rules of the user-agent and
//! author rule sets are matched, presentational hints and the `style`
//! attribute are added, the matched blocks are sorted by cascade priority
//! (origin and importance, then specificity, then source order) and the
//! style is computed from the parent's. `display: none` subtrees are
//! skipped entirely; text nodes share their parent's `Arc`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use xbro_css::{MediaContext, Specificity, Stylesheet};
use xbro_dom::{Document, ElementData, NodeData, NodeId};

use crate::bloom::AncestorFilter;
use crate::compute::{self as computing, initial_style, Block, Computed, Env, Extra, Parent};
use crate::computed::{ComputedStyle, Content, Display};
use crate::dom::{DomElement, TreeInfo};
use crate::hints::{self, HintContext};
use crate::properties::{DeclBlock, Decls, LonghandId, PropertyDeclaration};
use crate::rules::{RuleSet, SelectorMap};
use crate::ua::{quirks_stylesheet, user_agent_stylesheet};
use crate::vars::{CustomMap, VarContext};
use crate::{StyleContext, StyleMap};

/// Cascade ranks, lowest priority first.
const UA_NORMAL: u8 = 0;
const HINTS: u8 = 1;
const AUTHOR_NORMAL: u8 = 2;
const STYLE_ATTR_NORMAL: u8 = 3;
const AUTHOR_IMPORTANT: u8 = 4;
const STYLE_ATTR_IMPORTANT: u8 = 5;
const UA_IMPORTANT: u8 = 6;

/// Source order offset of the quirks-mode UA rules (after the main UA sheet).
const QUIRKS_ORDER_BASE: u32 = 1 << 24;

/// Index of the author rule set in [`Shared::sets`] (after the UA and
/// quirks-mode UA sets).
const SET_AUTHOR: usize = 2;

/// The parsed user-agent rules, built once.
pub(crate) fn ua_rules() -> &'static RuleSet<'static> {
    static RULES: OnceLock<RuleSet<'static>> = OnceLock::new();
    RULES.get_or_init(|| {
        RuleSet::new(
            std::slice::from_ref(user_agent_stylesheet()),
            &MediaContext::default(),
            false,
            0,
        )
    })
}

/// The quirks-mode user-agent rules, built once.
pub(crate) fn quirks_rules() -> &'static RuleSet<'static> {
    static RULES: OnceLock<RuleSet<'static>> = OnceLock::new();
    RULES.get_or_init(|| {
        RuleSet::new(
            std::slice::from_ref(quirks_stylesheet()),
            &MediaContext::default(),
            false,
            QUIRKS_ORDER_BASE,
        )
    })
}

#[derive(Copy, Clone, Debug)]
enum MapKind {
    Main = 0,
    Before = 1,
    After = 2,
}

#[derive(Copy, Clone, Debug)]
enum BlockRef {
    Rule {
        set: usize,
        block: u32,
        important: bool,
    },
    Hints,
    StyleAttr {
        important: bool,
    },
}

#[derive(Copy, Clone, Debug)]
struct Matched {
    key: (u8, Specificity, u32),
    block: BlockRef,
}

/// Read-only inputs of a style computation.
struct Shared<'a> {
    doc: &'a Document,
    tree: TreeInfo<'a>,
    sets: [Option<&'a RuleSet<'a>>; 3],
    hints: HintContext,
    quirks: bool,
}

/// Scratch state reused across elements.
#[derive(Default)]
struct Scratch<'a> {
    /// Per (rule set, map kind): stamps deduplicating candidate rules.
    seen: [Vec<u32>; 9],
    stamp: u32,
    filter: Option<AncestorFilter>,
    matched: Vec<Matched>,
    hints: Vec<PropertyDeclaration>,
    /// Parsed `style` attributes. Entries are never removed during a
    /// traversal: their custom declaration lists are memoization keys.
    style_attrs: HashMap<&'a str, Arc<DeclBlock>>,
    /// Custom property / `var()` caches and work budget.
    vars: VarContext,
}

/// Elements that never generate `::before` / `::after` boxes (replaced or void).
fn supports_pseudo_elements(name: &str) -> bool {
    !matches!(
        name,
        "img"
            | "input"
            | "select"
            | "textarea"
            | "iframe"
            | "video"
            | "audio"
            | "canvas"
            | "embed"
            | "object"
            | "br"
            | "wbr"
            | "area"
            | "meter"
            | "progress"
            | "frame"
    )
}

impl<'a> Scratch<'a> {
    fn next_stamp(&mut self) -> u32 {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            for s in &mut self.seen {
                s.iter_mut().for_each(|v| *v = 0);
            }
            self.stamp = 1;
        }
        self.stamp
    }

    /// Adds the matching rules of every rule set for `kind` to `matched`.
    fn collect_rules(&mut self, shared: &Shared<'a>, el: &DomElement<'_>, kind: MapKind) {
        for (set_index, set) in shared.sets.iter().enumerate() {
            let Some(set) = set else {
                continue;
            };
            let map: &SelectorMap<'_> = match kind {
                MapKind::Main => &set.main,
                MapKind::Before => &set.before,
                MapKind::After => &set.after,
            };
            if map.is_empty() {
                continue;
            }
            let stamp = self.next_stamp();
            let Scratch {
                seen,
                filter,
                matched,
                ..
            } = self;
            let Some(seen) = seen.get_mut(set_index * 3 + kind as usize) else {
                continue;
            };
            if seen.len() < map.entries.len() {
                seen.resize(map.entries.len(), 0);
            }
            let Some(filter) = filter.as_ref() else {
                continue;
            };
            let (normal_rank, important_rank) = if set_index == SET_AUTHOR {
                (AUTHOR_NORMAL, AUTHOR_IMPORTANT)
            } else {
                (UA_NORMAL, UA_IMPORTANT)
            };
            map.for_each_match(el, filter, seen, stamp, |entry| {
                let Some(block) = set.block(entry.block) else {
                    return;
                };
                for (important, non_empty, rank) in [
                    (false, !block.normal.is_empty(), normal_rank),
                    (true, !block.important.is_empty(), important_rank),
                ] {
                    if non_empty {
                        matched.push(Matched {
                            key: (rank, entry.specificity, entry.order),
                            block: BlockRef::Rule {
                                set: set_index,
                                block: entry.block,
                                important,
                            },
                        });
                    }
                }
            });
        }
    }

    fn style_attr(&mut self, value: &'a str, quirks: bool) -> Arc<DeclBlock> {
        self.style_attrs
            .entry(value)
            .or_insert_with(|| {
                Arc::new(DeclBlock::parse(
                    &xbro_css::parse_declaration_list(value),
                    quirks,
                ))
            })
            .clone()
    }
}

/// Resolves matched block references (sorted) to declaration slices.
fn resolve_blocks<'b>(
    shared: &'b Shared<'_>,
    matched: &[Matched],
    hints: &'b [PropertyDeclaration],
    style_attr: Option<&'b DeclBlock>,
) -> Vec<Block<'b>> {
    let pick = |b: &'b DeclBlock, important: bool| -> &'b Decls {
        if important {
            &b.important
        } else {
            &b.normal
        }
    };
    matched
        .iter()
        .filter_map(|m| match m.block {
            BlockRef::Rule {
                set,
                block,
                important,
            } => {
                let b = pick(
                    shared.sets.get(set).copied().flatten()?.block(block)?,
                    important,
                );
                Some(Block {
                    decls: &b.longhands,
                    customs: &b.customs,
                    user_agent: set != SET_AUTHOR,
                })
            }
            // Hints never declare custom properties.
            BlockRef::Hints => Some(Block {
                decls: hints,
                customs: &[],
                user_agent: false,
            }),
            BlockRef::StyleAttr { important } => {
                let b = pick(style_attr?, important);
                Some(Block {
                    decls: &b.longhands,
                    customs: &b.customs,
                    user_agent: false,
                })
            }
        })
        .collect()
}

/// Inherited state of an element on the traversal stack.
struct Frame {
    node: NodeId,
    style: Arc<ComputedStyle>,
    extra: Extra,
    customs: CustomMap,
    /// Display of the box children are laid out in (skips `display: contents`).
    box_display: Option<Display>,
    next_child: usize,
    is_element: bool,
}

pub(crate) fn compute(
    doc: &Document,
    author_sheets: &[Stylesheet],
    ctx: &StyleContext,
) -> StyleMap {
    let author = RuleSet::new(author_sheets, &ctx.media, doc.quirks_mode, 0);
    let shared = Shared {
        doc,
        tree: TreeInfo::new(doc, ctx),
        sets: [
            Some(ua_rules()),
            doc.quirks_mode.then(quirks_rules),
            Some(&author),
        ],
        hints: HintContext::new(doc),
        quirks: doc.quirks_mode,
    };
    let mut scratch = Scratch {
        filter: Some(AncestorFilter::new()),
        ..Scratch::default()
    };
    let env = Env {
        viewport: (ctx.media.width, ctx.media.height),
        root_font_size: initial_style().font_size,
    };
    walk(&shared, &mut scratch, env)
}

fn walk<'a>(shared: &Shared<'a>, scratch: &mut Scratch<'a>, mut env: Env) -> StyleMap {
    let doc = shared.doc;
    let n = doc.len();
    let mut map = StyleMap {
        styles: vec![None; n],
        before: HashMap::new(),
        after: HashMap::new(),
    };
    let mut visited = vec![false; n];
    let root = doc.root();
    if let Some(v) = visited.get_mut(root.index()) {
        *v = true;
    }
    let mut stack = vec![Frame {
        node: root,
        style: Arc::new(initial_style().clone()),
        extra: Extra::default(),
        customs: CustomMap::default(),
        box_display: None,
        next_child: 0,
        is_element: false,
    }];
    while let Some(top) = stack.last_mut() {
        let Some(&child) = doc.children(top.node).get(top.next_child) else {
            if let Some(frame) = stack.pop() {
                if frame.is_element {
                    if let (Some(e), Some(filter)) =
                        (doc.element(frame.node), scratch.filter.as_mut())
                    {
                        filter.pop(e);
                    }
                }
            }
            continue;
        };
        top.next_child += 1;
        match visited.get_mut(child.index()) {
            Some(v) if !*v => *v = true,
            _ => continue, // out of range or already styled (corrupted tree)
        }
        let Some(frame) = stack.last() else {
            break;
        };
        match &doc.node(child).data {
            NodeData::Text(_) => {
                if frame.is_element {
                    if let Some(slot) = map.styles.get_mut(child.index()) {
                        *slot = Some(frame.style.clone());
                    }
                }
            }
            NodeData::Element(e) => {
                let parent = Parent {
                    style: &frame.style,
                    extra: &frame.extra,
                    customs: &frame.customs,
                    box_display: frame.box_display,
                };
                let parent_box = frame.box_display;
                let is_root = shared.tree.is_root(child);
                let Some(computed) =
                    style_element(shared, scratch, child, e, &parent, &env, is_root)
                else {
                    continue;
                };
                if computed.style.display == Display::None {
                    continue;
                }
                if is_root {
                    env.root_font_size = computed.style.font_size;
                }
                let style = Arc::new(computed.style);
                let box_display = if style.display == Display::Contents {
                    parent_box
                } else {
                    Some(style.display)
                };
                if supports_pseudo_elements(&e.name) {
                    let element_parent = Parent {
                        style: &style,
                        extra: &computed.extra,
                        customs: &computed.customs,
                        box_display,
                    };
                    for kind in [MapKind::Before, MapKind::After] {
                        if let Some(pseudo) =
                            style_pseudo(shared, scratch, child, e, &element_parent, &env, kind)
                        {
                            let target = match kind {
                                MapKind::After => &mut map.after,
                                _ => &mut map.before,
                            };
                            target.insert(child, Arc::new(pseudo));
                        }
                    }
                }
                if let Some(slot) = map.styles.get_mut(child.index()) {
                    *slot = Some(style.clone());
                }
                if let Some(filter) = scratch.filter.as_mut() {
                    filter.push(e);
                }
                stack.push(Frame {
                    node: child,
                    style,
                    extra: computed.extra,
                    customs: computed.customs,
                    box_display,
                    next_child: 0,
                    is_element: true,
                });
            }
            _ => {}
        }
    }
    map
}

/// Matches, cascades and computes the style of one element.
fn style_element<'a>(
    shared: &Shared<'a>,
    scratch: &mut Scratch<'a>,
    node: NodeId,
    e: &'a ElementData,
    parent: &Parent<'_>,
    env: &Env,
    is_root: bool,
) -> Option<Computed> {
    let el = shared.tree.element(node)?;
    scratch.matched.clear();
    scratch.hints.clear();
    scratch.collect_rules(shared, &el, MapKind::Main);
    hints::collect(shared.doc, node, e, &shared.hints, &mut scratch.hints);
    if !scratch.hints.is_empty() {
        scratch.matched.push(Matched {
            key: (HINTS, Specificity::default(), 0),
            block: BlockRef::Hints,
        });
    }
    let style_attr = e
        .attr("style")
        .map(|value| scratch.style_attr(value, shared.quirks));
    if let Some(block) = &style_attr {
        if !block.normal.is_empty() {
            scratch.matched.push(Matched {
                key: (STYLE_ATTR_NORMAL, Specificity::default(), 0),
                block: BlockRef::StyleAttr { important: false },
            });
        }
        if !block.important.is_empty() {
            scratch.matched.push(Matched {
                key: (STYLE_ATTR_IMPORTANT, Specificity::default(), 0),
                block: BlockRef::StyleAttr { important: true },
            });
        }
    }
    scratch.matched.sort_by_key(|m| m.key);
    let blocks = resolve_blocks(
        shared,
        &scratch.matched,
        &scratch.hints,
        style_attr.as_deref(),
    );
    Some(computing::compute(
        &blocks,
        parent,
        env,
        Some(e),
        is_root,
        &mut scratch.vars,
    ))
}

/// Computes `::before` / `::after`; `None` when it generates no box.
fn style_pseudo<'a>(
    shared: &Shared<'a>,
    scratch: &mut Scratch<'a>,
    node: NodeId,
    e: &'a ElementData,
    element: &Parent<'_>,
    env: &Env,
    kind: MapKind,
) -> Option<ComputedStyle> {
    let el = shared.tree.element(node)?;
    scratch.matched.clear();
    scratch.collect_rules(shared, &el, kind);
    if scratch.matched.is_empty() {
        return None;
    }
    scratch.matched.sort_by_key(|m| m.key);
    let blocks = resolve_blocks(shared, &scratch.matched, &[], None);
    // `content` is not inherited: without a declaration for it the
    // pseudo-element computes to `content: normal` and generates no box
    // (common with `*::before { box-sizing: ... }` resets).
    let declares_content = blocks.iter().any(|b| {
        b.decls
            .iter()
            .any(|d| d.longhand() == Some(LonghandId::Content))
    });
    if !declares_content {
        return None;
    }
    let computed = computing::compute(&blocks, element, env, Some(e), false, &mut scratch.vars);
    let style = computed.style;
    if style.display == Display::None || matches!(style.content, Content::Normal) {
        return None;
    }
    Some(style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_sets_are_cached() {
        assert!(std::ptr::eq(ua_rules(), ua_rules()));
        assert!(ua_rules().rule_count() > 60);
        assert!(quirks_rules().rule_count() >= 2);
    }

    #[test]
    fn pseudo_support() {
        assert!(supports_pseudo_elements("div"));
        assert!(supports_pseudo_elements("q"));
        assert!(!supports_pseudo_elements("img"));
        assert!(!supports_pseudo_elements("input"));
    }
}
