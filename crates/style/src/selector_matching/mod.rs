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
//!   `::marker`, `::placeholder`, `::selection` and `::backdrop`; and any
//!   `::-webkit-` name that is not a function, which never matches
//!   (selectors-4 Appendix B). Only the user-action pseudo-classes may
//!   follow it.
//! * CSS Scoping 1's (gap batch H, D69): `:host`, `:host()` and
//!   `:host-context()`, which match a shadow host from its shadow tree's
//!   rules alone, the host featureless there but for them; `::slotted()`
//!   of a compound, a slot's assigned elements; and CSS Shadow Parts'
//!   `::part()` of names, an element of a host's shadow tree its `part`
//!   names, through the hosts' `exportparts`.
//!
//! Names (types, attributes, pseudo-classes and pseudo-elements) are
//! compared in ASCII lowercase, as HTML and selectors-4 say.
//!
//! Not supported — hard parse errors, so the containing rule is dropped
//! exactly as css-syntax-3 prescribes: other pseudo-elements, a
//! pseudo-element inside `:not()`, `:is()` or `:where()` (which the
//! last two forgive), namespaces (`|`), and anything unknown.

use std::collections::HashMap;

mod language;
mod matcher;
mod parse;

pub use matcher::{Matcher, matches_selector, matches_selector_list};
pub use parse::{
    Nest, Relative, parse_nested_selector_list, parse_pseudo_element_argument, parse_selector_list,
    parse_selector_list_in, supports_selector,
};

/// A comma-separated selector list.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectorList(pub Vec<Selector>);

/// The most features a selector asks of its ancestors that the
/// ancestor filter checks (M35.2).
pub const ANCESTOR_HASHES: usize = 4;

/// What the ancestor filter keys a feature by (M35.2): an FNV-1a hash
/// of its kind (`b'#'` an ID, `b'.'` a class, `b'<'` a tag, lowercased)
/// and its name; never zero, which stands for none.
pub(crate) fn feature_hash(kind: u8, name: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in std::iter::once(kind).chain(name.bytes()) {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash.max(1)
}

/// The hashes of what `compounds` left of the subject require of its
/// ancestors: walking left from the subject while the combinators are
/// descendant or child ones, each compound's IDs, classes and tag,
/// until [`ANCESTOR_HASHES`] are found. A sibling combinator ends the
/// walk: what lies left of it need not be an ancestor.
pub(crate) fn ancestor_hashes(
    compounds: &[CompoundSelector],
    combinators: &[Combinator],
) -> [u32; ANCESTOR_HASHES] {
    let mut hashes = [0; ANCESTOR_HASHES];
    let mut found = 0;
    for k in (0..compounds.len().saturating_sub(1)).rev() {
        if !matches!(
            combinators.get(k),
            Some(Combinator::Descendant | Combinator::Child)
        ) {
            break;
        }
        for simple in &compounds[k].simple {
            let hash = match simple {
                SimpleSelector::Id(id) => feature_hash(b'#', id),
                SimpleSelector::Class(class) => feature_hash(b'.', class),
                SimpleSelector::Type(tag) => feature_hash(b'<', &tag.to_ascii_lowercase()),
                _ => continue,
            };
            if found < ANCESTOR_HASHES && !hashes.contains(&hash) {
                hashes[found] = hash;
                found += 1;
            }
        }
    }
    hashes
}

/// One complex selector: compounds joined by combinators, in source
/// order (`compounds.len() == combinators.len() + 1`). Matching walks
/// right-to-left, per selectors-4.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    /// Compound selectors in source order.
    pub compounds: Vec<CompoundSelector>,
    /// `combinators[i]` joins `compounds[i]` and `compounds[i+1]`.
    pub combinators: Vec<Combinator>,
    /// What the compounds left of the subject require of its ancestors,
    /// through descendant and child combinators alone: the hashes of
    /// their IDs, classes and tags ([`feature_hash`]), at most
    /// [`ANCESTOR_HASHES`], zero past the last (M35.2). An element no
    /// ancestor of which has one of them is matched no further.
    pub ancestor_hashes: [u32; ANCESTOR_HASHES],
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
    /// What a functional one takes: `::slotted()`'s compound,
    /// `::part()`'s names, `::highlight()`'s name.
    pub argument: PseudoArgument,
    /// The pseudo-classes after it, in source order: the user-action
    /// ones (`:hover`, `:focus`, `:focus-visible`, `:focus-within`,
    /// `:active`), and `:is()`, `:where()` and `:not()` of them, as
    /// selectors-4 §3.6.3 allows. No pseudo-element is in a user-action
    /// state until M26.4, and they are matched so.
    pub states: Vec<PseudoClass>,
}

/// The pseudo-elements a selector may end in (Pseudo-Elements 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
    /// `::target-text`, `::spelling-error`, `::grammar-error` and
    /// `::search-text` (CSS Pseudo-Elements 4 section 3): highlight
    /// pseudo-elements of what the engine does not mark (a URL's text
    /// fragment, a spelling or grammar checker's findings, a find in the
    /// page), valid and matching nothing.
    TextHighlight,
    /// `::details-content` (CSS Pseudo-Elements 4 section 5.1, D74): a
    /// `details` element's content but its summary, in a box of its own.
    DetailsContent,
    /// `::details-content::before` and `::details-content::after`: the
    /// boxes generated at the start and the end of that box, as the
    /// reference browser takes them.
    DetailsContentBefore,
    DetailsContentAfter,
    /// `::file-selector-button`: a file input's button; recognised, the
    /// control drawn whole.
    FileSelectorButton,
    /// `::picker(select)`, `::picker-icon` and `::checkmark`: the parts
    /// of a select the reference browser lets a page style; recognised,
    /// the control drawn whole.
    Picker,
    PickerIcon,
    Checkmark,
    /// `::backdrop` (CSS Position 4 section 4.1, gap batch H): the box
    /// beneath an element in the top layer, as big as the viewport,
    /// inheriting from its element.
    Backdrop,
    /// `::highlight(name)`: a custom highlight's (CSS Highlight API
    /// section 3, gap batch H), which the engine paints over the text
    /// its ranges hold; its name is the selector's argument
    /// ([`PseudoArgument::Highlight`]).
    Highlight,
    /// A `::-webkit-` name that is not a function: valid, so that the
    /// list it is in keeps its other selectors, and matching nothing
    /// (selectors-4 Appendix B).
    WebkitPrefixed,
    /// `::slotted()` (CSS Scoping 1 section 3.2.2): of a slot, the
    /// elements assigned to it its compound matches. A real element, not
    /// a box of its originating one.
    Slotted,
    /// `::part()` (CSS Shadow Parts 1): of a shadow host, the elements of
    /// its shadow tree with every part name given.
    Part,
}

/// What a functional pseudo-element takes.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PseudoArgument {
    #[default]
    None,
    /// `::slotted()`'s compound.
    Slotted(Box<CompoundSelector>),
    /// `::part()`'s names, in their case.
    Part(Vec<String>),
    /// `::highlight()`'s name (gap batch H), kept here rather than in the
    /// [`PseudoElement`], which boxes and fragments carry, a byte each.
    Highlight(HighlightName),
}

/// A custom highlight's name as `::highlight()` names it (CSS Highlight
/// API section 3), interned so that a pseudo-element stays a copy of a
/// few bytes: at most [`MAX_HIGHLIGHT_NAMES`] distinct names a process,
/// each at most [`MAX_HIGHLIGHT_NAME_LENGTH`] bytes, past which a
/// selector naming another is refused (`docs/security.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HighlightName(u32);

/// The most distinct highlight names a process interns.
pub const MAX_HIGHLIGHT_NAMES: usize = 1 << 12;

/// The longest highlight name interned, in bytes.
pub const MAX_HIGHLIGHT_NAME_LENGTH: usize = 1 << 10;

/// The names interned, by number, and the numbers by name.
type Interned = (Vec<std::sync::Arc<str>>, HashMap<std::sync::Arc<str>, u32>);

static HIGHLIGHT_NAMES: std::sync::LazyLock<std::sync::Mutex<Interned>> =
    std::sync::LazyLock::new(Default::default);

impl HighlightName {
    /// The name `name`, interned if it is not yet; `None` past the
    /// limits.
    pub fn new(name: &str) -> Option<HighlightName> {
        let mut names = HIGHLIGHT_NAMES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&n) = names.1.get(name) {
            return Some(HighlightName(n));
        }
        if names.0.len() >= MAX_HIGHLIGHT_NAMES || name.len() > MAX_HIGHLIGHT_NAME_LENGTH {
            return None;
        }
        let n = names.0.len() as u32;
        let shared: std::sync::Arc<str> = std::sync::Arc::from(name);
        names.0.push(shared.clone());
        names.1.insert(shared, n);
        Some(HighlightName(n))
    }

    /// The name `name` if it was interned: what a highlight registered
    /// by that name is styled by, none when no selector names it.
    pub fn find(name: &str) -> Option<HighlightName> {
        let names = HIGHLIGHT_NAMES.lock().unwrap_or_else(|e| e.into_inner());
        names.1.get(name).map(|&n| HighlightName(n))
    }

    /// The name.
    pub fn name(self) -> std::sync::Arc<str> {
        let names = HIGHLIGHT_NAMES.lock().unwrap_or_else(|e| e.into_inner());
        names.0[self.0 as usize].clone()
    }
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
    /// A type selector: the tag name as written, which matches an
    /// element's in any ASCII case in an HTML document and exactly in
    /// an XML one (Selectors 4 section 5.1).
    Type(String),
    /// `*`
    Universal,
    /// `#id`
    Id(String),
    /// `.class`
    Class(String),
    /// An attribute selector.
    Attribute {
        /// The attribute's name as written, matched as a type
        /// selector's is.
        name: String,
        /// The comparison.
        op: AttrOp,
        /// The comparison value (empty for `Exists`).
        value: String,
        /// How the value compares: the flag, or HTML's rule without one.
        case: CaseFlag,
        /// Its namespace component (CSS Namespaces 3 section 6.4).
        namespace: AttrNamespace,
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

/// An attribute selector's namespace component (CSS Namespaces 3
/// section 6.4): none written, matched by the attribute's name as it is
/// kept; `*|` for any namespace, and `|` or a declared prefix for no
/// namespace or one, matched by the attribute's local name (D67).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AttrNamespace {
    #[default]
    Unwritten,
    Any,
    Is(Option<String>),
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
    /// `&` in a style rule nested in another (CSS Nesting 1 section 4):
    /// the parent rule's selectors, matched and counted as `:is()` of
    /// them, which every rule nested in the parent shares.
    Nesting(std::sync::Arc<SelectorList>),
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
    /// `:has(relative selectors)` (Selectors 4 section 4.5): an element
    /// some relative selector of which matches an element, searched for
    /// among its descendants or following siblings as its combinators
    /// say (D74). Each selector of the list begins with the compound of
    /// [`PseudoClass::RelativeAnchor`] alone.
    Has(SelectorList),
    /// The anchor of a relative selector: the element the `:has()` it is
    /// in is matched at. Never written; it is the compound a relative
    /// selector of `:has()` begins with.
    RelativeAnchor,
    /// `:dir(ltr)` and `:dir(rtl)` (Selectors 4 section 7.1): an element
    /// of that directionality, as HTML gives it from `dir` (D74). `None`
    /// for any other identifier, valid and matching nothing.
    Dir(Option<crate::computed::Direction>),
    /// `:open` (Selectors 4 section 13.4): a `details` or a `dialog` with
    /// `open`, a `select` or an `input` whose picker is shown.
    Open,
    /// `:autofill` (and `:-webkit-autofill`): an input the user agent
    /// filled in; this engine fills in none, and it never matches.
    Autofill,
    /// `:host` and `:host(compound)` (CSS Scoping 1 section 3.2.1): the
    /// shadow host, from its shadow tree's rules, when it matches the
    /// compound.
    Host(Option<Box<CompoundSelector>>),
    /// `:host-context(compound)`: the shadow host, from its shadow
    /// tree's rules, when it or a shadow-including ancestor matches the
    /// compound.
    HostContext(Box<CompoundSelector>),
    /// `:modal` (gap batch H): a dialog shown by `showModal()`.
    Modal,
    /// `:popover-open`: a popover in the showing state.
    PopoverOpen,
    /// `:interest-source` and `:interest-target`: an interest invoker
    /// whose target has its interest, and that target.
    InterestSource,
    InterestTarget,
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
