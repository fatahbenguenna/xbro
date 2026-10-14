//! URL rewriting over a parsed stylesheet ([`super::Stylesheet::rewrite_urls`]).

use super::{ComponentValue as CV, CssRule};
use crate::tokenizer::Token;

pub(crate) fn rewrite_rules(rules: &mut [CssRule], f: &mut dyn FnMut(&str) -> String) {
    for rule in rules {
        match rule {
            CssRule::Style(style) => {
                for decl in &mut style.declarations {
                    rewrite_values(&mut decl.value, f);
                }
            }
            CssRule::Media { rules, .. }
            | CssRule::Supports { rules, .. }
            | CssRule::Group { rules } => rewrite_rules(rules, f),
            CssRule::Import { url, .. } => rewrite(url, f),
        }
    }
}

fn rewrite(url: &mut String, f: &mut dyn FnMut(&str) -> String) {
    if !url.is_empty() {
        *url = f(url);
    }
}

/// Rewrites URLs inside component values: `url(x)` tokens, the string
/// argument of `url("x")` / `src("x")`, and the string arguments of
/// `image-set()`; recurses into functions and blocks.
pub(crate) fn rewrite_values(values: &mut [CV], f: &mut dyn FnMut(&str) -> String) {
    for value in values {
        match value {
            CV::Token(Token::Url(url)) => rewrite(url, f),
            CV::Token(_) => {}
            CV::Function { name, args } => {
                let lower = name.to_ascii_lowercase();
                match lower.as_str() {
                    "url" | "src" => {
                        if let Some(CV::Token(Token::String(url))) =
                            args.iter_mut().find(|v| !v.is_whitespace())
                        {
                            rewrite(url, f);
                        }
                    }
                    "image-set" | "-webkit-image-set" => {
                        for arg in args.iter_mut() {
                            if let CV::Token(Token::String(url)) = arg {
                                rewrite(url, f);
                            }
                        }
                        rewrite_values(args, f);
                    }
                    _ => rewrite_values(args, f),
                }
            }
            CV::Block { content, .. } => rewrite_values(content, f),
        }
    }
}
