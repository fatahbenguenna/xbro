//! Per-element predicates used by selector matching: type/id/class and
//! attribute selectors, and the pseudo-classes that only look at one element
//! and its siblings or ancestors (no nested selector lists).

use super::nth::nth_matches;
use super::{AttrOperator, AttrSelector, Element, PseudoClass};

fn eq(a: &str, b: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn starts_with(value: &str, prefix: &str, case_insensitive: bool) -> bool {
    value.as_bytes().get(..prefix.len()).is_some_and(|head| {
        head == prefix.as_bytes()
            || (case_insensitive && head.eq_ignore_ascii_case(prefix.as_bytes()))
    })
}

fn ends_with(value: &str, suffix: &str, case_insensitive: bool) -> bool {
    value
        .len()
        .checked_sub(suffix.len())
        .and_then(|start| value.as_bytes().get(start..))
        .is_some_and(|tail| {
            tail == suffix.as_bytes()
                || (case_insensitive && tail.eq_ignore_ascii_case(suffix.as_bytes()))
        })
}

/// Substring search, ASCII case-insensitive when asked (linear time).
fn contains(value: &str, needle: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        value
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    } else {
        value.contains(needle)
    }
}

pub(super) fn matches_attribute<E: Element>(selector: &AttrSelector, element: &E) -> bool {
    let Some(value) = element.attr(&selector.name) else {
        return false;
    };
    let wanted = selector.value.as_str();
    let ci = selector.case_insensitive;
    match selector.op {
        AttrOperator::Exists => true,
        AttrOperator::Equals => eq(value, wanted, ci),
        AttrOperator::Includes => {
            !wanted.is_empty()
                && !wanted.contains(|c: char| c.is_ascii_whitespace())
                && value
                    .split(|c: char| c.is_ascii_whitespace())
                    .any(|word| eq(word, wanted, ci))
        }
        AttrOperator::DashMatch => {
            eq(value, wanted, ci)
                || (value.as_bytes().get(wanted.len()) == Some(&b'-')
                    && starts_with(value, wanted, ci))
        }
        AttrOperator::Prefix => !wanted.is_empty() && starts_with(value, wanted, ci),
        AttrOperator::Suffix => !wanted.is_empty() && ends_with(value, wanted, ci),
        AttrOperator::Substring => !wanted.is_empty() && contains(value, wanted, ci),
    }
}

/// Default implementation of [`Element::nth_index`]: walks the siblings.
pub(super) fn walk_nth_index<E: Element>(element: &E, of_type: bool, from_end: bool) -> usize {
    let mut index = 1usize;
    let name = element.local_name();
    let next = |e: &E| {
        if from_end {
            e.next_sibling_element()
        } else {
            e.prev_sibling_element()
        }
    };
    let mut current = next(element);
    while let Some(sibling) = current {
        if !of_type || sibling.local_name() == name {
            index = index.saturating_add(1);
        }
        current = next(&sibling);
    }
    index
}

fn nth<E: Element>(element: &E, a: i32, b: i32, of_type: bool, from_end: bool) -> bool {
    let index = i64::try_from(element.nth_index(of_type, from_end)).unwrap_or(i64::MAX);
    nth_matches(a, b, index)
}

fn has_sibling_of_type<E: Element>(element: &E, before: bool) -> bool {
    let name = element.local_name();
    let step = |e: &E| {
        if before {
            e.prev_sibling_element()
        } else {
            e.next_sibling_element()
        }
    };
    let mut current = step(element);
    while let Some(sibling) = current {
        if sibling.local_name() == name {
            return true;
        }
        current = step(&sibling);
    }
    false
}

/// Value of the nearest inclusive ancestor's `name` attribute.
fn inherited_attr<E: Element>(element: &E, name: &str) -> Option<String> {
    let mut current = Some(element.clone());
    while let Some(e) = current {
        if let Some(value) = e.attr(name) {
            return Some(value.to_string());
        }
        current = e.parent_element();
    }
    None
}

fn lang_matches(lang: &str, range: &str) -> bool {
    if lang.is_empty() {
        return false;
    }
    if range == "*" {
        return true;
    }
    if let Some(rest) = range.strip_prefix("*-") {
        // `*-CH`: any language with that subtag.
        return lang
            .split('-')
            .skip(1)
            .any(|subtag| subtag.eq_ignore_ascii_case(rest));
    }
    starts_with(lang, range, true)
        && (lang.len() == range.len() || lang.as_bytes().get(range.len()) == Some(&b'-'))
}

fn is_form_control(name: &str) -> bool {
    matches!(
        name,
        "button" | "input" | "select" | "textarea" | "optgroup" | "option" | "fieldset"
    )
}

/// `<input>` types whose value is editable text.
fn is_text_input<E: Element>(element: &E) -> bool {
    let kind = element.attr("type").unwrap_or("text");
    [
        "text",
        "search",
        "url",
        "tel",
        "email",
        "password",
        "date",
        "month",
        "week",
        "time",
        "datetime-local",
        "number",
    ]
    .iter()
    .any(|t| t.eq_ignore_ascii_case(kind))
}

fn is_read_write<E: Element>(element: &E) -> bool {
    let name = element.local_name();
    if name == "textarea" || (name == "input" && is_text_input(element)) {
        return element.attr("readonly").is_none() && !element.is_disabled();
    }
    match inherited_attr(element, "contenteditable") {
        Some(v) => {
            v.is_empty()
                || v.eq_ignore_ascii_case("true")
                || v.eq_ignore_ascii_case("plaintext-only")
        }
        None => false,
    }
}

/// Matches a pseudo-class that does not contain selectors. The logical
/// pseudo-classes (`:not()`, `:is()`, `:where()`, `:has()`) are handled by
/// the matcher and return false here.
pub(super) fn matches_simple_pseudo_class<E: Element>(pc: &PseudoClass, element: &E) -> bool {
    use PseudoClass as P;
    match pc {
        P::Root => element.is_root(),
        P::Empty => element.is_empty(),
        P::FirstChild => element.prev_sibling_element().is_none(),
        P::LastChild => element.next_sibling_element().is_none(),
        P::OnlyChild => {
            element.prev_sibling_element().is_none() && element.next_sibling_element().is_none()
        }
        P::FirstOfType => !has_sibling_of_type(element, true),
        P::LastOfType => !has_sibling_of_type(element, false),
        P::OnlyOfType => {
            !has_sibling_of_type(element, true) && !has_sibling_of_type(element, false)
        }
        P::NthChild { a, b } => nth(element, *a, *b, false, false),
        P::NthLastChild { a, b } => nth(element, *a, *b, false, true),
        P::NthOfType { a, b } => nth(element, *a, *b, true, false),
        P::NthLastOfType { a, b } => nth(element, *a, *b, true, true),
        P::Link => element.is_link() && !element.is_visited(),
        P::AnyLink => element.is_link(),
        P::Visited => element.is_visited(),
        P::Hover => element.is_hovered(),
        P::Active => element.is_active(),
        P::Focus | P::FocusVisible => element.is_focused(),
        P::FocusWithin => element.is_focus_within(),
        P::Checked => element.is_checked(),
        P::Disabled => element.is_disabled(),
        P::Enabled => is_form_control(element.local_name()) && !element.is_disabled(),
        P::Not(_) | P::Is(_) | P::Where(_) | P::Has(_) | P::NeverMatches => false,
        P::Target => element.is_target(),
        P::Lang(ranges) => match inherited_attr(element, "lang") {
            Some(lang) => ranges.iter().any(|r| lang_matches(&lang, r)),
            None => false,
        },
        P::Dir { rtl } => {
            let is_rtl =
                inherited_attr(element, "dir").is_some_and(|d| d.eq_ignore_ascii_case("rtl"));
            is_rtl == *rtl
        }
        P::Defined => true,
        P::Required | P::Optional => {
            matches!(element.local_name(), "input" | "select" | "textarea")
                && (element.attr("required").is_some() == matches!(pc, P::Required))
        }
        P::ReadWrite => is_read_write(element),
        P::ReadOnly => !is_read_write(element),
        P::PlaceholderShown => {
            element.attr("placeholder").is_some()
                && match element.local_name() {
                    "input" => {
                        is_text_input(element) && element.attr("value").is_none_or(str::is_empty)
                    }
                    "textarea" => element.is_empty(),
                    _ => false,
                }
        }
        P::Indeterminate => element.local_name() == "progress" && element.attr("value").is_none(),
        P::Default => match element.local_name() {
            "option" => element.attr("selected").is_some(),
            "input" => {
                element.attr("checked").is_some()
                    && element.attr("type").is_some_and(|t| {
                        t.eq_ignore_ascii_case("checkbox") || t.eq_ignore_ascii_case("radio")
                    })
            }
            _ => false,
        },
        P::Open | P::Closed => {
            matches!(element.local_name(), "details" | "dialog")
                && (element.attr("open").is_some() == matches!(pc, P::Open))
        }
    }
}
