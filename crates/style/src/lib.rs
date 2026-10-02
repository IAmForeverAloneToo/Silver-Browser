//! Style: selector matching, the cascade, and computed values.
//!
//! [`selector_matching`] (M4) parses selector-list preludes from
//! `css::Token` slices and matches them against the elements of a
//! [`dom::Document`]. [`cascade`] (M5) uses it to turn a document plus
//! parsed stylesheets into [`Styles`]: every element's
//! [`computed::ComputedStyle`] by its [`dom::NodeId`] (M13), cascaded
//! from the built-in UA stylesheet, the given author stylesheets, and
//! inline `style` attributes, with inheritance resolved. `layout` (M6)
//! builds its box tree from the document and those styles.

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

/// Style invalidation (M13.1): which elements a change to a document
/// can restyle.
pub mod invalidation;

/// Selector matching: does a selector match an element?
///
/// The selectors-4 subset this engine supports, a right-to-left matcher
/// over a [`dom::Document`]'s elements, and selector-list parsing over
/// `css::Token` slices. See the module docs for the exact supported
/// grammar and its documented omissions.
pub mod selector_matching;

pub use selector_matching::{matches_selector, matches_selector_list, parse_selector_list};

pub use cascade::{CascadeContext, FontMeasure, Restyle, Styles, Stylesheets, cascade_document};
pub use computed::ComputedStyle;
pub use invalidation::SelectorFeatures;
