//! Container queries (CSS Conditional 5 section 7, D72): the
//! `container-type`, `container-name` and `container` properties, the
//! preludes of `@container` rules, and their evaluation against an
//! element's query containers.
//!
//! The cascade evaluates a size query against its container's content
//! box as the last layout laid it out, [`ContainerSizes`], and keeps
//! which containers it asked about, with the sizes it was given: after
//! a layout, the engine measures them again and restyles the subtrees of
//! those whose size changed, then lays the document out again, at most
//! [`MAX_CONTAINER_PASSES`] times. Each pass settles one more level of
//! nested containers, as size containment makes a container's size
//! independent of its content; past the bound the styles of a deeper
//! level are those of the sizes of the pass before. A container never
//! laid out has no size: a size query of it is unknown, and the
//! container units fall back to the small viewport's.
//!
//! A query's condition is three-valued, as a media query's: a feature
//! this engine does not know, a value its feature does not take, and
//! anything else in parentheses the grammar has no place for are
//! `<general-enclosed>`, unknown; `not` of unknown is unknown, and a
//! rule applies when one of its queries is true. Style queries read the
//! container's computed custom properties, equal as their values' text
//! once substituted, or ordered as numbers and lengths in the range form.
//! Anchored queries (`anchored()`, CSS Anchor Positioning 2) are taken
//! as the reference browser takes them, and answer as of a container
//! laid out with no position fallback, the engine having no anchor
//! positioning (see [`AnchoredCondition`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use css::Token;
use dom::{Document, NodeId};

use crate::calc;
use crate::cascade::Styles;
use crate::computed::{ComputedStyle, ContainerSize, ContainerType, Display};
use crate::custom::{Substituted, Substitutions};

/// How many times, at most, a document is laid out again for the
/// containers whose size changed what their queries answer (D72): each
/// pass settles one more level of nested containers. Past it the
/// deepest levels keep the styles of the sizes of the pass before.
pub const MAX_CONTAINER_PASSES: usize = 64;

/// How deep conditions in parentheses may nest before what is inside is
/// left unknown.
const MAX_NESTING: usize = 32;

/// The content boxes of the size containers as a layout laid them out,
/// their widths and heights in CSS pixels, by node; and the scroll states
/// of the scroll-state containers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContainerSizes {
    sizes: HashMap<NodeId, (f32, f32)>,
    scroll: HashMap<NodeId, ScrollState>,
}

impl ContainerSizes {
    /// No container laid out.
    pub fn new() -> Self {
        ContainerSizes::default()
    }

    /// Records `node`'s content box, `width` by `height`.
    pub fn insert(&mut self, node: NodeId, width: f32, height: f32) {
        self.sizes.insert(node, (width, height));
    }

    /// `node`'s content box, if it was laid out as a size container.
    pub fn get(&self, node: NodeId) -> Option<(f32, f32)> {
        self.sizes.get(&node).copied()
    }

    /// Records `node`'s scroll state, a scroll-state container's.
    pub fn insert_scroll(&mut self, node: NodeId, state: ScrollState) {
        self.scroll.insert(node, state);
    }

    /// `node`'s scroll state, if it was laid out as a scroll-state
    /// container.
    pub fn scroll(&self, node: NodeId) -> Option<ScrollState> {
        self.scroll.get(&node).copied()
    }
}

/// What a scroll-state query asks of its container (section 7.4.4), as
/// a layout, the scroll offsets and the scrolls since give it, each by
/// its sides, top, right, bottom and left, or its axes, across and down.
/// A container is in the states its box can be: a scroll container
/// scrollable and scrolled, a sticky box stuck, a snap target snapped;
/// in none of the others.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScrollState {
    /// The sides a scroll container can be scrolled towards.
    pub scrollable: [bool; 4],
    /// The sides the last relative scroll of a scroll container (a
    /// `scrollBy()`, a wheel or a key) that moved its offset moved it
    /// towards, one of each axis at most, each axis keeping its own.
    pub scrolled: [bool; 4],
    /// The sides of its scrollport a `position: sticky` box is held off
    /// by its insets.
    pub stuck: [bool; 4],
    /// The axes its scroll container rests snapped to it in.
    pub snapped: [bool; 2],
}

/// What a cascade read of one container: its content box and its scroll
/// state, each `None` where it was not laid out as one.
pub type ContainerState = (Option<(f32, f32)>, Option<ScrollState>);

/// The containers a cascade asked about, each with what it was given.
/// What a layout's sizes are compared with (D72).
pub type Queried = HashMap<NodeId, ContainerState>;

// ---- The properties ------------------------------------------------------

/// The identifiers of `tokens`, white space aside; `None` when anything
/// else is among them.
fn idents(tokens: &[Token]) -> Option<Vec<&str>> {
    tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .map(|t| match t {
            Token::Ident(word) => Some(word.as_str()),
            _ => None,
        })
        .collect()
}

/// `container-type`: `normal`, or `size` or `inline-size`, `scroll-state`
/// and `anchored`, each once, in any order.
pub(crate) fn parse_container_type(tokens: &[Token]) -> Option<ContainerType> {
    let words = idents(tokens)?;
    if let [word] = words[..]
        && word.eq_ignore_ascii_case("normal")
    {
        return Some(ContainerType::NORMAL);
    }
    let mut kind = ContainerType::NORMAL;
    let mut sized = false;
    for word in &words {
        match word.to_ascii_lowercase().as_str() {
            "size" if !sized => {
                kind.size = ContainerSize::Both;
                sized = true;
            }
            "inline-size" if !sized => {
                kind.size = ContainerSize::Inline;
                sized = true;
            }
            "scroll-state" if !kind.scroll_state => kind.scroll_state = true,
            "anchored" if !kind.anchored => kind.anchored = true,
            _ => return None,
        }
    }
    (!words.is_empty()).then_some(kind)
}

/// Whether `name` may name a container: a `<custom-ident>` but `none`,
/// `and`, `or`, `not` and `default`.
fn is_container_name(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "none"
            | "and"
            | "or"
            | "not"
            | "default"
            | "initial"
            | "inherit"
            | "unset"
            | "revert"
            | "revert-layer"
    )
}

/// The names a `container-name` value gives: `None` for `none`.
pub(crate) type ContainerNames = Option<Arc<Vec<Box<str>>>>;

/// `container-name`: `none`, or one or more names, kept as written.
pub(crate) fn parse_container_name(tokens: &[Token]) -> Option<ContainerNames> {
    let words = idents(tokens)?;
    if let [word] = words[..]
        && word.eq_ignore_ascii_case("none")
    {
        return Some(None);
    }
    if words.is_empty() || !words.iter().all(|word| is_container_name(word)) {
        return None;
    }
    Some(Some(Arc::new(words.into_iter().map(Box::from).collect())))
}

/// The `container` shorthand: `<'container-name'> [ / <'container-type'> ]?`.
pub(crate) fn parse_container(tokens: &[Token]) -> Option<(ContainerNames, ContainerType)> {
    let slash = tokens.iter().position(|t| matches!(t, Token::Delim('/')));
    let (names, kind) = match slash {
        Some(at) => (
            &tokens[..at],
            Some(parse_container_type(&tokens[at + 1..])?),
        ),
        None => (tokens, None),
    };
    Some((
        parse_container_name(names)?,
        kind.unwrap_or(ContainerType::NORMAL),
    ))
}

/// `container-type`'s computed value as text.
pub(crate) fn container_type_text(kind: ContainerType) -> String {
    let size = match kind.size {
        ContainerSize::None => None,
        ContainerSize::Inline => Some("inline-size"),
        ContainerSize::Both => Some("size"),
    };
    let words: Vec<&str> = size
        .into_iter()
        .chain(kind.scroll_state.then_some("scroll-state"))
        .chain(kind.anchored.then_some("anchored"))
        .collect();
    if words.is_empty() {
        "normal".to_owned()
    } else {
        words.join(" ")
    }
}

/// A `container-type` value as a declaration block keeps it: its words
/// in their canonical order; `None` for one not valid.
pub fn container_type_value(tokens: &[Token]) -> Option<String> {
    parse_container_type(tokens).map(container_type_text)
}

/// A `container-name` value as a declaration block keeps it: its names
/// serialized, `none`; `None` for one not valid.
pub fn container_name_value(tokens: &[Token]) -> Option<String> {
    parse_container_name(tokens).map(|names| container_name_text(&names))
}

/// `container-name`'s computed value as text.
pub(crate) fn container_name_text(names: &ContainerNames) -> String {
    match names {
        None => "none".to_owned(),
        Some(names) => names
            .iter()
            .map(|name| crate::media::ident(name))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// An `@container` rule's prelude as the CSSOM shows it (section 7.1):
/// its `conditionText`, and each query's name and condition, what
/// `conditions` lists; `None` when it is not a valid one, which leaves
/// the rule out.
pub fn container_prelude(prelude: &[Token]) -> Option<(String, Vec<(String, String)>)> {
    let rule = ContainerRule::parse(prelude)?;
    let conditions = rule
        .queries
        .iter()
        .map(|query| {
            (
                query
                    .name
                    .as_deref()
                    .map(crate::media::ident)
                    .unwrap_or_default(),
                query
                    .condition
                    .as_ref()
                    .map(Condition::text)
                    .unwrap_or_default(),
            )
        })
        .collect();
    Some((rule.text(), conditions))
}

// ---- The queries ----------------------------------------------------------

/// An `@container` rule's prelude, parsed: its queries, the rule's
/// block applying where one of them holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContainerRule {
    queries: Vec<Query>,
}

/// One query: a container's name, a condition, or both.
#[derive(Debug, Clone, PartialEq)]
struct Query {
    name: Option<String>,
    condition: Option<Condition>,
}

/// A container condition (section 7.1).
#[derive(Debug, Clone, PartialEq)]
enum Condition {
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    /// A condition in parentheses.
    Parens(Box<Condition>),
    Size(SizeFeature),
    /// `style()`.
    Style(Box<StyleCondition>),
    /// `scroll-state()`.
    ScrollState(Box<ScrollCondition>),
    /// `anchored()`.
    Anchored(Box<AnchoredCondition>),
    /// `<general-enclosed>`: unknown, with its text.
    Unknown(String),
}

/// An anchored query's condition (CSS Anchor Positioning 2): of the
/// position fallback its container, an anchor-positioned box, is laid
/// out with.
#[derive(Debug, Clone, PartialEq)]
enum AnchoredCondition {
    Not(Box<AnchoredCondition>),
    And(Vec<AnchoredCondition>),
    Or(Vec<AnchoredCondition>),
    Parens(Box<AnchoredCondition>),
    /// `fallback`, and the value it is compared with, as it serializes,
    /// if any.
    Fallback(Option<String>),
}

/// The try tactics a `fallback` value may name.
const TRY_TACTICS: [&str; 5] = [
    "flip-block",
    "flip-inline",
    "flip-start",
    "flip-x",
    "flip-y",
];

/// The axis a `<position-area>` keyword is of, by the groups its grammar
/// pairs: 0 and 1 the physical (or `x-`, `y-`) ones across and down, 2
/// and 3 the logical block and inline ones, 4 and 5 their `self-` forms,
/// 6 the ones of either logical axis, 7 their `self-` forms; `None` for
/// `center` and `span-all`, which are of any, and for a word not one.
fn area_axis(word: &str) -> Option<Option<u8>> {
    let bare = word.strip_prefix("span-").unwrap_or(word);
    Some(Some(match bare {
        "all" if word == "span-all" => return Some(None),
        "center" if word == "center" => return Some(None),
        "left" | "right" | "x-start" | "x-end" | "self-x-start" | "self-x-end" => 0,
        "top" | "bottom" | "y-start" | "y-end" | "self-y-start" | "self-y-end" => 1,
        "block-start" | "block-end" => 2,
        "inline-start" | "inline-end" => 3,
        "self-block-start" | "self-block-end" => 4,
        "self-inline-start" | "self-inline-end" => 5,
        "start" | "end" => 6,
        "self-start" | "self-end" => 7,
        _ => return None,
    }))
}

/// A `fallback` feature's value as it serializes: `none`, a
/// `<dashed-ident>` and try tactics (the name first), or a
/// `<position-area>` (the axis across or the block axis first); `None`
/// for one not valid, which leaves the query `<general-enclosed>`.
fn fallback_value(tokens: &[Token]) -> Option<String> {
    let words = idents(tokens)?;
    let lower: Vec<String> = words
        .iter()
        .map(|w| {
            if w.starts_with("--") {
                (*w).to_owned()
            } else {
                w.to_ascii_lowercase()
            }
        })
        .collect();
    match &lower[..] {
        [] => return None,
        [one] if one == "none" => return Some("none".to_owned()),
        _ => {}
    }
    // `<dashed-ident> || <try-tactic>`, each tactic once.
    let names: Vec<&String> = lower.iter().filter(|w| w.starts_with("--")).collect();
    let tactics: Vec<&String> = lower
        .iter()
        .filter(|w| TRY_TACTICS.contains(&w.as_str()))
        .collect();
    if names.len() + tactics.len() == lower.len() {
        let distinct = tactics
            .iter()
            .enumerate()
            .all(|(i, t)| !tactics[..i].contains(t));
        if names.len() > 1 || !distinct {
            return None;
        }
        return Some(
            names
                .into_iter()
                .chain(tactics)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    // `<position-area>`: one keyword, or two of the axes its grammar
    // pairs, the second dropped when it says the first again.
    let axes: Option<Vec<Option<u8>>> = lower.iter().map(|w| area_axis(w)).collect();
    match (&lower[..], &axes?[..]) {
        ([one], [_]) => Some(one.clone()),
        ([a, b], [x, y]) => {
            let pair = |first: u8, second: u8| -> Option<bool> {
                // Whether `a` goes first, for a pair of `first` and
                // `second`, either ambiguous.
                match (*x, *y) {
                    (Some(p), Some(q)) if p == first && q == second => Some(true),
                    (Some(p), Some(q)) if p == second && q == first => Some(false),
                    (Some(p), None) if p == first => Some(true),
                    (Some(p), None) if p == second => Some(false),
                    (None, Some(q)) if q == second => Some(true),
                    (None, Some(q)) if q == first => Some(false),
                    _ => None,
                }
            };
            let order = match (*x, *y) {
                (None, None) => Some(true),
                (Some(6), Some(6) | None) | (None, Some(6)) => Some(true),
                (Some(7), Some(7) | None) | (None, Some(7)) => Some(true),
                _ => pair(0, 1).or_else(|| pair(2, 3)).or_else(|| pair(4, 5)),
            }?;
            let (first, second) = if order { (a, b) } else { (b, a) };
            if first == second {
                Some(first.clone())
            } else {
                Some(format!("{first} {second}"))
            }
        }
        _ => None,
    }
}

/// Parses an anchored query's condition at `at`, as a scroll-state
/// query's is parsed.
fn parse_anchored_condition(
    tokens: &[Token],
    at: &mut usize,
    depth: usize,
) -> Option<AnchoredCondition> {
    skip_ws(tokens, at);
    if depth > MAX_NESTING {
        return None;
    }
    let first = tokens.get(*at);
    if !matches!(first, Some(Token::OpenParen))
        && !matches!(first, Some(Token::Ident(w)) if w.eq_ignore_ascii_case("not"))
    {
        let feature = parse_anchored_feature(&tokens[*at..])?;
        *at = tokens.len();
        return Some(feature);
    }
    if let Some(Token::Ident(word)) = first
        && word.eq_ignore_ascii_case("not")
    {
        *at += 1;
        skip_ws(tokens, at);
        let inner = parse_anchored_in_parens(tokens, at, depth)?;
        skip_ws(tokens, at);
        return Some(AnchoredCondition::Not(Box::new(inner)));
    }
    let mut joined = vec![parse_anchored_in_parens(tokens, at, depth)?];
    let mut joiner: Option<bool> = None;
    loop {
        skip_ws(tokens, at);
        let Some(Token::Ident(word)) = tokens.get(*at) else {
            break;
        };
        let and = match word.to_ascii_lowercase().as_str() {
            "and" => true,
            "or" => false,
            _ => return None,
        };
        if joiner.is_some_and(|was| was != and) {
            return None;
        }
        joiner = Some(and);
        *at += 1;
        skip_ws(tokens, at);
        joined.push(parse_anchored_in_parens(tokens, at, depth)?);
    }
    Some(match joiner {
        None => joined.pop().expect("one condition"),
        Some(true) => AnchoredCondition::And(joined),
        Some(false) => AnchoredCondition::Or(joined),
    })
}

/// An anchored condition in parentheses.
fn parse_anchored_in_parens(
    tokens: &[Token],
    at: &mut usize,
    depth: usize,
) -> Option<AnchoredCondition> {
    let rest = &tokens[*at..];
    if !matches!(rest.first(), Some(Token::OpenParen)) {
        return None;
    }
    let (inside, after) = block(rest)?;
    *at += rest.len() - after.len();
    let inside = trim(inside);
    let mut pos = 0;
    let inner = parse_anchored_condition(inside, &mut pos, depth + 1)?;
    (pos == inside.len()).then(|| AnchoredCondition::Parens(Box::new(inner)))
}

/// The `fallback` feature, with a value after a colon or not.
fn parse_anchored_feature(tokens: &[Token]) -> Option<AnchoredCondition> {
    let tokens = trim(tokens);
    let Some(Token::Ident(name)) = tokens.first() else {
        return None;
    };
    if !name.eq_ignore_ascii_case("fallback") {
        return None;
    }
    match trim(&tokens[1..]) {
        [] => Some(AnchoredCondition::Fallback(None)),
        [Token::Colon, value @ ..] => Some(AnchoredCondition::Fallback(Some(fallback_value(
            trim(value),
        )?))),
        _ => None,
    }
}

impl AnchoredCondition {
    fn text(&self) -> String {
        match self {
            AnchoredCondition::Not(inner) => format!("not {}", inner.text()),
            AnchoredCondition::And(items) => {
                join(items.iter().map(AnchoredCondition::text), " and ")
            }
            AnchoredCondition::Or(items) => join(items.iter().map(AnchoredCondition::text), " or "),
            AnchoredCondition::Parens(inner) => format!("({})", inner.text()),
            AnchoredCondition::Fallback(None) => "fallback".to_owned(),
            AnchoredCondition::Fallback(Some(value)) => format!("fallback: {value}"),
        }
    }

    /// Its answer for an anchored container. This engine has no anchor
    /// positioning (`position-anchor`, `position-area` and
    /// `position-try` are not properties here): no box is laid out with
    /// a position fallback, so `fallback: none` holds and every other
    /// fallback, and `fallback` alone, does not, as the reference
    /// browser answers for a container that is not anchor-positioned.
    fn evaluate(&self) -> Truth {
        match self {
            AnchoredCondition::Not(inner) => inner.evaluate().not(),
            AnchoredCondition::And(items) => Truth::and(items.iter().map(Self::evaluate)),
            AnchoredCondition::Or(items) => Truth::or(items.iter().map(Self::evaluate)),
            AnchoredCondition::Parens(inner) => inner.evaluate(),
            AnchoredCondition::Fallback(value) => Truth::of(value.as_deref() == Some("none")),
        }
    }
}

/// A scroll-state query's condition (section 7.4.4).
#[derive(Debug, Clone, PartialEq)]
enum ScrollCondition {
    Not(Box<ScrollCondition>),
    And(Vec<ScrollCondition>),
    Or(Vec<ScrollCondition>),
    Parens(Box<ScrollCondition>),
    /// A feature, and the value it is compared with, if any.
    Feature(ScrollFeature, Option<ScrollValue>),
}

/// The scroll-state features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollFeature {
    Scrollable,
    Scrolled,
    Stuck,
    Snapped,
}

impl ScrollFeature {
    fn of(name: &str) -> Option<ScrollFeature> {
        Some(match name {
            "scrollable" => ScrollFeature::Scrollable,
            "scrolled" => ScrollFeature::Scrolled,
            "stuck" => ScrollFeature::Stuck,
            "snapped" => ScrollFeature::Snapped,
            _ => return None,
        })
    }

    fn text(self) -> &'static str {
        match self {
            ScrollFeature::Scrollable => "scrollable",
            ScrollFeature::Scrolled => "scrolled",
            ScrollFeature::Stuck => "stuck",
            ScrollFeature::Snapped => "snapped",
        }
    }

    /// The value `word` is for it, if it takes it.
    fn value(self, word: &str) -> Option<ScrollValue> {
        use ScrollValue as V;
        let value = match word {
            "none" => V::None,
            "top" => V::Side(TOP_SIDE),
            "right" => V::Side(RIGHT_SIDE),
            "bottom" => V::Side(BOTTOM_SIDE),
            "left" => V::Side(LEFT_SIDE),
            "block-start" => V::BlockStart,
            "block-end" => V::BlockEnd,
            "inline-start" => V::InlineStart,
            "inline-end" => V::InlineEnd,
            "x" => V::X,
            "y" => V::Y,
            "block" => V::Block,
            "inline" => V::Inline,
            "both" => V::Both,
            _ => return None,
        };
        let sided = matches!(
            value,
            V::Side(_) | V::BlockStart | V::BlockEnd | V::InlineStart | V::InlineEnd
        );
        let axial = matches!(value, V::X | V::Y | V::Block | V::Inline);
        let takes = match self {
            ScrollFeature::Scrollable | ScrollFeature::Scrolled => sided || axial,
            ScrollFeature::Stuck => sided,
            ScrollFeature::Snapped => axial || value == V::Both,
        };
        (takes || value == V::None).then_some(value)
    }
}

/// A side's index in a [`ScrollState`]'s sides.
const TOP_SIDE: usize = 0;
const RIGHT_SIDE: usize = 1;
const BOTTOM_SIDE: usize = 2;
const LEFT_SIDE: usize = 3;

/// A scroll-state feature's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollValue {
    None,
    Side(usize),
    BlockStart,
    BlockEnd,
    InlineStart,
    InlineEnd,
    X,
    Y,
    Block,
    Inline,
    Both,
}

impl ScrollValue {
    fn text(self) -> &'static str {
        match self {
            ScrollValue::None => "none",
            ScrollValue::Side(TOP_SIDE) => "top",
            ScrollValue::Side(RIGHT_SIDE) => "right",
            ScrollValue::Side(BOTTOM_SIDE) => "bottom",
            ScrollValue::Side(_) => "left",
            ScrollValue::BlockStart => "block-start",
            ScrollValue::BlockEnd => "block-end",
            ScrollValue::InlineStart => "inline-start",
            ScrollValue::InlineEnd => "inline-end",
            ScrollValue::X => "x",
            ScrollValue::Y => "y",
            ScrollValue::Block => "block",
            ScrollValue::Inline => "inline",
            ScrollValue::Both => "both",
        }
    }

    /// The physical sides it names, in horizontal writing with the
    /// container's `direction`: one for a side, two for an axis.
    fn sides(self, rtl: bool) -> Vec<usize> {
        let (start, end) = if rtl {
            (RIGHT_SIDE, LEFT_SIDE)
        } else {
            (LEFT_SIDE, RIGHT_SIDE)
        };
        match self {
            ScrollValue::None => Vec::new(),
            ScrollValue::Side(side) => vec![side],
            ScrollValue::BlockStart => vec![TOP_SIDE],
            ScrollValue::BlockEnd => vec![BOTTOM_SIDE],
            ScrollValue::InlineStart => vec![start],
            ScrollValue::InlineEnd => vec![end],
            ScrollValue::Y | ScrollValue::Block => vec![TOP_SIDE, BOTTOM_SIDE],
            ScrollValue::X | ScrollValue::Inline => vec![LEFT_SIDE, RIGHT_SIDE],
            ScrollValue::Both => vec![TOP_SIDE, RIGHT_SIDE, BOTTOM_SIDE, LEFT_SIDE],
        }
    }
}

/// Parses a scroll-state query's condition at `at`: a feature alone, or
/// features in parentheses joined as a container condition's are.
fn parse_scroll_condition(
    tokens: &[Token],
    at: &mut usize,
    depth: usize,
) -> Option<ScrollCondition> {
    skip_ws(tokens, at);
    if depth > MAX_NESTING {
        return None;
    }
    let first = tokens.get(*at);
    if !matches!(first, Some(Token::OpenParen))
        && !matches!(first, Some(Token::Ident(w)) if w.eq_ignore_ascii_case("not"))
    {
        let feature = parse_scroll_feature(&tokens[*at..])?;
        *at = tokens.len();
        return Some(feature);
    }
    if let Some(Token::Ident(word)) = first
        && word.eq_ignore_ascii_case("not")
    {
        *at += 1;
        skip_ws(tokens, at);
        let inner = parse_scroll_in_parens(tokens, at, depth)?;
        skip_ws(tokens, at);
        return Some(ScrollCondition::Not(Box::new(inner)));
    }
    let mut joined = vec![parse_scroll_in_parens(tokens, at, depth)?];
    let mut joiner: Option<bool> = None;
    loop {
        skip_ws(tokens, at);
        let Some(Token::Ident(word)) = tokens.get(*at) else {
            break;
        };
        let and = match word.to_ascii_lowercase().as_str() {
            "and" => true,
            "or" => false,
            _ => return None,
        };
        if joiner.is_some_and(|was| was != and) {
            return None;
        }
        joiner = Some(and);
        *at += 1;
        skip_ws(tokens, at);
        joined.push(parse_scroll_in_parens(tokens, at, depth)?);
    }
    Some(match joiner {
        None => joined.pop().expect("one condition"),
        Some(true) => ScrollCondition::And(joined),
        Some(false) => ScrollCondition::Or(joined),
    })
}

/// A scroll-state condition in parentheses.
fn parse_scroll_in_parens(
    tokens: &[Token],
    at: &mut usize,
    depth: usize,
) -> Option<ScrollCondition> {
    let rest = &tokens[*at..];
    if !matches!(rest.first(), Some(Token::OpenParen)) {
        return None;
    }
    let (inside, after) = block(rest)?;
    *at += rest.len() - after.len();
    let inside = trim(inside);
    let mut pos = 0;
    let inner = parse_scroll_condition(inside, &mut pos, depth + 1)?;
    (pos == inside.len()).then(|| ScrollCondition::Parens(Box::new(inner)))
}

/// A scroll-state feature: its name, with a value after a colon or not.
fn parse_scroll_feature(tokens: &[Token]) -> Option<ScrollCondition> {
    let tokens = trim(tokens);
    let Some(Token::Ident(name)) = tokens.first() else {
        return None;
    };
    let feature = ScrollFeature::of(&name.to_ascii_lowercase())?;
    match trim(&tokens[1..]) {
        [] => Some(ScrollCondition::Feature(feature, None)),
        [Token::Colon, value @ ..] => match trim(value) {
            [Token::Ident(word)] => Some(ScrollCondition::Feature(
                feature,
                Some(feature.value(&word.to_ascii_lowercase())?),
            )),
            _ => None,
        },
        _ => None,
    }
}

impl ScrollCondition {
    fn text(&self) -> String {
        match self {
            ScrollCondition::Not(inner) => format!("not {}", inner.text()),
            ScrollCondition::And(items) => join(items.iter().map(ScrollCondition::text), " and "),
            ScrollCondition::Or(items) => join(items.iter().map(ScrollCondition::text), " or "),
            ScrollCondition::Parens(inner) => format!("({})", inner.text()),
            ScrollCondition::Feature(feature, None) => feature.text().to_owned(),
            ScrollCondition::Feature(feature, Some(value)) => {
                format!("{}: {}", feature.text(), value.text())
            }
        }
    }

    /// Its answer for `node`, the container, of `style`: false for one
    /// not laid out as a scroll-state container.
    fn evaluate(&self, node: NodeId, style: &ComputedStyle, c: &Containers<'_>) -> Truth {
        match self {
            ScrollCondition::Not(inner) => inner.evaluate(node, style, c).not(),
            ScrollCondition::And(items) => {
                Truth::and(items.iter().map(|i| i.evaluate(node, style, c)))
            }
            ScrollCondition::Or(items) => {
                Truth::or(items.iter().map(|i| i.evaluate(node, style, c)))
            }
            ScrollCondition::Parens(inner) => inner.evaluate(node, style, c),
            ScrollCondition::Feature(feature, value) => {
                let state = c.state_of(node).1.unwrap_or_default();
                // The sides it is in the state of, an axis snapped as its
                // two sides.
                let held = match feature {
                    ScrollFeature::Scrollable => state.scrollable,
                    ScrollFeature::Scrolled => state.scrolled,
                    ScrollFeature::Stuck => state.stuck,
                    ScrollFeature::Snapped => {
                        let [across, down] = state.snapped;
                        [down, across, down, across]
                    }
                };
                let rtl = style.direction == crate::computed::Direction::Rtl;
                Truth::of(match value {
                    None => held.contains(&true),
                    Some(ScrollValue::None) => !held.contains(&true),
                    Some(ScrollValue::Both) => held[TOP_SIDE] && held[LEFT_SIDE],
                    Some(value) => value.sides(rtl).iter().any(|&side| held[side]),
                })
            }
        }
    }
}

/// A style query's condition (section 7.4.3).
#[derive(Debug, Clone, PartialEq)]
enum StyleCondition {
    Not(Box<StyleCondition>),
    And(Vec<StyleCondition>),
    Or(Vec<StyleCondition>),
    Parens(Box<StyleCondition>),
    /// `--name`, or `--name: value`.
    Declaration(String, Option<Vec<Token>>),
    /// The range form: operands and the comparisons between them.
    Range(Vec<Vec<Token>>, Vec<Comparison>),
    Unknown(String),
}

/// A size feature (section 7.4.2), as written.
#[derive(Debug, Clone, PartialEq)]
enum SizeFeature {
    /// `(name)`: true when it is not zero, or `none` for `orientation`.
    Boolean(Name),
    /// `(name: value)`, a `min-` or `max-` prefix made a comparison.
    Plain(Name, Option<Comparison>, Value),
    /// The range form: the value before and its comparison, and the one
    /// after.
    Range(
        Name,
        Option<(Value, Comparison)>,
        Option<(Comparison, Value)>,
    ),
}

/// The size features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Name {
    Width,
    Height,
    InlineSize,
    BlockSize,
    AspectRatio,
    Orientation,
}

impl Name {
    fn of(name: &str) -> Option<Name> {
        Some(match name {
            "width" => Name::Width,
            "height" => Name::Height,
            "inline-size" => Name::InlineSize,
            "block-size" => Name::BlockSize,
            "aspect-ratio" => Name::AspectRatio,
            "orientation" => Name::Orientation,
            _ => return None,
        })
    }

    fn text(self) -> &'static str {
        match self {
            Name::Width => "width",
            Name::Height => "height",
            Name::InlineSize => "inline-size",
            Name::BlockSize => "block-size",
            Name::AspectRatio => "aspect-ratio",
            Name::Orientation => "orientation",
        }
    }

    /// Whether it asks of the block axis, which only a `size` container
    /// answers.
    fn block_axis(self) -> bool {
        !matches!(self, Name::Width | Name::InlineSize)
    }
}

/// A range comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comparison {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Comparison {
    fn text(self) -> &'static str {
        match self {
            Comparison::Lt => "<",
            Comparison::Le => "<=",
            Comparison::Gt => ">",
            Comparison::Ge => ">=",
            Comparison::Eq => "=",
        }
    }

    fn holds(self, a: f64, b: f64) -> bool {
        match self {
            Comparison::Lt => a < b,
            Comparison::Le => a <= b,
            Comparison::Gt => a > b,
            Comparison::Ge => a >= b,
            Comparison::Eq => a == b,
        }
    }

    /// Whether it points as `other` does, as both of a range's must.
    fn same_way(self, other: Comparison) -> bool {
        let less = |c: Comparison| matches!(c, Comparison::Lt | Comparison::Le);
        let more = |c: Comparison| matches!(c, Comparison::Gt | Comparison::Ge);
        (less(self) && less(other)) || (more(self) && more(other))
    }
}

/// A size feature's value: a length, as written; a ratio; or an
/// orientation.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Length(Vec<Token>),
    Ratio(f64, f64),
    Portrait,
    Landscape,
}

/// The tokens of a value, white space at its ends aside.
fn trim(mut tokens: &[Token]) -> &[Token] {
    while let [Token::Whitespace, rest @ ..] = tokens {
        tokens = rest;
    }
    while let [rest @ .., Token::Whitespace] = tokens {
        tokens = rest;
    }
    tokens
}

/// The pieces of `tokens` parted by top-level commas.
fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let mut pieces = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (at, token) in tokens.iter().enumerate() {
        match token {
            Token::Function(_) | Token::OpenParen | Token::OpenSquare | Token::OpenCurly => {
                depth += 1
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                depth = depth.saturating_sub(1)
            }
            Token::Comma if depth == 0 => {
                pieces.push(&tokens[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    pieces.push(&tokens[start..]);
    pieces
}

/// A block's tokens: from the token after an opening one to its closing
/// one, nested blocks counted; the inside and what follows the close.
fn block(tokens: &[Token]) -> Option<(&[Token], &[Token])> {
    let mut depth = 0usize;
    for (at, token) in tokens.iter().enumerate() {
        match token {
            Token::Function(_) | Token::OpenParen | Token::OpenSquare | Token::OpenCurly => {
                depth += 1
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                depth -= 1;
                if depth == 0 {
                    return Some((&tokens[1..at], &tokens[at + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

impl ContainerRule {
    /// The rule an `@container` prelude is; `None` when it is not one.
    pub(crate) fn parse(prelude: &[Token]) -> Option<ContainerRule> {
        let queries: Option<Vec<Query>> = split_commas(prelude)
            .into_iter()
            .map(|piece| Query::parse(trim(piece)))
            .collect();
        Some(ContainerRule { queries: queries? })
    }

    /// The prelude as the CSSOM gives it as `conditionText`: each query's
    /// name and condition, parted by `, `.
    pub(crate) fn text(&self) -> String {
        self.queries
            .iter()
            .map(Query::text)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether one of its queries holds for an element whose query
    /// containers are `from` and its ancestors.
    pub(crate) fn holds(&self, from: Option<NodeId>, containers: &Containers<'_>) -> bool {
        self.queries
            .iter()
            .any(|query| query.evaluate(from, containers) == Truth::True)
    }
}

impl Query {
    fn parse(tokens: &[Token]) -> Option<Query> {
        let (name, rest) = match tokens.first() {
            Some(Token::Ident(word)) if !word.eq_ignore_ascii_case("not") => {
                if !is_container_name(word) {
                    return None;
                }
                (Some(word.clone()), trim(&tokens[1..]))
            }
            _ => (None, tokens),
        };
        let condition = if rest.is_empty() {
            None
        } else {
            let mut at = 0;
            let condition = parse_condition(rest, &mut at, 0)?;
            if at != rest.len() {
                return None;
            }
            Some(condition)
        };
        if name.is_none() && condition.is_none() {
            return None;
        }
        Some(Query { name, condition })
    }

    fn text(&self) -> String {
        match (&self.name, &self.condition) {
            (Some(name), Some(condition)) => {
                format!("{} {}", crate::media::ident(name), condition.text())
            }
            (Some(name), None) => crate::media::ident(name),
            (None, Some(condition)) => condition.text(),
            (None, None) => String::new(),
        }
    }

    /// The query's answer for an element whose query containers are
    /// `from` and its ancestors: of the nearest that has its name and
    /// can answer its features (section 7.1); unknown when none can.
    fn evaluate(&self, from: Option<NodeId>, containers: &Containers<'_>) -> Truth {
        let mut needs = Needs::default();
        if let Some(condition) = &self.condition {
            condition.needs(&mut needs);
            // A `<general-enclosed>` anywhere leaves the whole unknown,
            // what is around it true or not, as the reference browser
            // evaluates it; a feature a container cannot answer is
            // unknown where it stands.
            if condition.encloses_unknown() {
                return Truth::Unknown;
            }
        }
        let document = containers.document;
        let mut at = from;
        while let Some(node) = at {
            at = document.flat_parent_element(node);
            let Some(style) = containers.style(node) else {
                continue;
            };
            let named = self.name.as_deref().is_none_or(|name| {
                style
                    .container_names
                    .as_deref()
                    .is_some_and(|names| names.iter().any(|n| &**n == name))
            });
            let kind = style.container_type;
            let sized = match needs.size {
                None => true,
                Some(false) => kind.is_size(),
                Some(true) => kind.size == ContainerSize::Both,
            };
            if !named
                || !sized
                || (needs.scroll_state && !kind.scroll_state)
                || (needs.anchored && !kind.anchored)
            {
                continue;
            }
            return match &self.condition {
                None => Truth::True,
                Some(condition) => condition.evaluate(node, style, containers),
            };
        }
        Truth::Unknown
    }
}

/// What a condition asks of its container.
#[derive(Default)]
struct Needs {
    /// A size: `Some(true)` of the block axis too.
    size: Option<bool>,
    scroll_state: bool,
    anchored: bool,
}

/// Parses a condition at `at`: `not` and a query in parentheses, or one
/// or more joined by `and` or by `or`, not both.
fn parse_condition(tokens: &[Token], at: &mut usize, depth: usize) -> Option<Condition> {
    skip_ws(tokens, at);
    if let Some(Token::Ident(word)) = tokens.get(*at)
        && word.eq_ignore_ascii_case("not")
    {
        *at += 1;
        skip_ws(tokens, at);
        let inner = parse_in_parens(tokens, at, depth)?;
        skip_ws(tokens, at);
        return Some(Condition::Not(Box::new(inner)));
    }
    let first = parse_in_parens(tokens, at, depth)?;
    let mut joined = vec![first];
    let mut joiner: Option<bool> = None;
    loop {
        skip_ws(tokens, at);
        let Some(Token::Ident(word)) = tokens.get(*at) else {
            break;
        };
        let and = if word.eq_ignore_ascii_case("and") {
            true
        } else if word.eq_ignore_ascii_case("or") {
            false
        } else {
            return None;
        };
        if joiner.is_some_and(|was| was != and) {
            return None;
        }
        joiner = Some(and);
        *at += 1;
        skip_ws(tokens, at);
        joined.push(parse_in_parens(tokens, at, depth)?);
    }
    Some(match joiner {
        None => joined.pop().expect("one condition"),
        Some(true) => Condition::And(joined),
        Some(false) => Condition::Or(joined),
    })
}

fn skip_ws(tokens: &[Token], at: &mut usize) {
    while matches!(tokens.get(*at), Some(Token::Whitespace)) {
        *at += 1;
    }
}

/// Parses a `<query-in-parens>` at `at`: a condition or a size feature in
/// parentheses, `style()`, or anything else in parentheses or a function,
/// unknown.
fn parse_in_parens(tokens: &[Token], at: &mut usize, depth: usize) -> Option<Condition> {
    let rest = &tokens[*at..];
    let (inside, after) = match rest.first() {
        Some(Token::OpenParen | Token::Function(_)) => block(rest)?,
        _ => return None,
    };
    let length = rest.len() - after.len();
    let whole = &rest[..length];
    *at += length;
    let unknown = || Condition::Unknown(crate::serialize::tokens(whole).trim().to_owned());
    if depth > MAX_NESTING {
        return Some(unknown());
    }
    match &rest[0] {
        Token::Function(name) if name.eq_ignore_ascii_case("style") => {
            let inside = trim(inside);
            let mut pos = 0;
            match parse_style_condition(inside, &mut pos, depth + 1) {
                Some(condition) if pos == inside.len() => {
                    Some(Condition::Style(Box::new(condition)))
                }
                _ => Some(unknown()),
            }
        }
        Token::Function(name) if name.eq_ignore_ascii_case("scroll-state") => {
            let inside = trim(inside);
            let mut pos = 0;
            match parse_scroll_condition(inside, &mut pos, depth + 1) {
                Some(condition) if pos == inside.len() => {
                    Some(Condition::ScrollState(Box::new(condition)))
                }
                _ => Some(unknown()),
            }
        }
        Token::Function(name) if name.eq_ignore_ascii_case("anchored") => {
            let inside = trim(inside);
            let mut pos = 0;
            match parse_anchored_condition(inside, &mut pos, depth + 1) {
                Some(condition) if pos == inside.len() => {
                    Some(Condition::Anchored(Box::new(condition)))
                }
                _ => Some(unknown()),
            }
        }
        Token::Function(_) => Some(unknown()),
        _ => {
            let inside = trim(inside);
            let mut pos = 0;
            if let Some(condition) = parse_condition(inside, &mut pos, depth + 1)
                && pos == inside.len()
            {
                return Some(Condition::Parens(Box::new(condition)));
            }
            Some(match parse_size_feature(inside) {
                Some(feature) => Condition::Size(feature),
                None => unknown(),
            })
        }
    }
}

/// The comparison at the front of `tokens`, `<`, `<=`, `>`, `>=` or `=`,
/// and how many tokens it takes.
fn comparison(tokens: &[Token]) -> Option<(Comparison, usize)> {
    let equals = matches!(tokens.get(1), Some(Token::Delim('=')));
    Some(match tokens.first()? {
        Token::Delim('<') if equals => (Comparison::Le, 2),
        Token::Delim('<') => (Comparison::Lt, 1),
        Token::Delim('>') if equals => (Comparison::Ge, 2),
        Token::Delim('>') => (Comparison::Gt, 1),
        Token::Delim('=') => (Comparison::Eq, 1),
        _ => return None,
    })
}

/// The pieces of a range's tokens: operands and the comparisons between
/// them; `None` when two comparisons or two operands meet.
fn range_parts(tokens: &[Token]) -> Option<(Vec<Vec<Token>>, Vec<Comparison>)> {
    let mut operands: Vec<Vec<Token>> = vec![Vec::new()];
    let mut comparisons = Vec::new();
    let mut at = 0;
    let mut depth = 0usize;
    while at < tokens.len() {
        if depth == 0
            && let Some((c, len)) = comparison(&tokens[at..])
        {
            comparisons.push(c);
            operands.push(Vec::new());
            at += len;
            continue;
        }
        match &tokens[at] {
            Token::Function(_) | Token::OpenParen | Token::OpenSquare => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
            _ => {}
        }
        operands.last_mut().expect("one").push(tokens[at].clone());
        at += 1;
    }
    let operands: Vec<Vec<Token>> = operands.iter().map(|o| trim(o).to_vec()).collect();
    if operands.iter().any(Vec::is_empty) {
        return None;
    }
    Some((operands, comparisons))
}

/// The size feature in a pair of parentheses' `tokens`.
fn parse_size_feature(tokens: &[Token]) -> Option<SizeFeature> {
    let feature_name = |tokens: &[Token]| match tokens {
        [Token::Ident(name)] => Some(name.to_ascii_lowercase()),
        _ => None,
    };
    // `(name)`.
    if let Some(name) = feature_name(tokens) {
        return Name::of(&name).map(SizeFeature::Boolean);
    }
    // `(name: value)`.
    if let Some(colon) = tokens.iter().position(|t| matches!(t, Token::Colon)) {
        let name = feature_name(trim(&tokens[..colon]))?;
        let (prefix, bare) = match (name.strip_prefix("min-"), name.strip_prefix("max-")) {
            (Some(bare), _) => (Some(Comparison::Ge), bare),
            (_, Some(bare)) => (Some(Comparison::Le), bare),
            _ => (None, name.as_str()),
        };
        let name = Name::of(bare)?;
        if prefix.is_some() && name == Name::Orientation {
            return None;
        }
        let value = parse_value(name, trim(&tokens[colon + 1..]))?;
        return Some(SizeFeature::Plain(name, prefix, value));
    }
    // The range forms.
    let (operands, comparisons) = range_parts(tokens)?;
    let named: Vec<Option<Name>> = operands
        .iter()
        .map(|o| feature_name(o).and_then(|n| Name::of(&n)))
        .collect();
    match (operands.len(), &named[..]) {
        (2, [Some(name), None]) => {
            let value = parse_value(*name, &operands[1])?;
            Some(SizeFeature::Range(
                *name,
                None,
                Some((comparisons[0], value)),
            ))
        }
        (2, [None, Some(name)]) => {
            let value = parse_value(*name, &operands[0])?;
            Some(SizeFeature::Range(
                *name,
                Some((value, comparisons[0])),
                None,
            ))
        }
        (3, [None, Some(name), None])
            if comparisons[0].same_way(comparisons[1])
                && !comparisons.contains(&Comparison::Eq) =>
        {
            let low = parse_value(*name, &operands[0])?;
            let high = parse_value(*name, &operands[2])?;
            Some(SizeFeature::Range(
                *name,
                Some((low, comparisons[0])),
                Some((comparisons[1], high)),
            ))
        }
        _ => None,
    }
}

/// A size feature's value: a length (a dimension of a length unit, `0`,
/// or a math function of a length), a ratio, or an orientation.
fn parse_value(name: Name, tokens: &[Token]) -> Option<Value> {
    match name {
        Name::Orientation => match tokens {
            [Token::Ident(word)] if word.eq_ignore_ascii_case("portrait") => Some(Value::Portrait),
            [Token::Ident(word)] if word.eq_ignore_ascii_case("landscape") => {
                Some(Value::Landscape)
            }
            _ => None,
        },
        Name::AspectRatio => {
            let numbers: Vec<&Token> = tokens
                .iter()
                .filter(|t| !matches!(t, Token::Whitespace))
                .collect();
            let number = |t: &Token| match t {
                Token::Number { value, .. } if *value >= 0.0 => Some(*value),
                _ => None,
            };
            match numbers[..] {
                [a] => Some(Value::Ratio(number(a)?, 1.0)),
                [a, Token::Delim('/'), b] => Some(Value::Ratio(number(a)?, number(b)?)),
                _ => None,
            }
        }
        _ => {
            // A `var()` is substituted when the query is evaluated.
            let substituted = tokens
                .iter()
                .any(|t| matches!(t, Token::Function(name) if name.eq_ignore_ascii_case("var")));
            let length = substituted
                || match tokens {
                    [Token::Dimension { unit, .. }] => calc::LengthUnit::of(unit).is_some(),
                    [Token::Number { value, .. }] => *value == 0.0,
                    [Token::Function(_), ..] => calc::parse(tokens, calc::Want::Length).is_some(),
                    _ => false,
                };
            length.then(|| Value::Length(tokens.to_vec()))
        }
    }
}

/// Parses a style query's condition at `at`.
fn parse_style_condition(tokens: &[Token], at: &mut usize, depth: usize) -> Option<StyleCondition> {
    skip_ws(tokens, at);
    if depth > MAX_NESTING {
        return None;
    }
    // A feature alone, not in parentheses: what the whole is.
    if !matches!(tokens.get(*at), Some(Token::OpenParen))
        && !matches!(tokens.get(*at), Some(Token::Ident(w)) if w.eq_ignore_ascii_case("not"))
    {
        let feature = parse_style_feature(&tokens[*at..])?;
        *at = tokens.len();
        return Some(feature);
    }
    if let Some(Token::Ident(word)) = tokens.get(*at)
        && word.eq_ignore_ascii_case("not")
    {
        *at += 1;
        skip_ws(tokens, at);
        let inner = parse_style_in_parens(tokens, at, depth)?;
        skip_ws(tokens, at);
        return Some(StyleCondition::Not(Box::new(inner)));
    }
    let first = parse_style_in_parens(tokens, at, depth)?;
    let mut joined = vec![first];
    let mut joiner: Option<bool> = None;
    loop {
        skip_ws(tokens, at);
        let Some(Token::Ident(word)) = tokens.get(*at) else {
            break;
        };
        let and = if word.eq_ignore_ascii_case("and") {
            true
        } else if word.eq_ignore_ascii_case("or") {
            false
        } else {
            return None;
        };
        if joiner.is_some_and(|was| was != and) {
            return None;
        }
        joiner = Some(and);
        *at += 1;
        skip_ws(tokens, at);
        joined.push(parse_style_in_parens(tokens, at, depth)?);
    }
    Some(match joiner {
        None => joined.pop().expect("one condition"),
        Some(true) => StyleCondition::And(joined),
        Some(false) => StyleCondition::Or(joined),
    })
}

/// A style condition in parentheses: a condition, or a feature; anything
/// else unknown.
fn parse_style_in_parens(tokens: &[Token], at: &mut usize, depth: usize) -> Option<StyleCondition> {
    let rest = &tokens[*at..];
    if !matches!(rest.first(), Some(Token::OpenParen)) {
        return None;
    }
    let (inside, after) = block(rest)?;
    let length = rest.len() - after.len();
    *at += length;
    let inside = trim(inside);
    let mut pos = 0;
    let parsed = if matches!(inside.first(), Some(Token::OpenParen))
        || matches!(inside.first(), Some(Token::Ident(w)) if w.eq_ignore_ascii_case("not"))
    {
        parse_style_condition(inside, &mut pos, depth + 1).filter(|_| pos == inside.len())
    } else {
        parse_style_feature(inside)
    };
    Some(StyleCondition::Parens(Box::new(parsed.unwrap_or_else(
        || StyleCondition::Unknown(crate::serialize::tokens(inside).trim().to_owned()),
    ))))
}

/// A style feature: a custom property's name, with a value after a colon
/// or not, or the range form; `None` for what is neither.
fn parse_style_feature(tokens: &[Token]) -> Option<StyleCondition> {
    let tokens = trim(tokens);
    match tokens {
        [Token::Ident(name)] if name.starts_with("--") => {
            return Some(StyleCondition::Declaration(name.clone(), None));
        }
        [Token::Ident(name), rest @ ..] if name.starts_with("--") => {
            let rest = trim(rest);
            if let [Token::Colon, value @ ..] = rest {
                // A value is what a custom property takes: no `;` and no
                // `!` but `!important` at its end.
                if value
                    .iter()
                    .any(|t| matches!(t, Token::Semicolon | Token::CloseCurly))
                {
                    return None;
                }
                let value = without_important(trim(value));
                return Some(StyleCondition::Declaration(
                    name.clone(),
                    Some(value.to_vec()),
                ));
            }
        }
        _ => {}
    }
    // A declaration of a property not custom: unknown.
    if tokens.iter().any(|t| matches!(t, Token::Colon)) {
        return None;
    }
    let (operands, comparisons) = range_parts(tokens)?;
    let ok = match comparisons.len() {
        1 => true,
        2 => comparisons[0].same_way(comparisons[1]),
        _ => false,
    };
    let bang = operands
        .iter()
        .flatten()
        .any(|t| matches!(t, Token::Delim('!')));
    (ok && !bang).then_some(StyleCondition::Range(operands, comparisons))
}

/// A value's tokens without a trailing `!important`.
fn without_important(value: &[Token]) -> &[Token] {
    if let [rest @ .., Token::Delim('!'), Token::Ident(word)] = value
        && word.eq_ignore_ascii_case("important")
    {
        return trim(rest);
    }
    value
}

/// Three-valued logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Truth {
    True,
    False,
    Unknown,
}

impl Truth {
    fn of(b: bool) -> Truth {
        if b { Truth::True } else { Truth::False }
    }

    fn not(self) -> Truth {
        match self {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            Truth::Unknown => Truth::Unknown,
        }
    }

    fn and(items: impl Iterator<Item = Truth>) -> Truth {
        let mut out = Truth::True;
        for item in items {
            match item {
                Truth::False => return Truth::False,
                Truth::Unknown => out = Truth::Unknown,
                Truth::True => {}
            }
        }
        out
    }

    fn or(items: impl Iterator<Item = Truth>) -> Truth {
        let mut out = Truth::False;
        for item in items {
            match item {
                Truth::True => return Truth::True,
                Truth::Unknown => out = Truth::Unknown,
                Truth::False => {}
            }
        }
        out
    }
}

impl Condition {
    fn text(&self) -> String {
        match self {
            Condition::Not(inner) => format!("not {}", inner.text()),
            Condition::And(items) => join(items.iter().map(Condition::text), " and "),
            Condition::Or(items) => join(items.iter().map(Condition::text), " or "),
            Condition::Parens(inner) => format!("({})", inner.text()),
            Condition::Size(feature) => feature.text(),
            Condition::Style(inner) => format!("style({})", inner.text()),
            Condition::ScrollState(inner) => format!("scroll-state({})", inner.text()),
            Condition::Anchored(inner) => format!("anchored({})", inner.text()),
            Condition::Unknown(text) => text.clone(),
        }
    }

    /// Whether a `<general-enclosed>` is in it, at any depth.
    fn encloses_unknown(&self) -> bool {
        match self {
            Condition::Not(inner) | Condition::Parens(inner) => inner.encloses_unknown(),
            Condition::And(items) | Condition::Or(items) => {
                items.iter().any(Condition::encloses_unknown)
            }
            Condition::Style(inner) => inner.encloses_unknown(),
            Condition::Unknown(_) => true,
            Condition::Size(_) | Condition::ScrollState(_) | Condition::Anchored(_) => false,
        }
    }

    fn needs(&self, needs: &mut Needs) {
        match self {
            Condition::Not(inner) | Condition::Parens(inner) => inner.needs(needs),
            Condition::And(items) | Condition::Or(items) => {
                for item in items {
                    item.needs(needs);
                }
            }
            Condition::Size(feature) => {
                let block = feature.name().block_axis();
                needs.size = Some(needs.size.unwrap_or(false) || block);
            }
            Condition::ScrollState(_) => needs.scroll_state = true,
            Condition::Anchored(_) => needs.anchored = true,
            Condition::Style(_) | Condition::Unknown(_) => {}
        }
    }

    fn evaluate(&self, node: NodeId, style: &ComputedStyle, c: &Containers<'_>) -> Truth {
        match self {
            Condition::Not(inner) => inner.evaluate(node, style, c).not(),
            Condition::And(items) => Truth::and(items.iter().map(|i| i.evaluate(node, style, c))),
            Condition::Or(items) => Truth::or(items.iter().map(|i| i.evaluate(node, style, c))),
            Condition::Parens(inner) => inner.evaluate(node, style, c),
            Condition::Size(feature) => feature.evaluate(node, style, c),
            Condition::Style(inner) => inner.evaluate(node, style, c),
            Condition::ScrollState(inner) => inner.evaluate(node, style, c),
            Condition::Anchored(inner) => inner.evaluate(),
            Condition::Unknown(_) => Truth::Unknown,
        }
    }
}

fn join(items: impl Iterator<Item = String>, by: &str) -> String {
    items.collect::<Vec<_>>().join(by)
}

/// A value's tokens as text, white space as one space and trimmed.
fn value_text(tokens: &[Token]) -> String {
    crate::serialize::tokens(tokens).trim().to_owned()
}

impl Value {
    fn text(&self) -> String {
        match self {
            Value::Length(tokens) => value_text(tokens),
            Value::Ratio(a, b) => format!(
                "{} / {}",
                crate::color::number(*a as f32),
                crate::color::number(*b as f32)
            ),
            Value::Portrait => "portrait".to_owned(),
            Value::Landscape => "landscape".to_owned(),
        }
    }
}

impl SizeFeature {
    fn name(&self) -> Name {
        match self {
            SizeFeature::Boolean(name)
            | SizeFeature::Plain(name, ..)
            | SizeFeature::Range(name, ..) => *name,
        }
    }

    fn text(&self) -> String {
        match self {
            SizeFeature::Boolean(name) => format!("({})", name.text()),
            SizeFeature::Plain(name, prefix, value) => {
                let prefix = match prefix {
                    Some(Comparison::Ge) => "min-",
                    Some(_) => "max-",
                    None => "",
                };
                format!("({prefix}{}: {})", name.text(), value.text())
            }
            SizeFeature::Range(name, before, after) => {
                let mut out = String::from("(");
                if let Some((value, c)) = before {
                    out.push_str(&format!("{} {} ", value.text(), c.text()));
                }
                out.push_str(name.text());
                if let Some((c, value)) = after {
                    out.push_str(&format!(" {} {}", c.text(), value.text()));
                }
                out.push(')');
                out
            }
        }
    }

    /// The feature's answer for `node`, the container: unknown when it
    /// was not laid out as a size container or makes no box that size
    /// containment applies to.
    fn evaluate(&self, node: NodeId, style: &ComputedStyle, c: &Containers<'_>) -> Truth {
        let size = c.size_of(node);
        let Some((width, height)) = size.filter(|_| eligible(style)) else {
            return Truth::Unknown;
        };
        let name = self.name();
        let actual = match name {
            Name::Width | Name::InlineSize => f64::from(width),
            Name::Height | Name::BlockSize => f64::from(height),
            Name::AspectRatio => f64::from(width) / f64::from(height),
            Name::Orientation => {
                return match self {
                    SizeFeature::Boolean(_) => Truth::True,
                    SizeFeature::Plain(_, _, Value::Portrait) => Truth::of(height >= width),
                    SizeFeature::Plain(_, _, Value::Landscape) => Truth::of(width > height),
                    _ => Truth::Unknown,
                };
            }
        };
        let resolve = |value: &Value| -> Option<f64> {
            match value {
                Value::Length(tokens) => c.length(node, style, tokens),
                Value::Ratio(a, b) => Some(a / b),
                _ => None,
            }
        };
        let holds = |a: f64, comparison: Comparison, b: f64| {
            // A ratio compares as a fraction, `0/0` as infinite.
            Truth::of(comparison.holds(a, b))
        };
        match self {
            SizeFeature::Boolean(_) => Truth::of(actual != 0.0 && !actual.is_nan()),
            SizeFeature::Plain(_, prefix, value) => match resolve(value) {
                Some(v) => holds(actual, prefix.unwrap_or(Comparison::Eq), v),
                None => Truth::Unknown,
            },
            SizeFeature::Range(_, before, after) => {
                let mut out = Truth::True;
                if let Some((value, comparison)) = before {
                    match resolve(value) {
                        Some(v) => {
                            out = Truth::and([out, holds(v, *comparison, actual)].into_iter())
                        }
                        None => return Truth::Unknown,
                    }
                }
                if let Some((comparison, value)) = after {
                    match resolve(value) {
                        Some(v) => {
                            out = Truth::and([out, holds(actual, *comparison, v)].into_iter())
                        }
                        None => return Truth::Unknown,
                    }
                }
                out
            }
        }
    }
}

/// Whether size containment applies to a box of `style`, which a size
/// query asks: one that makes a principal box whose inner display type
/// is not `table`, and that is not an internal table or ruby box nor an
/// inline box that is not atomic (CSS Contain 2 section 3.2). A table
/// and its cells so take their sizes from their content, and a query of
/// one is unknown, as in the reference browser.
pub fn eligible(style: &ComputedStyle) -> bool {
    if style.display == Display::InlineListItem {
        return style.flow_root;
    }
    !matches!(
        style.display,
        Display::None
            | Display::Contents
            | Display::Inline
            | Display::Ruby
            | Display::Table
            | Display::InlineTable
            | Display::TableRowGroup
            | Display::TableHeaderGroup
            | Display::TableFooterGroup
            | Display::TableRow
            | Display::TableColumnGroup
            | Display::TableColumn
            | Display::TableCell
            | Display::RubyBase
            | Display::RubyText
            | Display::RubyBaseContainer
            | Display::RubyTextContainer
    )
}

impl StyleCondition {
    /// Whether a `<general-enclosed>` is in it, at any depth.
    fn encloses_unknown(&self) -> bool {
        match self {
            StyleCondition::Not(inner) | StyleCondition::Parens(inner) => inner.encloses_unknown(),
            StyleCondition::And(items) | StyleCondition::Or(items) => {
                items.iter().any(StyleCondition::encloses_unknown)
            }
            StyleCondition::Unknown(_) => true,
            StyleCondition::Declaration(..) | StyleCondition::Range(..) => false,
        }
    }

    fn text(&self) -> String {
        match self {
            StyleCondition::Not(inner) => format!("not {}", inner.text()),
            StyleCondition::And(items) => join(items.iter().map(StyleCondition::text), " and "),
            StyleCondition::Or(items) => join(items.iter().map(StyleCondition::text), " or "),
            StyleCondition::Parens(inner) => format!("({})", inner.text()),
            StyleCondition::Declaration(name, None) => crate::media::ident(name),
            StyleCondition::Declaration(name, Some(value)) => {
                format!("{}: {}", crate::media::ident(name), value_text(value))
            }
            StyleCondition::Range(operands, comparisons) => {
                let mut out = String::new();
                for (at, operand) in operands.iter().enumerate() {
                    if at > 0 {
                        out.push_str(&format!(" {} ", comparisons[at - 1].text()));
                    }
                    match operand.as_slice() {
                        [Token::Ident(name)] => out.push_str(&crate::media::ident(name)),
                        _ => out.push_str(&value_text(operand)),
                    }
                }
                out
            }
            StyleCondition::Unknown(text) => text.clone(),
        }
    }

    fn evaluate(&self, node: NodeId, style: &ComputedStyle, c: &Containers<'_>) -> Truth {
        match self {
            StyleCondition::Not(inner) => inner.evaluate(node, style, c).not(),
            StyleCondition::And(items) => {
                Truth::and(items.iter().map(|i| i.evaluate(node, style, c)))
            }
            StyleCondition::Or(items) => {
                Truth::or(items.iter().map(|i| i.evaluate(node, style, c)))
            }
            StyleCondition::Parens(inner) => inner.evaluate(node, style, c),
            StyleCondition::Declaration(name, value) => {
                Truth::of(c.declaration_holds(node, style, name, value.as_deref()))
            }
            StyleCondition::Range(operands, comparisons) => {
                let values: Option<Vec<(f64, u8)>> = operands
                    .iter()
                    .map(|operand| c.style_number(node, style, operand))
                    .collect();
                let Some(values) = values else {
                    return Truth::False;
                };
                // Each pair alike in kind, a number zero alike with a
                // length, and in order.
                let zero_length = |a: (f64, u8), b: (f64, u8)| a == (0.0, 0) && b.1 == 1;
                let alike = |a: (f64, u8), b: (f64, u8)| {
                    a.1 == b.1 || zero_length(a, b) || zero_length(b, a)
                };
                Truth::of(
                    values
                        .windows(2)
                        .zip(comparisons)
                        .all(|(pair, comparison)| {
                            alike(pair[0], pair[1]) && comparison.holds(pair[0].0, pair[1].0)
                        }),
                )
            }
            StyleCondition::Unknown(_) => Truth::Unknown,
        }
    }
}

// ---- Evaluation -----------------------------------------------------------

/// What a query or a container unit reads of the document: its elements'
/// computed styles, which an element's ancestors already have when the
/// element is styled, the containers' sizes as laid out, and where the
/// containers asked about are recorded.
#[derive(Clone, Copy)]
pub(crate) struct Containers<'a> {
    pub(crate) document: &'a Document,
    pub(crate) styles: &'a Styles,
    /// An element whose style is being computed, with that style, which
    /// `styles` do not have yet: what its pseudo-elements' queries read
    /// of it, their first query container.
    pub(crate) own: Option<(NodeId, &'a ComputedStyle)>,
    pub(crate) sizes: &'a ContainerSizes,
    pub(crate) queried: &'a RefCell<Queried>,
    /// The substitution of `var()` in a style query's value.
    pub(crate) substitutions: &'a Substitutions,
    /// The viewport, for the units of a query's lengths and the
    /// container units' fallback.
    pub(crate) viewport: (f32, f32),
    /// The root element's font size, for `rem`.
    pub(crate) rem: f32,
    /// What measures a font's figures, for `ex`, `ch`, `cap` and `ic`;
    /// their fallbacks without one.
    pub(crate) fonts: Option<&'a dyn crate::cascade::FontMeasure>,
}

impl Containers<'_> {
    /// `node`'s computed style: [`Containers::own`]'s for its element.
    fn style(&self, node: NodeId) -> Option<&ComputedStyle> {
        match self.own {
            Some((own, style)) if own == node => Some(style),
            _ => self.styles.get(node),
        }
    }

    /// `node`'s content box as laid out, recorded as asked about.
    fn size_of(&self, node: NodeId) -> Option<(f32, f32)> {
        self.state_of(node).0
    }

    /// `node`'s content box and scroll state as laid out, recorded as
    /// asked about.
    fn state_of(&self, node: NodeId) -> ContainerState {
        let state = (self.sizes.get(node), self.sizes.scroll(node));
        self.queried.borrow_mut().insert(node, state);
        state
    }

    /// The sizes the container units of an element whose query
    /// containers are `from` and its ancestors measure (section 7.3):
    /// the nearest eligible size container's width, and the nearest
    /// eligible `size` one's height; the small viewport's where none is,
    /// or it was not laid out.
    pub(crate) fn unit_sizes(&self, from: Option<NodeId>) -> (f32, f32) {
        let (mut width, mut height) = (None, None);
        let mut at = from;
        while let Some(node) = at {
            at = self.document.flat_parent_element(node);
            if width.is_some() && height.is_some() {
                break;
            }
            let Some(style) = self.style(node) else {
                continue;
            };
            let kind = style.container_type.size;
            if kind == ContainerSize::None || !eligible(style) {
                continue;
            }
            let size = self.size_of(node);
            if width.is_none() {
                width = Some(size.map(|(w, _)| w));
            }
            if height.is_none() && kind == ContainerSize::Both {
                height = Some(size.map(|(_, h)| h));
            }
        }
        (
            width.flatten().unwrap_or(self.viewport.0),
            height.flatten().unwrap_or(self.viewport.1),
        )
    }

    /// A length of a query of `node`, the container, in pixels: its
    /// relative units the container's (section 7.4.2), its container
    /// units the container's own containers'.
    fn length(&self, node: NodeId, style: &ComputedStyle, tokens: &[Token]) -> Option<f64> {
        // Its `var()` against the container's custom properties.
        let substituted;
        let tokens = if tokens
            .iter()
            .any(|t| matches!(t, Token::Function(name) if name.eq_ignore_ascii_case("var")))
        {
            substituted =
                match self
                    .substitutions
                    .substitute(tokens, None, style.custom.as_ref(), &[], false)
                {
                    Substituted::Value(value) => value,
                    _ => return None,
                };
            trim(&substituted)
        } else {
            tokens
        };
        let units = Units {
            containers: self,
            node,
            style,
        };
        match tokens {
            [Token::Number { value, .. }] if *value == 0.0 => Some(0.0),
            [Token::Dimension { value, unit, .. }] => {
                let unit = calc::LengthUnit::of(unit)?;
                Some(match unit.absolute() {
                    Some(px) => value * px,
                    None => calc::Lengths::px(&units, *value, unit),
                })
            }
            _ => match calc::parse(tokens, calc::Want::Length)?.compute(&units, None) {
                calc::Computed::Value(v) => Some(v),
                _ => None,
            },
        }
    }

    /// Whether `style`'s custom property `name` has a value, and that
    /// `value` once substituted in the container, when one is given
    /// (section 7.4.3).
    fn declaration_holds(
        &self,
        node: NodeId,
        style: &ComputedStyle,
        name: &str,
        value: Option<&[Token]>,
    ) -> bool {
        let own = style.custom_property(name);
        // A CSS-wide keyword is the value it would give the container's
        // property: the initial one, or the parent's.
        if let Some(keyword) = value.and_then(crate::cascade::css_wide_keyword) {
            let registered = self.substitutions.registry.get(name);
            let inherits = registered.is_none_or(|r| r.inherits);
            let initial = registered.and_then(|r| r.initial.as_deref());
            let inherited = || {
                self.document
                    .flat_parent_element(node)
                    .and_then(|parent| self.style(parent))
                    .and_then(|parent| parent.custom_property(name))
            };
            let wanted = match keyword {
                crate::cascade::CssWide::Initial => initial,
                crate::cascade::CssWide::Inherit => inherited(),
                crate::cascade::CssWide::Unset if inherits => inherited(),
                crate::cascade::CssWide::Unset => initial,
            };
            return own == wanted;
        }
        let Some(value) = value else {
            // Any value but the initial one.
            let registered = self.substitutions.registry.get(name);
            return match (own, registered) {
                (Some(own), Some(registration)) => registration.initial.as_deref() != Some(own),
                (Some(_), None) => true,
                (None, _) => false,
            };
        };
        let substituted =
            match self
                .substitutions
                .substitute(value, None, style.custom.as_ref(), &[], false)
            {
                Substituted::Value(value) => value,
                _ => return false,
            };
        let wanted = match self.substitutions.registry.get(name) {
            Some(registration) if !registration.syntax.is_universal() => {
                match crate::cascade::compute_registered_alone(&registration.syntax, &substituted) {
                    Some(computed) => computed.into(),
                    None => return false,
                }
            }
            _ => substituted,
        };
        let text = |tokens: &[Token]| value_text(tokens);
        match own {
            Some(own) => text(own) == text(&wanted),
            None => text(&wanted).is_empty(),
        }
    }

    /// An operand of a style query's range form as a number and its
    /// kind: a custom property's computed value, or a value with its
    /// `var()` substituted, a number (kind 0), a length in pixels (1) or
    /// a percentage (2); `None` for what is none of them.
    fn style_number(
        &self,
        node: NodeId,
        style: &ComputedStyle,
        operand: &[Token],
    ) -> Option<(f64, u8)> {
        let value: Arc<[Token]> = match operand {
            [Token::Ident(name)] if name.starts_with("--") => {
                Arc::from(style.custom_property(name)?)
            }
            _ => match self.substitutions.substitute(
                operand,
                None,
                style.custom.as_ref(),
                &[],
                false,
            ) {
                Substituted::Value(value) => value,
                _ => return None,
            },
        };
        let value = trim(&value);
        // Angles in degrees, times in seconds, resolutions in dots per
        // pixel, each a kind of its own.
        let dimension = |value: f64, unit: &str| -> Option<(f64, u8)> {
            Some(match unit.to_ascii_lowercase().as_str() {
                "deg" => (value, 3),
                "grad" => (value * 0.9, 3),
                "rad" => (value.to_degrees(), 3),
                "turn" => (value * 360.0, 3),
                "s" => (value, 4),
                "ms" => (value / 1000.0, 4),
                "dppx" | "x" => (value, 5),
                "dpi" => (value / 96.0, 5),
                "dpcm" => (value * 2.54 / 96.0, 5),
                _ => return None,
            })
        };
        match value {
            [Token::Number { value, .. }] => Some((*value, 0)),
            [Token::Percentage { value, .. }] => Some((*value, 2)),
            [Token::Dimension { value, unit, .. }] if dimension(*value, unit).is_some() => {
                dimension(*value, unit)
            }
            _ => {
                if let Some(px) = self.length(node, style, value) {
                    return Some((px, 1));
                }
                let number = calc::parse(value, calc::Want::Number)?;
                let units = Units {
                    containers: self,
                    node,
                    style,
                };
                match number.compute(&units, None) {
                    calc::Computed::Value(v) => Some((v, 0)),
                    _ => None,
                }
            }
        }
    }
}

/// The units of a container query's lengths: the relative ones of the
/// container's style, `style`.
struct Units<'c, 'a> {
    containers: &'c Containers<'a>,
    node: NodeId,
    style: &'c ComputedStyle,
}

impl Units<'_, '_> {
    /// The figures of `font` at `size`, by the measure the cascade has.
    fn figures(&self, font: &crate::computed::FontProps, size: f32) -> crate::cascade::FontUnits {
        match self.containers.fonts {
            Some(fonts) => fonts.units(font, size),
            None => crate::cascade::FontUnits::fallback(size),
        }
    }
}

impl calc::Lengths for Units<'_, '_> {
    fn px(&self, value: f64, unit: calc::LengthUnit) -> f64 {
        use calc::LengthUnit as U;
        let (vw, vh) = self.containers.viewport;
        let container = |pick: fn(f32, f32) -> f32| {
            let parent = self.containers.document.flat_parent_element(self.node);
            let (w, h) = self.containers.unit_sizes(parent);
            value * f64::from(pick(w, h)) / 100.0
        };
        match unit {
            U::Em => value * f64::from(self.style.font_size_px),
            U::Ex | U::Ch | U::Cap | U::Ic | U::Lh => {
                let size = self.style.font_size_px;
                let figures = self.figures(self.style.font(), size);
                let figure = match unit {
                    U::Ex => figures.x_height,
                    U::Ch => figures.zero_advance,
                    U::Cap => figures.cap_height,
                    U::Ic => figures.ic_advance,
                    _ => match self.style.font().line_height {
                        crate::computed::LineHeight::Normal => figures.normal_line_height,
                        crate::computed::LineHeight::Number(n) => n * size,
                        crate::computed::LineHeight::Px(px) => px,
                    },
                };
                value * f64::from(figure)
            }
            U::Rem => value * f64::from(self.containers.rem),
            // The root's figures, of the initial font at its size.
            U::Rex | U::Rch | U::Rcap | U::Ric | U::Rlh => {
                let rem = self.containers.rem;
                let figures = self.figures(crate::computed::initial_font(), rem);
                value
                    * f64::from(match unit {
                        U::Rex => figures.x_height,
                        U::Rch => figures.zero_advance,
                        U::Rcap => figures.cap_height,
                        U::Ric => figures.ic_advance,
                        _ => figures.normal_line_height,
                    })
            }
            U::Vw => value * f64::from(vw) / 100.0,
            U::Vh => value * f64::from(vh) / 100.0,
            U::Vmin => value * f64::from(vw.min(vh)) / 100.0,
            U::Vmax => value * f64::from(vw.max(vh)) / 100.0,
            U::Cqw | U::Cqi => container(|w, _| w),
            U::Cqh | U::Cqb => container(|_, h| h),
            U::Cqmin => container(f32::min),
            U::Cqmax => container(f32::max),
            absolute => value * absolute.absolute().unwrap_or(1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CascadeContext, Stylesheets};

    fn prelude(text: &str) -> Option<(String, Vec<(String, String)>)> {
        container_prelude(&css::tokenize(text))
    }

    #[test]
    fn preludes_parse_and_serialize_as_the_reference_browser_writes_them() {
        let text = |t: &str| prelude(t).map(|(text, _)| text);
        assert_eq!(text("(  wiDTh  )").as_deref(), Some("(width)"));
        assert_eq!(
            text("(   MAX-WIDTH:100px  )").as_deref(),
            Some("(max-width: 100px)")
        );
        assert_eq!(
            text("(100px>WIDTH>10px)").as_deref(),
            Some("(100px > width > 10px)")
        );
        assert_eq!(
            text("(width),(height) ,--foo ,--bar").as_deref(),
            Some("(width), (height), --foo, --bar")
        );
        assert_eq!(text("STyle(--foo:)").as_deref(), Some("style(--foo: )"));
        assert_eq!(
            text("scroll-state((scrollable: left) or (stuck: top))").as_deref(),
            Some("scroll-state((scrollable: left) or (stuck: top))")
        );
        // Unknown, and kept as written.
        assert_eq!(text("(asdf)").as_deref(), Some("(asdf)"));
        assert_eq!(
            text("scroll-state(stuck: x)").as_deref(),
            Some("scroll-state(stuck: x)")
        );
        for invalid in [
            "",
            "(width),",
            "foo foo",
            "none",
            "not",
            "screen and (width)",
            "\"a\"",
        ] {
            assert!(prelude(invalid).is_none(), "{invalid}");
        }
        let (_, conditions) = prelude("(width > 300px), Name (width < 1000px)").unwrap();
        assert_eq!(
            conditions,
            [
                (String::new(), "(width > 300px)".to_owned()),
                ("Name".to_owned(), "(width < 1000px)".to_owned())
            ]
        );
    }

    /// Whether `query` holds for the element `#t` of `html` styled by
    /// `css`, its container `#c` laid out at `size` and able to scroll
    /// towards `scroll`'s sides.
    fn holds(html: &str, css: &str, query: &str, size: (f32, f32), scroll: [bool; 4]) -> bool {
        let state = ScrollState {
            scrollable: scroll,
            ..ScrollState::default()
        };
        holds_in(html, css, query, size, state)
    }

    /// [`holds`] with the container in the scroll `state`.
    fn holds_in(html: &str, css: &str, query: &str, size: (f32, f32), state: ScrollState) -> bool {
        let document = dom::Document::from_node(&html::parse_document(html).unwrap());
        let node = |id: &str| {
            document
                .subtree(document.root())
                .find(|&n| document.attr(n, "id") == Some(id))
                .unwrap()
        };
        let sheet = format!("{css} @container {query} {{ #t {{ --hit: 1 }} }}");
        let sheets = Stylesheets::new(vec![css::parse_stylesheet(&sheet).unwrap()]);
        let context = CascadeContext::default();
        let mut styles = sheets.cascade(&document, &context);
        let mut sizes = ContainerSizes::new();
        sizes.insert(node("c"), size.0, size.1);
        sizes.insert_scroll(node("c"), state);
        sheets.restyle_containers(&document, &context, &mut styles, &sizes);
        styles
            .get(node("t"))
            .unwrap()
            .custom_property("--hit")
            .is_some()
    }

    #[test]
    fn queries_are_three_valued_and_a_general_enclosed_leaves_them_unknown() {
        let html = "<div id=c><p id=t>x</p></div>";
        let css = "#c { container-type: size }";
        let at = |query: &str| holds(html, css, query, (100.0, 50.0), [false; 4]);
        assert!(at("(width = 100px)"));
        assert!(at("(orientation: landscape) and (aspect-ratio > 1)"));
        assert!(at("(50px < height <= 50px) or (width > 0)"));
        assert!(!at("(height > 50px)"));
        assert!(at("not (height > 50px)"));
        // A `<general-enclosed>` anywhere: unknown, whatever else holds.
        assert!(!at("(width) or (unknown)"));
        assert!(!at("not (unknown)"));
        // A feature an inline-size container cannot answer selects no
        // container; style queries read the container's properties.
        let inline = "#c { container-type: inline-size; --x: y; --n: 5 }";
        let at = |query: &str| holds(html, inline, query, (100.0, 50.0), [false; 4]);
        assert!(!at("(height > 0) or style(--x: y)"));
        assert!(at("style(--x: y)"));
        assert!(at("style(--x)"));
        assert!(!at("style(--x: z)"));
        assert!(at("style(--n > 4)"));
        assert!(!at("style(--n > 4px)"));
    }

    #[test]
    fn scroll_state_queries_read_where_the_container_can_scroll() {
        let html = "<div id=c><p id=t>x</p></div>";
        let css = "#c { container-type: scroll-state; overflow: auto }";
        let down = [false, false, true, false];
        let at = |query: &str, scroll| holds(html, css, query, (0.0, 0.0), scroll);
        assert!(at("scroll-state(scrollable)", down));
        assert!(at("scroll-state(scrollable: bottom)", down));
        assert!(at("scroll-state(scrollable: y)", down));
        assert!(at("scroll-state(scrollable: block-end)", down));
        assert!(!at("scroll-state(scrollable: top)", down));
        assert!(!at("scroll-state(scrollable: x)", down));
        assert!(at("scroll-state(scrollable: none)", [false; 4]));
        // Right to left, the inline start is the right.
        let rtl = "#c { container-type: scroll-state; overflow: auto; direction: rtl }";
        let right = [false, true, false, false];
        assert!(holds(
            html,
            rtl,
            "scroll-state(scrollable: inline-start)",
            (0.0, 0.0),
            right
        ));
        // A container of another type answers none.
        let sized = "#c { container-type: size }";
        assert!(!holds(
            html,
            sized,
            "scroll-state(scrollable)",
            (0.0, 0.0),
            down
        ));
        // Stuck, snapped and scrolled as the state says, each apart.
        assert!(at(
            "scroll-state(stuck: none) and scroll-state(snapped: none)",
            down
        ));
        assert!(!at("scroll-state(scrolled: top)", down));
        let state = |f: &dyn Fn(&mut ScrollState)| {
            let mut state = ScrollState::default();
            f(&mut state);
            state
        };
        let in_state =
            |query: &str, state: ScrollState| holds_in(html, css, query, (0.0, 0.0), state);
        let scrolled = state(&|s| s.scrolled = [true, false, false, true]);
        for (query, wanted) in [
            ("scroll-state(scrolled)", true),
            ("scroll-state(scrolled: top)", true),
            ("scroll-state(scrolled: y)", true),
            ("scroll-state(scrolled: left)", true),
            ("scroll-state(scrolled: inline-start)", true),
            ("scroll-state(scrolled: bottom)", false),
            ("scroll-state(scrolled: right)", false),
            ("scroll-state(scrolled: none)", false),
            ("scroll-state(scrollable)", false),
            ("scroll-state(stuck)", false),
        ] {
            assert_eq!(in_state(query, scrolled), wanted, "{query}");
        }
        let stuck = state(&|s| s.stuck = [false, false, true, false]);
        for (query, wanted) in [
            ("scroll-state(stuck: bottom)", true),
            ("scroll-state(stuck: block-end)", true),
            ("scroll-state(stuck: top)", false),
            ("scroll-state(stuck: none)", false),
            ("scroll-state(scrolled)", false),
        ] {
            assert_eq!(in_state(query, stuck), wanted, "{query}");
        }
        let down_only = state(&|s| s.snapped = [false, true]);
        let both = state(&|s| s.snapped = [true, true]);
        for (query, snapped, wanted) in [
            ("scroll-state(snapped: y)", down_only, true),
            ("scroll-state(snapped: block)", down_only, true),
            ("scroll-state(snapped: x)", down_only, false),
            ("scroll-state(snapped: inline)", down_only, false),
            ("scroll-state(snapped: both)", down_only, false),
            ("scroll-state(snapped: both)", both, true),
            ("scroll-state(snapped)", down_only, true),
            ("scroll-state(snapped: none)", down_only, false),
            ("scroll-state(snapped: none)", ScrollState::default(), true),
        ] {
            assert_eq!(in_state(query, snapped), wanted, "{query}");
        }
    }

    #[test]
    fn a_pseudo_element_queries_its_own_element_first() {
        let document = dom::Document::from_node(
            &html::parse_document("<div id=o><div id=t>x</div></div>").unwrap(),
        );
        let t = document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some("t"))
            .unwrap();
        let sheet = "#t { container-type: inline-size; --c: x }
            #t::before { color: red; content: 'b' }
            @container (width >= 300px) { #t::before { color: green } }
            @container style(--c: x) { #t::after { content: 'a'; color: blue } }";
        let sheets = Stylesheets::new(vec![css::parse_stylesheet(sheet).unwrap()]);
        let context = CascadeContext::default();
        let mut styles = sheets.cascade(&document, &context);
        let color = |styles: &Styles, pseudo| {
            styles
                .pseudo(t, pseudo)
                .map(|style: &ComputedStyle| style.color)
        };
        use crate::selector_matching::PseudoElement;
        let red = crate::computed::Color::rgb8(255, 0, 0);
        let green = crate::computed::Color::rgb8(0, 128, 0);
        let blue = crate::computed::Color::rgb8(0, 0, 255);
        // A style query of the element itself holds before any layout.
        assert_eq!(color(&styles, PseudoElement::After), Some(blue));
        let mut sizes = ContainerSizes::new();
        sizes.insert(t, 200.0, 20.0);
        sheets.restyle_containers(&document, &context, &mut styles, &sizes);
        assert_eq!(color(&styles, PseudoElement::Before), Some(red));
        sizes.insert(t, 300.0, 20.0);
        sheets.restyle_containers(&document, &context, &mut styles, &sizes);
        assert_eq!(color(&styles, PseudoElement::Before), Some(green));
    }

    #[test]
    fn a_table_and_its_parts_and_inline_boxes_take_no_size_containment() {
        let html = "<div id=c><p id=t>x</p></div>";
        let at = |display: &str| {
            holds(
                html,
                &format!("#c {{ container-type: size; display: {display} }}"),
                "(width > 0)",
                (100.0, 50.0),
                [false; 4],
            )
        };
        for display in [
            "block",
            "inline-block",
            "flow-root",
            "flex",
            "table-caption",
            "list-item",
        ] {
            assert!(at(display), "{display}");
        }
        for display in [
            "table",
            "inline-table",
            "table-cell",
            "table-row",
            "inline",
            "ruby",
            "inline list-item",
            "contents",
        ] {
            assert!(!at(display), "{display}");
        }
        assert!(at("inline flow-root list-item"));
    }

    #[test]
    fn anchored_queries_parse_as_the_reference_browser_and_answer_unfallen() {
        let text = |t: &str| prelude(t).map(|(text, _)| text);
        for (written, serialized) in [
            (
                "anchored(fallback: FLIP-BLOCK)",
                "anchored(fallback: flip-block)",
            ),
            ("ANCHORED(fallback)", "anchored(fallback)"),
            ("anchored( fallback : NONE )", "anchored(fallback: none)"),
            (
                "anchored(fallback: flip-block --a)",
                "anchored(fallback: --a flip-block)",
            ),
            (
                "anchored(fallback: top left)",
                "anchored(fallback: left top)",
            ),
            (
                "anchored(fallback: top span-all)",
                "anchored(fallback: span-all top)",
            ),
            (
                "anchored(fallback: inline-start block-end)",
                "anchored(fallback: block-end inline-start)",
            ),
            (
                "anchored(not (fallback: none))",
                "anchored(not (fallback: none))",
            ),
            // Not valid, so kept as written.
            ("anchored(fallback: 1px)", "anchored(fallback: 1px)"),
            (
                "anchored(fallback: --a, --b)",
                "anchored(fallback: --a, --b)",
            ),
            (
                "anchored(fallback: flip-block flip-block)",
                "anchored(fallback: flip-block flip-block)",
            ),
            (
                "anchored(fallback: left right)",
                "anchored(fallback: left right)",
            ),
            ("anchored(foo: none)", "anchored(foo: none)"),
        ] {
            assert_eq!(text(written).as_deref(), Some(serialized), "{written}");
        }
        let html = "<div id=c><p id=t>x</p></div>";
        let at = |css: &str, query: &str| holds(html, css, query, (0.0, 0.0), [false; 4]);
        let anchored = "#c { container-type: anchored }";
        assert!(at(anchored, "anchored(fallback: none)"));
        assert!(at(anchored, "not anchored(fallback)"));
        assert!(!at(anchored, "anchored(fallback: flip-block)"));
        assert!(!at(anchored, "anchored(fallback)"));
        // Of no anchored container: unknown.
        assert!(!at(
            "#c { container-type: size }",
            "anchored(fallback: none)"
        ));
        assert!(!at("#c { container-type: size }", "not anchored(fallback)"));
        // A malformed value is `<general-enclosed>`.
        assert!(!at(anchored, "not anchored(fallback: 1px)"));
    }
}
