//! Property value parsing and computation (longhands, shorthands, `calc()`, `var()`).
//!
//! Declarations are parsed once per rule into typed per-longhand
//! declarations, shorthands being expanded at parse time (logical
//! properties are mapped to physical sides for horizontal left-to-right
//! text). Values containing `var()` are kept unparsed and re-parsed per
//! element after substitution. Specified values are turned into computed
//! values per element by the (private) `compute` module.

mod background;
mod border;
mod box_model;
mod content;
pub(crate) mod declaration;
mod flex;
mod font;
pub(crate) mod ids;
pub(crate) mod specified;
pub(crate) mod table;
mod text;

use xbro_css::ComponentValue as CV;

pub(crate) use declaration::{
    parse_value, CssWide, CustomValue, DeclBlock, Decls, PropertyDeclaration, UnparsedValue,
};
pub(crate) use ids::{LonghandId, ALL_LONGHANDS, LONGHAND_COUNT};
pub(crate) use specified::*;

type Out = Vec<PropertyDeclaration>;

fn push(out: &mut Out, id: LonghandId, spec: Spec) {
    out.push(PropertyDeclaration::Value(id, spec));
}

/// Distributes 1–4 values over `ids` with the usual side expansion:
/// 4 ids (top, right, bottom, left) take 1–4 values, 2 ids (start, end)
/// take 1–2 values, 1 id takes exactly one value.
fn expand(ids: &[LonghandId], values: Vec<Spec>, out: &mut Out) -> Option<()> {
    let n = values.len();
    let pick: &[usize] = match (ids.len(), n) {
        (1, 1) => &[0],
        (2, 1) => &[0, 0],
        (2, 2) => &[0, 1],
        (4, 1) => &[0, 0, 0, 0],
        (4, 2) => &[0, 1, 0, 1],
        (4, 3) => &[0, 1, 2, 1],
        (4, 4) => &[0, 1, 2, 3],
        _ => return None,
    };
    for (id, &i) in ids.iter().zip(pick) {
        push(out, *id, values.get(i)?.clone());
    }
    Some(())
}

/// Parses each item with `f` and expands the results over `ids`.
fn per_item(
    items: &[&CV],
    ids: &[LonghandId],
    out: &mut Out,
    f: impl Fn(&CV) -> Option<Spec>,
) -> Option<()> {
    let values = items.iter().map(|v| f(v)).collect::<Option<Vec<_>>>()?;
    expand(ids, values, out)
}

/// The single longhand of a longhand property.
fn only(ids: &[LonghandId]) -> Option<LonghandId> {
    match ids {
        [id] => Some(*id),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    //! Helpers to parse a declaration in tests.
    use super::*;

    /// Parses `name: value` and returns the produced declarations (None if invalid).
    pub fn parse(name: &str, value: &str) -> Option<Vec<PropertyDeclaration>> {
        let mut out = Vec::new();
        let values = xbro_css::parse_component_values(value);
        declaration::parse_declaration(name, &values, false, &mut out).then_some(out)
    }

    /// The specified value for `id` produced by `name: value`.
    pub fn spec(name: &str, value: &str, id: LonghandId) -> Option<Spec> {
        parse(name, value)?.into_iter().rev().find_map(|d| match d {
            PropertyDeclaration::Value(i, s) if i == id => Some(s),
            _ => None,
        })
    }
}
