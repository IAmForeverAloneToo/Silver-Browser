//! Style: selector matching, the cascade, and computed values.
//!
//! [`selector_matching`] (M4) parses selector-list preludes from
//! `css::Token` slices and matches them against a `dom::Node` tree.
//! [`cascade`] (M5) uses it to turn a `dom::Node` tree plus parsed
//! stylesheets into a [`cascade::StyledDom`]: every element's
//! [`computed::ComputedStyle`], cascaded from the built-in UA
//! stylesheet, the given author stylesheets, and inline `style`
//! attributes, with inheritance resolved. `layout` (M6) is the next
//! consumer down the pipeline — for now `paint` reads `StyledDom`
//! directly (see `paint::paint_styled_document`).

use std::fmt;

/// Errors produced by style parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The selector, or one selector in a comma-separated list, could not
    /// be parsed. Per css-syntax-3 an invalid selector invalidates the
    /// whole list it appears in; the *caller* (the future cascade, M5)
    /// drops the rule that owned it, never the whole stylesheet — this
    /// crate only reports the parse failure.
    InvalidSelector,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidSelector => write!(f, "invalid selector"),
        }
    }
}

impl std::error::Error for ParseError {}

/// The cascade: origin sorting, specificity, inheritance, and the
/// built-in UA stylesheet. See the module docs for exact v1 scope
/// (which properties, which value syntaxes) and its documented gaps.
pub mod cascade;

/// Computed values resolved for one element (the cascade's output).
pub mod computed;

/// Selector matching: does a selector match an element?
///
/// The selectors-4 subset this engine supports, a right-to-left matcher
/// against a flattened DOM view, and selector-list parsing over
/// `css::Token` slices. See the module docs for the exact supported
/// grammar and its documented omissions.
pub mod selector_matching;

pub use selector_matching::{matches_selector, matches_selector_list, parse_selector_list, FlatDom};

pub use cascade::{style_document, StyledDom};
pub use computed::ComputedStyle;
