//! CSS for xbro: tokenizer, stylesheet parser, selectors (parsing, matching,
//! specificity), media queries and generic value helpers.
//!
//! This crate is DOM-agnostic: selector matching goes through the
//! [`selector::Element`] trait, which `xbro-style` implements for DOM nodes.
//! Property-specific interpretation (shorthands, computed values) lives in
//! `xbro-style`; this crate only provides the building blocks.

pub mod color;
pub mod media;
pub mod parser;
pub mod selector;
pub mod tokenizer;
pub mod values;

pub use color::Color;
pub use media::{MediaContext, MediaQueryList};
pub use parser::{
    parse_declaration_list, parse_stylesheet, ComponentValue, CssRule, Declaration, StyleRule,
    Stylesheet,
};
pub use selector::{
    parse_selector_list, AttrOperator, AttrSelector, Combinator, Compound, Element, PseudoClass,
    PseudoElement, Selector, Specificity,
};
pub use tokenizer::Token;
pub use values::{Length, LengthUnit};
