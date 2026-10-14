//! Generic rule-level syntax (§5.4 "consume a stylesheet's contents",
//! "consume a block's contents", at-rules, qualified rules, declarations).
//! The interpretation of these generic rules into [`super::CssRule`]s lives
//! in `builder`.

use super::{ComponentValue as CV, Declaration};
use crate::tokenizer::Token;

/// A generic at-rule.
pub(crate) struct AtRule {
    pub name: String,
    pub prelude: Vec<CV>,
    pub block: Option<Vec<CV>>,
}

/// A generic qualified rule (prelude + `{}` block contents).
pub(crate) struct QualifiedRule {
    pub prelude: Vec<CV>,
    pub block: Vec<CV>,
}

/// An item of a stylesheet's top level.
pub(crate) enum TopItem {
    At(AtRule),
    Qualified(QualifiedRule),
}

/// An item of a block's contents (declarations mixed with nested rules).
pub(crate) enum BlockItem {
    Decl(Declaration),
    At(AtRule),
    Qualified(QualifiedRule),
}

fn is_token(value: &CV, f: impl Fn(&Token) -> bool) -> bool {
    matches!(value, CV::Token(t) if f(t))
}

fn is_curly_block(value: &CV) -> bool {
    matches!(value, CV::Block { open: '{', .. })
}

/// "consume a stylesheet's contents": the top level, where CDO/CDC are
/// ignored and a qualified rule's prelude runs up to its `{}` block.
pub(crate) fn consume_rule_list(values: Vec<CV>) -> Vec<TopItem> {
    let mut out = Vec::new();
    let mut it = values.into_iter();
    while let Some(value) = it.next() {
        match value {
            CV::Token(Token::Whitespace | Token::Cdo | Token::Cdc) => {}
            CV::Token(Token::AtKeyword(name)) => {
                out.push(TopItem::At(consume_at_rule(name, &mut it, false)));
            }
            CV::Block { open: '{', content } => out.push(TopItem::Qualified(QualifiedRule {
                prelude: Vec::new(),
                block: content,
            })),
            first => {
                let mut prelude = vec![first];
                let mut block = None;
                for value in it.by_ref() {
                    match value {
                        CV::Block { open: '{', content } => {
                            block = Some(content);
                            break;
                        }
                        other => prelude.push(other),
                    }
                }
                // A qualified rule cut by the end of input is dropped.
                if let Some(block) = block {
                    if !looks_like_custom_property(&prelude) {
                        out.push(TopItem::Qualified(QualifiedRule { prelude, block }));
                    }
                }
            }
        }
    }
    out
}

/// A prelude starting with `--name:` is a custom property in disguise; the
/// rule is consumed and dropped.
fn looks_like_custom_property(prelude: &[CV]) -> bool {
    let mut non_ws = prelude.iter().filter(|v| !v.is_whitespace());
    let first = non_ws.next();
    let second = non_ws.next();
    first
        .and_then(CV::ident)
        .is_some_and(|s| s.starts_with("--"))
        && second.is_some_and(|v| is_token(v, |t| matches!(t, Token::Colon)))
}

/// "consume an at-rule": the prelude runs up to `;` or a `{}` block. In a
/// nested context a stray `}` also ends the rule (without being consumed).
fn consume_at_rule(name: String, it: &mut std::vec::IntoIter<CV>, nested: bool) -> AtRule {
    let mut prelude = Vec::new();
    let mut block = None;
    loop {
        if nested
            && it
                .as_slice()
                .first()
                .is_some_and(|v| is_token(v, |t| matches!(t, Token::CloseCurly)))
        {
            break;
        }
        match it.next() {
            None | Some(CV::Token(Token::Semicolon)) => break,
            Some(CV::Block { open: '{', content }) => {
                block = Some(content);
                break;
            }
            Some(other) => prelude.push(other),
        }
    }
    AtRule {
        name,
        prelude,
        block,
    }
}

fn ends_item(value: &CV) -> bool {
    is_token(value, |t| matches!(t, Token::Semicolon | Token::CloseCurly))
}

/// "consume a block's contents": declarations and nested rules, in order.
///
/// Runs in linear time: a candidate declaration is scanned only until it is
/// known to be invalid (at the latest just after its first `{}` block), and
/// a qualified rule ends at its first `{}` block, so no part of the input is
/// scanned more than a bounded number of times.
pub(crate) fn consume_block_contents(values: Vec<CV>) -> Vec<BlockItem> {
    let mut out = Vec::new();
    let mut it = values.into_iter();
    loop {
        let rest = it.as_slice();
        let Some(first) = rest.first() else { break };
        if is_token(first, |t| matches!(t, Token::Whitespace | Token::Semicolon)) {
            it.next();
            continue;
        }
        if is_token(first, |t| matches!(t, Token::CloseCurly)) {
            break;
        }
        if is_token(first, |t| matches!(t, Token::AtKeyword(_))) {
            if let Some(CV::Token(Token::AtKeyword(name))) = it.next() {
                out.push(BlockItem::At(consume_at_rule(name, &mut it, true)));
            }
            continue;
        }
        if let Some((colon, end)) = scan_declaration(rest) {
            let items: Vec<CV> = it.by_ref().take(end).collect();
            if let Some(decl) = build_declaration(items, colon) {
                out.push(BlockItem::Decl(decl));
            }
            continue;
        }
        // Not a declaration: a nested qualified rule ending at its `{}` block,
        // or garbage up to the next `;` (or stray `}`).
        let stop = rest
            .iter()
            .position(|v| is_curly_block(v) || ends_item(v))
            .unwrap_or(rest.len());
        if rest.get(stop).is_some_and(is_curly_block) {
            let prelude: Vec<CV> = it.by_ref().take(stop).collect();
            if let Some(CV::Block { content, .. }) = it.next() {
                if !looks_like_custom_property(&prelude) {
                    out.push(BlockItem::Qualified(QualifiedRule {
                        prelude,
                        block: content,
                    }));
                }
            }
        } else {
            it.by_ref().take(stop).for_each(drop);
        }
    }
    out
}

/// "consume a declaration" on `items`: if they start with a valid
/// declaration (`ident ws* : value`, the value running up to the next
/// top-level `;` or stray `}`), returns the index of its colon and the
/// declaration's length.
///
/// A top-level `{}` block is only allowed as the entire value of a
/// non-custom property (besides a trailing `!important`); the scan stops as
/// soon as that rule is broken, so `a:hover { ... }` rules are recognized
/// without scanning past their block.
fn scan_declaration(items: &[CV]) -> Option<(usize, usize)> {
    let name = items.first()?.ident()?;
    let mut i = 1;
    while items.get(i).is_some_and(CV::is_whitespace) {
        i += 1;
    }
    if !items
        .get(i)
        .is_some_and(|v| is_token(v, |t| matches!(t, Token::Colon)))
    {
        return None;
    }
    let colon = i;
    let custom = name.starts_with("--");
    let mut saw_block = false;
    let mut saw_other = false;
    let mut j = colon + 1;
    while let Some(item) = items.get(j) {
        if ends_item(item) {
            break;
        }
        if !custom && !item.is_whitespace() {
            if is_curly_block(item) {
                if saw_block || saw_other {
                    return None;
                }
                saw_block = true;
            } else if saw_block {
                // Only `! important` may follow the block.
                let end = trailing_important_end(items, j)?;
                return Some((colon, end));
            } else {
                saw_other = true;
            }
        }
        j += 1;
    }
    Some((colon, j))
}

/// If `items[start..]` is `! ws* important ws*` up to the end of the
/// declaration (`;`, stray `}` or end of input), returns that end's index.
fn trailing_important_end(items: &[CV], start: usize) -> Option<usize> {
    if !items.get(start)?.is_delim('!') {
        return None;
    }
    let mut j = start + 1;
    while items.get(j).is_some_and(CV::is_whitespace) {
        j += 1;
    }
    if !items.get(j)?.is_ident("important") {
        return None;
    }
    j += 1;
    while items.get(j).is_some_and(CV::is_whitespace) {
        j += 1;
    }
    match items.get(j) {
        None => Some(j),
        Some(item) if ends_item(item) => Some(j),
        Some(_) => None,
    }
}

/// If the last two non-whitespace values are `!` and `important` (ASCII
/// case-insensitive), returns the index of the `!`.
fn important_start(value: &[CV]) -> Option<usize> {
    let last = value.iter().rposition(|v| !v.is_whitespace())?;
    if !value[last].is_ident("important") {
        return None;
    }
    let bang = value[..last].iter().rposition(|v| !v.is_whitespace())?;
    value[bang].is_delim('!').then_some(bang)
}

/// Builds a declaration from its items (`name ws* : value`).
fn build_declaration(items: Vec<CV>, colon: usize) -> Option<Declaration> {
    let mut it = items.into_iter();
    let name = match it.next() {
        Some(CV::Token(Token::Ident(name))) => name,
        _ => return None,
    };
    let mut value: Vec<CV> = it.skip(colon).collect();
    let leading = value.iter().take_while(|v| v.is_whitespace()).count();
    value.drain(..leading);
    let important = match important_start(&value) {
        Some(bang) => {
            value.truncate(bang);
            true
        }
        None => false,
    };
    while value.last().is_some_and(CV::is_whitespace) {
        value.pop();
    }
    let name = if name.starts_with("--") {
        name
    } else {
        name.to_ascii_lowercase()
    };
    Some(Declaration {
        name,
        value,
        important,
    })
}

/// Parses a declaration list (style attribute), ignoring nested rules.
pub(crate) fn declarations_only(values: Vec<CV>) -> Vec<Declaration> {
    consume_block_contents(values)
        .into_iter()
        .filter_map(|item| match item {
            BlockItem::Decl(d) => Some(d),
            _ => None,
        })
        .collect()
}
