//! The properties of scroll-driven animations (Scroll-driven Animations 1
//! sections 2 to 4; D75) and of the triggers that play animations as a
//! timeline's progress enters and leaves a range (CSS Animations 2's
//! triggers, as the reference browser ships them): the timeline an
//! animation runs on (`animation-timeline`), the range of it the
//! animation is attached to (`animation-range`), the named timelines an
//! element declares (`scroll-timeline`, `view-timeline`) and the scope a
//! name is looked up in (`timeline-scope`); the triggers an element
//! declares (`timeline-trigger`), those an animation follows
//! (`animation-trigger`) and their scope (`trigger-scope`).
//!
//! Each value is read by one grammar into a value generic over its
//! lengths: computed, a [`LengthPercentage`] of the element's bases; or
//! as written, [`Written`], its canonical text, which a declaration block
//! keeps (CSSOM section 6.7.2). The text of either is written by one
//! writer too, which leaves out what the grammar fills in, as the
//! reference browser does: `scroll(nearest block)` is `scroll()`, a
//! range's `cover 0%` at its start `cover`, a view timeline's insets
//! `auto auto` nothing. The shorthands are read into their longhands'
//! values and written from them.

use std::sync::Arc;

use css::Token;

use super::{
    LengthBases, Range, components, length_percentage, single_ident, trim, with_default_bases,
};
use crate::computed::{ComputedStyle, LengthPercentage, Motion};
use crate::media::split_commas;

/// A timeline's axis (Scroll-driven Animations 1 section 2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    #[default]
    Block,
    Inline,
    X,
    Y,
}

impl Axis {
    /// The axis a keyword names, ASCII case-insensitively.
    pub fn from_name(word: &str) -> Option<Axis> {
        Some(match word.to_ascii_lowercase().as_str() {
            "block" => Axis::Block,
            "inline" => Axis::Inline,
            "x" => Axis::X,
            "y" => Axis::Y,
            _ => return None,
        })
    }

    /// Its keyword.
    pub fn name(self) -> &'static str {
        match self {
            Axis::Block => "block",
            Axis::Inline => "inline",
            Axis::X => "x",
            Axis::Y => "y",
        }
    }

    /// Whether it is the vertical one, for horizontal text (the engine's
    /// only writing mode): `block` and `y`.
    pub fn vertical(self) -> bool {
        matches!(self, Axis::Block | Axis::Y)
    }
}

/// The scroll container a `scroll()` timeline follows (section 2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scroller {
    /// The element's nearest ancestor scroll container.
    #[default]
    Nearest,
    /// The document's viewport.
    Root,
    /// The element itself (`self`).
    Own,
}

/// A length of these values: what a percentage among them is, and how
/// it is written.
pub trait Length: Clone + PartialEq + std::fmt::Debug {
    /// The percentage it is when it is a plain one.
    fn percentage(&self) -> Option<f32>;
    /// Its text.
    fn text(&self) -> String;
}

impl Length for LengthPercentage {
    fn percentage(&self) -> Option<f32> {
        match self {
            LengthPercentage::Percent(p) => Some(*p),
            _ => None,
        }
    }

    fn text(&self) -> String {
        crate::serialize::length_percentage(self).unwrap_or_default()
    }
}

/// A length as a declaration block keeps it: its canonical text, a
/// unitless zero `0px` and a math function simplified.
#[derive(Debug, Clone, PartialEq)]
pub struct Written(pub String);

impl Length for Written {
    fn percentage(&self) -> Option<f32> {
        let number = self.0.strip_suffix('%')?;
        if number.contains(|c: char| c.is_ascii_alphabetic() || c == '(') {
            return None;
        }
        number.parse().ok()
    }

    fn text(&self) -> String {
        self.0.clone()
    }
}

/// One side of a view timeline's inset (section 3.1.2).
#[derive(Debug, Clone, PartialEq)]
pub enum Inset<L> {
    Auto,
    Length(L),
}

impl<L: Length> Inset<L> {
    fn text(&self) -> String {
        match self {
            Inset::Auto => "auto".to_owned(),
            Inset::Length(l) => l.text(),
        }
    }
}

/// A view timeline's inset, its start side and its end side.
pub type Insets<L> = [Inset<L>; 2];

/// One item of `animation-timeline` (section 4.1), or of
/// `timeline-trigger-source`.
#[derive(Debug, Clone, PartialEq)]
pub enum Timeline<L> {
    /// The document's timeline.
    Auto,
    /// No timeline.
    None,
    /// A named timeline, looked up from the element.
    Named(Arc<str>),
    /// An anonymous scroll progress timeline (`scroll()`).
    Scroll { scroller: Scroller, axis: Axis },
    /// An anonymous view progress timeline of the element (`view()`).
    View { axis: Axis, inset: Insets<L> },
}

/// A named timeline range (section 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeName {
    Cover,
    Contain,
    Entry,
    Exit,
    EntryCrossing,
    ExitCrossing,
}

impl RangeName {
    /// The range a keyword names, ASCII case-insensitively.
    pub fn from_name(word: &str) -> Option<RangeName> {
        Some(match word.to_ascii_lowercase().as_str() {
            "cover" => RangeName::Cover,
            "contain" => RangeName::Contain,
            "entry" => RangeName::Entry,
            "exit" => RangeName::Exit,
            "entry-crossing" => RangeName::EntryCrossing,
            "exit-crossing" => RangeName::ExitCrossing,
            _ => return None,
        })
    }

    /// Its keyword.
    pub fn name(self) -> &'static str {
        match self {
            RangeName::Cover => "cover",
            RangeName::Contain => "contain",
            RangeName::Entry => "entry",
            RangeName::Exit => "exit",
            RangeName::EntryCrossing => "entry-crossing",
            RangeName::ExitCrossing => "exit-crossing",
        }
    }
}

/// Where a range starts or ends (section 4.2): `normal`, an offset in
/// a named range or in the whole timeline, or, for a trigger's active
/// range, `auto`. A name written alone has the offset its place gives
/// it, 0% at a start and 100% at an end.
#[derive(Debug, Clone, PartialEq)]
pub enum Edge<L> {
    Normal,
    Auto,
    At(Option<RangeName>, L),
}

impl<L: Length> Edge<L> {
    /// Its text at a start (`end` false) or an end: a name's default
    /// offset left out.
    pub fn text(&self, end: bool) -> String {
        let default = if end { 100.0 } else { 0.0 };
        match self {
            Edge::Normal => "normal".to_owned(),
            Edge::Auto => "auto".to_owned(),
            Edge::At(None, l) => l.text(),
            Edge::At(Some(name), l) if l.percentage() == Some(default) => name.name().to_owned(),
            Edge::At(Some(name), l) => format!("{} {}", name.name(), l.text()),
        }
    }
}

/// `timeline-scope` and `trigger-scope` (section 4.4).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Scope {
    #[default]
    None,
    All,
    Names(Vec<Arc<str>>),
}

/// What an animation does at a trigger's activation or deactivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Play,
    PlayOnce,
    PlayForwards,
    PlayBackwards,
    Pause,
    Reset,
    Replay,
}

impl Action {
    /// The action a keyword names, ASCII case-insensitively.
    pub fn from_name(word: &str) -> Option<Action> {
        Some(match word.to_ascii_lowercase().as_str() {
            "none" => Action::None,
            "play" => Action::Play,
            "play-once" => Action::PlayOnce,
            "play-forwards" => Action::PlayForwards,
            "play-backwards" => Action::PlayBackwards,
            "pause" => Action::Pause,
            "reset" => Action::Reset,
            "replay" => Action::Replay,
            _ => return None,
        })
    }

    /// Its keyword.
    pub fn name(self) -> &'static str {
        match self {
            Action::None => "none",
            Action::Play => "play",
            Action::PlayOnce => "play-once",
            Action::PlayForwards => "play-forwards",
            Action::PlayBackwards => "play-backwards",
            Action::Pause => "pause",
            Action::Reset => "reset",
            Action::Replay => "replay",
        }
    }
}

/// One item of `animation-trigger`: the trigger named, what the
/// animation does as it activates, and as it deactivates when given.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationTrigger {
    pub name: Arc<str>,
    pub activate: Action,
    pub deactivate: Option<Action>,
}

/// The longhands of `timeline-trigger`, in its order.
pub const TIMELINE_TRIGGER: [&str; 6] = [
    "timeline-trigger-name",
    "timeline-trigger-source",
    "timeline-trigger-activation-range-start",
    "timeline-trigger-activation-range-end",
    "timeline-trigger-active-range-start",
    "timeline-trigger-active-range-end",
];

/// The longhands of the shorthands here, `animation`'s new ones apart.
pub fn longhands(shorthand: &str) -> Option<&'static [&'static str]> {
    Some(match shorthand {
        "animation-range" => &["animation-range-start", "animation-range-end"],
        "scroll-timeline" => &["scroll-timeline-name", "scroll-timeline-axis"],
        "view-timeline" => &[
            "view-timeline-name",
            "view-timeline-axis",
            "view-timeline-inset",
        ],
        "timeline-trigger" => &TIMELINE_TRIGGER,
        "timeline-trigger-activation-range" => &[
            "timeline-trigger-activation-range-start",
            "timeline-trigger-activation-range-end",
        ],
        "timeline-trigger-active-range" => &[
            "timeline-trigger-active-range-start",
            "timeline-trigger-active-range-end",
        ],
        _ => return None,
    })
}

/// Whether `name` is one of the properties here, a longhand or a
/// shorthand.
pub fn is_timeline_property(name: &str) -> bool {
    longhands(name).is_some()
        || matches!(
            name,
            "animation-timeline"
                | "animation-range-start"
                | "animation-range-end"
                | "animation-trigger"
                | "scroll-timeline-name"
                | "scroll-timeline-axis"
                | "view-timeline-name"
                | "view-timeline-axis"
                | "view-timeline-inset"
                | "timeline-scope"
                | "trigger-scope"
        )
        || TIMELINE_TRIGGER.contains(&name)
}

// ---- Reading ----------------------------------------------------------

/// A dashed identifier (`--name`), case kept.
fn dashed(tokens: &[Token]) -> Option<Arc<str>> {
    match tokens {
        [Token::Ident(name)] if name.starts_with("--") => Some(Arc::from(name.as_str())),
        _ => None,
    }
}

/// Whether `tokens` are the keyword `word`.
fn is_word(tokens: &[Token], word: &str) -> bool {
    single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case(word))
}

/// A comma-separated list, each item one `item` takes, none empty.
fn list<T>(tokens: &[Token], mut item: impl FnMut(&[Token]) -> Option<T>) -> Option<Vec<T>> {
    split_commas(tokens)
        .into_iter()
        .map(|part| {
            let part = trim(part);
            if part.is_empty() { None } else { item(part) }
        })
        .collect()
}

/// `none` or a dashed identifier: one item of a timeline's or a
/// trigger's names.
fn name_or_none(tokens: &[Token]) -> Option<Option<Arc<str>>> {
    if is_word(tokens, "none") {
        return Some(None);
    }
    dashed(tokens).map(Some)
}

/// `auto` or a length: one side of an inset.
fn inset<L>(part: &[Token], len: &impl Fn(&[Token]) -> Option<L>) -> Option<Inset<L>> {
    if is_word(part, "auto") {
        Some(Inset::Auto)
    } else {
        len(part).map(Inset::Length)
    }
}

/// One or two sides of an inset, at the front of `parts`: how many it
/// took, and the insets, the end the start when one is given.
fn insets<L: Clone>(
    parts: &[&[Token]],
    len: &impl Fn(&[Token]) -> Option<L>,
) -> Option<(usize, Insets<L>)> {
    let start = inset(parts.first()?, len)?;
    match parts.get(1).and_then(|p| inset(p, len)) {
        Some(end) => Some((2, [start, end])),
        None => Some((1, [start.clone(), start])),
    }
}

/// `<single-animation-timeline>` (section 4.1).
fn timeline<L: Clone>(
    tokens: &[Token],
    len: &impl Fn(&[Token]) -> Option<L>,
) -> Option<Timeline<L>> {
    let parts = components(tokens);
    let [part] = parts.as_slice() else {
        return None;
    };
    if is_word(part, "auto") {
        return Some(Timeline::Auto);
    }
    if is_word(part, "none") {
        return Some(Timeline::None);
    }
    if let Some(name) = dashed(part) {
        return Some(Timeline::Named(name));
    }
    let (Some(Token::Function(name)), [inside @ .., Token::CloseParen]) =
        (part.first(), &part[1..])
    else {
        return None;
    };
    let args = components(inside);
    match name.to_ascii_lowercase().as_str() {
        // `scroll( [ <scroller> || <axis> ]? )`.
        "scroll" => {
            let (mut scroller, mut axis) = (None, None);
            for arg in args {
                let word = single_ident(arg)?;
                if let Some(a) = Axis::from_name(word)
                    && axis.is_none()
                {
                    axis = Some(a);
                    continue;
                }
                let s = match word.to_ascii_lowercase().as_str() {
                    "nearest" => Scroller::Nearest,
                    "root" => Scroller::Root,
                    "self" => Scroller::Own,
                    _ => return None,
                };
                if scroller.replace(s).is_some() {
                    return None;
                }
            }
            Some(Timeline::Scroll {
                scroller: scroller.unwrap_or_default(),
                axis: axis.unwrap_or_default(),
            })
        }
        // `view( [ <axis> || <'view-timeline-inset'> ]? )`.
        "view" => {
            let (mut axis, mut inset) = (None, None);
            let mut i = 0;
            while i < args.len() {
                if axis.is_none()
                    && let Some(a) = single_ident(args[i]).and_then(Axis::from_name)
                {
                    axis = Some(a);
                    i += 1;
                    continue;
                }
                if inset.is_some() {
                    return None;
                }
                let (taken, found) = insets(&args[i..], len)?;
                inset = Some(found);
                i += taken;
            }
            Some(Timeline::View {
                axis: axis.unwrap_or_default(),
                inset: inset.unwrap_or([Inset::Auto, Inset::Auto]),
            })
        }
        _ => None,
    }
}

/// A range's edge at the front of `parts` (section 4.2): `normal`, a
/// length, or a range's name and a length, given or its place's default
/// (`end` for an end); `auto` too where `auto` is allowed. How many
/// parts it took.
fn edge<L>(
    parts: &[&[Token]],
    end: bool,
    auto: bool,
    len: &impl Fn(&[Token]) -> Option<L>,
    percent: &impl Fn(f32) -> L,
) -> Option<(usize, Edge<L>)> {
    let first = parts.first()?;
    if is_word(first, "normal") {
        return Some((1, Edge::Normal));
    }
    if auto && is_word(first, "auto") {
        return Some((1, Edge::Auto));
    }
    if let Some(name) = single_ident(first).and_then(RangeName::from_name) {
        return Some(match parts.get(1).and_then(|p| len(p)) {
            Some(offset) => (2, Edge::At(Some(name), offset)),
            None => (
                1,
                Edge::At(Some(name), percent(if end { 100.0 } else { 0.0 })),
            ),
        });
    }
    len(first).map(|l| (1, Edge::At(None, l)))
}

/// One edge, the whole of `tokens`.
fn single_edge<L>(
    tokens: &[Token],
    end: bool,
    auto: bool,
    len: &impl Fn(&[Token]) -> Option<L>,
    percent: &impl Fn(f32) -> L,
) -> Option<Edge<L>> {
    let parts = components(tokens);
    let (taken, found) = edge(&parts, end, auto, len, percent)?;
    (taken == parts.len()).then_some(found)
}

/// A range, a start and an end, from the front of `parts` (section 4.2,
/// `animation-range`): the end, left out, is the start's name at 100%,
/// or `normal` (`auto` where `auto` is allowed) for a start of no name.
fn range<L: Clone>(
    parts: &[&[Token]],
    auto: bool,
    len: &impl Fn(&[Token]) -> Option<L>,
    percent: &impl Fn(f32) -> L,
) -> Option<(usize, Edge<L>, Edge<L>)> {
    let (first, start) = edge(parts, false, auto, len, percent)?;
    if let Some((second, end)) = edge(&parts[first..], true, auto, len, percent) {
        return Some((first + second, start, end));
    }
    let end = match &start {
        Edge::At(Some(name), _) => Edge::At(Some(*name), percent(100.0)),
        Edge::Auto => Edge::Auto,
        _ if auto => Edge::Auto,
        _ => Edge::Normal,
    };
    Some((first, start, end))
}

/// `timeline-scope` and `trigger-scope`: `none`, `all`, or names.
fn scope(tokens: &[Token]) -> Option<Scope> {
    if is_word(tokens, "none") {
        return Some(Scope::None);
    }
    if is_word(tokens, "all") {
        return Some(Scope::All);
    }
    list(tokens, dashed).map(Scope::Names)
}

/// One item of `animation-trigger`: `none`, or a trigger's name and one
/// or two actions.
fn animation_trigger(tokens: &[Token]) -> Option<Option<AnimationTrigger>> {
    let parts = components(tokens);
    match parts.as_slice() {
        [none] if is_word(none, "none") => Some(None),
        [name, actions @ ..] if (1..=2).contains(&actions.len()) => {
            let name = dashed(name)?;
            let action = |p: &[Token]| single_ident(p).and_then(Action::from_name);
            Some(Some(AnimationTrigger {
                name,
                activate: action(actions[0])?,
                deactivate: match actions.get(1) {
                    Some(p) => Some(action(p)?),
                    None => None,
                },
            }))
        }
        _ => None,
    }
}

/// One item of `scroll-timeline`: a name, then an axis.
fn scroll_timeline_item(tokens: &[Token]) -> Option<(Option<Arc<str>>, Axis)> {
    match components(tokens).as_slice() {
        [name] => Some((name_or_none(name)?, Axis::Block)),
        [name, axis] => Some((
            name_or_none(name)?,
            single_ident(axis).and_then(Axis::from_name)?,
        )),
        _ => None,
    }
}

/// One item of `view-timeline`: a name, then an axis or an inset or
/// both, in either order.
fn view_timeline_item<L: Clone>(
    tokens: &[Token],
    len: &impl Fn(&[Token]) -> Option<L>,
) -> Option<(Option<Arc<str>>, Axis, Insets<L>)> {
    let parts = components(tokens);
    let name = name_or_none(parts.first()?)?;
    let (mut axis, mut found) = (None, None);
    let mut i = 1;
    while i < parts.len() {
        if axis.is_none()
            && let Some(a) = single_ident(parts[i]).and_then(Axis::from_name)
        {
            axis = Some(a);
            i += 1;
            continue;
        }
        if found.is_some() {
            return None;
        }
        let (taken, inset) = insets(&parts[i..], len)?;
        found = Some(inset);
        i += taken;
    }
    Some((
        name,
        axis.unwrap_or_default(),
        found.unwrap_or([Inset::Auto, Inset::Auto]),
    ))
}

/// One item of `timeline-trigger`: its name, its source, its activation
/// range and, after a `/`, its active range, each but the name optional.
type TriggerItem<L> = (Option<Arc<str>>, Timeline<L>, [Edge<L>; 4]);

fn timeline_trigger_item<L: Clone>(
    tokens: &[Token],
    len: &impl Fn(&[Token]) -> Option<L>,
    percent: &impl Fn(f32) -> L,
) -> Option<TriggerItem<L>> {
    let all = components(tokens);
    let slash = all.iter().position(|p| matches!(p, [Token::Delim('/')]));
    let (before, after) = match slash {
        Some(at) => (&all[..at], Some(&all[at + 1..])),
        None => (&all[..], None),
    };
    let mut i = 0;
    let mut name = None;
    if let Some(n) = before.first().and_then(|p| dashed(p)) {
        name = Some(n);
        i = 1;
    }
    let mut source = Timeline::Auto;
    if let Some(found) = before.get(i).and_then(|p| timeline(p, len)) {
        // `none` names no trigger before a source, and is the source after
        // a name.
        source = found;
        i += 1;
    }
    if name.is_none() && i == 0 {
        return None;
    }
    let mut activation = [Edge::Normal, Edge::Normal];
    if i < before.len() {
        let (taken, start, end) = range(&before[i..], false, len, percent)?;
        if i + taken != before.len() {
            return None;
        }
        activation = [start, end];
    }
    let mut active = [Edge::Auto, Edge::Auto];
    if let Some(after) = after {
        let (taken, start, end) = range(after, true, len, percent)?;
        if taken != after.len() {
            return None;
        }
        active = [start, end];
    }
    let [a, b] = activation;
    let [c, d] = active;
    Some((name, source, [a, b, c, d]))
}

/// What a value reads as, for the property `name`: each longhand it sets
/// and its value. One value read for a longhand alone sets it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value<L> {
    Timelines(Vec<Timeline<L>>),
    Edges(Vec<Edge<L>>),
    Triggers(Vec<Option<AnimationTrigger>>),
    Names(Vec<Option<Arc<str>>>),
    Axes(Vec<Axis>),
    Insets(Vec<Insets<L>>),
    Scope(Scope),
}

/// The value `tokens` of the property `name`, its lengths read by `len`
/// and a percentage made by `percent`: each longhand it sets and its
/// value; `None` where it is not one the property takes.
pub fn read<L: Clone>(
    name: &str,
    tokens: &[Token],
    len: &impl Fn(&[Token]) -> Option<L>,
    percent: &impl Fn(f32) -> L,
) -> Option<Vec<(&'static str, Value<L>)>> {
    let one = |longhand: &'static str, value: Value<L>| Some(vec![(longhand, value)]);
    let edges = |end: bool, auto: bool| {
        list(tokens, |item| single_edge(item, end, auto, len, percent)).map(Value::Edges)
    };
    let ranges = |start: &'static str, end: &'static str, auto: bool| {
        let items = list(tokens, |item| {
            let parts = components(item);
            let (taken, s, e) = range(&parts, auto, len, percent)?;
            (taken == parts.len()).then_some((s, e))
        })?;
        let (s, e): (Vec<_>, Vec<_>) = items.into_iter().unzip();
        Some(vec![(start, Value::Edges(s)), (end, Value::Edges(e))])
    };
    match name {
        "animation-timeline" => one(
            "animation-timeline",
            Value::Timelines(list(tokens, |item| timeline(item, len))?),
        ),
        "timeline-trigger-source" => one(
            "timeline-trigger-source",
            Value::Timelines(list(tokens, |item| timeline(item, len))?),
        ),
        "animation-range-start" => one("animation-range-start", edges(false, false)?),
        "animation-range-end" => one("animation-range-end", edges(true, false)?),
        "timeline-trigger-activation-range-start" => one(
            "timeline-trigger-activation-range-start",
            edges(false, false)?,
        ),
        "timeline-trigger-activation-range-end" => {
            one("timeline-trigger-activation-range-end", edges(true, false)?)
        }
        "timeline-trigger-active-range-start" => {
            one("timeline-trigger-active-range-start", edges(false, true)?)
        }
        "timeline-trigger-active-range-end" => {
            one("timeline-trigger-active-range-end", edges(true, true)?)
        }
        "animation-range" => ranges("animation-range-start", "animation-range-end", false),
        "timeline-trigger-activation-range" => ranges(
            "timeline-trigger-activation-range-start",
            "timeline-trigger-activation-range-end",
            false,
        ),
        "timeline-trigger-active-range" => ranges(
            "timeline-trigger-active-range-start",
            "timeline-trigger-active-range-end",
            true,
        ),
        "animation-trigger" => one(
            "animation-trigger",
            Value::Triggers(list(tokens, animation_trigger)?),
        ),
        "scroll-timeline-name" | "view-timeline-name" | "timeline-trigger-name" => {
            let longhand = longhand_name(name)?;
            one(longhand, Value::Names(list(tokens, name_or_none)?))
        }
        "scroll-timeline-axis" | "view-timeline-axis" => {
            let longhand = longhand_name(name)?;
            one(
                longhand,
                Value::Axes(list(tokens, |item| {
                    single_ident(item).and_then(Axis::from_name)
                })?),
            )
        }
        "view-timeline-inset" => one(
            "view-timeline-inset",
            Value::Insets(list(tokens, |item| {
                let parts = components(item);
                let (taken, found) = insets(&parts, len)?;
                (taken == parts.len()).then_some(found)
            })?),
        ),
        "timeline-scope" => one("timeline-scope", Value::Scope(scope(tokens)?)),
        "trigger-scope" => one("trigger-scope", Value::Scope(scope(tokens)?)),
        "scroll-timeline" => {
            let items = list(tokens, scroll_timeline_item)?;
            let (names, axes): (Vec<_>, Vec<_>) = items.into_iter().unzip();
            Some(vec![
                ("scroll-timeline-name", Value::Names(names)),
                ("scroll-timeline-axis", Value::Axes(axes)),
            ])
        }
        "view-timeline" => {
            let items = list(tokens, |item| view_timeline_item(item, len))?;
            let mut names = Vec::new();
            let mut axes = Vec::new();
            let mut insets = Vec::new();
            for (n, a, i) in items {
                names.push(n);
                axes.push(a);
                insets.push(i);
            }
            Some(vec![
                ("view-timeline-name", Value::Names(names)),
                ("view-timeline-axis", Value::Axes(axes)),
                ("view-timeline-inset", Value::Insets(insets)),
            ])
        }
        "timeline-trigger" => {
            let items = list(tokens, |item| timeline_trigger_item(item, len, percent))?;
            let mut names = Vec::new();
            let mut sources = Vec::new();
            let mut edges: [Vec<Edge<L>>; 4] = Default::default();
            for (n, s, e) in items {
                names.push(n);
                sources.push(s);
                for (list, edge) in edges.iter_mut().zip(e) {
                    list.push(edge);
                }
            }
            let [a, b, c, d] = edges;
            Some(vec![
                (TIMELINE_TRIGGER[0], Value::Names(names)),
                (TIMELINE_TRIGGER[1], Value::Timelines(sources)),
                (TIMELINE_TRIGGER[2], Value::Edges(a)),
                (TIMELINE_TRIGGER[3], Value::Edges(b)),
                (TIMELINE_TRIGGER[4], Value::Edges(c)),
                (TIMELINE_TRIGGER[5], Value::Edges(d)),
            ])
        }
        _ => None,
    }
}

/// The `'static` name of the longhand `name`.
fn longhand_name(name: &str) -> Option<&'static str> {
    [
        "scroll-timeline-name",
        "view-timeline-name",
        "timeline-trigger-name",
        "scroll-timeline-axis",
        "view-timeline-axis",
    ]
    .into_iter()
    .find(|l| *l == name)
}

// ---- Computing ---------------------------------------------------------

/// Applies the declaration `name: tokens` to `style`, its lengths
/// computed against `bases`; whether the value was one the property
/// takes.
pub(super) fn apply(
    style: &mut ComputedStyle,
    name: &str,
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> bool {
    let len = |t: &[Token]| length_percentage(t, bases, Range::All);
    let Some(values) = read(name, tokens, &len, &LengthPercentage::Percent) else {
        return false;
    };
    let motion = style.motion_mut();
    for (longhand, value) in values {
        set(motion, longhand, value);
    }
    true
}

/// Sets the longhand `longhand` of `motion` to `value`.
fn set(motion: &mut Motion, longhand: &str, value: Value<LengthPercentage>) {
    match (longhand, value) {
        ("animation-timeline", Value::Timelines(v)) => motion.animation_timeline = v,
        ("timeline-trigger-source", Value::Timelines(v)) => motion.timeline_trigger_source = v,
        ("animation-range-start", Value::Edges(v)) => motion.animation_range_start = v,
        ("animation-range-end", Value::Edges(v)) => motion.animation_range_end = v,
        ("timeline-trigger-activation-range-start", Value::Edges(v)) => {
            motion.timeline_trigger_activation_range_start = v
        }
        ("timeline-trigger-activation-range-end", Value::Edges(v)) => {
            motion.timeline_trigger_activation_range_end = v
        }
        ("timeline-trigger-active-range-start", Value::Edges(v)) => {
            motion.timeline_trigger_active_range_start = v
        }
        ("timeline-trigger-active-range-end", Value::Edges(v)) => {
            motion.timeline_trigger_active_range_end = v
        }
        ("animation-trigger", Value::Triggers(v)) => motion.animation_trigger = v,
        ("scroll-timeline-name", Value::Names(v)) => motion.scroll_timeline_name = v,
        ("view-timeline-name", Value::Names(v)) => motion.view_timeline_name = v,
        ("timeline-trigger-name", Value::Names(v)) => motion.timeline_trigger_name = v,
        ("scroll-timeline-axis", Value::Axes(v)) => motion.scroll_timeline_axis = v,
        ("view-timeline-axis", Value::Axes(v)) => motion.view_timeline_axis = v,
        ("view-timeline-inset", Value::Insets(v)) => motion.view_timeline_inset = v,
        ("timeline-scope", Value::Scope(v)) => motion.timeline_scope = v,
        ("trigger-scope", Value::Scope(v)) => motion.trigger_scope = v,
        _ => {}
    }
}

/// Copies the property `name` (a longhand, or each of a shorthand's)
/// from `src` to `style`: `inherit`'s and `initial`'s work.
pub(super) fn copy(style: &mut ComputedStyle, src: &ComputedStyle, name: &str) {
    if style.motion.is_none() && src.motion.is_none() {
        return;
    }
    let names: Vec<&str> = match longhands(name) {
        Some(list) => list.to_vec(),
        None => vec![name],
    };
    let src = src.motion().clone();
    let motion = style.motion_mut();
    for name in names {
        match name {
            "animation-timeline" => motion.animation_timeline = src.animation_timeline.clone(),
            "animation-range-start" => {
                motion.animation_range_start = src.animation_range_start.clone()
            }
            "animation-range-end" => motion.animation_range_end = src.animation_range_end.clone(),
            "animation-trigger" => motion.animation_trigger = src.animation_trigger.clone(),
            "scroll-timeline-name" => {
                motion.scroll_timeline_name = src.scroll_timeline_name.clone()
            }
            "scroll-timeline-axis" => {
                motion.scroll_timeline_axis = src.scroll_timeline_axis.clone()
            }
            "view-timeline-name" => motion.view_timeline_name = src.view_timeline_name.clone(),
            "view-timeline-axis" => motion.view_timeline_axis = src.view_timeline_axis.clone(),
            "view-timeline-inset" => motion.view_timeline_inset = src.view_timeline_inset.clone(),
            "timeline-scope" => motion.timeline_scope = src.timeline_scope.clone(),
            "trigger-scope" => motion.trigger_scope = src.trigger_scope.clone(),
            "timeline-trigger-name" => {
                motion.timeline_trigger_name = src.timeline_trigger_name.clone()
            }
            "timeline-trigger-source" => {
                motion.timeline_trigger_source = src.timeline_trigger_source.clone()
            }
            "timeline-trigger-activation-range-start" => {
                motion.timeline_trigger_activation_range_start =
                    src.timeline_trigger_activation_range_start.clone()
            }
            "timeline-trigger-activation-range-end" => {
                motion.timeline_trigger_activation_range_end =
                    src.timeline_trigger_activation_range_end.clone()
            }
            "timeline-trigger-active-range-start" => {
                motion.timeline_trigger_active_range_start =
                    src.timeline_trigger_active_range_start.clone()
            }
            "timeline-trigger-active-range-end" => {
                motion.timeline_trigger_active_range_end =
                    src.timeline_trigger_active_range_end.clone()
            }
            _ => {}
        }
    }
}

// ---- Writing -----------------------------------------------------------

/// A timeline's text.
fn timeline_text<L: Length>(timeline: &Timeline<L>) -> String {
    match timeline {
        Timeline::Auto => "auto".to_owned(),
        Timeline::None => "none".to_owned(),
        Timeline::Named(name) => name.to_string(),
        Timeline::Scroll { scroller, axis } => {
            let mut parts = Vec::new();
            match scroller {
                Scroller::Nearest => {}
                Scroller::Root => parts.push("root"),
                Scroller::Own => parts.push("self"),
            }
            if *axis != Axis::Block {
                parts.push(axis.name());
            }
            format!("scroll({})", parts.join(" "))
        }
        Timeline::View { axis, inset } => {
            let mut parts = Vec::new();
            if *axis != Axis::Block {
                parts.push(axis.name().to_owned());
            }
            if inset.iter().any(|i| *i != Inset::Auto) {
                parts.push(insets_text(inset));
            }
            format!("view({})", parts.join(" "))
        }
    }
}

/// An inset's text: its end left out when it is its start.
fn insets_text<L: Length>(inset: &Insets<L>) -> String {
    if inset[0] == inset[1] {
        inset[0].text()
    } else {
        format!("{} {}", inset[0].text(), inset[1].text())
    }
}

fn joined<T>(items: &[T], text: impl Fn(&T) -> String) -> String {
    items.iter().map(text).collect::<Vec<_>>().join(", ")
}

fn names_text(names: &[Option<Arc<str>>]) -> String {
    joined(names, |n| {
        n.as_ref()
            .map_or_else(|| "none".to_owned(), |n| n.to_string())
    })
}

fn scope_text(scope: &Scope) -> String {
    match scope {
        Scope::None => "none".to_owned(),
        Scope::All => "all".to_owned(),
        Scope::Names(names) => joined(names, |n| n.to_string()),
    }
}

fn trigger_text(trigger: &Option<AnimationTrigger>) -> String {
    match trigger {
        None => "none".to_owned(),
        Some(t) => {
            let mut text = format!("{} {}", t.name, t.activate.name());
            if let Some(d) = t.deactivate {
                text.push(' ');
                text.push_str(d.name());
            }
            text
        }
    }
}

/// A range's text, its end left out where it is what the start alone
/// gives (`auto` the end a start alone gives where `auto` is allowed).
fn range_text<L: Length>(start: &Edge<L>, end: &Edge<L>, auto: bool) -> String {
    let start_text = start.text(false);
    let implied = match (start, end) {
        (_, Edge::Auto) if auto => true,
        (Edge::At(Some(a), _), Edge::At(Some(b), l)) => a == b && l.percentage() == Some(100.0),
        (Edge::At(Some(_), _), _) => false,
        (_, Edge::Normal) => !auto,
        (_, Edge::Auto) => auto,
        (_, Edge::At(None, l)) => !auto && l.percentage() == Some(100.0),
        _ => false,
    };
    if implied {
        start_text
    } else {
        format!("{start_text} {}", end.text(true))
    }
}

/// The text of a value of the longhand `longhand`.
fn value_text<L: Length>(longhand: &str, value: &Value<L>) -> String {
    let end = longhand.ends_with("-end");
    match value {
        Value::Timelines(v) => joined(v, timeline_text),
        Value::Edges(v) => joined(v, |e| e.text(end)),
        Value::Triggers(v) => joined(v, trigger_text),
        Value::Names(v) => names_text(v),
        Value::Axes(v) => joined(v, |a| a.name().to_owned()),
        Value::Insets(v) => joined(v, insets_text),
        Value::Scope(s) => scope_text(s),
    }
}

/// The text of the shorthand `name` from its longhands' values, in its
/// longhands' order; `None` where they cannot be said by it: lists of
/// other lengths.
fn shorthand_text<L: Length>(name: &str, values: &[&Value<L>]) -> Option<String> {
    let len = |v: &Value<L>| match v {
        Value::Timelines(v) => v.len(),
        Value::Edges(v) => v.len(),
        Value::Triggers(v) => v.len(),
        Value::Names(v) => v.len(),
        Value::Axes(v) => v.len(),
        Value::Insets(v) => v.len(),
        Value::Scope(_) => 1,
    };
    let count = len(values.first()?);
    if values.iter().any(|v| len(v) != count) {
        return None;
    }
    let edge = |v: &Value<L>, i: usize| match v {
        Value::Edges(e) => e.get(i).cloned(),
        _ => None,
    };
    let mut items = Vec::new();
    for i in 0..count {
        let item = match name {
            "animation-range" | "timeline-trigger-activation-range" => {
                range_text(&edge(values[0], i)?, &edge(values[1], i)?, false)
            }
            "timeline-trigger-active-range" => {
                range_text(&edge(values[0], i)?, &edge(values[1], i)?, true)
            }
            "scroll-timeline" => {
                let (Value::Names(n), Value::Axes(a)) = (values[0], values[1]) else {
                    return None;
                };
                let mut text = names_text(&n[i..=i]);
                if a[i] != Axis::Block {
                    text = format!("{text} {}", a[i].name());
                }
                text
            }
            "view-timeline" => {
                let (Value::Names(n), Value::Axes(a), Value::Insets(s)) =
                    (values[0], values[1], values[2])
                else {
                    return None;
                };
                let mut text = names_text(&n[i..=i]);
                if a[i] != Axis::Block {
                    text = format!("{text} {}", a[i].name());
                }
                if s[i].iter().any(|i| *i != Inset::Auto) {
                    text = format!("{text} {}", insets_text(&s[i]));
                }
                text
            }
            "timeline-trigger" => {
                let (Value::Names(n), Value::Timelines(s)) = (values[0], values[1]) else {
                    return None;
                };
                let mut parts = Vec::new();
                if let Some(name) = &n[i] {
                    parts.push(name.to_string());
                }
                if s[i] != Timeline::Auto || n[i].is_none() {
                    parts.push(timeline_text(&s[i]));
                }
                let (a, b) = (edge(values[2], i)?, edge(values[3], i)?);
                if !(a == Edge::Normal && b == Edge::Normal) {
                    parts.push(range_text(&a, &b, false));
                }
                let (c, d) = (edge(values[4], i)?, edge(values[5], i)?);
                if !(c == Edge::Auto && d == Edge::Auto) {
                    parts.push("/".to_owned());
                    parts.push(range_text(&c, &d, true));
                }
                parts.join(" ")
            }
            _ => return None,
        };
        items.push(item);
    }
    Some(items.join(", "))
}

/// The computed value of `motion`'s longhand `longhand`.
fn computed(motion: &Motion, longhand: &str) -> Option<Value<LengthPercentage>> {
    Some(match longhand {
        "animation-timeline" => Value::Timelines(motion.animation_timeline.clone()),
        "timeline-trigger-source" => Value::Timelines(motion.timeline_trigger_source.clone()),
        "animation-range-start" => Value::Edges(motion.animation_range_start.clone()),
        "animation-range-end" => Value::Edges(motion.animation_range_end.clone()),
        "timeline-trigger-activation-range-start" => {
            Value::Edges(motion.timeline_trigger_activation_range_start.clone())
        }
        "timeline-trigger-activation-range-end" => {
            Value::Edges(motion.timeline_trigger_activation_range_end.clone())
        }
        "timeline-trigger-active-range-start" => {
            Value::Edges(motion.timeline_trigger_active_range_start.clone())
        }
        "timeline-trigger-active-range-end" => {
            Value::Edges(motion.timeline_trigger_active_range_end.clone())
        }
        "animation-trigger" => Value::Triggers(motion.animation_trigger.clone()),
        "scroll-timeline-name" => Value::Names(motion.scroll_timeline_name.clone()),
        "view-timeline-name" => Value::Names(motion.view_timeline_name.clone()),
        "timeline-trigger-name" => Value::Names(motion.timeline_trigger_name.clone()),
        "scroll-timeline-axis" => Value::Axes(motion.scroll_timeline_axis.clone()),
        "view-timeline-axis" => Value::Axes(motion.view_timeline_axis.clone()),
        "view-timeline-inset" => Value::Insets(motion.view_timeline_inset.clone()),
        "timeline-scope" => Value::Scope(motion.timeline_scope.clone()),
        "trigger-scope" => Value::Scope(motion.trigger_scope.clone()),
        _ => return None,
    })
}

/// The computed value of the property `name` of `style` as text, a
/// shorthand's from its longhands' (`getComputedStyle`); `None` for a
/// property not here.
pub fn computed_text(style: &ComputedStyle, name: &str) -> Option<String> {
    let motion = style.motion();
    if let Some(parts) = longhands(name) {
        let values: Option<Vec<_>> = parts.iter().map(|l| computed(motion, l)).collect();
        let values = values?;
        let refs: Vec<&Value<LengthPercentage>> = values.iter().collect();
        return Some(shorthand_text(name, &refs).unwrap_or_default());
    }
    computed(motion, name).map(|v| value_text(name, &v))
}

/// A length as written, `tokens` valid as a `<length-percentage>`: its
/// canonical text, a unitless zero `0px`; a math function of
/// percentages alone that percentage where `plain`, as the reference
/// browser keeps a trigger's ranges.
fn written(tokens: &[Token], bases: &LengthBases<'_>, plain: bool) -> Option<Written> {
    let computed = length_percentage(tokens, bases, Range::All)?;
    if let [Token::Number { value, .. }] = trim(tokens)
        && *value == 0.0
    {
        return Some(Written("0px".to_owned()));
    }
    let percentages_alone = tokens.iter().all(|t| !matches!(t, Token::Dimension { .. }));
    if plain
        && percentages_alone
        && let LengthPercentage::Percent(p) = computed
    {
        return Some(Written(format!("{}%", crate::serialize::number(p))));
    }
    Some(Written(crate::serialize::specified(tokens)))
}

/// Whether the lengths of the property `name` are kept plain as written
/// (a trigger's ranges).
fn plain(name: &str) -> bool {
    name.starts_with("timeline-trigger")
}

/// The value `tokens` of the property `name` as a declaration block
/// keeps it: each longhand it sets and its value's canonical text;
/// `None` for a value the property does not take.
pub fn specified_longhands(name: &str, tokens: &[Token]) -> Option<Vec<(&'static str, String)>> {
    with_default_bases(|bases| {
        let len = |t: &[Token]| written(t, bases, plain(name));
        let percent = |p: f32| Written(format!("{}%", crate::serialize::number(p)));
        let values = read(name, tokens, &len, &percent)?;
        Some(values.iter().map(|(l, v)| (*l, value_text(l, v))).collect())
    })
}

/// The text of the shorthand `name` from its longhands' texts as a
/// declaration block keeps them, in its longhands' order; `None` where
/// they cannot be said by it.
pub fn specified_shorthand(name: &str, values: &[&str]) -> Option<String> {
    let longhands = longhands(name)?;
    with_default_bases(|bases| {
        let len = |t: &[Token]| written(t, bases, plain(name));
        let percent = |p: f32| Written(format!("{}%", crate::serialize::number(p)));
        let mut read_values = Vec::new();
        for (longhand, text) in longhands.iter().zip(values) {
            let tokens = css::tokenize(text);
            let (_, value) = read(longhand, &tokens, &len, &percent)?.pop()?;
            read_values.push(value);
        }
        let refs: Vec<&Value<Written>> = read_values.iter().collect();
        shorthand_text(name, &refs)
    })
}

/// A range's edge a script gives as text (Web Animations 2's
/// `rangeStart` and `rangeEnd`), its lengths computed against a 16px
/// font; `None` where it is not one. `end` for an end, which a name
/// alone puts at 100%.
pub fn computed_edge(tokens: &[Token], end: bool) -> Option<Edge<LengthPercentage>> {
    with_default_bases(|bases| {
        let len = |t: &[Token]| length_percentage(t, bases, Range::All);
        single_edge(tokens, end, false, &len, &LengthPercentage::Percent)
    })
}

/// A view timeline's inset a script gives as text (the `ViewTimeline`
/// constructor), as `view-timeline-inset` takes one; `None` where it is
/// not one.
pub fn computed_insets(tokens: &[Token]) -> Option<Insets<LengthPercentage>> {
    with_default_bases(|bases| {
        let len = |t: &[Token]| length_percentage(t, bases, Range::All);
        let parts = components(tokens);
        let (taken, found) = insets(&parts, &len)?;
        (taken == parts.len()).then_some(found)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specified(name: &str, value: &str) -> Option<String> {
        let longhands = specified_longhands(name, &css::tokenize(value))?;
        if longhands.len() == 1 {
            return Some(longhands[0].1.clone());
        }
        let texts: Vec<&str> = longhands.iter().map(|(_, t)| t.as_str()).collect();
        specified_shorthand(name, &texts)
    }

    #[test]
    fn a_timeline_is_written_with_its_defaults_left_out() {
        let t = |v: &str| specified("animation-timeline", v);
        assert_eq!(t("scroll(block)").as_deref(), Some("scroll()"));
        assert_eq!(t("scroll(y root)").as_deref(), Some("scroll(root y)"));
        assert_eq!(t("scroll(self block)").as_deref(), Some("scroll(self)"));
        assert_eq!(t("view(1px 1px)").as_deref(), Some("view(1px)"));
        assert_eq!(t("view(auto x)").as_deref(), Some("view(x)"));
        assert_eq!(t("view(1px 2px y)").as_deref(), Some("view(y 1px 2px)"));
        assert_eq!(t("--a, none, auto").as_deref(), Some("--a, none, auto"));
        assert_eq!(t("view(1px 2px 3px)"), None);
        assert_eq!(t("scroll(abc)"), None);
        assert_eq!(t("test"), None);
        assert_eq!(t("auto auto"), None);
    }

    #[test]
    fn a_range_is_written_as_the_reference_browser_writes_it() {
        let r = |v: &str| specified("animation-range", v);
        assert_eq!(r("normal normal").as_deref(), Some("normal"));
        assert_eq!(r("entry 0% entry 100%").as_deref(), Some("entry"));
        assert_eq!(r("0% 100%").as_deref(), Some("0%"));
        assert_eq!(r("entry normal").as_deref(), Some("entry normal"));
        assert_eq!(r("100px normal").as_deref(), Some("100px"));
        assert_eq!(
            r("entry 50% exit 50%").as_deref(),
            Some("entry 50% exit 50%")
        );
        assert_eq!(r("thing 0%"), None);
        assert_eq!(r("1s"), None);
        assert_eq!(
            specified("animation-range-start", "0").as_deref(),
            Some("0px")
        );
        assert_eq!(
            specified("animation-range-end", "contain 100%").as_deref(),
            Some("contain")
        );
        let longhands = specified_longhands("animation-range", &css::tokenize("cover")).unwrap();
        assert_eq!(longhands[1], ("animation-range-end", "cover".to_owned()));
    }

    #[test]
    fn the_named_timelines_and_triggers_are_read() {
        assert_eq!(
            specified("view-timeline", "--a block 1px 1px").as_deref(),
            Some("--a 1px")
        );
        assert_eq!(
            specified("scroll-timeline", "--a inline, --b block").as_deref(),
            Some("--a inline, --b")
        );
        assert_eq!(specified("scroll-timeline", "--abc --abc"), None);
        assert_eq!(specified("timeline-scope", "none, abc"), None);
        assert_eq!(
            specified("animation-trigger", "--a play pause, none").as_deref(),
            Some("--a play pause, none")
        );
        assert_eq!(specified("animation-trigger", "--a play pause reset"), None);
        assert_eq!(
            specified(
                "timeline-trigger",
                "--t view() contain 0% contain 100% / cover 0% cover 100%"
            )
            .as_deref(),
            Some("--t view() contain / cover")
        );
        assert_eq!(
            specified(
                "timeline-trigger",
                "--t view() normal normal / cover 10% auto"
            )
            .as_deref(),
            Some("--t view() / cover 10%")
        );
    }
}
