//! CSS custom properties (`--name`) and `var()` substitution.
//!
//! Custom properties are inherited: each element shares its parent's map
//! (an `Arc`) unless it declares custom properties with new values. A map
//! that does change stores only its own entries and points to its parent's
//! map; chains are flattened every few levels (unless the maps are large),
//! and the flattened map is cached on the parent so that its children share
//! it. Declaring a few variables on every element (as CSS frameworks do)
//! therefore never copies the whole inherited set.
//!
//! Values stored in the map are already substituted (computed values), so
//! children never re-resolve them. Every property that is part of a
//! dependency cycle (fallback references included) is invalid (removed);
//! cycles are found with strongly connected components (see [`graph`]).
//! Substitution depth is bounded and the size of substituted values is
//! capped so `var()` chains cannot blow up.
//!
//! Work is bounded per document as well: results are memoized (see
//! [`cache`]) and every substitution, custom declaration and map flattening
//! is charged to a [`Budget`]. Once a hostile page exhausts it, further
//! substitutions fail (the properties behave as `unset`) instead of taking
//! time proportional to elements × value size.

mod cache;
mod graph;
mod memo;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

pub(crate) use cache::VarContext;
use memo::Owner;
pub(crate) use memo::Substitutions;

use xbro_css::values::trim;
use xbro_css::ComponentValue as CV;

use crate::properties::{CssWide, CustomValue};
use crate::values::contains_var;

/// Computed custom properties of an element.
pub(crate) type CustomMap = Arc<CustomProps>;

/// An element's own custom property values, keyed by name (`None` = removed).
type OwnValues<'a> = HashMap<&'a str, (&'a Arc<str>, Option<Arc<[CV]>>)>;

/// Chains longer than this are flattened.
const MAX_CHAIN_DEPTH: u8 = 6;

/// Chains whose maps hold more entries than this in total are not
/// flattened (copying them for every element would be quadratic); their
/// lookups walk the chain instead, whose length is bounded by the DOM depth.
const FLATTEN_LIMIT: usize = 1024;

/// Work units allowed per document for custom properties: tokens produced
/// by `var()` substitution, custom declarations processed and entries
/// copied when flattening maps. Ordinary pages use a small fraction of it.
pub(crate) const DOCUMENT_BUDGET: usize = 32_000_000;

/// A work budget shared by all custom-property computations of a document.
#[derive(Debug)]
pub(crate) struct Budget(Cell<usize>);

impl Budget {
    pub fn new(units: usize) -> Self {
        Budget(Cell::new(units))
    }

    pub fn remaining(&self) -> usize {
        self.0.get()
    }

    /// Consumes `n` units (saturating at zero).
    pub fn consume(&self, n: usize) {
        self.0.set(self.0.get().saturating_sub(n));
    }

    /// Consumes `n` units if available. Otherwise exhausts the budget and
    /// returns false, so failed attempts are not repeated for free.
    pub fn charge(&self, n: usize) -> bool {
        match self.0.get().checked_sub(n) {
            Some(rest) => {
                self.0.set(rest);
                true
            }
            None => {
                self.0.set(0);
                false
            }
        }
    }
}

/// A persistent map of custom properties: own entries over a parent map.
#[derive(Debug, Default)]
pub(crate) struct CustomProps {
    /// Own entries; `None` hides an inherited value (guaranteed-invalid).
    local: HashMap<Arc<str>, Option<Arc<[CV]>>>,
    parent: Option<CustomMap>,
    depth: u8,
    /// Upper bound of the number of entries of the flattened map (the sum
    /// of the local sizes along the chain).
    total: usize,
    /// Equivalent map without parent, built on demand.
    flat: OnceLock<CustomMap>,
}

impl Drop for CustomProps {
    /// Unlinks the parent chain iteratively: dropping a long chain of maps
    /// must not recurse once per level.
    fn drop(&mut self) {
        let mut next = self.parent.take();
        while let Some(map) = next {
            next = match Arc::try_unwrap(map) {
                // Last owner: detach its parent before it is dropped.
                Ok(mut inner) => inner.parent.take(),
                // Still shared: dropping this reference only decrements.
                Err(_) => None,
            };
        }
    }
}

impl CustomProps {
    /// The computed value of `name`, if it is defined.
    pub fn get(&self, name: &str) -> Option<&Arc<[CV]>> {
        let mut current = self;
        loop {
            if let Some(v) = current.local.get(name) {
                return v.as_ref();
            }
            current = current.parent.as_deref()?;
        }
    }

    /// This map without parent (shared by every caller).
    fn flattened(self: &Arc<Self>) -> CustomMap {
        if self.parent.is_none() {
            return self.clone();
        }
        self.flat
            .get_or_init(|| {
                let mut chain = Vec::new();
                let mut current = Some(self);
                while let Some(m) = current {
                    chain.push(m);
                    current = m.parent.as_ref();
                }
                let mut local = HashMap::new();
                for m in chain.iter().rev() {
                    for (k, v) in &m.local {
                        match v {
                            Some(v) => {
                                local.insert(k.clone(), Some(v.clone()));
                            }
                            None => {
                                local.remove(k);
                            }
                        }
                    }
                }
                let total = local.len();
                Arc::new(CustomProps {
                    local,
                    parent: None,
                    depth: 0,
                    total,
                    flat: OnceLock::new(),
                })
            })
            .clone()
    }

    /// A child map: `entries` over `parent`. Long chains are flattened when
    /// they are small enough and the budget allows it.
    fn derive(
        parent: &CustomMap,
        entries: HashMap<Arc<str>, Option<Arc<[CV]>>>,
        budget: &Budget,
    ) -> CustomMap {
        let base = if parent.depth < MAX_CHAIN_DEPTH {
            parent.clone()
        } else if let Some(flat) = parent.flat.get() {
            flat.clone()
        } else if parent.total <= FLATTEN_LIMIT && budget.charge(parent.total) {
            parent.flattened()
        } else {
            parent.clone()
        };
        let depth = base.depth.saturating_add(1);
        let total = base.total.saturating_add(entries.len());
        Arc::new(CustomProps {
            local: entries,
            parent: Some(base),
            depth,
            total,
            flat: OnceLock::new(),
        })
    }
}

/// Maximum number of component values produced by one substitution.
const MAX_SUBSTITUTED: usize = 100_000;
/// Maximum nesting of fallbacks / functions during one substitution.
const MAX_NESTING: usize = 64;

/// Replaces `var()` (and `env()`) references in `values` using `lookup`.
/// Returns `None` when a reference cannot be resolved and has no fallback,
/// or when limits are exceeded ("invalid at computed-value time"). The
/// tokens produced are charged to `budget` (also when it fails).
pub(crate) fn substitute(
    values: &[CV],
    lookup: &mut dyn FnMut(&str) -> Option<Arc<[CV]>>,
    budget: &Budget,
) -> Option<Vec<CV>> {
    let mut out = Vec::new();
    let cap = MAX_SUBSTITUTED.min(budget.remaining());
    let mut local = cap;
    let ok = substitute_into(values, lookup, &mut out, &mut local, 0);
    budget.consume(cap.saturating_sub(local));
    ok.then_some(out)
}

fn charge(budget: &mut usize, n: usize) -> bool {
    match budget.checked_sub(n) {
        Some(rest) => {
            *budget = rest;
            true
        }
        None => false,
    }
}

/// Parses `var()` arguments: `(name, fallback)`.
fn var_args(args: &[CV]) -> Option<(&str, Option<&[CV]>)> {
    let args = trim(args);
    let (first, rest) = args.split_first()?;
    let name = first.ident().filter(|n| n.starts_with("--"))?;
    let rest = trim(rest);
    let fallback = match rest.split_first() {
        None => None,
        Some((comma, fb)) if comma.is_comma() => Some(trim(fb)),
        Some(_) => return None,
    };
    Some((name, fallback))
}

fn substitute_into(
    values: &[CV],
    lookup: &mut dyn FnMut(&str) -> Option<Arc<[CV]>>,
    out: &mut Vec<CV>,
    budget: &mut usize,
    depth: usize,
) -> bool {
    if depth > MAX_NESTING {
        return false;
    }
    for v in values {
        match v {
            CV::Function { name, args } if name.eq_ignore_ascii_case("var") => {
                let Some((var, fallback)) = var_args(args) else {
                    return false;
                };
                match lookup(var) {
                    Some(value) => {
                        if !charge(budget, value.len()) {
                            return false;
                        }
                        out.extend(value.iter().cloned());
                    }
                    None => match fallback {
                        Some(fb) => {
                            if !substitute_into(fb, lookup, out, budget, depth + 1) {
                                return false;
                            }
                        }
                        None => return false,
                    },
                }
            }
            CV::Function { name, args } if name.eq_ignore_ascii_case("env") => {
                // No environment variables are defined: use the fallback, or
                // 0px for the safe-area insets (as on a rectangular screen).
                let args_t = trim(args);
                let env_name = args_t.first().and_then(|a| a.ident()).unwrap_or("");
                let fallback = args_t
                    .iter()
                    .position(|a| a.is_comma())
                    .and_then(|i| args_t.get(i + 1..))
                    .map(trim);
                match fallback {
                    Some(fb) => {
                        if !substitute_into(fb, lookup, out, budget, depth + 1) {
                            return false;
                        }
                    }
                    None if env_name.starts_with("safe-area-inset-") => {
                        out.push(CV::Token(xbro_css::Token::Dimension {
                            value: 0.0,
                            unit: "px".into(),
                        }));
                    }
                    None => return false,
                }
            }
            CV::Function { name, args } if contains_var(args) => {
                let mut inner = Vec::new();
                if !substitute_into(args, lookup, &mut inner, budget, depth + 1) {
                    return false;
                }
                out.push(CV::Function {
                    name: name.clone(),
                    args: inner,
                });
            }
            CV::Block { open, content } if contains_var(content) => {
                let mut inner = Vec::new();
                if !substitute_into(content, lookup, &mut inner, budget, depth + 1) {
                    return false;
                }
                out.push(CV::Block {
                    open: *open,
                    content: inner,
                });
            }
            other => {
                if !charge(budget, 1) {
                    return false;
                }
                out.push(other.clone());
            }
        }
    }
    true
}

/// Pending properties: key → (name, raw value containing `var()`).
type Pending<'a> = HashMap<&'a str, (&'a Arc<str>, &'a Arc<[CV]>)>;

/// Computes an element's custom properties from its own declarations (in
/// cascade order, lowest priority first) applied over `base`.
///
/// `base` is normally the parent's map; it may also already contain some of
/// the element's own (lower-priority, `var()`-free) values, in which case
/// `inherited` is the parent's map, used by `inherit` / `unset` / `revert`.
/// References to undeclared names resolve against `base`. When the
/// document's budget is exhausted the declarations are ignored.
pub(crate) fn compute_custom_properties(
    base: &CustomMap,
    inherited: &CustomMap,
    declared: &[(&Arc<str>, &CustomValue)],
    subst: &mut Substitutions,
) -> CustomMap {
    let parent = base;
    if declared.is_empty() || !subst.budget.charge(declared.len()) {
        return parent.clone();
    }
    // The last declaration of each name wins.
    let mut winners: HashMap<&str, (&Arc<str>, &CustomValue)> =
        HashMap::with_capacity(declared.len());
    for (name, value) in declared {
        winners.insert(&***name, (*name, *value));
    }
    // Own values (`None` = removed), before resolving references.
    let mut own: OwnValues<'_> = HashMap::new();
    let mut pending: Pending<'_> = HashMap::new();
    for (key, (name, value)) in winners {
        match value {
            CustomValue::Wide(CssWide::Initial) => {
                own.insert(key, (name, None));
            }
            // Custom properties are inherited: inherit / unset / revert keep
            // the parent's value.
            CustomValue::Wide(_) => {
                if !Arc::ptr_eq(base, inherited) {
                    own.insert(key, (name, inherited.get(name).cloned()));
                }
            }
            CustomValue::Value {
                values,
                has_var: false,
            } => {
                own.insert(key, (name, Some(values.clone())));
            }
            CustomValue::Value {
                values,
                has_var: true,
            } => {
                pending.insert(key, (name, values));
            }
        }
    }
    if !pending.is_empty() {
        resolve_pending(&pending, &mut own, parent, subst);
    }
    // Keep only the entries that differ from the inherited values (the
    // comparison is skipped, keeping the entry, once the budget runs out).
    let entries: HashMap<Arc<str>, Option<Arc<[CV]>>> = own
        .into_values()
        .filter(|(name, value)| match (value, parent.get(name)) {
            (Some(v), Some(inherited)) => {
                !Arc::ptr_eq(v, inherited) && (!subst.budget.charge(v.len()) || **v != **inherited)
            }
            (None, None) => false,
            _ => true,
        })
        .map(|(name, value)| (name.clone(), value))
        .collect();
    if entries.is_empty() {
        return parent.clone();
    }
    CustomProps::derive(parent, entries, &subst.budget)
}

/// Computes the values of the element's custom properties that reference
/// other variables (`pending`) and adds them to `own`.
///
/// The dependency graph between pending properties (fallback references
/// included) is split into strongly connected components: properties in a
/// cycle are invalid; the others are substituted in dependency order, each
/// reference resolving to a pending property's result, an own value or an
/// inherited one. Unlike a depth-first search memoizing finished nodes,
/// this finds every cycle whatever the iteration order of `pending`.
fn resolve_pending<'a>(
    pending: &Pending<'a>,
    own: &mut OwnValues<'a>,
    parent: &CustomMap,
    subst: &mut Substitutions,
) {
    // (key, name, raw value) of each pending property.
    let keys: Vec<_> = pending
        .iter()
        .map(|(key, (name, raw))| (*key, *name, *raw))
        .collect();
    let position: HashMap<&str, usize> = keys
        .iter()
        .enumerate()
        .map(|(i, (key, _, _))| (*key, i))
        .collect();
    let owners: Vec<Owner> = keys
        .iter()
        .map(|(_, _, raw)| Owner::Custom((*raw).clone()))
        .collect();
    let refs: Vec<Arc<[Arc<str>]>> = owners.iter().map(|o| subst.names(o)).collect();
    let mut self_loop = vec![false; keys.len()];
    let mut adj: Vec<Vec<usize>> = Vec::with_capacity(keys.len());
    for (i, names) in refs.iter().enumerate() {
        let mut edges = Vec::new();
        for name in names.iter() {
            if let Some(&j) = position.get(&**name) {
                if j == i {
                    if let Some(flag) = self_loop.get_mut(i) {
                        *flag = true;
                    }
                }
                edges.push(j);
            }
        }
        adj.push(edges);
    }
    let mut done: Vec<Option<Arc<[CV]>>> = vec![None; keys.len()];
    for component in graph::components(&adj) {
        let cyclic = component.len() > 1
            || component
                .first()
                .is_some_and(|&i| self_loop.get(i) == Some(&true));
        if cyclic {
            continue; // every property of a cycle stays invalid (`None`)
        }
        for i in component {
            let (Some(owner), Some(names)) = (owners.get(i), refs.get(i)) else {
                continue;
            };
            let deps: Vec<Option<Arc<[CV]>>> = names
                .iter()
                .map(|name| match position.get(&**name) {
                    // Dependencies come first: already computed (or invalid).
                    Some(&j) => done.get(j).cloned().flatten(),
                    None => match own.get(&**name) {
                        Some((_, value)) => value.clone(),
                        None => parent.get(name).cloned(),
                    },
                })
                .collect();
            let value = subst.substitute(owner, names, deps);
            if let Some(slot) = done.get_mut(i) {
                *slot = value;
            }
        }
    }
    for ((key, name, _), value) in keys.into_iter().zip(done) {
        own.insert(key, (name, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use xbro_css::parse_component_values;
    use xbro_css::parser::to_css as to_css_list;

    fn custom(css: &str) -> CustomValue {
        let values = parse_component_values(css);
        let has_var = contains_var(&values);
        CustomValue::Value {
            values: Arc::from(trim(&values)),
            has_var,
        }
    }

    fn compute(parent: &CustomMap, decls: &[(&str, CustomValue)]) -> CustomMap {
        let names: Vec<Arc<str>> = decls.iter().map(|(n, _)| Arc::from(*n)).collect();
        let declared: Vec<(&Arc<str>, &CustomValue)> =
            names.iter().zip(decls.iter().map(|(_, v)| v)).collect();
        let mut subst = Substitutions::new(Budget::new(DOCUMENT_BUDGET));
        compute_custom_properties(parent, parent, &declared, &mut subst)
    }

    fn get(map: &CustomMap, name: &str) -> Option<String> {
        map.get(name).map(|v| to_css_list(v))
    }

    #[test]
    fn plain_and_inherited() {
        let root = compute(
            &CustomMap::default(),
            &[("--a", custom("1px")), ("--b", custom("red"))],
        );
        assert_eq!(get(&root, "--a").as_deref(), Some("1px"));
        // No declarations: the parent's map is shared.
        let child = compute(&root, &[]);
        assert!(Arc::ptr_eq(&root, &child));
        let child = compute(&root, &[("--a", CustomValue::Wide(CssWide::Initial))]);
        assert_eq!(get(&child, "--a"), None);
        assert_eq!(get(&child, "--b").as_deref(), Some("red"));
        let child = compute(&root, &[("--b", CustomValue::Wide(CssWide::Inherit))]);
        assert_eq!(get(&child, "--b").as_deref(), Some("red"));
    }

    #[test]
    fn references_and_fallbacks() {
        let root = compute(&CustomMap::default(), &[("--base", custom("4px"))]);
        let m = compute(
            &root,
            &[
                ("--x", custom("calc(var(--base) * 2)")),
                ("--y", custom("var(--missing, var(--x))")),
                ("--z", custom("var(--missing)")),
                ("--base", custom("8px")),
            ],
        );
        // `--base` on the same element wins over the inherited value.
        assert_eq!(get(&m, "--x").as_deref(), Some("calc(8px * 2)"));
        assert_eq!(get(&m, "--y").as_deref(), Some("calc(8px * 2)"));
        assert_eq!(get(&m, "--z"), None);
    }

    #[test]
    fn cycles_are_invalid() {
        let m = compute(
            &CustomMap::default(),
            &[
                ("--a", custom("var(--b)")),
                ("--b", custom("var(--a, 5px)")),
                ("--c", custom("var(--a, 7px)")),
                ("--self", custom("var(--self)")),
            ],
        );
        assert_eq!(get(&m, "--a"), None);
        assert_eq!(get(&m, "--b"), None);
        assert_eq!(get(&m, "--self"), None);
        assert_eq!(get(&m, "--c").as_deref(), Some("7px"));
    }

    #[test]
    fn exponential_growth_is_capped() {
        let mut decls = vec![("--v0".to_string(), custom("x x x x x x x x x x"))];
        for i in 1..12 {
            let prev = format!("var(--v{})", i - 1);
            let value = [prev.as_str(); 10].join(" ");
            decls.push((format!("--v{i}"), custom(&value)));
        }
        let decls: Vec<(&str, CustomValue)> =
            decls.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
        let m = compute(&CustomMap::default(), &decls);
        assert!(m.get("--v3").is_some());
        assert!(m.get("--v11").is_none());
    }

    #[test]
    fn unchanged_redeclarations_share_the_parent_map() {
        let decl = custom("0px");
        let root = compute(&CustomMap::default(), &[("--tw", decl.clone())]);
        // Same value object (same rule) re-declared on a child: shared.
        let child = compute(&root, &[("--tw", decl.clone())]);
        assert!(Arc::ptr_eq(&root, &child));
        // Equal value from another rule: shared too.
        let child = compute(&root, &[("--tw", custom("0px"))]);
        assert!(Arc::ptr_eq(&root, &child));
        let child = compute(&root, &[("--tw", custom("1px"))]);
        assert!(!Arc::ptr_eq(&root, &child));
    }

    #[test]
    fn long_chains_are_flattened_and_correct() {
        let mut map = compute(&CustomMap::default(), &[("--base", custom("0"))]);
        for i in 0..100 {
            map = compute(
                &map,
                &[
                    (format!("--v{i}").leak() as &str, custom(&format!("{i}px"))),
                    ("--last", custom(&format!("{i}"))),
                    (
                        "--gone",
                        if i % 2 == 0 {
                            custom("x")
                        } else {
                            CustomValue::Wide(CssWide::Initial)
                        },
                    ),
                ],
            );
            assert!(map.depth <= MAX_CHAIN_DEPTH + 1);
        }
        assert_eq!(get(&map, "--v0").as_deref(), Some("0px"));
        assert_eq!(get(&map, "--v99").as_deref(), Some("99px"));
        assert_eq!(get(&map, "--last").as_deref(), Some("99"));
        assert_eq!(get(&map, "--base").as_deref(), Some("0"));
        assert_eq!(get(&map, "--gone"), None);
        // Many siblings below a long chain share one flattened parent.
        let siblings: Vec<CustomMap> = (0..1000)
            .map(|i| compute(&map, &[("--s", custom(&format!("{i}")))]))
            .collect();
        let parents: HashSet<*const CustomProps> = siblings
            .iter()
            .filter_map(|m| m.parent.as_ref().map(Arc::as_ptr))
            .collect();
        assert_eq!(parents.len(), 1);
        assert_eq!(get(&siblings[999], "--v50").as_deref(), Some("50px"));
    }

    #[test]
    fn substitution() {
        let mut lookup = |n: &str| -> Option<Arc<[CV]>> {
            (n == "--w").then(|| Arc::from(parse_component_values("10px")))
        };
        let budget = Budget::new(DOCUMENT_BUDGET);
        let out = substitute(
            &parse_component_values("var(--w) solid var(--c, red)"),
            &mut lookup,
            &budget,
        );
        assert_eq!(
            out.map(|v| to_css_list(&v)).as_deref(),
            Some("10px solid red")
        );
        assert!(substitute(&parse_component_values("var(--nope)"), &mut lookup, &budget).is_none());
        assert!(substitute(&parse_component_values("var(nope)"), &mut lookup, &budget).is_none());
        let out = substitute(
            &parse_component_values("env(safe-area-inset-top)"),
            &mut lookup,
            &budget,
        );
        assert_eq!(out.map(|v| to_css_list(&v)).as_deref(), Some("0px"));
        let out = substitute(
            &parse_component_values("env(foo, 3px)"),
            &mut lookup,
            &budget,
        );
        assert_eq!(out.map(|v| to_css_list(&v)).as_deref(), Some("3px"));
        // Empty fallback is valid.
        let out = substitute(
            &parse_component_values("a var(--nope,)"),
            &mut lookup,
            &budget,
        );
        assert_eq!(out.map(|v| to_css_list(&v)).as_deref(), Some("a "));
    }

    #[test]
    fn budget_bounds_substitution_work() {
        let mut lookup = |n: &str| -> Option<Arc<[CV]>> {
            (n == "--w").then(|| Arc::from(parse_component_values("a b c d e")))
        };
        let values = parse_component_values("var(--w) var(--w)");
        // Each substitution produces 19 tokens (whitespace included).
        let budget = Budget::new(25);
        let mut ok = 0;
        while substitute(&values, &mut lookup, &budget).is_some() {
            ok += 1;
            assert!(ok < 100, "budget never ran out");
        }
        assert_eq!(ok, 1);
        // What is left cannot pay for another substitution, which keeps
        // failing; empty values still substitute (no work).
        assert!(budget.remaining() < 19);
        assert!(substitute(&values, &mut lookup, &budget).is_none());
        assert!(substitute(&[], &mut lookup, &budget).is_some());
        assert!(budget.charge(budget.remaining()));
        assert_eq!(budget.remaining(), 0);
        assert!(!budget.charge(1));
        // Custom declarations are ignored once the budget is gone.
        let root = CustomMap::default();
        let name: Arc<str> = Arc::from("--a");
        let value = custom("1px");
        let mut subst = Substitutions::new(Budget::new(0));
        let m = compute_custom_properties(&root, &root, &[(&name, &value)], &mut subst);
        assert!(Arc::ptr_eq(&m, &root));
    }

    #[test]
    fn large_maps_are_chained_not_copied() {
        // A root with many variables, then a deep chain of small changes:
        // flattening would copy the whole map at every few levels.
        let decls: Vec<(String, CustomValue)> = (0..5_000)
            .map(|i| (format!("--r{i}"), custom(&format!("{i}"))))
            .collect();
        let decls: Vec<(&str, CustomValue)> =
            decls.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
        let mut map = compute(&CustomMap::default(), &decls);
        for i in 0..200 {
            map = compute(&map, &[("--level", custom(&format!("{i}")))]);
        }
        // Never flattened (a flattened base would reset the depth).
        assert!(map.depth >= 200, "{}", map.depth);
        assert_eq!(get(&map, "--level").as_deref(), Some("199"));
        assert_eq!(get(&map, "--r4321").as_deref(), Some("4321"));
        assert_eq!(get(&map, "--missing"), None);
    }

    #[test]
    fn dropping_long_chains_does_not_recurse() {
        // Built directly (bypassing flattening) to get a very long chain.
        let mut map = CustomMap::default();
        for i in 0..200_000usize {
            let mut local = HashMap::new();
            local.insert(
                Arc::<str>::from("--x"),
                Some(Arc::from(parse_component_values(&i.to_string()))),
            );
            map = Arc::new(CustomProps {
                local,
                parent: Some(map),
                depth: u8::MAX,
                total: usize::MAX,
                flat: OnceLock::new(),
            });
        }
        assert_eq!(get(&map, "--x").as_deref(), Some("199999"));
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || drop(map))
            .expect("thread");
        handle
            .join()
            .expect("dropped without overflowing the stack");
    }
}
