//! The transition and animation properties (Transitions 1 and 2 §2,
//! Animations 1 and 2 §4; M22.1, M22.2) and the easing functions they
//! take (Easing 1 §2, Easing 2 §2.1). Each parser takes a declaration's
//! value as the cascade has it and says `None` when the value is not one
//! the property takes, so that the declaration is dropped.
//!
//! Times are seconds in an `f64`. A written duration below zero is no
//! duration; a math function's is clamped to zero, as its range is
//! checked where it is computed (Values 4 §10.9), and its infinite time
//! is the largest finite one, as the reference browser computes
//! `calc(infinity * 1s)` to `1.79769e+308s`.
//!
//! The shorthands match their grammars' `||` as the reference browser
//! does: a time is the duration while there is none and it can be one,
//! a negative time written being none, and the delay after; a keyword
//! another property of an animation takes is that property's while it
//! has no value, before it can be a name, so that `animation: none none`
//! is a fill mode and a name, and `animation: auto 1s` a duration of
//! `auto` and a delay.

use std::sync::Arc;

use css::Token;

use super::{LengthBases, components, single_ident, single_token, trim};
use crate::calc::{self, Want};
use crate::computed::{
    AnimationDirection, AnimationFillMode, AnimationPlayState, TransitionBehavior,
    TransitionProperty,
};
use crate::custom::is_custom_ident;
use crate::easing::{Easing, StepPosition, canonical_points};
use crate::media::split_commas;

/// The longhands of `transition` and of `animation`, and the two
/// shorthands, by name: those a `::marker` takes besides its own
/// (Pseudo-Elements 4 §4.3).
pub(super) fn is_motion_property(name: &str) -> bool {
    matches!(
        name,
        "transition"
            | "transition-property"
            | "transition-duration"
            | "transition-timing-function"
            | "transition-delay"
            | "transition-behavior"
            | "animation"
            | "animation-name"
            | "animation-duration"
            | "animation-timing-function"
            | "animation-iteration-count"
            | "animation-direction"
            | "animation-play-state"
            | "animation-delay"
            | "animation-fill-mode"
    )
}

/// A `<time>` read: its seconds, and whether a math function made it.
struct Time {
    seconds: f64,
    math: bool,
}

/// A `<time>`: a dimension in `s` or `ms`, ASCII case-insensitively, or
/// a math function's time, an infinity the largest finite time either
/// way and a `NaN` zero.
fn time(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Time> {
    if calc::is_math_function(tokens) {
        let calc::Computed::Value(seconds) = calc::parse(tokens, Want::Time)?.compute(bases, None)
        else {
            return None;
        };
        return Some(Time {
            seconds: seconds.clamp(-f64::MAX, f64::MAX),
            math: true,
        });
    }
    match single_token(tokens)? {
        Token::Dimension { value, unit } if value.is_finite() => {
            let seconds = if unit.eq_ignore_ascii_case("s") {
                *value
            } else if unit.eq_ignore_ascii_case("ms") {
                *value / 1000.0
            } else {
                return None;
            };
            Some(Time {
                seconds,
                math: false,
            })
        }
        _ => None,
    }
}

/// A `<time [0s,∞]>`: a written negative time is none, and a math
/// function's is clamped to zero.
fn duration(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f64> {
    let time = time(tokens, bases)?;
    (time.math || time.seconds >= 0.0).then_some(time.seconds.max(0.0))
}

/// A delay: any `<time>`.
fn delay(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f64> {
    time(tokens, bases).map(|time| time.seconds)
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

/// A `<number>` and whether a math function made it: written, finite;
/// or a math function's, an infinity the largest finite number and a
/// `NaN` zero.
fn number(tokens: &[Token], bases: &LengthBases<'_>) -> Option<(f64, bool)> {
    if calc::is_math_function(tokens) {
        let calc::Computed::Value(v) = calc::parse(tokens, Want::Number)?.compute(bases, None)
        else {
            return None;
        };
        return Some((v.clamp(-f64::MAX, f64::MAX), true));
    }
    match single_token(tokens)? {
        Token::Number { value, .. } if value.is_finite() => Some((*value, false)),
        _ => None,
    }
}

/// A `<percentage>` as a fraction, `50%` being `0.5`: written, finite,
/// or a math function's, its infinity the largest finite one.
fn percentage(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f64> {
    if calc::is_math_function(tokens) {
        // A basis of 100 makes each percentage its own number, so that
        // `min(50%, 60%)` folds to 50%, as a percentage of nothing else
        // can.
        let calc::Computed::Percent(p) =
            calc::parse(tokens, Want::Percentage)?.compute(bases, Some(100.0))
        else {
            return None;
        };
        return Some(p.clamp(-f64::MAX, f64::MAX) / 100.0);
    }
    match single_token(tokens)? {
        Token::Percentage { value } if value.is_finite() => Some(*value / 100.0),
        _ => None,
    }
}

/// An `<easing-function>` (Easing 1 §2, Easing 2 §2.1).
pub(super) fn parse_easing(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Easing> {
    if let Some(keyword) = single_ident(tokens) {
        return match keyword.to_ascii_lowercase().as_str() {
            "linear" => Some(Easing::Linear),
            "ease" => Some(Easing::EASE),
            "ease-in" => Some(Easing::EASE_IN),
            "ease-out" => Some(Easing::EASE_OUT),
            "ease-in-out" => Some(Easing::EASE_IN_OUT),
            "step-start" => Some(Easing::Steps {
                count: 1,
                position: StepPosition::JumpStart,
            }),
            "step-end" => Some(Easing::Steps {
                count: 1,
                position: StepPosition::JumpEnd,
            }),
            _ => None,
        };
    }
    let [function] = components(tokens)[..] else {
        return None;
    };
    let (Token::Function(name), [inside @ .., Token::CloseParen]) =
        (function.first()?, &function[1..])
    else {
        return None;
    };
    let arguments: Vec<&[Token]> = split_commas(inside).into_iter().map(trim).collect();
    if arguments.iter().any(|argument| argument.is_empty()) {
        return None;
    }
    match name.to_ascii_lowercase().as_str() {
        "cubic-bezier" => cubic_bezier(&arguments, bases),
        "steps" => steps(&arguments, bases),
        "linear" => linear(&arguments, bases),
        _ => None,
    }
}

/// `cubic-bezier()`'s four numbers. An x written outside [0, 1] makes it
/// invalid; a math function's is clamped to it, as the web-platform-tests
/// and the draft have `cubic-bezier(calc(-2), …)` computing to 0, where
/// the reference browser rejects it.
fn cubic_bezier(arguments: &[&[Token]], bases: &LengthBases<'_>) -> Option<Easing> {
    let [x1, y1, x2, y2] = arguments else {
        return None;
    };
    let x = |tokens: &[Token]| {
        let (value, math) = number(tokens, bases)?;
        if math {
            Some(value.clamp(0.0, 1.0))
        } else {
            (0.0..=1.0).contains(&value).then_some(value)
        }
    };
    let y = |tokens: &[Token]| number(tokens, bases).map(|(value, _)| value);
    Some(Easing::CubicBezier {
        x1: x(x1)?,
        y1: y(y1)?,
        x2: x(x2)?,
        y2: y(y2)?,
    })
}

/// `steps()`: a count and a step position, `end` if none is written. A
/// written count is an integer, at least one, two at `jump-none`; a math
/// function's is rounded to the nearest integer, a half up, and raised
/// to that least, but a `NaN` one makes it invalid, as the
/// web-platform-tests have `steps(calc(0/0), jump-none)`.
fn steps(arguments: &[&[Token]], bases: &LengthBases<'_>) -> Option<Easing> {
    let (count, position) = match arguments {
        [count] => (count, StepPosition::JumpEnd),
        [count, position] => (
            count,
            match single_ident(position)?.to_ascii_lowercase().as_str() {
                "jump-start" | "start" => StepPosition::JumpStart,
                "jump-end" | "end" => StepPosition::JumpEnd,
                "jump-none" => StepPosition::JumpNone,
                "jump-both" => StepPosition::JumpBoth,
                _ => return None,
            },
        ),
        _ => return None,
    };
    let least = if position == StepPosition::JumpNone {
        2.0
    } else {
        1.0
    };
    let count = if calc::is_math_function(count) {
        let value = calc::parse(count, Want::Number)?.uncensored(bases)?;
        if value.is_nan() {
            return None;
        }
        (value + 0.5).floor().max(least)
    } else {
        match single_token(count)? {
            Token::Number {
                value,
                is_integer: true,
            } if *value >= least => *value,
            _ => return None,
        }
    };
    Some(Easing::Steps {
        count: count.min(f64::from(u32::MAX)) as u32,
        position,
    })
}

/// `linear()`'s arguments (Easing 2 §2.1): each a number with up to two
/// percentages before or after it, which make one control point or two;
/// two arguments at least, as the web-platform-tests and the reference
/// browser both have it, where the draft's grammar takes one.
fn linear(arguments: &[&[Token]], bases: &LengthBases<'_>) -> Option<Easing> {
    if arguments.len() < 2 {
        return None;
    }
    let mut written: Vec<(f64, Option<f64>)> = Vec::new();
    for argument in arguments {
        let parts = components(argument);
        let (output, inputs) = match parts.as_slice() {
            [only] => (*only, &[][..]),
            [first, rest @ ..] if rest.len() <= 2 && number(first, bases).is_some() => {
                (*first, rest)
            }
            [rest @ .., last] if rest.len() <= 2 => (*last, rest),
            _ => return None,
        };
        let (output, _) = number(output, bases)?;
        if inputs.is_empty() {
            written.push((output, None));
        }
        for input in inputs {
            written.push((output, Some(percentage(input, bases)?)));
        }
    }
    Some(Easing::Points(Arc::from(canonical_points(&written))))
}

/// One item of `transition-property`: `all` or a `<custom-ident>` other
/// than `none`, a name of no property kept.
fn transition_property_item(tokens: &[Token]) -> Option<TransitionProperty> {
    let Token::Ident(name) = single_token(tokens)? else {
        return None;
    };
    if name.eq_ignore_ascii_case("all") {
        return Some(TransitionProperty::All);
    }
    if !is_custom_ident(name) || name.eq_ignore_ascii_case("none") {
        return None;
    }
    Some(TransitionProperty::Name(if name.starts_with("--") {
        Arc::from(name.as_str())
    } else {
        Arc::from(name.to_ascii_lowercase())
    }))
}

/// `transition-property`: `none` alone, an empty list, or a list of
/// items (Transitions 1 §2.1).
pub(super) fn parse_transition_property(tokens: &[Token]) -> Option<Vec<TransitionProperty>> {
    if single_ident(tokens).is_some_and(|word| word.eq_ignore_ascii_case("none")) {
        return Some(Vec::new());
    }
    list(tokens, transition_property_item)
}

/// `transition-duration` and `animation-duration`'s times.
pub(super) fn parse_durations(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Vec<f64>> {
    list(tokens, |item| duration(item, bases))
}

/// `animation-duration`: times, and `auto` (Animations 2 §4.1), which
/// is zero for an animation on a document's timeline.
pub(super) fn parse_animation_durations(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<f64>> {
    list(tokens, |item| match single_ident(item) {
        Some(word) if word.eq_ignore_ascii_case("auto") => Some(0.0),
        _ => duration(item, bases),
    })
}

/// `transition-delay` and `animation-delay`.
pub(super) fn parse_delays(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Vec<f64>> {
    list(tokens, |item| delay(item, bases))
}

/// `transition-timing-function` and `animation-timing-function`.
pub(super) fn parse_easings(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Vec<Easing>> {
    list(tokens, |item| parse_easing(item, bases))
}

fn behavior(word: &str) -> Option<TransitionBehavior> {
    match word.to_ascii_lowercase().as_str() {
        "normal" => Some(TransitionBehavior::Normal),
        "allow-discrete" => Some(TransitionBehavior::AllowDiscrete),
        _ => None,
    }
}

/// `transition-behavior` (Transitions 2 §2.5).
pub(super) fn parse_behaviors(tokens: &[Token]) -> Option<Vec<TransitionBehavior>> {
    list(tokens, |item| behavior(single_ident(item)?))
}

/// One item of `animation-name`: `None` for `none`, or a name, a
/// `<custom-ident>` or a string not empty. The draft and the
/// web-platform-tests take a string, which the reference browser does
/// not; the draft is followed.
fn animation_name_item(tokens: &[Token]) -> Option<Option<Arc<str>>> {
    match single_token(tokens)? {
        Token::Ident(name) if name.eq_ignore_ascii_case("none") => Some(None),
        Token::Ident(name) if is_custom_ident(name) => Some(Some(Arc::from(name.as_str()))),
        Token::String { value, bad: false } if !value.is_empty() => {
            Some(Some(Arc::from(value.as_str())))
        }
        _ => None,
    }
}

/// `animation-name`.
pub(super) fn parse_animation_names(tokens: &[Token]) -> Option<Vec<Option<Arc<str>>>> {
    list(tokens, animation_name_item)
}

/// One `<single-animation-iteration-count>`: `infinite`, or a number
/// not below zero, a math function's clamped to it.
fn iteration_count(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f64> {
    if single_ident(tokens).is_some_and(|word| word.eq_ignore_ascii_case("infinite")) {
        return Some(f64::INFINITY);
    }
    if calc::is_math_function(tokens) {
        let calc::Computed::Value(v) = calc::parse(tokens, Want::Number)?.compute(bases, None)
        else {
            return None;
        };
        return Some(v.max(0.0));
    }
    match single_token(tokens)? {
        Token::Number { value, .. } if value.is_finite() && *value >= 0.0 => Some(*value),
        _ => None,
    }
}

/// `animation-iteration-count`.
pub(super) fn parse_iteration_counts(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<f64>> {
    list(tokens, |item| iteration_count(item, bases))
}

fn direction(word: &str) -> Option<AnimationDirection> {
    match word.to_ascii_lowercase().as_str() {
        "normal" => Some(AnimationDirection::Normal),
        "reverse" => Some(AnimationDirection::Reverse),
        "alternate" => Some(AnimationDirection::Alternate),
        "alternate-reverse" => Some(AnimationDirection::AlternateReverse),
        _ => None,
    }
}

/// `animation-direction`.
pub(super) fn parse_directions(tokens: &[Token]) -> Option<Vec<AnimationDirection>> {
    list(tokens, |item| direction(single_ident(item)?))
}

fn play_state(word: &str) -> Option<AnimationPlayState> {
    match word.to_ascii_lowercase().as_str() {
        "running" => Some(AnimationPlayState::Running),
        "paused" => Some(AnimationPlayState::Paused),
        _ => None,
    }
}

/// `animation-play-state`.
pub(super) fn parse_play_states(tokens: &[Token]) -> Option<Vec<AnimationPlayState>> {
    list(tokens, |item| play_state(single_ident(item)?))
}

fn fill_mode(word: &str) -> Option<AnimationFillMode> {
    match word.to_ascii_lowercase().as_str() {
        "none" => Some(AnimationFillMode::None),
        "forwards" => Some(AnimationFillMode::Forwards),
        "backwards" => Some(AnimationFillMode::Backwards),
        "both" => Some(AnimationFillMode::Both),
        _ => None,
    }
}

/// `animation-fill-mode`.
pub(super) fn parse_fill_modes(tokens: &[Token]) -> Option<Vec<AnimationFillMode>> {
    list(tokens, |item| fill_mode(single_ident(item)?))
}

/// The longhands `transition` sets, one item each for every transition.
#[derive(Debug, Default)]
pub(super) struct TransitionLists {
    pub(super) property: Vec<TransitionProperty>,
    pub(super) duration: Vec<f64>,
    pub(super) timing_function: Vec<Easing>,
    pub(super) delay: Vec<f64>,
    pub(super) behavior: Vec<TransitionBehavior>,
}

/// `transition` (Transitions 2 §2.6): `<single-transition>#`, `none` a
/// transition only alone.
pub(super) fn parse_transition(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<TransitionLists> {
    let items = split_commas(tokens);
    let mut lists = TransitionLists::default();
    let mut none = false;
    for item in &items {
        let item = trim(item);
        if item.is_empty() {
            return None;
        }
        let mut property: Option<Option<TransitionProperty>> = None;
        let mut duration_value: Option<f64> = None;
        let mut delay_value: Option<f64> = None;
        let mut easing: Option<Easing> = None;
        let mut behavior_value: Option<TransitionBehavior> = None;
        for part in components(item) {
            if let Some(time) = time(part, bases) {
                if duration_value.is_none() && (time.math || time.seconds >= 0.0) {
                    duration_value = Some(time.seconds.max(0.0));
                } else if delay_value.is_none() {
                    delay_value = Some(time.seconds);
                } else {
                    return None;
                }
                continue;
            }
            if easing.is_none()
                && let Some(value) = parse_easing(part, bases)
            {
                easing = Some(value);
                continue;
            }
            if behavior_value.is_none()
                && let Some(value) = single_ident(part).and_then(behavior)
            {
                behavior_value = Some(value);
                continue;
            }
            if property.is_none() {
                if single_ident(part).is_some_and(|word| word.eq_ignore_ascii_case("none")) {
                    property = Some(None);
                    continue;
                }
                if let Some(value) = transition_property_item(part) {
                    property = Some(Some(value));
                    continue;
                }
            }
            return None;
        }
        match property {
            Some(None) => none = true,
            Some(Some(property)) => lists.property.push(property),
            None => lists.property.push(TransitionProperty::All),
        }
        lists.duration.push(duration_value.unwrap_or(0.0));
        lists.timing_function.push(easing.unwrap_or(Easing::EASE));
        lists.delay.push(delay_value.unwrap_or(0.0));
        lists
            .behavior
            .push(behavior_value.unwrap_or(TransitionBehavior::Normal));
    }
    if none && items.len() > 1 {
        return None;
    }
    Some(lists)
}

/// The longhands `animation` sets, one item each for every animation.
#[derive(Debug, Default)]
pub(super) struct AnimationLists {
    pub(super) name: Vec<Option<Arc<str>>>,
    pub(super) duration: Vec<f64>,
    pub(super) timing_function: Vec<Easing>,
    pub(super) iteration_count: Vec<f64>,
    pub(super) direction: Vec<AnimationDirection>,
    pub(super) play_state: Vec<AnimationPlayState>,
    pub(super) delay: Vec<f64>,
    pub(super) fill_mode: Vec<AnimationFillMode>,
}

/// `animation` (Animations 2 §4.12): `<single-animation>#`.
pub(super) fn parse_animation(tokens: &[Token], bases: &LengthBases<'_>) -> Option<AnimationLists> {
    let mut lists = AnimationLists::default();
    for item in split_commas(tokens) {
        let item = trim(item);
        if item.is_empty() {
            return None;
        }
        let mut name: Option<Option<Arc<str>>> = None;
        let mut duration_value: Option<f64> = None;
        let mut easing: Option<Easing> = None;
        let mut count: Option<f64> = None;
        let mut direction_value: Option<AnimationDirection> = None;
        let mut play: Option<AnimationPlayState> = None;
        let mut delay_value: Option<f64> = None;
        let mut fill: Option<AnimationFillMode> = None;
        for part in components(item) {
            if let Some(time) = time(part, bases) {
                if duration_value.is_none() && (time.math || time.seconds >= 0.0) {
                    duration_value = Some(time.seconds.max(0.0));
                } else if delay_value.is_none() {
                    delay_value = Some(time.seconds);
                } else {
                    return None;
                }
                continue;
            }
            if easing.is_none()
                && let Some(value) = parse_easing(part, bases)
            {
                easing = Some(value);
                continue;
            }
            if count.is_none()
                && let Some(value) = iteration_count(part, bases)
            {
                count = Some(value);
                continue;
            }
            if let Some(word) = single_ident(part) {
                if duration_value.is_none() && word.eq_ignore_ascii_case("auto") {
                    duration_value = Some(0.0);
                    continue;
                }
                if direction_value.is_none()
                    && let Some(value) = direction(word)
                {
                    direction_value = Some(value);
                    continue;
                }
                if fill.is_none()
                    && let Some(value) = fill_mode(word)
                {
                    fill = Some(value);
                    continue;
                }
                if play.is_none()
                    && let Some(value) = play_state(word)
                {
                    play = Some(value);
                    continue;
                }
            }
            if name.is_none()
                && let Some(value) = animation_name_item(part)
            {
                name = Some(value);
                continue;
            }
            return None;
        }
        lists.name.push(name.unwrap_or(None));
        lists.duration.push(duration_value.unwrap_or(0.0));
        lists.timing_function.push(easing.unwrap_or(Easing::EASE));
        lists.iteration_count.push(count.unwrap_or(1.0));
        lists
            .direction
            .push(direction_value.unwrap_or(AnimationDirection::Normal));
        lists
            .play_state
            .push(play.unwrap_or(AnimationPlayState::Running));
        lists.delay.push(delay_value.unwrap_or(0.0));
        lists
            .fill_mode
            .push(fill.unwrap_or(AnimationFillMode::None));
    }
    Some(lists)
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;

    fn tokens(value: &str) -> Vec<Token> {
        css::tokenize(value)
    }

    fn easing(value: &str) -> Option<Easing> {
        with_bases(|bases| parse_easing(&tokens(value), bases))
    }

    fn steps(count: u32, position: StepPosition) -> Option<Easing> {
        Some(Easing::Steps { count, position })
    }

    fn bezier(x1: f64, y1: f64, x2: f64, y2: f64) -> Option<Easing> {
        Some(Easing::CubicBezier { x1, y1, x2, y2 })
    }

    fn points(easing: Option<Easing>) -> Vec<(f64, f64)> {
        match easing {
            Some(Easing::Points(points)) => points.to_vec(),
            other => panic!("not linear(): {other:?}"),
        }
    }

    #[test]
    fn the_easing_keywords_are_their_functions() {
        assert_eq!(easing("linear"), Some(Easing::Linear));
        assert_eq!(easing("EASE"), Some(Easing::EASE));
        assert_eq!(easing("ease-in"), Some(Easing::EASE_IN));
        assert_eq!(easing("ease-out"), Some(Easing::EASE_OUT));
        assert_eq!(easing("ease-in-out"), Some(Easing::EASE_IN_OUT));
        assert_eq!(easing("step-start"), steps(1, StepPosition::JumpStart));
        assert_eq!(easing("step-end"), steps(1, StepPosition::JumpEnd));
        for bad in ["auto", "ease-in ease-out", "initial", "", "steps", "1"] {
            assert_eq!(easing(bad), None, "{bad}");
        }
    }

    #[test]
    fn cubic_bezier_takes_four_numbers_its_x_in_range() {
        // The web-platform-tests' timing-functions-syntax tests.
        assert_eq!(
            easing("cubic-bezier(0.1, 0.2, 0.8, 0.9)"),
            bezier(0.1, 0.2, 0.8, 0.9)
        );
        assert_eq!(
            easing("cubic-bezier(0, -2, 1, 3)"),
            bezier(0.0, -2.0, 1.0, 3.0)
        );
        // A math function's x is clamped to [0, 1], as the tests and the
        // draft have it, where the reference browser rejects it.
        assert_eq!(
            easing("cubic-bezier(calc(-2), calc(0.7 / 2), calc(1.5), calc(0))"),
            bezier(0.0, 0.35, 1.0, 0.0)
        );
        // A y of infinity is the largest finite number, as the reference
        // browser computes it.
        assert_eq!(
            easing("cubic-bezier(calc(0.5), calc(infinity), 0.5, 1)"),
            bezier(0.5, f64::MAX, 0.5, 1.0)
        );
        for bad in [
            "cubic-bezier(1, 2, 3)",
            "cubic-bezier(1, 2, 3, infinite)",
            "cubic-bezier(1, 2, 3, 4, 5)",
            "cubic-bezier(-0.1, 0.1, 0.5, 0.9)",
            "cubic-bezier(0.5, 0.1, 1.1, 0.9)",
            "cubic-bezier(0.5, , 1, 0.9)",
            "cubic-bezier(0.5 0.1 1 0.9)",
            "cubic-bezier(0.5, 1px, 1, 0.9)",
        ] {
            assert_eq!(easing(bad), None, "{bad}");
        }
    }

    #[test]
    fn steps_take_a_count_and_a_position() {
        use StepPosition::*;
        assert_eq!(easing("steps(4)"), steps(4, JumpEnd));
        assert_eq!(easing("steps(4, start)"), steps(4, JumpStart));
        assert_eq!(easing("steps(2, end)"), steps(2, JumpEnd));
        assert_eq!(easing("steps( 2, end )"), steps(2, JumpEnd));
        assert_eq!(easing("STEPS(2, Jump-Both)"), steps(2, JumpBoth));
        assert_eq!(easing("steps(1, jump-start)"), steps(1, JumpStart));
        assert_eq!(easing("steps(2, jump-none)"), steps(2, JumpNone));
        // A math function's count is rounded and raised to its least.
        assert_eq!(easing("steps(calc(-10), start)"), steps(1, JumpStart));
        assert_eq!(easing("steps(calc(5 / 2), start)"), steps(3, JumpStart));
        assert_eq!(easing("steps(calc(1), jump-none)"), steps(2, JumpNone));
        for bad in [
            "steps(0, start)",
            "steps(0, end)",
            "steps(0, jump-both)",
            "steps(1, jump-none)",
            "steps(-100, start)",
            "steps(-100, jump-none)",
            "steps(2.5)",
            "steps(calc(0/0), jump-none)",
            "steps(2, middle)",
            "steps(2, start, end)",
            "steps()",
        ] {
            assert_eq!(easing(bad), None, "{bad}");
        }
    }

    #[test]
    fn linear_points_are_canonicalized_where_they_are_computed() {
        // The web-platform-tests' linear-timing-functions-syntax tests.
        assert_eq!(
            points(easing("linear(0 0%, 1 100%)")),
            vec![(0.0, 0.0), (1.0, 1.0)]
        );
        assert_eq!(points(easing("linear(0, 1)")), vec![(0.0, 0.0), (1.0, 1.0)]);
        assert_eq!(
            points(easing("linear(0 0% 50%, 1 50% 100%)")),
            vec![(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (1.0, 1.0)]
        );
        assert_eq!(
            points(easing("linear(0 calc(50% - 50%), 0 calc(50% + 50%))")),
            vec![(0.0, 0.0), (1.0, 0.0)]
        );
        assert_eq!(
            points(easing("linear(0 calc(min(50%, 60%)), 0 100%)")),
            vec![(0.5, 0.0), (1.0, 0.0)]
        );
        // A percentage may come first; a `NaN` number is 0.
        assert_eq!(
            points(easing("linear(0% 0, 100% 1)")),
            vec![(0.0, 0.0), (1.0, 1.0)]
        );
        assert_eq!(
            points(easing("linear(calc(0/0), 1)")),
            vec![(0.0, 0.0), (1.0, 1.0)]
        );
        for bad in [
            "linear()",
            "linear(0)",
            "linear(100%)",
            "linear(0% 1 50%)",
            "linear(0 0% 100%)",
            "linear(0% 100% 0)",
            "linear(0 calc(50px - 50%), 0 calc(50em + 50em))",
            "linear(0 calc(50%, 50%), 0 calc(50% + 50%))",
            "linear(0 1 2)",
            "linear(0 10% 20% 30%, 1)",
            "linear(0, , 1)",
        ] {
            assert_eq!(easing(bad), None, "{bad}");
        }
    }

    #[test]
    fn times_are_seconds_written_or_computed() {
        let durations = |value: &str| with_bases(|bases| parse_durations(&tokens(value), bases));
        let delays = |value: &str| with_bases(|bases| parse_delays(&tokens(value), bases));
        assert_eq!(
            durations("1s, 250ms, 1S, 0s"),
            Some(vec![1.0, 0.25, 1.0, 0.0])
        );
        assert_eq!(delays("-1s, 2ms"), Some(vec![-1.0, 0.002]));
        // A math function's negative duration is zero, its infinity the
        // largest finite time and its `NaN` zero, as the reference
        // browser computes them.
        assert_eq!(durations("calc(-1s)"), Some(vec![0.0]));
        assert_eq!(durations("calc(infinity * 1s)"), Some(vec![f64::MAX]));
        assert_eq!(delays("calc(-infinity * 1s)"), Some(vec![-f64::MAX]));
        assert_eq!(delays("calc(1s * NaN)"), Some(vec![0.0]));
        assert_eq!(delays("calc(100ms + 1s)"), Some(vec![1.1]));
        for bad in [
            "-1s",
            "0",
            "1",
            "1px",
            "1s 2s",
            "1s,",
            ", 1s",
            "auto",
            "calc(1px)",
        ] {
            assert_eq!(durations(bad), None, "{bad}");
        }
        assert_eq!(delays("0"), None);
    }

    #[test]
    fn the_lists_take_their_keywords() {
        let names = |value: &str| {
            parse_animation_names(&tokens(value)).map(|names| {
                names
                    .iter()
                    .map(|n| n.as_deref().map(str::to_string))
                    .collect::<Vec<_>>()
            })
        };
        let some = |name: &str| Some(name.to_string());
        // The web-platform-tests' animation-name tests.
        assert_eq!(names("none"), Some(vec![None]));
        assert_eq!(names("NONE"), Some(vec![None]));
        assert_eq!(names("foo"), Some(vec![some("foo")]));
        assert_eq!(names("Both"), Some(vec![some("Both")]));
        assert_eq!(
            names("first, second, third"),
            Some(vec![some("first"), some("second"), some("third")])
        );
        assert_eq!(names("none, a"), Some(vec![None, some("a")]));
        // Strings, which the draft and the tests take and the reference
        // browser does not.
        assert_eq!(names("\"something\""), Some(vec![some("something")]));
        assert_eq!(names("\"none\""), Some(vec![some("none")]));
        assert_eq!(
            names("\"initial\", both"),
            Some(vec![some("initial"), some("both")])
        );
        for bad in [
            "12",
            "one two",
            "one, initial",
            "one, inherit",
            "one, unset",
            "default, two",
            "revert, three",
            "revert-layer, four",
            "\"\"",
            "default",
        ] {
            assert_eq!(names(bad), None, "{bad}");
        }
        let counts =
            |value: &str| with_bases(|bases| parse_iteration_counts(&tokens(value), bases));
        assert_eq!(
            counts("infinite, 2.5, 0"),
            Some(vec![f64::INFINITY, 2.5, 0.0])
        );
        assert_eq!(counts("calc(-1)"), Some(vec![0.0]));
        assert_eq!(counts("calc(infinity)"), Some(vec![f64::INFINITY]));
        assert_eq!(counts("-1"), None);
        assert_eq!(
            parse_directions(&tokens("normal, REVERSE, alternate, alternate-reverse")),
            Some(vec![
                AnimationDirection::Normal,
                AnimationDirection::Reverse,
                AnimationDirection::Alternate,
                AnimationDirection::AlternateReverse
            ])
        );
        assert_eq!(
            parse_play_states(&tokens("paused, running")),
            Some(vec![
                AnimationPlayState::Paused,
                AnimationPlayState::Running
            ])
        );
        assert_eq!(
            parse_fill_modes(&tokens("none, forwards, backwards, both")),
            Some(vec![
                AnimationFillMode::None,
                AnimationFillMode::Forwards,
                AnimationFillMode::Backwards,
                AnimationFillMode::Both
            ])
        );
        assert_eq!(
            parse_behaviors(&tokens("allow-discrete, normal")),
            Some(vec![
                TransitionBehavior::AllowDiscrete,
                TransitionBehavior::Normal
            ])
        );
        assert_eq!(parse_fill_modes(&tokens("none, initial")), None);
        let durations =
            |value: &str| with_bases(|bases| parse_animation_durations(&tokens(value), bases));
        assert_eq!(durations("auto, 2s"), Some(vec![0.0, 2.0]));
    }

    #[test]
    fn transition_property_keeps_names_it_does_not_know() {
        let name = |name: &str| TransitionProperty::Name(Arc::from(name));
        assert_eq!(parse_transition_property(&tokens("none")), Some(vec![]));
        assert_eq!(
            parse_transition_property(&tokens("ALL, Opacity, --Foo, -webkit-transform, foo")),
            Some(vec![
                TransitionProperty::All,
                name("opacity"),
                name("--Foo"),
                name("-webkit-transform"),
                name("foo")
            ])
        );
        for bad in [
            "none, opacity",
            "opacity, none",
            "default",
            "inherit, opacity",
            "1px",
            "",
        ] {
            assert_eq!(parse_transition_property(&tokens(bad)), None, "{bad}");
        }
    }

    fn transition(value: &str) -> Option<TransitionLists> {
        with_bases(|bases| parse_transition(&tokens(value), bases))
    }

    #[test]
    fn the_transition_shorthand_reads_as_the_reference_browser_reads_it() {
        let lists = transition("opacity 1s, all 2s").expect("valid");
        assert_eq!(
            lists.property,
            vec![
                TransitionProperty::Name(Arc::from("opacity")),
                TransitionProperty::All
            ]
        );
        assert_eq!(lists.duration, vec![1.0, 2.0]);
        assert_eq!(lists.delay, vec![0.0, 0.0]);
        // A negative time is no duration and so the delay.
        let lists = transition("-1s").expect("valid");
        assert_eq!((lists.duration[0], lists.delay[0]), (0.0, -1.0));
        let lists = transition("-1s 2s").expect("valid");
        assert_eq!((lists.duration[0], lists.delay[0]), (2.0, -1.0));
        let lists = transition("width 1s linear 0.5s allow-discrete").expect("valid");
        assert_eq!(lists.timing_function, vec![Easing::Linear]);
        assert_eq!(lists.behavior, vec![TransitionBehavior::AllowDiscrete]);
        assert_eq!(lists.delay, vec![0.5]);
        // A second easing keyword is a property's name, as the grammar
        // reads it.
        let lists = transition("ease ease-in").expect("valid");
        assert_eq!(
            lists.property,
            vec![TransitionProperty::Name(Arc::from("ease-in"))]
        );
        let lists = transition("normal 1s").expect("valid");
        assert_eq!(lists.property, vec![TransitionProperty::All]);
        let lists = transition("none").expect("valid");
        assert!(lists.property.is_empty());
        assert_eq!(lists.duration, vec![0.0]);
        for bad in [
            "1s 2s 3s",
            "none, opacity 1s",
            "opacity, none",
            "default 1s",
            "foo bar",
            "opacity 1s,",
            "",
            "1px",
        ] {
            assert!(transition(bad).is_none(), "{bad}");
        }
    }

    fn animation(value: &str) -> Option<AnimationLists> {
        with_bases(|bases| parse_animation(&tokens(value), bases))
    }

    #[test]
    fn the_animation_shorthand_gives_keywords_to_the_other_properties_first() {
        let name = |lists: &AnimationLists| lists.name[0].as_deref().map(str::to_string);
        let lists = animation("1s 2s ease-in infinite reverse both paused slide").expect("valid");
        assert_eq!(name(&lists), Some("slide".into()));
        assert_eq!((lists.duration[0], lists.delay[0]), (1.0, 2.0));
        assert_eq!(lists.timing_function[0], Easing::EASE_IN);
        assert_eq!(lists.iteration_count[0], f64::INFINITY);
        assert_eq!(lists.direction[0], AnimationDirection::Reverse);
        assert_eq!(lists.fill_mode[0], AnimationFillMode::Both);
        assert_eq!(lists.play_state[0], AnimationPlayState::Paused);
        // `ease ease`: the easing, then the name; `paused running`, the
        // play state then the name; `none none`, the fill mode then the
        // name; `forwards backwards` likewise; `auto 1s` a duration of
        // `auto` and a delay; `auto auto` a duration and a name — each as
        // the reference browser reads it.
        assert_eq!(name(&animation("ease ease").unwrap()), Some("ease".into()));
        let paused = animation("paused running").unwrap();
        assert_eq!(paused.play_state[0], AnimationPlayState::Paused);
        assert_eq!(name(&paused), Some("running".into()));
        let none = animation("none none").unwrap();
        assert_eq!(
            (none.fill_mode[0], name(&none)),
            (AnimationFillMode::None, None)
        );
        let fill = animation("forwards backwards").unwrap();
        assert_eq!(fill.fill_mode[0], AnimationFillMode::Forwards);
        assert_eq!(name(&fill), Some("backwards".into()));
        let auto = animation("auto 1s").unwrap();
        assert_eq!(
            (auto.duration[0], auto.delay[0], name(&auto)),
            (0.0, 1.0, None)
        );
        assert_eq!(name(&animation("auto auto").unwrap()), Some("auto".into()));
        assert_eq!(
            name(&animation("normal normal").unwrap()),
            Some("normal".into())
        );
        assert_eq!(
            name(&animation("infinite infinite").unwrap()),
            Some("infinite".into())
        );
        let negative = animation("-1s").unwrap();
        assert_eq!((negative.duration[0], negative.delay[0]), (0.0, -1.0));
        // A string name, which the draft takes.
        assert_eq!(name(&animation("\"x\" 1s").unwrap()), Some("x".into()));
        // The web-platform-tests' animation-shorthand items.
        let lists = animation("anim paused both reverse, 4 1s -3s cubic-bezier(0, -2, 1, 3)")
            .expect("valid");
        assert_eq!(lists.name.len(), 2);
        assert_eq!(lists.iteration_count, vec![1.0, 4.0]);
        assert_eq!(lists.delay, vec![0.0, -3.0]);
        assert_eq!(
            lists.timing_function[1],
            Easing::CubicBezier {
                x1: 0.0,
                y1: -2.0,
                x2: 1.0,
                y2: 3.0
            }
        );
        for bad in [
            "none none none",
            "2 3",
            "default",
            "inherit 1s",
            "1s 2s 3s",
            "a b",
            "a,",
        ] {
            assert!(animation(bad).is_none(), "{bad}");
        }
    }
}
