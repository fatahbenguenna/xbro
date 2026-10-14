//! Memoized `var()` substitution.
//!
//! The result of substituting a value depends only on the value itself and
//! on the values its `var()` references resolve to. Both are identified by
//! address (values are shared `Arc`s), so substituting the same declaration
//! for many elements whose referenced variables are the same objects is done
//! once, and the result is itself a shared `Arc` — which keeps the addresses
//! of derived values stable too (canonical results), so chains of variables
//! are memoized as well. Entries keep the `Arc`s of their keys alive.

use std::collections::HashMap;
use std::sync::Arc;

use xbro_css::values::trim;
use xbro_css::ComponentValue as CV;

use super::{substitute, Budget};
use crate::properties::UnparsedValue;

/// Entries kept per table before it is cleared.
const MAX_ENTRIES: usize = 4096;

/// Lookups among this many referenced names or more use a hash map.
const HASHED_LOOKUP: usize = 16;

/// The object owning a raw value (kept alive by entries keyed by its address).
#[derive(Clone)]
pub(crate) enum Owner {
    /// A custom property value.
    Custom(Arc<[CV]>),
    /// A longhand declaration containing `var()`.
    Unparsed(Arc<UnparsedValue>),
}

impl Owner {
    fn values(&self) -> &[CV] {
        match self {
            Owner::Custom(v) => v,
            Owner::Unparsed(u) => &u.values,
        }
    }

    fn addr(&self) -> usize {
        match self {
            Owner::Custom(v) => Arc::as_ptr(v) as *const CV as usize,
            Owner::Unparsed(u) => Arc::as_ptr(u) as usize,
        }
    }
}

/// Address identifying a looked-up value (0 for an undefined variable).
pub(crate) fn value_addr(v: &Option<Arc<[CV]>>) -> usize {
    v.as_ref()
        .map_or(0, |a| Arc::as_ptr(a) as *const CV as usize)
}

struct NamesEntry {
    _owner: Owner,
    names: Arc<[Arc<str>]>,
}

struct ResultEntry {
    _owner: Owner,
    _deps: Vec<Option<Arc<[CV]>>>,
    result: Option<Arc<[CV]>>,
}

/// Memo tables for substitutions, with the document's work budget.
pub(crate) struct Substitutions {
    pub budget: Budget,
    names: HashMap<usize, NamesEntry>,
    results: HashMap<(usize, Vec<usize>), ResultEntry>,
}

/// Collects the names referenced by `var()` in `values` (fallbacks and
/// nested functions included), without duplicates, in first-use order.
fn collect_names(values: &[CV], out: &mut Vec<Arc<str>>, depth: usize) {
    if depth > 64 {
        return;
    }
    for v in values {
        match v {
            CV::Function { name, args } => {
                if name.eq_ignore_ascii_case("var") {
                    let referenced = trim(args)
                        .first()
                        .and_then(|a| a.ident())
                        .filter(|n| n.starts_with("--"));
                    if let Some(n) = referenced {
                        if !out.iter().any(|x| &**x == n) {
                            out.push(Arc::from(n));
                        }
                    }
                }
                collect_names(args, out, depth + 1);
            }
            CV::Block { content, .. } => collect_names(content, out, depth + 1),
            CV::Token(_) => {}
        }
    }
}

/// Substitutes `values` whose referenced `names` resolve to `deps`.
pub(crate) fn substitute_deps(
    values: &[CV],
    names: &[Arc<str>],
    deps: &[Option<Arc<[CV]>>],
    budget: &Budget,
) -> Option<Vec<CV>> {
    let index: Option<HashMap<&str, usize>> = (names.len() >= HASHED_LOOKUP)
        .then(|| names.iter().enumerate().map(|(i, n)| (&**n, i)).collect());
    let mut lookup = |name: &str| -> Option<Arc<[CV]>> {
        let i = match &index {
            Some(index) => index.get(name).copied(),
            None => names.iter().position(|n| &**n == name),
        }?;
        deps.get(i).cloned().flatten()
    };
    substitute(values, &mut lookup, budget)
}

impl Substitutions {
    pub fn new(budget: Budget) -> Self {
        Substitutions {
            budget,
            names: HashMap::new(),
            results: HashMap::new(),
        }
    }

    /// The variable names `owner`'s value references (memoized).
    pub fn names(&mut self, owner: &Owner) -> Arc<[Arc<str>]> {
        let addr = owner.addr();
        if let Some(entry) = self.names.get(&addr) {
            return entry.names.clone();
        }
        let mut names = Vec::new();
        collect_names(owner.values(), &mut names, 0);
        let names: Arc<[Arc<str>]> = Arc::from(names);
        // Walking the value is charged like copying it.
        self.budget.consume(owner.values().len());
        if self.names.len() >= MAX_ENTRIES {
            self.names.clear();
        }
        self.names.insert(
            addr,
            NamesEntry {
                _owner: owner.clone(),
                names: names.clone(),
            },
        );
        names
    }

    /// Substitutes `owner`'s value given the values `deps` of its referenced
    /// `names` (as returned by [`Substitutions::names`], same order). The
    /// result is trimmed; `None` when invalid at computed-value time.
    pub fn substitute(
        &mut self,
        owner: &Owner,
        names: &[Arc<str>],
        deps: Vec<Option<Arc<[CV]>>>,
    ) -> Option<Arc<[CV]>> {
        let key = (
            owner.addr(),
            deps.iter().map(value_addr).collect::<Vec<_>>(),
        );
        if let Some(entry) = self.results.get(&key) {
            return entry.result.clone();
        }
        let result = substitute_deps(owner.values(), names, &deps, &self.budget)
            .map(|v| Arc::from(trim(&v)));
        if self.results.len() >= MAX_ENTRIES {
            self.results.clear();
        }
        self.results.insert(
            key,
            ResultEntry {
                _owner: owner.clone(),
                _deps: deps,
                result: result.clone(),
            },
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vars::DOCUMENT_BUDGET;
    use xbro_css::parse_component_values;
    use xbro_css::parser::to_css;

    fn owner(css: &str) -> Owner {
        Owner::Custom(Arc::from(parse_component_values(css)))
    }

    #[test]
    fn referenced_names() {
        let mut s = Substitutions::new(Budget::new(DOCUMENT_BUDGET));
        let o = owner("calc(var(--a) + var(--b, var(--c, var(--a)))) [var(--d)] var(nope)");
        let names: Vec<String> = s.names(&o).iter().map(|n| n.to_string()).collect();
        assert_eq!(names, ["--a", "--b", "--c", "--d"]);
    }

    #[test]
    fn results_are_shared_for_identical_inputs() {
        let mut s = Substitutions::new(Budget::new(DOCUMENT_BUDGET));
        let o = owner("var(--w) solid var(--c, red)");
        let names = s.names(&o);
        let w: Option<Arc<[CV]>> = Some(Arc::from(parse_component_values("2px")));
        let first = s.substitute(&o, &names, vec![w.clone(), None]);
        assert_eq!(
            first.as_deref().map(to_css).as_deref(),
            Some("2px solid red")
        );
        let again = s.substitute(&o, &names, vec![w.clone(), None]);
        assert!(Arc::ptr_eq(
            first.as_ref().unwrap(),
            again.as_ref().unwrap()
        ));
        // An equal value in another allocation is another key, same result.
        let w2: Option<Arc<[CV]>> = Some(Arc::from(parse_component_values("2px")));
        let other = s.substitute(&o, &names, vec![w2, None]);
        assert_eq!(other.as_deref().map(to_css), first.as_deref().map(to_css));
        // Missing reference without fallback: invalid.
        assert!(s.substitute(&o, &names, vec![None, None]).is_none());
    }

    #[test]
    fn many_referenced_names_use_hashed_lookups() {
        let mut s = Substitutions::new(Budget::new(DOCUMENT_BUDGET));
        let css: Vec<String> = (0..100).map(|i| format!("var(--v{i})")).collect();
        let o = owner(&css.join(" "));
        let names = s.names(&o);
        assert_eq!(names.len(), 100);
        let deps = (0..100)
            .map(|i| Some(Arc::from(parse_component_values(&format!("x{i}")))))
            .collect();
        let out = s.substitute(&o, &names, deps).expect("valid");
        assert!(to_css(&out).ends_with("x98 x99"));
    }
}
