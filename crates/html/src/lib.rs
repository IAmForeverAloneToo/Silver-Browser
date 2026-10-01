//! HTML parsing: source text in, DOM tree out.
//!
//! [`parse_document`] runs the [`tokenizer`] (a pragmatic WHATWG subset)
//! into the [`tree_builder`], producing a `dom::Node` tree. HTML parsing
//! is defined to always recover, so the parse never panics and currently
//! never fails; the `Result` keeps the layering contract (parsers return
//! `Result`, never panic) and leaves room for strict-mode diagnostics.
//! Parsers here never perform I/O — resource loading is `net` +
//! `engine`'s job.

pub mod tokenizer;
pub mod tree_builder;

pub use tokenizer::{Token, Tokenizer, tokenize};
pub use tree_builder::TreeBuilder;

use dom::Node;

/// Errors produced by HTML parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Never produced today: HTML parsing always recovers. Reserved for
    /// future strict-mode diagnostics.
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

/// Parses an HTML document into a DOM tree.
///
/// Any input yields a tree: unclosed elements close at EOF, unmatched
/// end tags are dropped, and malformed markup degrades gracefully (see
/// the [`tokenizer`] and [`tree_builder`] docs for the exact subset).
pub fn parse_document(input: &str) -> Result<Node, ParseError> {
    let mut tokenizer = Tokenizer::new(input);
    let mut builder = TreeBuilder::new();
    while let Some(token) = tokenizer.next_token() {
        builder.process(token);
    }
    Ok(builder.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_small_document() {
        let doc = parse_document(
            "<html><body><h1>Hello from Random Browser</h1><p>A hobby browser engine in Rust.</p></body></html>",
        )
        .expect("parsing never fails");
        assert_eq!(
            doc.text_content(),
            "Hello from Random BrowserA hobby browser engine in Rust."
        );
    }

    #[test]
    fn empty_input_yields_an_empty_document() {
        let doc = parse_document("").expect("parsing never fails");
        assert_eq!(doc.text_content(), "");
    }

    #[test]
    fn formatting_whitespace_survives_as_text() {
        let doc = parse_document("<p>a</p>\n<p>b</p>").expect("parsing never fails");
        assert_eq!(doc.text_content(), "a\nb");
    }
}
