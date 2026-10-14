//! Parsed declarations and declaration blocks.

use std::collections::HashSet;
use std::sync::Arc;

use xbro_css::values::trim;
use xbro_css::{ComponentValue as CV, Declaration};

use super::ids::{LonghandId, LONGHAND_COUNT};
use super::specified::Spec;
use super::table::{lookup, PropertyDef};
use crate::values::{contains_var, ParseCtx};

/// CSS-wide keywords.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CssWide {
    Inherit,
    Initial,
    Unset,
    /// `revert` / `revert-layer`: in author rules, rolls back to the
    /// user-agent value; elsewhere (and without one) behaves like `unset`.
    Revert,
}

impl CssWide {
    pub fn parse(v: &CV) -> Option<CssWide> {
        let ident = v.ident()?;
        Some(match ident.to_ascii_lowercase().as_str() {
            "inherit" => CssWide::Inherit,
            "initial" => CssWide::Initial,
            "unset" => CssWide::Unset,
            "revert" | "revert-layer" => CssWide::Revert,
            _ => return None,
        })
    }
}

/// A declaration whose value contains `var()`: parsed per element after substitution.
#[derive(Debug)]
pub(crate) struct UnparsedValue {
    pub def: &'static PropertyDef,
    pub values: Vec<CV>,
    pub quirks: bool,
}

/// A custom property (`--name`) value.
#[derive(Clone, Debug)]
pub(crate) enum CustomValue {
    Wide(CssWide),
    Value { values: Arc<[CV]>, has_var: bool },
}

/// One cascaded unit: a longhand value, or a custom property.
#[derive(Clone, Debug)]
pub(crate) enum PropertyDeclaration {
    Value(LonghandId, Spec),
    Wide(LonghandId, CssWide),
    Unparsed(LonghandId, Arc<UnparsedValue>),
    Custom(Arc<str>, CustomValue),
}

impl PropertyDeclaration {
    /// The longhand this declaration sets (`None` for custom properties).
    pub fn longhand(&self) -> Option<LonghandId> {
        match self {
            PropertyDeclaration::Value(id, _)
            | PropertyDeclaration::Wide(id, _)
            | PropertyDeclaration::Unparsed(id, _) => Some(*id),
            PropertyDeclaration::Custom(..) => None,
        }
    }
}

/// The declarations of one importance level of a block.
///
/// Within a block the last declaration of a property always wins, so only
/// that one is kept: at most one declaration per longhand and one per
/// custom property name. A rule repeating a declaration thousands of times
/// therefore costs nothing per element.
#[derive(Clone, Debug, Default)]
pub(crate) struct Decls {
    /// Longhand declarations (no [`PropertyDeclaration::Custom`]).
    pub longhands: Vec<PropertyDeclaration>,
    /// Custom property declarations ([`PropertyDeclaration::Custom`] only).
    pub customs: Vec<PropertyDeclaration>,
}

impl Decls {
    pub fn is_empty(&self) -> bool {
        self.longhands.is_empty() && self.customs.is_empty()
    }

    /// Splits `parsed` (in source order) into longhands and custom
    /// properties, keeping only the last declaration of each.
    fn from_parsed(parsed: Vec<PropertyDeclaration>) -> Decls {
        let mut seen = [false; LONGHAND_COUNT];
        let mut seen_custom: HashSet<Arc<str>> = HashSet::new();
        let mut decls = Decls::default();
        for decl in parsed.into_iter().rev() {
            match &decl {
                PropertyDeclaration::Custom(name, _) => {
                    if seen_custom.insert(name.clone()) {
                        decls.customs.push(decl);
                    }
                }
                other => {
                    let Some(slot) = other.longhand().and_then(|id| seen.get_mut(id.index()))
                    else {
                        continue;
                    };
                    if !*slot {
                        *slot = true;
                        decls.longhands.push(decl);
                    }
                }
            }
        }
        decls.longhands.reverse();
        decls.customs.reverse();
        decls
    }
}

/// The declarations of one rule (or `style` attribute), split by importance.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeclBlock {
    pub normal: Decls,
    pub important: Decls,
}

impl DeclBlock {
    /// Parses raw declarations; invalid ones are dropped (so an earlier valid
    /// declaration of the same property keeps applying).
    pub fn parse(declarations: &[Declaration], quirks: bool) -> DeclBlock {
        let (mut normal, mut important) = (Vec::new(), Vec::new());
        for d in declarations {
            let out = if d.important {
                &mut important
            } else {
                &mut normal
            };
            parse_declaration(&d.name, &d.value, quirks, out);
        }
        DeclBlock {
            normal: Decls::from_parsed(normal),
            important: Decls::from_parsed(important),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.normal.is_empty() && self.important.is_empty()
    }
}

/// Parses one declaration into longhand declarations appended to `out`.
/// Returns false (leaving `out` untouched) for unknown properties and invalid values.
pub(crate) fn parse_declaration(
    name: &str,
    value: &[CV],
    quirks: bool,
    out: &mut Vec<PropertyDeclaration>,
) -> bool {
    let value = trim(value);
    if name.starts_with("--") {
        let wide = match value {
            [single] => CssWide::parse(single),
            _ => None,
        };
        let custom = match wide {
            Some(wide) => CustomValue::Wide(wide),
            None => CustomValue::Value {
                values: Arc::from(value),
                has_var: contains_var(value),
            },
        };
        out.push(PropertyDeclaration::Custom(Arc::from(name), custom));
        return true;
    }
    let Some(def) = lookup(name) else {
        return false;
    };
    if let [single] = value {
        if let Some(wide) = CssWide::parse(single) {
            out.extend(
                def.longhands
                    .iter()
                    .map(|id| PropertyDeclaration::Wide(*id, wide)),
            );
            return true;
        }
    }
    if contains_var(value) {
        let unparsed = Arc::new(UnparsedValue {
            def,
            values: value.to_vec(),
            quirks,
        });
        out.extend(
            def.longhands
                .iter()
                .map(|id| PropertyDeclaration::Unparsed(*id, unparsed.clone())),
        );
        return true;
    }
    parse_value(def, value, quirks, out)
}

/// Parses a value (without `var()`) for `def`. On failure `out` is restored.
pub(crate) fn parse_value(
    def: &'static PropertyDef,
    value: &[CV],
    quirks: bool,
    out: &mut Vec<PropertyDeclaration>,
) -> bool {
    let items: Vec<&CV> = value.iter().filter(|v| !v.is_whitespace()).collect();
    if items.is_empty() {
        return false;
    }
    if let [single] = items.as_slice() {
        if let Some(wide) = CssWide::parse(single) {
            out.extend(
                def.longhands
                    .iter()
                    .map(|id| PropertyDeclaration::Wide(*id, wide)),
            );
            return true;
        }
    }
    let cx = ParseCtx {
        unitless_quirk: quirks && def.quirk,
    };
    let start = out.len();
    if (def.parse)(&items, def.longhands, &cx, out).is_some() {
        true
    } else {
        out.truncate(start);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_css::parse_declaration_list;

    fn parse(css: &str) -> DeclBlock {
        DeclBlock::parse(&parse_declaration_list(css), false)
    }

    #[test]
    fn invalid_and_unknown_are_dropped() {
        let b = parse("width: 10px; width: bogus; frobnicate: 1; height: 5");
        assert_eq!(b.normal.longhands.len(), 1);
        assert!(matches!(
            b.normal.longhands[0],
            PropertyDeclaration::Value(LonghandId::Width, _)
        ));
    }

    #[test]
    fn importance_split() {
        let b = parse("color: red !important; margin: 0");
        assert_eq!(b.important.longhands.len(), 1);
        assert_eq!(b.normal.longhands.len(), 4);
    }

    #[test]
    fn css_wide_and_var_expand_to_longhands() {
        let b = parse("margin: inherit; padding: var(--p) 1px; --x: 1px 2px; --y: initial");
        let ids: Vec<_> = b
            .normal
            .longhands
            .iter()
            .filter_map(|d| d.longhand())
            .collect();
        assert_eq!(ids.len(), 8);
        assert!(b.normal.longhands[..4]
            .iter()
            .all(|d| matches!(d, PropertyDeclaration::Wide(_, CssWide::Inherit))));
        assert!(b.normal.longhands[4..8]
            .iter()
            .all(|d| matches!(d, PropertyDeclaration::Unparsed(..))));
        assert!(matches!(
            &b.normal.customs[0],
            PropertyDeclaration::Custom(n, CustomValue::Value { has_var: false, .. }) if &**n == "--x"
        ));
        assert!(matches!(
            &b.normal.customs[1],
            PropertyDeclaration::Custom(_, CustomValue::Wide(CssWide::Initial))
        ));
    }

    #[test]
    fn only_the_last_declaration_of_each_property_is_kept() {
        let mut css = String::new();
        for i in 0..5_000 {
            css.push_str(&format!("color: #{:06x}; --v: {i}; margin: {i}px; ", i));
        }
        css.push_str("margin-left: var(--v); --V: 1; color: red !important");
        let b = parse(&css);
        // 4 margins + color (normal); color (important).
        assert_eq!(b.normal.longhands.len(), 5);
        assert_eq!(b.important.longhands.len(), 1);
        // Custom property names are case-sensitive.
        assert_eq!(b.normal.customs.len(), 2);
        // The survivors are the last ones, in source order.
        let last_margin_left = b
            .normal
            .longhands
            .iter()
            .find(|d| d.longhand() == Some(LonghandId::MarginLeft));
        assert!(matches!(
            last_margin_left,
            Some(PropertyDeclaration::Unparsed(..))
        ));
        let v = b.normal.customs.iter().find_map(|d| match d {
            PropertyDeclaration::Custom(n, CustomValue::Value { values, .. }) if &**n == "--v" => {
                Some(xbro_css::parser::to_css(values))
            }
            _ => None,
        });
        assert_eq!(v.as_deref(), Some("4999"));
    }
}
