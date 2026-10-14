//! Memoization of custom-property computations across the elements of one
//! document.
//!
//! Custom properties and `var()` substitution are pure functions of their
//! inputs, which are identified by pointer:
//!
//! * an element's custom properties depend only on the parent's map and on
//!   the custom declaration lists of its matched blocks. Those lists live in
//!   rule sets / cached `style` attribute blocks that outlive the traversal,
//!   so their addresses identify them. Siblings and descendants matching the
//!   same rules (`* { --x: ... }`, utility classes) share one computation.
//! * a declaration containing `var()` parses to the same result for every
//!   element whose referenced variables have the same values (the same
//!   shared objects, see the `memo` module), whatever the rest of its map;
//! * a large `var()`-free declaration list (`* { --a: 1; ... }` with
//!   thousands of names) is applied incrementally: applying it to a map
//!   derived from an already-processed map only costs the size of the
//!   difference between the two (see [`VarContext::apply_plain`]), so
//!   elements with distinct maps do not each pay for the whole list.
//!
//! Each entry keeps the `Arc`s whose addresses form its key alive, so an
//! address can never be reused by another map while its entry exists.
//! Caches are cleared when they grow past [`MAX_ENTRIES`].

use std::collections::HashMap;
use std::sync::Arc;

use xbro_css::ComponentValue as CV;

use super::memo::{substitute_deps, value_addr, Owner};
use super::{
    compute_custom_properties, Budget, CustomMap, CustomProps, Substitutions, DOCUMENT_BUDGET,
};
use crate::properties::{parse_value, CssWide, CustomValue, PropertyDeclaration, UnparsedValue};

/// Entries kept per cache before it is cleared.
const MAX_ENTRIES: usize = 4096;

/// Declaration lists at least this long are applied incrementally.
const INCREMENTAL_MIN: usize = 16;

/// Address of the parent map + (address, length) of each non-empty custom
/// declaration list, in cascade order.
type CustomKey = (usize, Vec<(usize, usize)>);

/// (address, length) of a declaration list.
type ListKey = (usize, usize);

/// A custom property value: `None` is the guaranteed-invalid value.
type Value = Option<Arc<[CV]>>;

/// What a declaration list sets, for incremental application.
struct ListInfo {
    /// Value set for each name (`None` for `initial`).
    values: HashMap<Arc<str>, Value>,
    /// The list has `var()` values or `inherit` / `unset` / `revert`
    /// keywords, whose results depend on more than the list itself.
    complex: bool,
}

impl ListInfo {
    fn new(list: &[PropertyDeclaration]) -> ListInfo {
        let mut info = ListInfo {
            values: HashMap::new(),
            complex: false,
        };
        for decl in list {
            let PropertyDeclaration::Custom(name, value) = decl else {
                continue;
            };
            let v = match value {
                CustomValue::Value {
                    values,
                    has_var: false,
                } => Some(values.clone()),
                CustomValue::Wide(CssWide::Initial) => None,
                CustomValue::Value { has_var: true, .. } | CustomValue::Wide(_) => {
                    info.complex = true;
                    return info;
                }
            };
            info.values.insert(name.clone(), v);
        }
        info
    }
}

struct ApplyEntry {
    /// Keeps the input map (whose address is in the key) alive.
    _input: CustomMap,
    result: CustomMap,
}

/// Equality of two custom property values, charging deep comparisons.
fn same_value(a: &Value, b: &Value, budget: &Budget) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            Arc::ptr_eq(x, y) || (budget.charge(x.len()) && x.len() == y.len() && **x == **y)
        }
        _ => false,
    }
}

fn map_key(m: &CustomMap) -> usize {
    Arc::as_ptr(m) as usize
}

struct CustomEntry {
    /// Keeps the parent map (whose address is in the key) alive.
    _parent: CustomMap,
    result: CustomMap,
}

struct UnparsedEntry {
    /// Keep the declaration and the variable values whose addresses form
    /// the key alive.
    _value: Arc<UnparsedValue>,
    _deps: Vec<Value>,
    parsed: Option<Arc<[PropertyDeclaration]>>,
}

/// Per-document state for custom properties: memoized results and the
/// work [`Budget`].
pub(crate) struct VarContext {
    /// Memoized substitutions and the work budget.
    subst: Substitutions,
    customs: HashMap<CustomKey, CustomEntry>,
    /// Keyed by the declaration's address and the addresses of the values
    /// of the variables it references.
    unparsed: HashMap<(usize, Vec<usize>), UnparsedEntry>,
    /// Analysis of the large declaration lists seen so far.
    lists: HashMap<ListKey, Arc<ListInfo>>,
    /// Result of applying a plain list to a map.
    applied: HashMap<(usize, ListKey), ApplyEntry>,
}

impl Default for VarContext {
    fn default() -> Self {
        Self::with_budget(DOCUMENT_BUDGET)
    }
}

impl VarContext {
    /// The remaining work budget.
    #[cfg(test)]
    pub fn budget(&self) -> &Budget {
        &self.subst.budget
    }

    pub fn with_budget(units: usize) -> Self {
        VarContext {
            subst: Substitutions::new(Budget::new(units)),
            customs: HashMap::new(),
            unparsed: HashMap::new(),
            lists: HashMap::new(),
            applied: HashMap::new(),
        }
    }

    /// Computed custom properties of an element inheriting `parent` whose
    /// matched blocks declare `lists` (in cascade order; each list holds
    /// only [`PropertyDeclaration::Custom`] values and must stay alive, at
    /// the same address, for the lifetime of this context).
    pub fn custom_properties(
        &mut self,
        parent: &CustomMap,
        lists: &[&[PropertyDeclaration]],
    ) -> CustomMap {
        let lists: Vec<&[PropertyDeclaration]> =
            lists.iter().copied().filter(|l| !l.is_empty()).collect();
        if lists.is_empty() {
            return parent.clone();
        }
        let key = (
            map_key(parent),
            lists
                .iter()
                .map(|l| (l.as_ptr() as usize, l.len()))
                .collect::<Vec<_>>(),
        );
        if let Some(entry) = self.customs.get(&key) {
            return entry.result.clone();
        }
        if self.customs.len() >= MAX_ENTRIES {
            self.customs.clear();
        }
        if self.applied.len() >= MAX_ENTRIES {
            self.applied.clear();
        }
        // Large plain lists at the front (typically `* { ... }`) are applied
        // incrementally; the remaining lists are computed over the result.
        let mut base = parent.clone();
        let mut start = 0;
        for list in &lists {
            if list.len() < INCREMENTAL_MIN {
                break;
            }
            let info = self.list_info(list);
            if info.complex {
                break;
            }
            base = self.apply_plain(&base, (list.as_ptr() as usize, list.len()), &info);
            start += 1;
        }
        let rest = lists.get(start..).unwrap_or(&[]);
        let result = if rest.is_empty() {
            base
        } else {
            let declared: Vec<(&Arc<str>, &CustomValue)> = rest
                .iter()
                .flat_map(|l| l.iter())
                .filter_map(|d| match d {
                    PropertyDeclaration::Custom(name, value) => Some((name, value)),
                    _ => None,
                })
                .collect();
            compute_custom_properties(&base, parent, &declared, &mut self.subst)
        };
        self.customs.insert(
            key,
            CustomEntry {
                _parent: parent.clone(),
                result: result.clone(),
            },
        );
        result
    }

    fn list_info(&mut self, list: &[PropertyDeclaration]) -> Arc<ListInfo> {
        let key = (list.as_ptr() as usize, list.len());
        if let Some(info) = self.lists.get(&key) {
            return info.clone();
        }
        if self.lists.len() >= MAX_ENTRIES {
            self.lists.clear();
        }
        let info = Arc::new(ListInfo::new(list));
        self.lists.insert(key, info.clone());
        info
    }

    /// `map` with the plain list `info` applied over it (memoized).
    ///
    /// For a map `m` with parent `q` (so `m = q + m.local`), knowing
    /// `r = apply(q)`:
    /// * if `r` is `q` (the list changes nothing in `q`), `apply(m)` is `m`
    ///   with the names of `m.local` that the list sets restored;
    /// * otherwise `apply(m) = r + (m.local without the names the list sets)`.
    ///
    /// Either way only `m.local` is examined; the whole list is only
    /// scanned for maps without parent. The chain is processed iteratively
    /// from the nearest memoized ancestor down.
    fn apply_plain(&mut self, map: &CustomMap, list: ListKey, info: &ListInfo) -> CustomMap {
        let mut chain: Vec<CustomMap> = Vec::new();
        let mut current = map.clone();
        let mut above: Option<(CustomMap, CustomMap)> = None;
        loop {
            if let Some(entry) = self.applied.get(&(map_key(&current), list)) {
                above = Some((current.clone(), entry.result.clone()));
                break;
            }
            let parent = current.parent.clone();
            chain.push(current);
            match parent {
                Some(p) => current = p,
                None => break,
            }
        }
        if chain.is_empty() {
            if let Some((_, result)) = above {
                return result;
            }
        }
        for m in chain.iter().rev() {
            let result = match &above {
                Some((q, r)) if m.parent.as_ref().is_some_and(|p| Arc::ptr_eq(p, q)) => {
                    self.apply_step(m, q, r, info)
                }
                _ => self.apply_full(m, info),
            };
            self.applied.insert(
                (map_key(m), list),
                ApplyEntry {
                    _input: m.clone(),
                    result: result.clone(),
                },
            );
            above = Some((m.clone(), result));
        }
        above.map_or_else(|| map.clone(), |(_, result)| result)
    }

    /// `apply(m)` from `r = apply(q)` where `q` is the parent of `m`.
    fn apply_step(
        &self,
        m: &CustomMap,
        q: &CustomMap,
        r: &CustomMap,
        info: &ListInfo,
    ) -> CustomMap {
        if !self.subst.budget.charge(m.local.len()) {
            return m.clone();
        }
        let mut delta: HashMap<Arc<str>, Value> = HashMap::new();
        if Arc::ptr_eq(q, r) {
            for (name, value) in &m.local {
                if let Some(set) = info.values.get(name) {
                    if !same_value(value, set, &self.subst.budget) {
                        delta.insert(name.clone(), set.clone());
                    }
                }
            }
            if delta.is_empty() {
                return m.clone();
            }
            CustomProps::derive(m, delta, &self.subst.budget)
        } else {
            for (name, value) in &m.local {
                if !info.values.contains_key(name) {
                    delta.insert(name.clone(), value.clone());
                }
            }
            if delta.is_empty() {
                return r.clone();
            }
            CustomProps::derive(r, delta, &self.subst.budget)
        }
    }

    /// `apply(m)` by comparing every value of the list.
    fn apply_full(&self, m: &CustomMap, info: &ListInfo) -> CustomMap {
        if !self.subst.budget.charge(info.values.len()) {
            return m.clone();
        }
        let delta: HashMap<Arc<str>, Value> = info
            .values
            .iter()
            .filter(|(name, set)| !same_value(&m.get(name).cloned(), set, &self.subst.budget))
            .map(|(name, set)| (name.clone(), set.clone()))
            .collect();
        if delta.is_empty() {
            return m.clone();
        }
        CustomProps::derive(m, delta, &self.subst.budget)
    }

    /// The declarations `unparsed` expands to once its `var()`s are
    /// substituted with `customs`; `None` when invalid at computed-value time.
    pub fn parse_unparsed(
        &mut self,
        unparsed: &Arc<UnparsedValue>,
        customs: &CustomMap,
    ) -> Option<Arc<[PropertyDeclaration]>> {
        let owner = Owner::Unparsed(unparsed.clone());
        let names = self.subst.names(&owner);
        let deps: Vec<Value> = names.iter().map(|n| customs.get(n).cloned()).collect();
        let key = (
            Arc::as_ptr(unparsed) as usize,
            deps.iter().map(value_addr).collect::<Vec<_>>(),
        );
        if let Some(entry) = self.unparsed.get(&key) {
            return entry.parsed.clone();
        }
        let parsed = substitute_deps(&unparsed.values, &names, &deps, &self.subst.budget).and_then(
            |values| {
                let mut out = Vec::new();
                parse_value(unparsed.def, &values, unparsed.quirks, &mut out)
                    .then(|| Arc::<[PropertyDeclaration]>::from(out))
            },
        );
        if self.unparsed.len() >= MAX_ENTRIES {
            self.unparsed.clear();
        }
        self.unparsed.insert(
            key,
            UnparsedEntry {
                _value: unparsed.clone(),
                _deps: deps,
                parsed: parsed.clone(),
            },
        );
        parsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::properties::DeclBlock;

    fn block(css: &str) -> DeclBlock {
        DeclBlock::parse(&xbro_css::parse_declaration_list(css), false)
    }

    #[test]
    fn identical_inputs_share_results() {
        let mut cx = VarContext::default();
        let root = CustomMap::default();
        let a = block("--x: 1px; --y: var(--x) 2px");
        let first = cx.custom_properties(&root, &[&a.normal.customs]);
        let again = cx.custom_properties(&root, &[&a.normal.customs]);
        assert!(Arc::ptr_eq(&first, &again));
        // Same rule on a child whose parent map is the result: values are
        // unchanged, so the child shares its parent's map.
        let child = cx.custom_properties(&first, &[&a.normal.customs]);
        assert!(Arc::ptr_eq(&child, &first));
        // A different block (same text) is a different key but the same values.
        let b = block("--x: 1px; --y: var(--x) 2px");
        let other = cx.custom_properties(&root, &[&b.normal.customs]);
        assert_eq!(
            other.get("--y").map(|v| xbro_css::parser::to_css(v)),
            first.get("--y").map(|v| xbro_css::parser::to_css(v))
        );
        // No custom declarations at all: the parent's map, no entry.
        let none: &[PropertyDeclaration] = &[];
        assert!(Arc::ptr_eq(&cx.custom_properties(&root, &[none]), &root));
    }

    #[test]
    fn unparsed_values_are_parsed_once_per_map() {
        let mut cx = VarContext::default();
        let vars = block("--w: 7px");
        let map = cx.custom_properties(&CustomMap::default(), &[&vars.normal.customs]);
        let decl = block("margin: var(--w) 1px");
        let Some(PropertyDeclaration::Unparsed(_, unparsed)) = decl.normal.longhands.first() else {
            panic!("expected an unparsed declaration");
        };
        let first = cx.parse_unparsed(unparsed, &map).expect("valid");
        let again = cx.parse_unparsed(unparsed, &map).expect("valid");
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(first.len(), 4);
        // Without the variable it is invalid at computed-value time.
        assert!(cx.parse_unparsed(unparsed, &CustomMap::default()).is_none());
    }

    #[test]
    fn caches_are_bounded() {
        let mut cx = VarContext::default();
        let a = block("--x: 1");
        // Blocks must outlive the context (their addresses are keys).
        let blocks: Vec<DeclBlock> = (0..MAX_ENTRIES * 2)
            .map(|i| block(&format!("--y: {i}")))
            .collect();
        let mut parent = CustomMap::default();
        for b in &blocks {
            parent = cx.custom_properties(&parent, &[&a.normal.customs, &b.normal.customs]);
            assert!(cx.customs.len() <= MAX_ENTRIES);
        }
        assert!(parent.get("--x").is_some());
        let last = parent.get("--y").map(|v| xbro_css::parser::to_css(v));
        assert_eq!(last, Some(format!("{}", MAX_ENTRIES * 2 - 1)));
    }

    /// Deterministic xorshift generator.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n.max(1)
        }
    }

    fn random_block(rng: &mut Rng, large: bool) -> DeclBlock {
        let mut css = String::new();
        let count = if large {
            INCREMENTAL_MIN as u64 + rng.below(20)
        } else {
            1 + rng.below(4)
        };
        for _ in 0..count {
            let name = rng.below(30);
            let value = match rng.below(if large { 8 } else { 12 }) {
                0 => "initial".to_string(),
                1..=7 => format!("v{}", rng.below(4)),
                8 => "inherit".to_string(),
                9 => format!("var(--n{}, fb)", rng.below(30)),
                10 => format!("x var(--n{})", rng.below(30)),
                _ => "unset".to_string(),
            };
            css.push_str(&format!("--n{name}: {value};"));
        }
        block(&css)
    }

    fn snapshot(map: &CustomMap) -> Vec<Option<String>> {
        (0..30)
            .map(|i| {
                map.get(&format!("--n{i}"))
                    .map(|v| xbro_css::parser::to_css(v))
            })
            .collect()
    }

    #[test]
    fn memoized_results_match_direct_computation() {
        let mut incremental_runs = 0;
        for seed in 1..=40u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let blocks: Vec<DeclBlock> = (0..12).map(|i| random_block(&mut rng, i < 4)).collect();
            let mut cx = VarContext::default();
            let mut subst = Substitutions::new(Budget::new(DOCUMENT_BUDGET));
            // (memoized map, reference map) per element; element 0 is the root.
            let mut maps: Vec<(CustomMap, CustomMap)> =
                vec![(CustomMap::default(), CustomMap::default())];
            for _ in 0..300 {
                let parent = rng.below(maps.len() as u64) as usize;
                let (memo_parent, ref_parent) = maps[parent].clone();
                let mut lists: Vec<&[PropertyDeclaration]> = Vec::new();
                for _ in 0..rng.below(4) {
                    let b = &blocks[rng.below(blocks.len() as u64) as usize];
                    lists.push(&b.normal.customs);
                }
                // Large lists first, as `*` rules usually are (both orders
                // are correct; this exercises the incremental path).
                lists.sort_by_key(|l| std::cmp::Reverse(l.len() >= INCREMENTAL_MIN));
                let memo = cx.custom_properties(&memo_parent, &lists);
                let declared: Vec<(&Arc<str>, &CustomValue)> = lists
                    .iter()
                    .flat_map(|l| l.iter())
                    .filter_map(|d| match d {
                        PropertyDeclaration::Custom(n, v) => Some((n, v)),
                        _ => None,
                    })
                    .collect();
                let reference =
                    compute_custom_properties(&ref_parent, &ref_parent, &declared, &mut subst);
                assert_eq!(snapshot(&memo), snapshot(&reference), "seed {seed}");
                maps.push((memo, reference));
            }
            incremental_runs += usize::from(!cx.applied.is_empty());
        }
        assert!(incremental_runs > 20, "{incremental_runs}");
    }

    #[test]
    fn large_plain_lists_cost_only_the_differences() {
        // `* { 20k variables }` on elements whose maps all differ (each
        // declares its own variable): every element used to re-examine the
        // whole list.
        let mut css = String::new();
        for i in 0..20_000 {
            css.push_str(&format!("--a{i}: {i};"));
        }
        let universal = block(&css);
        let own: Vec<DeclBlock> = (0..3_000)
            .map(|i| block(&format!("--w: {i}px; --a5: x{i}")))
            .collect();
        let mut cx = VarContext::default();
        let root = cx.custom_properties(&CustomMap::default(), &[&universal.normal.customs]);
        let before = cx.budget().remaining();
        let start = std::time::Instant::now();
        for (i, b) in own.iter().enumerate() {
            let element =
                cx.custom_properties(&root, &[&universal.normal.customs, &b.normal.customs]);
            // Its child only matches the universal rule: `--a5` is reset by it.
            let child = cx.custom_properties(&element, &[&universal.normal.customs]);
            let w = child.get("--w").map(|v| xbro_css::parser::to_css(v));
            assert_eq!(w, Some(format!("{i}px")));
            let a5 = child.get("--a5").map(|v| xbro_css::parser::to_css(v));
            assert_eq!(a5.as_deref(), Some("5"));
            let a5 = element.get("--a5").map(|v| xbro_css::parser::to_css(v));
            assert_eq!(a5, Some(format!("x{i}")));
        }
        let used = before - cx.budget().remaining();
        assert!(used < 1_000_000, "used {used} budget units");
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }
}
