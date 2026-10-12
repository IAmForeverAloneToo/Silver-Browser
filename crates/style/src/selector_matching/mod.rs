//! Selector model (selectors-4 subset).
//!
//! Supported:
//!
//! * type (`div`), universal (`*`), id (`#x`), class (`.x`) selectors
//! * all attribute operators (`[a]`, `=`, `~=`, `|=`, `^=`, `$=`, `*=`,
//!   plus the `i`/`s` case-sensitivity flags), with HTML's rule that
//!   the values of some attributes compare ASCII case-insensitively
//! * all four combinators (descendant, `>`, `+`, `~`)
//! * structural pseudo-classes: `:first-child`, `:last-child`,
//!   `:only-child`, `:first-of-type`, `:last-of-type`, `:only-of-type`,
//!   `:empty`, `:root`, and the `:nth-*` family with the full An+B
//!   grammar (including `of S`)
//! * logical combinators: `:not()`, `:is()`, `:where()`
//! * `:lang()`, by the language HTML gives an element and RFC 4647's
//!   extended filtering
//! * `:link` and `:any-link`, on `a` and `area` elements with an `href`
//! * the form pseudo-classes `:disabled`, `:enabled`, `:checked`,
//!   `:required` and `:optional`, by the state HTML gives a control
//!   from its attributes, since no control has state of its own yet
//! * pseudo-classes that parse and never match: the user-action ones
//!   (`:hover`, `:focus`, `:focus-visible`, `:focus-within`,
//!   `:active`) until input events land (M26), `:visited`, since there
//!   is no history to read, and `:target`
//! * one pseudo-element at the end of a selector (M16,
//!   docs/design/selectors-pseudo.md): `::before`, `::after`,
//!   `::first-letter` and `::first-line`, also with one colon;
//!   `::marker`, `::placeholder` and `::selection`; and any
//!   `::-webkit-` name that is not a function, which never matches
//!   (selectors-4 Appendix B). Only the user-action pseudo-classes may
//!   follow it.
//!
//! Names (types, attributes, pseudo-classes and pseudo-elements) are
//! compared in ASCII lowercase, as HTML and selectors-4 say.
//!
//! Not supported — hard parse errors, so the containing rule is dropped
//! exactly as css-syntax-3 prescribes: other pseudo-elements, a
//! pseudo-element inside `:not()`, `:is()` or `:where()` (which the
//! last two forgive), namespaces (`|`), and anything unknown.

mod language;
mod matcher;
mod parse;

pub use matcher::{Matcher, matches_selector, matches_selector_list};
pub use parse::{parse_selector_list, parse_selector_list_in, supports_selector};

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
    /// The pseudo-element the selector ends in (`p::before`), if any.
    /// The compounds then match its *originating element*, and the
    /// selector represents that element's pseudo-element, not the
    /// element: it never matches an element itself.
    pub pseudo_element: Option<PseudoElementSelector>,
    /// Pre-computed at parse time (selectors-4 §17).
    pub specificity: Specificity,
}

/// A pseudo-element at the end of a selector, with the pseudo-classes
/// that follow it (`::before:hover`).
#[derive(Debug, Clone, PartialEq)]
pub struct PseudoElementSelector {
    /// Which pseudo-element.
    pub element: PseudoElement,
    /// The pseudo-classes after it, in source order: the user-action
    /// ones (`:hover`, `:focus`, `:focus-visible`, `:focus-within`,
    /// `:active`), and `:is()`, `:where()` and `:not()` of them, as
    /// selectors-4 §3.6.3 allows. No pseudo-element is in a user-action
    /// state until M26.4, and they are matched so.
    pub states: Vec<PseudoClass>,
}

/// The pseudo-elements a selector may end in (Pseudo-Elements 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    /// `::before` (or `:before`)
    Before,
    /// `::after` (or `:after`)
    After,
    /// `::first-letter` (or `:first-letter`)
    FirstLetter,
    /// `::first-line` (or `:first-line`)
    FirstLine,
    /// `::marker`: a list item's marker (M21.6).
    Marker,
    /// `::placeholder`: recognised; no control draws placeholder text
    /// yet.
    Placeholder,
    /// `::selection`: recognised; there is no selection yet (M26).
    Selection,
    /// A `::-webkit-` name that is not a function: valid, so that the
    /// list it is in keeps its other selectors, and matching nothing
    /// (selectors-4 Appendix B).
    WebkitPrefixed,
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
        /// Attribute name (ASCII lowercased).
        name: String,
        /// The comparison.
        op: AttrOp,
        /// The comparison value (empty for `Exists`).
        value: String,
        /// How the value compares: the flag, or HTML's rule without one.
        case: CaseFlag,
    },
    /// A pseudo-class.
    PseudoClass(PseudoClass),
    /// The element's namespace: a type or universal selector's prefix
    /// (`svg|rect`, `|x` for none), or the sheet's default namespace
    /// (CSS Namespaces 3, M32). It adds no specificity.
    Namespace(dom::Namespace),
}

/// A style sheet's namespace declarations (`@namespace`, CSS Namespaces
/// 3): its default namespace and its prefixes, which its selectors are
/// read with (M32).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Namespaces {
    /// The default namespace, if one is declared.
    pub default: Option<dom::Namespace>,
    /// Each prefix and its namespace, the last declaration of a prefix
    /// winning.
    pub prefixes: Vec<(String, dom::Namespace)>,
}

impl Namespaces {
    /// The namespace `prefix` names here, if declared.
    pub fn prefix(&self, prefix: &str) -> Option<&dom::Namespace> {
        self.prefixes
            .iter()
            .rev()
            .find(|(name, _)| name == prefix)
            .map(|(_, ns)| ns)
    }
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

/// An attribute selector's case-sensitivity flag (selectors-4 §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseFlag {
    /// No flag: the value compares as the document language says. In
    /// HTML, ASCII case-insensitively for the attributes it lists
    /// (`type`, `lang`, `rel` …), identically for every other.
    Unset,
    /// `i`: ASCII case-insensitively, whatever the attribute.
    I,
    /// `s`: identically, whatever the attribute.
    S,
}

/// Pseudo-classes (the M4 subset, and M16's).
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
    /// `:scope`: the element a query is made of (`querySelector`'s, M25),
    /// or the root element where there is none, as in a style rule.
    Scope,
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
    FocusVisible,
    /// Dynamic, see [`PseudoClass::Hover`].
    FocusWithin,
    /// Dynamic, see [`PseudoClass::Hover`].
    Active,
    /// `:visited`: never matched, there being no history; that also
    /// keeps a page from reading history through styles.
    Visited,
    /// `:link`: an `a` or `area` element with an `href`.
    Link,
    /// `:any-link`: as `:link`, every link being unvisited.
    AnyLink,
    /// `:target`: parseable, never matched until the engine keeps the
    /// part a fragment indicates.
    Target,
    /// `:lang(…)`: the language ranges, ASCII lowercased, that can
    /// match anything. A range that is not well-formed matches nothing
    /// and is left out when parsed, so a list of none matches nothing.
    Lang(Vec<String>),
    /// `:disabled`: a form element HTML calls actually disabled.
    Disabled,
    /// `:enabled`: a form element HTML can disable that is not.
    Enabled,
    /// `:checked`: a checked checkbox or radio button, or a selected
    /// option.
    Checked,
    /// `:indeterminate`: a checkbox whose indeterminate flag is set, a
    /// radio button none of whose group is checked, a `progress` with no
    /// value (M26.7).
    Indeterminate,
    /// `:placeholder-shown`: a text field or textarea showing its
    /// placeholder (M26.7).
    PlaceholderShown,
    /// `:valid` and `:invalid` (M26.9b): a control that is a candidate
    /// for constraint validation and passes or fails its constraints, a
    /// form or fieldset by its controls.
    Valid,
    Invalid,
    /// `:user-valid` and `:user-invalid`: the same, once the user has
    /// committed a change to the control or tried to submit its form.
    UserValid,
    UserInvalid,
    /// `:in-range` and `:out-of-range`: a number or range input with
    /// range limitations, within them or not.
    InRange,
    OutOfRange,
    /// `:read-only` and `:read-write`: whether the user can edit it.
    ReadOnly,
    ReadWrite,
    /// `:default`: a checkbox or radio button checked by its attribute,
    /// an option selected by its, a form's default button.
    Default,
    /// `:required`: an `input`, `select` or `textarea` that is required.
    Required,
    /// `:optional`: one of those that could be and is not.
    Optional,
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
