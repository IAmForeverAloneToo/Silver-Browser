//! CSS parsing: source text in, stylesheet model out.
//!
//! [`parse_stylesheet`] runs the [`tokenizer`] (a `css-syntax-3` token
//! model) through the [`parser`], producing a [`Stylesheet`] with
//! WHATWG-style error recovery: malformed input degrades, never fails,
//! never panics. Unknown at-rules are preserved structurally (raw block)
//! for forward compatibility, exactly as browsers do. Parsers never
//! perform I/O — resource loading is `net` + `engine`'s job (the
//! layering contract in `docs/architecture.md`).
//!
//! Selectors are not parsed yet: rule preludes are kept as token
//! vectors for the future `style` crate to consume.

pub mod parser;
pub mod tokenizer;

pub use parser::{
    AtRule, Block, Declaration, QualifiedRule, Rule, Stylesheet, parse_style_attribute,
    parse_stylesheet,
};
pub use tokenizer::{Token, Tokenizer, tokenize};

/// Errors produced by CSS parsing.
///
/// CSS parsing always recovers, so [`parse_stylesheet`] never returns
/// `Err` today; the variant is reserved for future strict-mode
/// diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Reserved for future strict-mode diagnostics.
    Reserved,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Reserved => write!(f, "reserved for future strict-mode parsing errors"),
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_to_end_small_sheet() {
        let sheet = parse_stylesheet("body { margin: 0; } @media print { body { color: black; } }")
            .expect("never fails");
        assert_eq!(sheet.rules.len(), 2);
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(rule.declarations.len(), 1);
    }

    #[test]
    fn empty_input_yields_empty_sheet() {
        assert!(parse_stylesheet("").expect("never fails").rules.is_empty());
    }
}
