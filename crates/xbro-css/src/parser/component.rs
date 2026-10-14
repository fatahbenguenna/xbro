//! Grouping tokens into component values (§5.4.7 "consume a component value",
//! §5.4.8 "consume a simple block", §5.4.9 "consume a function").
//!
//! The tree is built iteratively; blocks nested deeper than
//! [`MAX_NESTING_DEPTH`] are discarded so that every later (recursive) stage
//! works on a tree of bounded depth.

use super::ComponentValue;
use crate::tokenizer::{Token, Tokenizer};

/// Maximum nesting depth of blocks and functions kept in a component value
/// tree. Deeper blocks are dropped (with their contents).
pub const MAX_NESTING_DEPTH: usize = 64;

enum Opening {
    Function(String),
    Block(char),
}

struct Frame {
    opening: Opening,
    close: char,
    content: Vec<ComponentValue>,
}

impl Frame {
    fn into_value(self) -> ComponentValue {
        match self.opening {
            Opening::Function(name) => ComponentValue::Function {
                name,
                args: self.content,
            },
            Opening::Block(open) => ComponentValue::Block {
                open,
                content: self.content,
            },
        }
    }
}

/// The closing character expected for a token that opens a block or function.
fn closer_of(token: &Token) -> Option<char> {
    match token {
        Token::Function(_) | Token::OpenParen => Some(')'),
        Token::OpenSquare => Some(']'),
        Token::OpenCurly => Some('}'),
        _ => None,
    }
}

fn closing_char(token: &Token) -> Option<char> {
    match token {
        Token::CloseParen => Some(')'),
        Token::CloseSquare => Some(']'),
        Token::CloseCurly => Some('}'),
        _ => None,
    }
}

/// Tokenizes `css` and groups the tokens into component values.
pub(crate) fn parse(css: &str) -> Vec<ComponentValue> {
    build(Tokenizer::new(css))
}

/// Groups tokens into component values. Unclosed blocks and functions are
/// closed at the end of input; unmatched closing tokens are kept as tokens.
pub(crate) fn build<I: IntoIterator<Item = Token>>(tokens: I) -> Vec<ComponentValue> {
    let mut root: Vec<ComponentValue> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    // Expected closers of the over-deep subtree currently being discarded.
    let mut discarding: Vec<char> = Vec::new();

    for token in tokens {
        if !discarding.is_empty() {
            if let Some(close) = closer_of(&token) {
                discarding.push(close);
            } else if closing_char(&token).is_some()
                && closing_char(&token) == discarding.last().copied()
            {
                discarding.pop();
            }
            continue;
        }
        if let Some(close) = closer_of(&token) {
            if stack.len() >= MAX_NESTING_DEPTH {
                discarding.push(close);
                continue;
            }
            let opening = match token {
                Token::Function(name) => Opening::Function(name),
                Token::OpenSquare => Opening::Block('['),
                Token::OpenCurly => Opening::Block('{'),
                _ => Opening::Block('('),
            };
            stack.push(Frame {
                opening,
                close,
                content: Vec::new(),
            });
            continue;
        }
        if let Some(c) = closing_char(&token) {
            if stack.last().is_some_and(|f| f.close == c) {
                if let Some(frame) = stack.pop() {
                    push(&mut stack, &mut root, frame.into_value());
                }
                continue;
            }
        }
        push(&mut stack, &mut root, ComponentValue::Token(token));
    }
    while let Some(frame) = stack.pop() {
        push(&mut stack, &mut root, frame.into_value());
    }
    root
}

fn push(stack: &mut [Frame], root: &mut Vec<ComponentValue>, value: ComponentValue) {
    match stack.last_mut() {
        Some(frame) => frame.content.push(value),
        None => root.push(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(values: &[ComponentValue]) -> usize {
        values
            .iter()
            .map(|v| match v {
                ComponentValue::Token(_) => 0,
                ComponentValue::Function { args, .. } => 1 + depth(args),
                ComponentValue::Block { content, .. } => 1 + depth(content),
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn groups_blocks_and_functions() {
        let values = parse("a(b [c] {d}) )");
        assert_eq!(values.len(), 3);
        match &values[0] {
            ComponentValue::Function { name, args } => {
                assert_eq!(name, "a");
                assert_eq!(args.len(), 5);
                assert!(matches!(&args[2], ComponentValue::Block { open: '[', .. }));
                assert!(matches!(&args[4], ComponentValue::Block { open: '{', .. }));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(values[2], ComponentValue::Token(Token::CloseParen));
    }

    #[test]
    fn mismatched_closers_are_tokens() {
        let values = parse("[a)]");
        assert_eq!(
            values,
            vec![ComponentValue::Block {
                open: '[',
                content: vec![
                    ComponentValue::Token(Token::Ident("a".into())),
                    ComponentValue::Token(Token::CloseParen)
                ]
            }]
        );
    }

    #[test]
    fn unclosed_blocks_close_at_eof() {
        let values = parse("{a(b");
        assert_eq!(depth(&values), 2);
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let css = format!("{}x{}", "(".repeat(100_000), ")".repeat(100_000));
        let values = parse(&css);
        assert_eq!(values.len(), 1);
        assert_eq!(depth(&values), MAX_NESTING_DEPTH);
        // Content after an over-deep block is kept at the right level.
        let css = format!("a{{{}}}b", "[".repeat(200) + &"]".repeat(200));
        let values = parse(&css);
        assert_eq!(values.len(), 3);
        assert_eq!(values[2], ComponentValue::Token(Token::Ident("b".into())));
        // Unclosed deep input does not overflow either.
        let values = parse(&"{(".repeat(50_000));
        assert_eq!(depth(&values), MAX_NESTING_DEPTH);
    }
}
