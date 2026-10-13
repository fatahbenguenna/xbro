//! Stylesheet parser (CSS Syntax Module Level 3, §5) producing rules with
//! parsed selectors and raw declaration values.

use crate::media::MediaQueryList;
use crate::selector::Selector;
use crate::tokenizer::Token;

/// A preserved token or a simple block / function, as produced by
/// "consume a component value".
#[derive(Clone, Debug, PartialEq)]
pub enum ComponentValue {
    Token(Token),
    /// `name( args )` — `name` keeps its original case.
    Function {
        name: String,
        args: Vec<ComponentValue>,
    },
    /// `[ ]`, `( )` or `{ }` block; `open` is the opening character.
    Block {
        open: char,
        content: Vec<ComponentValue>,
    },
}

impl ComponentValue {
    pub fn is_whitespace(&self) -> bool {
        matches!(self, ComponentValue::Token(Token::Whitespace))
    }

    /// The identifier if this is an `Ident` token.
    pub fn ident(&self) -> Option<&str> {
        match self {
            ComponentValue::Token(Token::Ident(s)) => Some(s),
            _ => None,
        }
    }

    /// True if this is an identifier equal (ASCII case-insensitively) to `name`.
    pub fn is_ident(&self, name: &str) -> bool {
        self.ident().is_some_and(|s| s.eq_ignore_ascii_case(name))
    }

    pub fn is_comma(&self) -> bool {
        matches!(self, ComponentValue::Token(Token::Comma))
    }

    pub fn is_delim(&self, c: char) -> bool {
        matches!(self, ComponentValue::Token(Token::Delim(d)) if *d == c)
    }

    /// Re-serializes the value to CSS text (used e.g. for custom properties and debugging).
    pub fn to_css(&self) -> String {
        todo!("xbro-css: implement ComponentValue::to_css")
    }
}

/// Serializes a list of component values back to CSS text.
pub fn to_css(values: &[ComponentValue]) -> String {
    values.iter().map(ComponentValue::to_css).collect()
}

/// A `property: value [!important]` declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    /// Lowercased property name, except custom properties (`--foo`) which keep their case.
    pub name: String,
    /// Value with leading/trailing whitespace and the `!important` suffix removed.
    pub value: Vec<ComponentValue>,
    pub important: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    /// Parsed selector list; a rule whose selector list fails to parse is dropped entirely.
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CssRule {
    Style(StyleRule),
    /// `@media <query> { rules }`
    Media {
        query: MediaQueryList,
        rules: Vec<CssRule>,
    },
    /// `@supports <condition> { rules }` — `supported` is the evaluated condition
    /// (true for `(property: value)` with a property xbro knows, `not`, `and`, `or` combinations).
    Supports {
        supported: bool,
        rules: Vec<CssRule>,
    },
    /// `@import url(...) [media]` — `url` is the raw (unresolved) URL string.
    Import {
        url: String,
        media: MediaQueryList,
    },
    /// `@layer name { rules }` / `@container ... { rules }` / `@document`: rules applied unconditionally.
    Group {
        rules: Vec<CssRule>,
    },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stylesheet {
    pub rules: Vec<CssRule>,
}

impl Stylesheet {
    /// Rewrites every URL in the sheet (`url(x)` tokens, `url("x")` functions,
    /// `@import` URLs, including inside nested rules) through `f`. The engine
    /// uses this to make URLs absolute against the stylesheet's own location,
    /// so later stages can use them directly.
    pub fn rewrite_urls(&mut self, f: &mut dyn FnMut(&str) -> String) {
        let _ = f;
        todo!("xbro-css: implement Stylesheet::rewrite_urls")
    }
}

/// Parses a stylesheet. Never fails: invalid constructs are skipped per the
/// CSS error-recovery rules. Unknown at-rules (`@font-face`, `@keyframes`,
/// `@page`, `@charset`, `@namespace`, ...) are dropped.
pub fn parse_stylesheet(css: &str) -> Stylesheet {
    let _ = css;
    todo!("xbro-css: implement parse_stylesheet")
}

/// Parses the contents of a `style="..."` attribute or a `{ ... }` block body.
pub fn parse_declaration_list(css: &str) -> Vec<Declaration> {
    let _ = css;
    todo!("xbro-css: implement parse_declaration_list")
}

/// Parses a list of component values from a string (e.g. for tests or `var()` fallbacks).
pub fn parse_component_values(css: &str) -> Vec<ComponentValue> {
    let _ = css;
    todo!("xbro-css: implement parse_component_values")
}
