//! Selector model (selectors-4 subset).
//!
//! Supported:
//!
//! * type (`div`), universal (`*`), id (`#x`), class (`.x`) selectors
//! * all attribute operators (`[a]`, `=`, `~=`, `|=`, `^=`, `$=`, `*=`,
//!   plus the `i`/`s` case-sensitivity flags)
//! * all four combinators (descendant, `>`, `+`, `~`)
//! * structural pseudo-classes: `:first-child`, `:last-child`,
//!   `:only-child`, `:first-of-type`, `:last-of-type`, `:only-of-type`,
//!   `:empty`, `:root`, and the `:nth-*` family with the full An+B
//!   grammar (including `of S`)
//! * logical combinators: `:not()`, `:is()`, `:where()`
//! * dynamic pseudo-classes (`:hover`, `:focus`, `:active`, `:visited`,
//!   `:link`) parse but never match until input events land (M26)
//!
//! Not supported — hard parse errors, so the containing rule is dropped
//! exactly as css-syntax-3 prescribes: pseudo-elements (M16),
//! namespaces (`|`), and anything unknown.

mod flat_dom;
mod matcher;
mod parse;

pub use flat_dom::{FlatChild, FlatDom, FlatNode};
pub use matcher::{Matcher, matches_selector, matches_selector_list};
pub use parse::parse_selector_list;

/// A comma-separated selector list.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectorList(pub Vec<Selector>);

/// One complex selector: compounds joined by combinators, in source
/// order (`compounds.len() == combinators.len() + 1`). Matching walks
/// right-to-left, per selectors-4.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    /// Compound selectors in source order.
    pub compounds: Vec<CompoundSelector>,
    /// `combinators[i]` joins `compounds[i]` and `compounds[i+1]`.
    pub combinators: Vec<Combinator>,
    /// Pre-computed at parse time (selectors-4 §17).
    pub specificity: Specificity,
}

/// How two compound selectors relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// `A B` — B is a descendant of A.
    Descendant,
    /// `A > B` — B is a child of A.
    Child,
    /// `A + B` — B immediately follows A among element siblings.
    NextSibling,
    /// `A ~ B` — B follows A among element siblings.
    LaterSibling,
}

/// A sequence of simple selectors with no combinator between them.
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundSelector {
    /// Simple selectors in source order.
    pub simple: Vec<SimpleSelector>,
}

/// One simple selector.
#[derive(Debug, Clone, PartialEq)]
pub enum SimpleSelector {
    /// A type selector (tag name, lowercased at parse time).
    Type(String),
    /// `*`
    Universal,
    /// `#id`
    Id(String),
    /// `.class`
    Class(String),
    /// An attribute selector.
    Attribute {
        /// Attribute name (lowercased).
        name: String,
        /// The comparison.
        op: AttrOp,
        /// The comparison value (empty for `Exists`).
        value: String,
        /// ASCII case-insensitive comparison (`i` flag).
        case_insensitive: bool,
    },
    /// A pseudo-class.
    PseudoClass(PseudoClass),
}

/// Attribute selector operators (selectors-4 §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    /// `[attr]`
    Exists,
    /// `[attr=v]`
    Equals,
    /// `[attr~=v]` — whitespace-separated word match.
    IncludeSpace,
    /// `[attr|=v]` — the value or a `value-` prefix.
    Dash,
    /// `[attr^=v]`
    Prefix,
    /// `[attr$=v]`
    Suffix,
    /// `[attr*=v]`
    Substring,
}

/// Pseudo-classes (the M4 subset).
#[derive(Debug, Clone, PartialEq)]
pub enum PseudoClass {
    /// `:first-child`
    FirstChild,
    /// `:last-child`
    LastChild,
    /// `:only-child`
    OnlyChild,
    /// `:first-of-type`
    FirstOfType,
    /// `:last-of-type`
    LastOfType,
    /// `:only-of-type`
    OnlyOfType,
    /// `:empty` — no element children and no text children at all
    /// (per selectors-4, whitespace text makes an element non-empty).
    Empty,
    /// `:root`
    Root,
    /// `:nth-child(An+B [of S])`
    NthChild {
        /// The `A` in An+B.
        a: i32,
        /// The `B` in An+B.
        b: i32,
        /// The `of S` filter (selectors-4).
        of: Option<Box<SelectorList>>,
    },
    /// `:nth-last-child(An+B)`
    NthLastChild {
        /// The `A` in An+B.
        a: i32,
        /// The `B` in An+B.
        b: i32,
    },
    /// `:nth-of-type(An+B)`
    NthOfType {
        /// The `A` in An+B.
        a: i32,
        /// The `B` in An+B.
        b: i32,
    },
    /// `:nth-last-of-type(An+B)`
    NthLastOfType {
        /// The `A` in An+B.
        a: i32,
        /// The `B` in An+B.
        b: i32,
    },
    /// `:not(list)`
    Not(SelectorList),
    /// `:is(list)`
    Is(SelectorList),
    /// `:where(list)` — matches like `:is`, zero specificity.
    Where(SelectorList),
    /// Dynamic: parseable, never matched until M26 wires input state.
    Hover,
    /// Dynamic, see [`PseudoClass::Hover`].
    Focus,
    /// Dynamic, see [`PseudoClass::Hover`].
    Active,
    /// Dynamic, see [`PseudoClass::Hover`].
    Visited,
    /// Dynamic, see [`PseudoClass::Hover`].
    Link,
}

/// selectors-4 specificity: (ids, classes+attributes+pseudo-classes,
/// types). Ordered by the derive: lexicographic comparison is the spec
/// ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Specificity(pub u32, pub u32, pub u32);

impl std::ops::Add for Specificity {
    type Output = Specificity;
    fn add(self, rhs: Specificity) -> Specificity {
        Specificity(self.0 + rhs.0, self.1 + rhs.1, self.2 + rhs.2)
    }
}

impl std::ops::AddAssign for Specificity {
    fn add_assign(&mut self, rhs: Specificity) {
        *self = *self + rhs;
    }
}
