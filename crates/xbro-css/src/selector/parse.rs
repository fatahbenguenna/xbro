//! Selector parsing from component values, including the nesting selector
//! (`&`) and relative selectors (`:has()`, nested style rules).

use std::borrow::Cow;

use super::nth;
use super::{
    AttrOperator, AttrSelector, Combinator, Compound, PseudoClass, PseudoElement, Selector,
};
use crate::parser::ComponentValue as CV;
use crate::tokenizer::Token;
use crate::values::trim;

/// Maximum number of compounds in one complex selector (longer selectors are invalid).
pub const MAX_COMPOUNDS: usize = 64;

/// Maximum nesting depth of selector lists inside functional pseudo-classes
/// (`:is(:not(:has(...)))`) at parse time.
pub const MAX_SELECTOR_NESTING: usize = 16;

/// Maximum nesting depth of a resolved selector (CSS Nesting may wrap parent
/// selectors in `:is()` at each level).
const MAX_RESOLVED_NESTING: usize = 2 * MAX_SELECTOR_NESTING;

/// Maximum size, in compounds (including those inside functional
/// pseudo-classes), of the resolved selector list of one nested style rule.
const MAX_NESTED_LIST_WEIGHT: usize = 2048;

/// Maximum number of `&` in one complex selector.
const MAX_NESTING_SELECTORS: u32 = 16;

/// HTML attributes whose values are matched ASCII case-insensitively by
/// attribute selectors (HTML §4.16.2 "Case-sensitivity of selectors").
const CASE_INSENSITIVE_ATTRIBUTES: &[&str] = &[
    "accept",
    "accept-charset",
    "align",
    "alink",
    "axis",
    "bgcolor",
    "charset",
    "checked",
    "clear",
    "codetype",
    "color",
    "compact",
    "declare",
    "defer",
    "dir",
    "direction",
    "disabled",
    "enctype",
    "face",
    "frame",
    "hreflang",
    "http-equiv",
    "lang",
    "language",
    "link",
    "media",
    "method",
    "multiple",
    "nohref",
    "noresize",
    "noshade",
    "nowrap",
    "readonly",
    "rel",
    "rev",
    "rules",
    "scope",
    "scrolling",
    "selected",
    "shape",
    "target",
    "text",
    "type",
    "valign",
    "valuetype",
    "vlink",
];

struct ParsedCompound {
    compound: Compound,
    /// Number of `&` in this compound.
    nesting: u32,
    pseudo_element: Option<PseudoElement>,
}

struct ParsedComplex {
    /// Leading combinator of a relative selector (`> a`).
    leading: Option<Combinator>,
    /// Compounds left to right, with their number of `&`.
    compounds: Vec<(Compound, u32)>,
    /// `combinators[i]` sits between `compounds[i]` and `compounds[i + 1]`.
    combinators: Vec<Combinator>,
    pseudo_element: Option<PseudoElement>,
    /// True when `&` appears anywhere, including inside functional pseudo-classes.
    has_nesting: bool,
}

struct Parser<'a> {
    /// Selectors `&` stands for (without pseudo-elements); `None` at top level,
    /// where `&` means `:scope` (the root element).
    parent: Option<&'a [Selector]>,
    /// [`list_weight`] of `parent`.
    parent_weight: usize,
    /// Remaining weight of parent copies this parse may create (each `&`
    /// resolved to `:is(parent)` or spliced in costs `parent_weight`).
    clone_budget: usize,
    depth: usize,
    in_has: bool,
    saw_nesting: bool,
}

/// Parses a top-level selector list (see [`super::parse_selector_list_from`]).
pub(crate) fn parse_top_level(values: &[CV]) -> Option<Vec<Selector>> {
    let mut parser = Parser {
        parent: None,
        parent_weight: 0,
        clone_budget: 0,
        depth: 0,
        in_has: false,
        saw_nesting: false,
    };
    parser.list(values, false, |p, part| {
        let complex = p.complex(part, false)?;
        p.resolve(complex, false)
    })
}

/// Parses the selector list of a style rule nested inside a rule whose
/// selectors are `parent` (CSS Nesting), resolving it to absolute selectors:
///
/// * `&` stands for the parent selectors (`:is(parent)`, inlined when the
///   parent is a single selector);
/// * a selector without `&` is relative to the parent with a descendant
///   combinator (`.x` means `& .x`);
/// * a leading combinator is relative to the parent (`> a` means `& > a`).
///
/// Parent selectors with a pseudo-element are ignored (`&` cannot represent
/// pseudo-elements). Returns `None` when the list is invalid.
pub fn parse_nested_selector_list(values: &[CV], parent: &[Selector]) -> Option<Vec<Selector>> {
    let usable = nesting_parent(parent);
    parse_nested_with(values, &usable, list_weight(&usable))
}

/// The parent selectors `&` can represent: those without a pseudo-element.
pub(crate) fn nesting_parent(parent: &[Selector]) -> Cow<'_, [Selector]> {
    if parent.iter().any(|s| s.pseudo_element.is_some()) {
        Cow::Owned(
            parent
                .iter()
                .filter(|s| s.pseudo_element.is_none())
                .cloned()
                .collect(),
        )
    } else {
        Cow::Borrowed(parent)
    }
}

/// [`parse_nested_selector_list`] against a parent already filtered by
/// [`nesting_parent`], whose [`list_weight`] is `usable_weight`. Callers
/// resolving many nested rules against one parent compute these once, so
/// each nested rule costs no more than its own size until it resolves.
pub(crate) fn parse_nested_with(
    values: &[CV],
    usable: &[Selector],
    usable_weight: usize,
) -> Option<Vec<Selector>> {
    // Every nested selector holds at least one copy of the parent (an
    // explicit or implicit `&`), which must fit in the clone budget.
    if usable.is_empty() || usable_weight > MAX_NESTED_LIST_WEIGHT {
        return None;
    }
    let mut parser = Parser {
        parent: Some(usable),
        parent_weight: usable_weight,
        clone_budget: MAX_NESTED_LIST_WEIGHT,
        depth: 0,
        in_has: false,
        saw_nesting: false,
    };
    let list = parser.list(values, false, |p, part| {
        p.saw_nesting = false;
        let mut complex = p.complex(part, true)?;
        complex.has_nesting = p.saw_nesting;
        p.resolve(complex, true)
    })?;
    if list_weight(&list) > MAX_NESTED_LIST_WEIGHT
        || list.iter().any(|s| nesting_depth(s) > MAX_RESOLVED_NESTING)
    {
        return None;
    }
    Some(list)
}

/// Total number of compounds in a selector list, counting those inside
/// functional pseudo-classes (a measure of its size and matching cost).
pub fn list_weight(list: &[Selector]) -> usize {
    list.iter().map(weight).sum()
}

fn weight(selector: &Selector) -> usize {
    std::iter::once(&selector.subject)
        .chain(selector.ancestors.iter().map(|(_, c)| c))
        .map(|compound| {
            1 + compound
                .pseudo_classes
                .iter()
                .map(|pc| match pc {
                    PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Where(list) => {
                        list_weight(list)
                    }
                    PseudoClass::Has(list) => list.iter().map(|(_, s)| weight(s)).sum(),
                    _ => 0,
                })
                .sum::<usize>()
        })
        .sum()
}

fn nesting_depth(selector: &Selector) -> usize {
    std::iter::once(&selector.subject)
        .chain(selector.ancestors.iter().map(|(_, c)| c))
        .flat_map(|compound| compound.pseudo_classes.iter())
        .map(|pc| match pc {
            PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Where(list) => {
                1 + list.iter().map(nesting_depth).max().unwrap_or(0)
            }
            PseudoClass::Has(list) => {
                1 + list
                    .iter()
                    .map(|(_, s)| nesting_depth(s))
                    .max()
                    .unwrap_or(0)
            }
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

fn skip_whitespace(items: &[CV], i: &mut usize) -> bool {
    let start = *i;
    while items.get(*i).is_some_and(CV::is_whitespace) {
        *i += 1;
    }
    *i > start
}

fn combinator_of(item: &CV) -> Option<Combinator> {
    match item {
        CV::Token(Token::Delim('>')) => Some(Combinator::Child),
        CV::Token(Token::Delim('+')) => Some(Combinator::NextSibling),
        CV::Token(Token::Delim('~')) => Some(Combinator::SubsequentSibling),
        _ => None,
    }
}

/// Builds a [`Selector`] from left-to-right compounds and combinators.
fn build(
    mut compounds: Vec<Compound>,
    combinators: Vec<Combinator>,
    pseudo_element: Option<PseudoElement>,
) -> Selector {
    let subject = compounds.pop().unwrap_or_default();
    let ancestors = combinators.into_iter().zip(compounds).rev().collect();
    Selector {
        subject,
        ancestors,
        pseudo_element,
    }
}

fn merge_into(dst: &mut Compound, src: Compound) {
    if dst.tag.is_none() {
        dst.tag = src.tag;
    }
    dst.ids.extend(src.ids);
    dst.classes.extend(src.classes);
    dst.attrs.extend(src.attrs);
    dst.pseudo_classes.extend(src.pseudo_classes);
}

impl Parser<'_> {
    /// Parses a comma-separated list; in a forgiving list invalid items are
    /// dropped instead of invalidating the whole list.
    fn list<T>(
        &mut self,
        values: &[CV],
        forgiving: bool,
        mut item: impl FnMut(&mut Self, &[CV]) -> Option<T>,
    ) -> Option<Vec<T>> {
        let mut out = Vec::new();
        for part in values.split(CV::is_comma) {
            let part = trim(part);
            let parsed = if part.is_empty() {
                None
            } else {
                item(self, part)
            };
            match parsed {
                Some(value) => out.push(value),
                None if forgiving => {}
                None => return None,
            }
        }
        Some(out)
    }

    /// Parses one complex selector; `relative` allows a leading combinator.
    fn complex(&mut self, items: &[CV], relative: bool) -> Option<ParsedComplex> {
        let mut i = 0;
        let mut leading = None;
        if relative {
            if let Some(c) = items.first().and_then(combinator_of) {
                leading = Some(c);
                i = 1;
                skip_whitespace(items, &mut i);
            }
        }
        let mut compounds = Vec::new();
        let mut combinators = Vec::new();
        let mut pseudo_element = None;
        loop {
            let parsed = self.compound(items, &mut i)?;
            compounds.push((parsed.compound, parsed.nesting));
            if compounds.len() > MAX_COMPOUNDS {
                return None;
            }
            let had_whitespace = skip_whitespace(items, &mut i);
            if parsed.pseudo_element.is_some() {
                // A pseudo-element ends the selector.
                pseudo_element = parsed.pseudo_element;
                if i < items.len() {
                    return None;
                }
                break;
            }
            let Some(next) = items.get(i) else { break };
            let combinator = match combinator_of(next) {
                Some(c) => {
                    i += 1;
                    skip_whitespace(items, &mut i);
                    c
                }
                None if had_whitespace => Combinator::Descendant,
                None => return None,
            };
            if i >= items.len() {
                return None;
            }
            combinators.push(combinator);
        }
        let total_nesting: u32 = compounds.iter().map(|(_, n)| *n).sum();
        if total_nesting > MAX_NESTING_SELECTORS {
            return None;
        }
        Some(ParsedComplex {
            leading,
            compounds,
            combinators,
            pseudo_element,
            has_nesting: false,
        })
    }

    /// Parses one compound selector starting at `items[*i]`.
    fn compound(&mut self, items: &[CV], i: &mut usize) -> Option<ParsedCompound> {
        let mut compound = Compound::default();
        let mut nesting = 0u32;
        let mut pseudo_element: Option<PseudoElement> = None;
        let mut any = false;
        if let Some((tag, len)) = items.get(*i..).and_then(type_selector) {
            compound.tag = tag;
            *i += len;
            any = true;
        }
        while let Some(item) = items.get(*i) {
            let is_colon = matches!(item, CV::Token(Token::Colon));
            if pseudo_element.is_some() && !is_colon {
                break;
            }
            match item {
                CV::Token(Token::Hash { value, is_id: true }) => {
                    compound.ids.push(value.clone());
                    *i += 1;
                }
                CV::Token(Token::Delim('.')) => {
                    let class = items.get(*i + 1)?.ident()?;
                    compound.classes.push(class.to_string());
                    *i += 2;
                }
                CV::Block { open: '[', content } => {
                    compound.attrs.push(parse_attribute(content)?);
                    *i += 1;
                }
                CV::Token(Token::Delim('&')) => {
                    nesting += 1;
                    self.saw_nesting = true;
                    *i += 1;
                }
                CV::Token(Token::Colon) => {
                    let next = items.get(*i + 1)?;
                    if matches!(next, CV::Token(Token::Colon)) {
                        if pseudo_element.is_some() {
                            return None;
                        }
                        pseudo_element = Some(parse_pseudo_element(items.get(*i + 2)?)?);
                        *i += 3;
                    } else {
                        *i += 2;
                        match pseudo_element {
                            // Vendor pseudo-elements accept vendor state
                            // pseudo-classes (`::-webkit-scrollbar:horizontal`);
                            // such rules never apply anyway.
                            Some(PseudoElement::Unsupported) => {
                                if !matches!(next, CV::Token(Token::Ident(_)) | CV::Function { .. })
                                {
                                    return None;
                                }
                            }
                            Some(_) => {
                                let pc = self.pseudo_class(next)?;
                                if !is_user_action(&pc) {
                                    return None;
                                }
                                compound.pseudo_classes.push(pc);
                            }
                            None => match legacy_pseudo_element(next) {
                                Some(pe) => pseudo_element = Some(pe),
                                None => {
                                    let pc = self.pseudo_class(next)?;
                                    compound.pseudo_classes.push(pc);
                                }
                            },
                        }
                    }
                }
                _ => break,
            }
            any = true;
        }
        if !any {
            return None;
        }
        Some(ParsedCompound {
            compound,
            nesting,
            pseudo_element,
        })
    }

    fn pseudo_class(&mut self, target: &CV) -> Option<PseudoClass> {
        use PseudoClass as P;
        match target {
            CV::Token(Token::Ident(name)) => simple_pseudo_class(&name.to_ascii_lowercase()),
            CV::Function { name, args } => {
                let lower = name.to_ascii_lowercase();
                Some(match lower.as_str() {
                    "not" => P::Not(self.nested_list(args, false)?),
                    "is" | "matches" | "-webkit-any" | "-moz-any" => {
                        P::Is(self.nested_list(args, true)?)
                    }
                    "where" => P::Where(self.nested_list(args, true)?),
                    "has" => P::Has(self.has_list(args)?),
                    "nth-child" => {
                        let (a, b) = nth::parse_nth(args)?;
                        P::NthChild { a, b }
                    }
                    "nth-last-child" => {
                        let (a, b) = nth::parse_nth(args)?;
                        P::NthLastChild { a, b }
                    }
                    "nth-of-type" => {
                        let (a, b) = nth::parse_nth(args)?;
                        P::NthOfType { a, b }
                    }
                    "nth-last-of-type" => {
                        let (a, b) = nth::parse_nth(args)?;
                        P::NthLastOfType { a, b }
                    }
                    "lang" => P::Lang(parse_lang_ranges(args)?),
                    "dir" => match trim(args) {
                        [value] => match value.ident()?.to_ascii_lowercase().as_str() {
                            "ltr" => P::Dir { rtl: false },
                            "rtl" => P::Dir { rtl: true },
                            _ => P::NeverMatches,
                        },
                        _ => return None,
                    },
                    "host"
                    | "host-context"
                    | "state"
                    | "nth-col"
                    | "nth-last-col"
                    | "current"
                    | "active-view-transition-type"
                    | "heading" => P::NeverMatches,
                    _ => return None,
                })
            }
            _ => None,
        }
    }

    /// Selector list argument of `:not()` / `:is()` / `:where()`.
    fn nested_list(&mut self, args: &[CV], forgiving: bool) -> Option<Vec<Selector>> {
        if self.depth >= MAX_SELECTOR_NESTING {
            return None;
        }
        self.depth += 1;
        let result = self.list(args, forgiving, |p, part| {
            let complex = p.complex(part, false)?;
            if complex.pseudo_element.is_some() {
                return None;
            }
            p.resolve(complex, false)
        });
        self.depth -= 1;
        result
    }

    /// Relative selector list argument of `:has()` (which cannot be nested).
    fn has_list(&mut self, args: &[CV]) -> Option<Vec<(Combinator, Selector)>> {
        if self.in_has || self.depth >= MAX_SELECTOR_NESTING {
            return None;
        }
        self.in_has = true;
        self.depth += 1;
        let result = self.list(args, false, |p, part| {
            let mut complex = p.complex(part, true)?;
            if complex.pseudo_element.is_some() {
                return None;
            }
            let combinator = complex.leading.take().unwrap_or(Combinator::Descendant);
            Some((combinator, p.resolve(complex, false)?))
        });
        self.depth -= 1;
        self.in_has = false;
        result
    }

    /// Pays for `copies` copies of the parent selectors; false when the
    /// budget is exhausted.
    fn take_clones(&mut self, copies: usize) -> bool {
        match self.parent_weight.checked_mul(copies) {
            Some(cost) if cost <= self.clone_budget => {
                self.clone_budget -= cost;
                true
            }
            _ => false,
        }
    }

    /// What `&` stands for in a compound.
    fn nesting_pseudo_class(&mut self) -> Option<PseudoClass> {
        match self.parent {
            None => Some(PseudoClass::Root),
            Some([]) => None,
            Some(parent) => {
                if !self.take_clones(1) {
                    return None;
                }
                Some(PseudoClass::Is(parent.to_vec()))
            }
        }
    }

    /// Turns a parsed complex selector into a [`Selector`]. `implicit` is set
    /// for the selectors of a nested style rule, which are made relative to
    /// the parent rule.
    fn resolve(&mut self, mut complex: ParsedComplex, implicit: bool) -> Option<Selector> {
        if implicit {
            if complex.leading.is_some() || !complex.has_nesting {
                let combinator = complex.leading.take().unwrap_or(Combinator::Descendant);
                complex.compounds.insert(0, (Compound::default(), 1));
                complex.combinators.insert(0, combinator);
                if complex.compounds.len() > MAX_COMPOUNDS {
                    return None;
                }
            }
            if let Some([parent]) = self.parent {
                let copies: u32 = complex.compounds.iter().map(|(_, n)| *n).sum();
                // Paid only when the splice happens: otherwise each `&` pays
                // for its `:is()` copy below.
                if let Some(selector) = inline_parent(parent, &mut complex) {
                    if !self.take_clones(copies as usize) {
                        return None;
                    }
                    return (selector.ancestors.len() < MAX_COMPOUNDS).then_some(selector);
                }
            }
        } else if complex.leading.is_some() {
            return None;
        }
        let mut compounds = Vec::with_capacity(complex.compounds.len());
        for (mut compound, n) in complex.compounds {
            for _ in 0..n {
                compound.pseudo_classes.push(self.nesting_pseudo_class()?);
            }
            compounds.push(compound);
        }
        Some(build(
            compounds,
            complex.combinators,
            complex.pseudo_element,
        ))
    }
}

/// Splices a single parent selector into the selector instead of using
/// `:is(parent)`, which matches and weighs the same:
///
/// * a parent made of one compound is merged into every compound holding
///   `&` (`.a` + `.g & + &` gives `.g .a + .a`);
/// * a complex parent is spliced in when `&` appears once, in the leftmost
///   compound (`.x .a` + `&:hover .b` gives `.x .a:hover .b`).
///
/// Returns `None` (leaving `complex` untouched) when this is not possible:
/// when two type selectors would end up in one compound (`div&` with parent
/// `div` must keep the specificity of `div:is(div)`, and different types
/// cannot be merged at all).
fn inline_parent(parent: &Selector, complex: &mut ParsedComplex) -> Option<Selector> {
    if parent.subject.tag.is_some()
        && complex
            .compounds
            .iter()
            .any(|(c, n)| *n > 0 && c.tag.is_some())
    {
        return None;
    }
    let simple_parent = parent.ancestors.is_empty();
    let leftmost_only = complex
        .compounds
        .iter()
        .enumerate()
        .all(|(i, (_, n))| *n == u32::from(i == 0));
    if !simple_parent && !leftmost_only {
        return None;
    }
    let mut compounds = Vec::with_capacity(complex.compounds.len());
    for (compound, n) in std::mem::take(&mut complex.compounds) {
        let mut compound = compound;
        for _ in 0..n {
            let mut merged = parent.subject.clone();
            merge_into(&mut merged, compound);
            compound = merged;
        }
        compounds.push(compound);
    }
    let mut selector = build(
        compounds,
        std::mem::take(&mut complex.combinators),
        complex.pseudo_element,
    );
    // Only reachable with ancestors when `&` is in the leftmost compound.
    selector.ancestors.extend(parent.ancestors.iter().cloned());
    Some(selector)
}

/// Type or universal selector with an optional (ignored) namespace prefix:
/// `div`, `*`, `ns|div`, `*|*`, `|div`. Returns the lowercased tag (`None`
/// for `*`) and the number of component values consumed.
fn type_selector(items: &[CV]) -> Option<(Option<String>, usize)> {
    fn name_of(value: &CV) -> Option<Option<String>> {
        match value {
            CV::Token(Token::Ident(name)) => Some(Some(name.to_ascii_lowercase())),
            CV::Token(Token::Delim('*')) => Some(None),
            _ => None,
        }
    }
    let first = items.first()?;
    if first.is_delim('|') {
        return items.get(1).and_then(name_of).map(|name| (name, 2));
    }
    let name = name_of(first)?;
    if items.get(1).is_some_and(|v| v.is_delim('|')) {
        if let Some(local) = items.get(2).and_then(name_of) {
            return Some((local, 3));
        }
    }
    Some((name, 1))
}

/// `[name]`, `[name op value flag?]` with an optional namespace prefix.
fn parse_attribute(content: &[CV]) -> Option<AttrSelector> {
    let items = trim(content);
    let (name, mut i) = match items {
        [ns, bar, CV::Token(Token::Ident(local)), ..]
            if bar.is_delim('|') && (ns.ident().is_some() || ns.is_delim('*')) =>
        {
            (local, 3)
        }
        [bar, CV::Token(Token::Ident(local)), ..] if bar.is_delim('|') => (local, 2),
        [CV::Token(Token::Ident(name)), ..] => (name, 1),
        _ => return None,
    };
    let name = name.to_ascii_lowercase();
    skip_whitespace(items, &mut i);
    if i == items.len() {
        return Some(AttrSelector {
            name,
            op: AttrOperator::Exists,
            value: String::new(),
            case_insensitive: false,
        });
    }
    let op_item = items.get(i)?;
    let op = if op_item.is_delim('=') {
        i += 1;
        AttrOperator::Equals
    } else {
        let op = match op_item {
            CV::Token(Token::Delim('~')) => AttrOperator::Includes,
            CV::Token(Token::Delim('|')) => AttrOperator::DashMatch,
            CV::Token(Token::Delim('^')) => AttrOperator::Prefix,
            CV::Token(Token::Delim('$')) => AttrOperator::Suffix,
            CV::Token(Token::Delim('*')) => AttrOperator::Substring,
            _ => return None,
        };
        if !items.get(i + 1)?.is_delim('=') {
            return None;
        }
        i += 2;
        op
    };
    skip_whitespace(items, &mut i);
    let value = match items.get(i)? {
        CV::Token(Token::Ident(v) | Token::String(v)) => v.clone(),
        _ => return None,
    };
    i += 1;
    skip_whitespace(items, &mut i);
    let mut flag = None;
    if let Some(item) = items.get(i) {
        let f = item.ident()?;
        flag = Some(if f.eq_ignore_ascii_case("i") {
            true
        } else if f.eq_ignore_ascii_case("s") {
            false
        } else {
            return None;
        });
        i += 1;
        skip_whitespace(items, &mut i);
    }
    if i != items.len() {
        return None;
    }
    let case_insensitive =
        flag.unwrap_or_else(|| CASE_INSENSITIVE_ATTRIBUTES.contains(&name.as_str()));
    Some(AttrSelector {
        name,
        op,
        value,
        case_insensitive,
    })
}

/// `:lang()` arguments: comma-separated identifiers or strings.
fn parse_lang_ranges(args: &[CV]) -> Option<Vec<String>> {
    let mut ranges = Vec::new();
    for part in args.split(CV::is_comma) {
        match trim(part) {
            [CV::Token(Token::Ident(s) | Token::String(s))] => ranges.push(s.clone()),
            _ => return None,
        }
    }
    Some(ranges)
}

fn is_user_action(pc: &PseudoClass) -> bool {
    matches!(
        pc,
        PseudoClass::Hover
            | PseudoClass::Active
            | PseudoClass::Focus
            | PseudoClass::FocusVisible
            | PseudoClass::FocusWithin
    )
}

/// Non-functional pseudo-classes (lowercase name).
fn simple_pseudo_class(name: &str) -> Option<PseudoClass> {
    use PseudoClass as P;
    Some(match name {
        "root" | "scope" => P::Root,
        "empty" => P::Empty,
        "first-child" => P::FirstChild,
        "last-child" => P::LastChild,
        "only-child" => P::OnlyChild,
        "first-of-type" => P::FirstOfType,
        "last-of-type" => P::LastOfType,
        "only-of-type" => P::OnlyOfType,
        "link" => P::Link,
        "any-link" | "-webkit-any-link" | "-moz-any-link" => P::AnyLink,
        "visited" => P::Visited,
        "hover" => P::Hover,
        "active" => P::Active,
        "focus" => P::Focus,
        "focus-within" => P::FocusWithin,
        "focus-visible" => P::FocusVisible,
        "checked" => P::Checked,
        "disabled" => P::Disabled,
        "enabled" => P::Enabled,
        "target" => P::Target,
        "defined" => P::Defined,
        "required" => P::Required,
        "optional" => P::Optional,
        "read-only" | "-moz-read-only" => P::ReadOnly,
        "read-write" | "-moz-read-write" => P::ReadWrite,
        "placeholder-shown" => P::PlaceholderShown,
        "indeterminate" => P::Indeterminate,
        "default" => P::Default,
        "open" => P::Open,
        "closed" => P::Closed,
        "target-within"
        | "local-link"
        | "valid"
        | "invalid"
        | "user-valid"
        | "user-invalid"
        | "in-range"
        | "out-of-range"
        | "fullscreen"
        | "-webkit-full-screen"
        | "-moz-full-screen"
        | "modal"
        | "popover-open"
        | "autofill"
        | "-webkit-autofill"
        | "playing"
        | "paused"
        | "seeking"
        | "buffering"
        | "stalled"
        | "muted"
        | "volume-locked"
        | "picture-in-picture"
        | "current"
        | "past"
        | "future"
        | "first"
        | "left"
        | "right"
        | "blank"
        | "host"
        | "-moz-focusring"
        | "-moz-ui-invalid"
        | "-moz-ui-valid"
        | "has-slotted"
        | "active-view-transition"
        | "xr-overlay" => P::NeverMatches,
        _ => return None,
    })
}

/// CSS 2 pseudo-elements also accepted with a single colon.
fn legacy_pseudo_element(item: &CV) -> Option<PseudoElement> {
    let name = item.ident()?;
    if name.eq_ignore_ascii_case("before") {
        Some(PseudoElement::Before)
    } else if name.eq_ignore_ascii_case("after") {
        Some(PseudoElement::After)
    } else if name.eq_ignore_ascii_case("first-line") || name.eq_ignore_ascii_case("first-letter") {
        Some(PseudoElement::Unsupported)
    } else {
        None
    }
}

fn is_vendor_prefixed(name: &str) -> bool {
    name.starts_with("-webkit-") || name.starts_with("-moz-")
}

/// The name after `::`.
fn parse_pseudo_element(item: &CV) -> Option<PseudoElement> {
    match item {
        CV::Token(Token::Ident(name)) => {
            let lower = name.to_ascii_lowercase();
            match lower.as_str() {
                "before" => Some(PseudoElement::Before),
                "after" => Some(PseudoElement::After),
                "marker"
                | "first-line"
                | "first-letter"
                | "placeholder"
                | "selection"
                | "backdrop"
                | "file-selector-button"
                | "cue"
                | "cue-region"
                | "spelling-error"
                | "grammar-error"
                | "target-text"
                | "details-content"
                | "search-text"
                | "checkmark"
                | "picker-icon"
                | "column"
                | "scroll-marker"
                | "scroll-marker-group"
                | "view-transition" => Some(PseudoElement::Unsupported),
                n if is_vendor_prefixed(n) => Some(PseudoElement::Unsupported),
                _ => None,
            }
        }
        CV::Function { name, .. } => {
            let lower = name.to_ascii_lowercase();
            match lower.as_str() {
                "cue"
                | "cue-region"
                | "part"
                | "slotted"
                | "highlight"
                | "view-transition-group"
                | "view-transition-image-pair"
                | "view-transition-old"
                | "view-transition-new"
                | "picker"
                | "scroll-button" => Some(PseudoElement::Unsupported),
                n if is_vendor_prefixed(n) => Some(PseudoElement::Unsupported),
                _ => None,
            }
        }
        _ => None,
    }
}
