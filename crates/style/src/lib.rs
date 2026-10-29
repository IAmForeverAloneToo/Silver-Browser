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

#![forbid(unsafe_code)]

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

/// The timing model of Web Animations (M22): an animation effect's
/// phase, iteration and progress at a time.
pub mod animation;

/// Animating the properties (M22): their animation types, and their
/// computed values copied, compared and interpolated.
pub mod animate;

/// Math functions (M21.2): `calc()`, `min()`, `max()` and `clamp()`,
/// parsed, typed and simplified.
pub mod calc;

/// The cascade: origin sorting, specificity, inheritance, and the
/// built-in UA stylesheet. See the module docs for exact v1 scope
/// (which properties, which value syntaxes) and its documented gaps.
pub mod cascade;

/// Colour spaces past CSS Color 3 (gap batch E, D48): conversions,
/// interpolation, `color-mix()` and serialization.
pub mod color;

/// Computed values resolved for one element (the cascade's output).
pub mod computed;

/// `@supports` conditions (M20): parsed from tokens and decided by what
/// this engine supports, `selector()` included.
pub mod conditions;

/// Counter styles (M21.4): a counter's value written in a style, with
/// Counter Styles 3 §6's predefined styles.
pub mod counter_style;

/// Custom properties and the substitution functions `var()`, `env()`
/// and `attr()` (M21.1, M21.3, M21.5): their values checked, computed and
/// substituted, within bounds.
pub mod custom;

/// Easing functions (M22): the computed values of the timing functions
/// of transitions and animations, and the progress each gives.
pub mod easing;

/// Style invalidation (M13.1): which elements a change to a document
/// can restyle.
pub mod invalidation;

/// `@keyframes` rules (M22): their names, selectors and keyframes,
/// processed apart from any element.
mod keyframes;

/// Media queries (M20): the environment a document is styled for, and
/// query lists parsed from tokens and evaluated against it.
pub mod media;

/// The page box (M20): `@page` rules and what they say of a page.
pub mod page;

/// The style rules of a set of sheets with the `@media` blocks and the
/// cascade layers they are in (M20).
mod rule_set;

/// Selector matching: does a selector match an element?
///
/// The selectors-4 subset this engine supports, a right-to-left matcher
/// over a [`dom::Document`]'s elements, and selector-list parsing over
/// `css::Token` slices. See the module docs for the exact supported
/// grammar and its documented omissions.
pub mod selector_matching;

/// Computed values as text (M25): what a script's `getComputedStyle`
/// reads, for the properties this engine computes.
pub mod serialize;

/// The transformation matrix (M19): a four-by-four matrix, the matrix of
/// each transform function, and the arithmetic over it that layout,
/// paint and hit testing share.
pub mod transform;

/// SVG's transform list (M32): the grammar of the `transform`,
/// `gradientTransform` and `patternTransform` attributes, as a matrix.
pub mod svg_transform;

pub use selector_matching::{matches_selector, matches_selector_list, parse_selector_list};

pub use cascade::{
    CascadeContext, Composited, FontMeasure, InlineStyles, MatchedDeclaration, MatchedRule,
    PseudoStyles, Recascade, Recascader, Restyle, RuleOrigin, Styles, Stylesheets,
    cascade_document, parse_color_text, property_name, supports,
};
pub use computed::ComputedStyle;
pub use conditions::supports_condition;
pub use custom::CustomProperties;
pub use invalidation::SelectorFeatures;
pub use media::{
    ColorScheme, Media, MediaQueryList, MediaType, Preferences, parse_media_query_list,
    serialize_media_query_list,
};
pub use page::{Orientation, PageBox, PagePosition, PageSize, PageStyle};
pub use rule_set::is_layer_name;
