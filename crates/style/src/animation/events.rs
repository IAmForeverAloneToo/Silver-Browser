//! The events of CSS animations and transitions (Animations 2 §5.1,
//! Transitions 2 §5.1; M22.2): which a change of phase between two
//! animation frames queues, with the elapsed time the drafts' tables give.
//! Nothing listens to them yet; the engine hands each frame's to its
//! caller (M26 dispatches them).

use std::sync::Arc;

use dom::NodeId;

use super::{Phase, Timing};
use crate::computed::AnimationFillMode;
use crate::selector_matching::PseudoElement;

/// What an event says happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    AnimationStart,
    AnimationIteration,
    AnimationEnd,
    AnimationCancel,
    TransitionRun,
    TransitionStart,
    TransitionEnd,
    TransitionCancel,
}

impl EventKind {
    /// The event's type, as a listener names it.
    pub fn name(self) -> &'static str {
        match self {
            EventKind::AnimationStart => "animationstart",
            EventKind::AnimationIteration => "animationiteration",
            EventKind::AnimationEnd => "animationend",
            EventKind::AnimationCancel => "animationcancel",
            EventKind::TransitionRun => "transitionrun",
            EventKind::TransitionStart => "transitionstart",
            EventKind::TransitionEnd => "transitionend",
            EventKind::TransitionCancel => "transitioncancel",
        }
    }
}

/// An event an animation or a transition queued.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationEvent {
    pub kind: EventKind,
    /// The element it is targeted at: the animation's or transition's
    /// owning element, or the originating element of its
    /// pseudo-element.
    pub node: NodeId,
    pub pseudo: Option<PseudoElement>,
    /// The animation's `@keyframes` name, or the transition's property.
    pub name: Arc<str>,
    /// The elapsed time, in seconds.
    pub elapsed: f64,
}

/// An animation or a transition running, as [`super::Animations::running`]
/// lists them for the tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub node: NodeId,
    pub pseudo: Option<PseudoElement>,
    /// The animation's `@keyframes` name, or the transition's property.
    pub name: Arc<str>,
    /// Whether it is a transition.
    pub transition: bool,
    /// Its phase; `None` while it waits for a frame to start it.
    pub phase: Option<Phase>,
    /// Its current time, in seconds.
    pub current_time: f64,
    pub paused: bool,
}

/// The phase an animation or a transition was in at the last animation
/// frame, for the events: idle before its first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reported {
    Idle,
    Before,
    Active,
    After,
}

impl From<Phase> for Reported {
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::Before => Reported::Before,
            Phase::Active => Reported::Active,
            Phase::After => Reported::After,
        }
    }
}

/// The interval start of an effect's events: `max(min(-delay, active
/// duration), 0)`.
fn interval_start(timing: &Timing) -> f64 {
    (-timing.delay).min(timing.active_duration()).max(0.0)
}

/// The interval end: `max(min(end time - delay, active duration), 0)`.
fn interval_end(timing: &Timing) -> f64 {
    (timing.end_time() - timing.delay)
        .min(timing.active_duration())
        .max(0.0)
}

/// The events of a CSS animation whose phase went from `before` to `now`
/// and whose current iteration from `was` to `is` (Animations 2 §5.1).
pub(super) fn animation_events(
    timing: &Timing,
    before: Reported,
    now: Reported,
    was: Option<f64>,
    is: Option<f64>,
) -> Vec<(EventKind, f64)> {
    use EventKind::{AnimationEnd, AnimationIteration, AnimationStart};
    let (start, end) = (interval_start(timing), interval_end(timing));
    match (before, now) {
        (Reported::Idle | Reported::Before, Reported::Active) => vec![(AnimationStart, start)],
        (Reported::Idle | Reported::Before, Reported::After) => {
            vec![(AnimationStart, start), (AnimationEnd, end)]
        }
        (Reported::Active, Reported::Before) => vec![(AnimationEnd, start)],
        (Reported::Active, Reported::Active) => match (was, is) {
            (Some(was), Some(is)) if was != is => {
                let boundary = if was > is { is + 1.0 } else { is };
                vec![(AnimationIteration, boundary * timing.duration)]
            }
            _ => Vec::new(),
        },
        (Reported::Active, Reported::After) => vec![(AnimationEnd, end)],
        (Reported::After, Reported::Active) => vec![(AnimationStart, end)],
        (Reported::After, Reported::Before) => vec![(AnimationStart, end), (AnimationEnd, start)],
        _ => Vec::new(),
    }
}

/// The events of a transition whose phase went from `before` to `now`
/// (Transitions 2 §5.1).
pub(super) fn transition_events(
    timing: &Timing,
    before: Reported,
    now: Reported,
) -> Vec<(EventKind, f64)> {
    use EventKind::{TransitionEnd, TransitionRun, TransitionStart};
    let (start, end) = (interval_start(timing), interval_end(timing));
    match (before, now) {
        (Reported::Idle, Reported::Before) => vec![(TransitionRun, start)],
        (Reported::Idle, Reported::Active) => {
            vec![(TransitionRun, start), (TransitionStart, start)]
        }
        (Reported::Idle, Reported::After) => vec![
            (TransitionRun, start),
            (TransitionStart, start),
            (TransitionEnd, end),
        ],
        (Reported::Before, Reported::Active) => vec![(TransitionStart, start)],
        (Reported::Before, Reported::After) => {
            vec![(TransitionStart, start), (TransitionEnd, end)]
        }
        (Reported::Active, Reported::After) => vec![(TransitionEnd, end)],
        (Reported::Active, Reported::Before) => vec![(TransitionEnd, start)],
        (Reported::After, Reported::Active) => vec![(TransitionStart, end)],
        (Reported::After, Reported::Before) => {
            vec![(TransitionStart, end), (TransitionEnd, start)]
        }
        _ => Vec::new(),
    }
}

/// The elapsed time of a cancel event: the active time at `local`,
/// computed with a fill mode of both; `None` when the animation or
/// transition reported no phase, or the after phase, which no cancel
/// event follows.
pub(super) fn cancelled(timing: &Timing, reported: Reported, local: f64) -> Option<f64> {
    if matches!(reported, Reported::Idle | Reported::After) {
        return None;
    }
    let both = Timing {
        fill: AnimationFillMode::Both,
        ..timing.clone()
    };
    Some(both.progress(local).active_time.unwrap_or(0.0))
}
