//! The timing model of Web Animations (Web Animations Level 1 section
//! 4, section 5.3.4; M22, D75) and the animations a document runs: CSS
//! animations, CSS transitions and the animations scripts make, in one
//! store.
//!
//! [`Timing`](crate::animation::Timing) is an effect's timing: its
//! delay, end delay, iteration duration, iteration count and start,
//! direction, fill mode and easing. Given a local time and the
//! direction its animation plays in, it gives the effect's
//! [`Progress`](crate::animation::Progress): its phase, its active time
//! as the fill mode allows one, its current iteration, and its
//! transformed progress with the before flag.
//! [`interval`](crate::animation::interval) then finds the keyframes on
//! either side of that progress among one property's keyframes
//! (section 5.3.4).
//!
//! Times are milliseconds in an `f64`. An infinite iteration count makes
//! the active duration infinite; a zero iteration duration or count makes
//! it zero; no step computes an infinity less an infinity or zero times
//! an infinity, so that every result is a number or `None`.
//!
//! [`Animations`](crate::animation::Animations) is what a document runs:
//! its timeline's time, which the caller moves on and which never goes
//! back, and every animation by its number, each with its playback
//! ([`player`]), its keyframe effect, the timeline it runs on and its
//! replace state (D75). Its CSS animations are listed by their owning
//! element in `animation-name`'s order and its running transitions by
//! theirs; every effect is listed by its target. The cascade keeps the
//! CSS ones as it computes styles -- an element's `animation-name` matched
//! to the animations it had (Animations 1 section 2), its transitions
//! started, replaced and cancelled at a change of its style
//! (`transition`) -- and reads every effect on a target at the timeline's
//! time, in composite order; a frame moves the timeline on, runs the
//! pending tasks of the animations ready, updates their finished state,
//! ends the transitions past their end, and asks which targets'
//! animations give something new, to be restyled. Scripts reach the same
//! animations by their numbers (`script`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use dom::NodeId;

use crate::computed::{AnimationDirection, AnimationFillMode, Composite, Edge, LengthPercentage};
use crate::easing::Easing;
use crate::keyframes::ScriptKeyframes;
use crate::selector_matching::PseudoElement;

mod events;
pub mod player;
mod script;
mod timeline;
mod transition;

pub use events::{AnimationEvent, EventKind, PlaybackEvent, PlaybackKind, Running};
use events::{Reported, animation_events, cancelled};
use player::{Context, Notice, PlayState, Player};
pub use script::{
    AnimationState, ComputedTiming, EffectInput, KeyframesSource, ScriptError, TimingInput,
    TransitionValue, pseudo_element, pseudo_element_name,
};
pub use timeline::{Kind as TimelineKind, Measured, RANGES, Source as TimelineSource};
use timeline::{Kind, Timeline};
use transition::{Completed, TransitionValues};
pub(crate) use transition::{Property, Transitioning, Value};

/// An animation effect's timing (section 4.5), in milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    /// The start delay, negative to start part way.
    pub delay: f64,
    /// The end delay, after the active interval.
    pub end_delay: f64,
    /// The iteration duration, not below zero.
    pub duration: f64,
    /// The iteration count, not below zero, infinite for `infinite`.
    pub iterations: f64,
    /// The iteration start, not below zero.
    pub iteration_start: f64,
    /// The playback direction.
    pub direction: AnimationDirection,
    /// The fill mode.
    pub fill: AnimationFillMode,
    /// The effect's easing: a transition's timing function; linear for
    /// a CSS animation, whose timing function is its keyframes'.
    pub easing: Easing,
}

impl Default for Timing {
    /// Web Animations' defaults: no delays, a zero duration, one
    /// iteration from the start, forwards, no fill, linear.
    fn default() -> Self {
        Timing {
            delay: 0.0,
            end_delay: 0.0,
            duration: 0.0,
            iterations: 1.0,
            iteration_start: 0.0,
            direction: AnimationDirection::Normal,
            fill: AnimationFillMode::None,
            easing: Easing::Linear,
        }
    }
}

/// Where an effect's local time falls (section 4.6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Before its active interval.
    Before,
    /// In it.
    Active,
    /// After it.
    After,
}

/// What an effect gives at a local time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    /// The phase.
    pub phase: Phase,
    /// The active time, `None` when the effect is not in effect: in the
    /// before phase without a backwards fill, or the after phase without
    /// a forwards one.
    pub active_time: Option<f64>,
    /// The current iteration, from zero; `None` when not in effect.
    pub iteration: Option<f64>,
    /// The transformed progress, the iteration's progress through the
    /// direction and the easing; `None` when not in effect.
    pub progress: Option<f64>,
    /// The before flag (section 4.7.7): set going forwards in the before
    /// phase, or backwards in the after phase.
    pub before: bool,
}

impl Timing {
    /// The active duration (section 4.7.1): the iteration duration times
    /// the count, zero when either is.
    pub fn active_duration(&self) -> f64 {
        if self.duration == 0.0 || self.iterations == 0.0 {
            0.0
        } else {
            self.duration * self.iterations
        }
    }

    /// The end time: the delays and the active duration together, not
    /// below zero (section 4.6.1).
    pub fn end_time(&self) -> f64 {
        let end = self.delay + self.active_duration() + self.end_delay;
        if end.is_nan() { 0.0 } else { end.max(0.0) }
    }

    /// The phase at `local` (section 4.6.6), the playback going
    /// forwards.
    pub fn phase(&self, local: f64) -> Phase {
        self.phase_in(local, false)
    }

    /// The phase at `local`, the animation playing `backwards` (its
    /// playback rate below zero) or forwards: a boundary belongs to the
    /// phase the playback leaves.
    pub fn phase_in(&self, local: f64, backwards: bool) -> Phase {
        let end = self.end_time();
        let before_active = self.delay.min(end).max(0.0);
        let active_after = (self.delay + self.active_duration()).min(end).max(0.0);
        if local < before_active || (backwards && local == before_active) {
            Phase::Before
        } else if local > active_after || (!backwards && local == active_after) {
            Phase::After
        } else {
            Phase::Active
        }
    }

    /// What the effect gives at the local time `local` (section 4.7),
    /// the playback going forwards.
    pub fn progress(&self, local: f64) -> Progress {
        self.progress_in(local, false)
    }

    /// What the effect gives at the local time `local`, the animation
    /// playing `backwards` or forwards.
    pub fn progress_in(&self, local: f64, backwards: bool) -> Progress {
        let phase = self.phase_in(local, backwards);
        let active_duration = self.active_duration();
        let fills_backwards = matches!(
            self.fill,
            AnimationFillMode::Backwards | AnimationFillMode::Both
        );
        let fills_forwards = matches!(
            self.fill,
            AnimationFillMode::Forwards | AnimationFillMode::Both
        );
        // Section 4.7.2.
        let active_time = match phase {
            Phase::Before => fills_backwards.then(|| (local - self.delay).max(0.0)),
            Phase::Active => Some(local - self.delay),
            Phase::After => {
                fills_forwards.then(|| (local - self.delay).min(active_duration).max(0.0))
            }
        };
        let Some(active_time) = active_time else {
            return Progress {
                phase,
                active_time: None,
                iteration: None,
                progress: None,
                before: false,
            };
        };
        // Section 4.7.3: the overall progress, from the iteration start.
        let overall = if self.duration == 0.0 {
            if phase == Phase::Before {
                0.0
            } else {
                self.iterations
            }
        } else {
            active_time / self.duration
        } + self.iteration_start;
        // Section 4.7.4: the simple iteration progress, the end of an
        // iteration being one of it rather than zero of the next.
        let mut simple = if overall.is_infinite() {
            self.iteration_start % 1.0
        } else {
            overall % 1.0
        };
        if simple == 0.0
            && matches!(phase, Phase::Active | Phase::After)
            && active_time == active_duration
            && self.iterations != 0.0
        {
            simple = 1.0;
        }
        // Section 4.7.5: the current iteration.
        let iteration = if phase == Phase::After && self.iterations.is_infinite() {
            f64::INFINITY
        } else if simple == 1.0 {
            overall.floor() - 1.0
        } else {
            overall.floor()
        };
        // Section 4.7.6: the direction.
        let reverse = match self.direction {
            AnimationDirection::Normal => false,
            AnimationDirection::Reverse => true,
            AnimationDirection::Alternate | AnimationDirection::AlternateReverse => {
                let d = if self.direction == AnimationDirection::AlternateReverse {
                    iteration + 1.0
                } else {
                    iteration
                };
                d.is_finite() && d % 2.0 != 0.0
            }
        };
        let directed = if reverse { 1.0 - simple } else { simple };
        // Section 4.7.7: the before flag and the easing.
        let before = (!reverse && phase == Phase::Before) || (reverse && phase == Phase::After);
        Progress {
            phase,
            active_time: Some(active_time),
            iteration: Some(iteration.max(0.0)),
            progress: Some(self.easing.output(directed, before)),
            before,
        }
    }
}

/// The keyframes around a progress among one property's (section
/// 5.3.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Interval {
    /// One keyframe alone gives the value.
    At(usize),
    /// Between two, `distance` of the way from the first, before its
    /// easing.
    Between {
        from: usize,
        to: usize,
        distance: f64,
    },
}

/// The interval of a property's keyframes, whose offsets `offsets` are
/// in order and include 0 and 1, that the iteration progress `progress`
/// falls in (section 5.3.4): beyond either end, the end's first or last
/// keyframe alone when more than one sits there, and otherwise the two
/// whose offsets the progress lies between or the end two.
pub fn interval(offsets: &[f64], progress: f64) -> Interval {
    let at_zero = offsets.iter().filter(|&&o| o == 0.0).count();
    let at_one = offsets.iter().filter(|&&o| o == 1.0).count();
    if progress < 0.0 && at_zero > 1 {
        return Interval::At(0);
    }
    if progress >= 1.0 && at_one > 1 {
        return Interval::At(offsets.len() - 1);
    }
    let from = offsets
        .iter()
        .rposition(|&o| o <= progress && o < 1.0)
        .unwrap_or_else(|| {
            offsets
                .iter()
                .rposition(|&o| o == 0.0)
                .expect("a keyframe at 0")
        });
    let to = (from + 1).min(offsets.len() - 1);
    if to == from {
        return Interval::At(from);
    }
    let span = offsets[to] - offsets[from];
    let distance = if span == 0.0 {
        0.0
    } else {
        (progress - offsets[from]) / span
    };
    Interval::Between { from, to, distance }
}

/// The most animations a document keeps together, CSS's and scripts':
/// past it a new one does not start. No page meets it; one built to
/// exhaust the engine does.
pub const MAX_ANIMATIONS: usize = 65_536;

/// The most document timelines a document's scripts make beside its
/// own: past it the constructor gives the document's.
pub const MAX_TIMELINES: usize = 1024;

/// What an animation animates: an element, or its `::before`, `::after`
/// or `::marker`.
pub type Target = (NodeId, Option<PseudoElement>);

/// `map`'s entry for `target`, taken out. An empty map is not asked:
/// removing hashes the key even then, and the cascade asks for every
/// element and pseudo-element of a document that animates nothing.
fn take<V>(map: &mut HashMap<Target, V>, target: &Target) -> Option<V> {
    if map.is_empty() {
        None
    } else {
        map.remove(target)
    }
}

/// An animation's replace state (section 5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceState {
    Active,
    Removed,
    Persisted,
}

impl ReplaceState {
    /// Its name, as `replaceState` gives it.
    pub fn name(self) -> &'static str {
        match self {
            ReplaceState::Active => "active",
            ReplaceState::Removed => "removed",
            ReplaceState::Persisted => "persisted",
        }
    }
}

/// What kind an animation is, which decides its composite order (section
/// 5.4.2; Animations 2 section 4.3, Transitions 2 section 6.2), and what
/// its kind keeps.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Class {
    /// A CSS transition of `property`, owned by its element while it
    /// runs, of the `generation` of the change of style that started it
    /// (Transitions 2 section 6.2).
    Transition {
        owner: Option<Target>,
        property: Property,
        generation: u64,
    },
    /// A CSS animation of the `@keyframes` named `name`, owned by its
    /// element while its `animation-name` names it. `scripted` once a
    /// script played or paused it, after which `animation-play-state`
    /// no longer does; `paused` what that property last said.
    Css {
        owner: Option<Target>,
        name: Arc<str>,
        scripted: bool,
        paused: bool,
    },
    /// An animation a script made.
    Script,
}

/// An animation (section 4.4).
#[derive(Debug, Clone)]
pub(crate) struct Animation {
    pub(crate) class: Class,
    pub(crate) player: Player,
    /// Its associated effect's number.
    pub(crate) effect: Option<u64>,
    /// Its timeline, by its place among the document's; `None` for none.
    pub(crate) timeline: Option<usize>,
    /// The range of a progress-based timeline it is attached to, its
    /// start and its end (Web Animations 2 section 4.4.1.1, `rangeStart`
    /// and `rangeEnd`; `animation-range`).
    pub(crate) range: [Edge<LengthPercentage>; 2],
    /// Whether a script set its range, which its style then no longer
    /// does (Animations 2 section 4.2).
    pub(crate) range_set: bool,
    /// Whether a script set its timeline, which its style's
    /// `animation-timeline` then no longer does.
    pub(crate) timeline_set: bool,
    pub(crate) replace: ReplaceState,
    /// What the last sample gave, for the next frame to compare with.
    last: Sampled,
    /// The phase and current iteration at the last animation frame, for
    /// the CSS events.
    reported: Reported,
    reported_iteration: Option<f64>,
    /// Whether a script holds an object for it, which hears its notices
    /// and its playback events.
    pub(crate) held: bool,
    /// Whether a script's style flush made it and the frame's styles
    /// have not seen it yet.
    unseen: bool,
}

/// What a keyframe effect's keyframes are.
#[derive(Debug, Clone)]
pub(crate) enum Frames {
    /// A CSS animation's: its `@keyframes` rule's by name, read where the
    /// cascade reads the sheets, the easing of a keyframe that declares
    /// none, and the longhands the rule animated when its element last
    /// named it.
    Css {
        name: Arc<str>,
        easing: Easing,
        longhands: Arc<[crate::animate::Longhand]>,
    },
    /// A script's (`KeyframeEffect`, `setKeyframes`).
    Script(Arc<ScriptKeyframes>),
    /// A CSS transition's, from its start value to its end value.
    Transition(Box<TransitionValues>),
}

/// Which members of a CSS animation's effect a script set, which its
/// element's `animation-*` properties no longer do (Animations 2 section
/// 4.2): bits of [`Overridden`].
type Overridden = u16;
const SET_DELAY: Overridden = 1;
const SET_END_DELAY: Overridden = 1 << 1;
const SET_DURATION: Overridden = 1 << 2;
const SET_ITERATIONS: Overridden = 1 << 3;
const SET_DIRECTION: Overridden = 1 << 4;
const SET_FILL: Overridden = 1 << 5;
const SET_EASING: Overridden = 1 << 6;
const SET_ITERATION_START: Overridden = 1 << 7;
const SET_KEYFRAMES: Overridden = 1 << 8;
const SET_COMPOSITE: Overridden = 1 << 9;

/// A keyframe effect (section 5.3).
#[derive(Debug, Clone)]
pub(crate) struct Effect {
    /// Its effect target, and the pseudo-element it names apart from it:
    /// a script may name one before it gives the element.
    pub(crate) target: Option<Target>,
    pub(crate) pseudo: Option<PseudoElement>,
    /// The animation it is associated with.
    pub(crate) animation: Option<u64>,
    pub(crate) timing: Timing,
    /// Whether its duration is `auto` and its fill `auto`, as `getTiming`
    /// gives them; the timing holds what they compute to.
    pub(crate) duration_auto: bool,
    pub(crate) fill_auto: bool,
    /// What a script set of a CSS animation's effect.
    overridden: Overridden,
    pub(crate) composite: Composite,
    pub(crate) frames: Frames,
    /// Whether a script holds an object for it.
    pub(crate) held: bool,
}

/// What a sample gives that a style is computed from.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sampled {
    phase: Option<Phase>,
    progress: Option<f64>,
    before: bool,
}

impl Sampled {
    const NOTHING: Sampled = Sampled {
        phase: None,
        progress: None,
        before: false,
    };
}

impl From<Option<Progress>> for Sampled {
    fn from(progress: Option<Progress>) -> Self {
        match progress {
            Some(progress) => Sampled {
                phase: Some(progress.phase),
                progress: progress.progress,
                before: progress.before,
            },
            None => Sampled::NOTHING,
        }
    }
}

/// An animation an element's style asks for (Animations 1 section 2):
/// an item of its `animation-name` that names keyframes in force, with
/// the other properties' items at its place.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Wanted {
    pub(crate) name: Arc<str>,
    pub(crate) timing: Timing,
    pub(crate) easing: Easing,
    pub(crate) paused: bool,
    pub(crate) composite: Composite,
    /// The longhands its keyframes animate.
    pub(crate) longhands: Arc<[crate::animate::Longhand]>,
    /// Whether its duration is `auto`, the timing's zero.
    pub(crate) duration_auto: bool,
    /// Its `animation-timeline` and `animation-range`.
    pub(crate) timeline: crate::computed::Timeline<LengthPercentage>,
    pub(crate) range: [Edge<LengthPercentage>; 2],
}

/// The keyframes a sample reads.
#[derive(Debug, Clone)]
pub(crate) enum SampleFrames {
    /// The `@keyframes` rule of the name.
    Css(Arc<str>),
    /// A script's.
    Script(Arc<ScriptKeyframes>),
}

/// One of a target's animation effects at the timeline's time: the
/// keyframes it reads, its default keyframe easing, its composite
/// operation and what its effect gives.
#[derive(Debug, Clone)]
pub(crate) struct Sample {
    pub(crate) frames: SampleFrames,
    pub(crate) easing: Easing,
    pub(crate) composite: Composite,
    pub(crate) progress: Progress,
    /// Whether the effect is current (section 4.5.2), which makes the
    /// engine act as if `will-change` named what it animates.
    pub(crate) current: bool,
}

/// The animations a document runs, and its timelines (M22.1 to M22.3,
/// D75).
#[derive(Debug, Clone)]
pub struct Animations {
    /// The document timeline's time, in milliseconds from its origin.
    time: f64,
    /// The timelines: the document's own first, at the origin zero, then
    /// the document timelines scripts made and the progress-based ones
    /// scripts and styles made, each by its place.
    timelines: Vec<Timeline>,
    /// The number the next animation or effect takes.
    next: u64,
    /// Every animation by its number, which is its place in the global
    /// animation list.
    animations: BTreeMap<u64, Animation>,
    /// Every keyframe effect by its number.
    effects: HashMap<u64, Effect>,
    /// Each element's CSS animations, in its `animation-name`'s order.
    css: HashMap<Target, Vec<u64>>,
    /// Each element's running transitions, one for a property.
    transitions: HashMap<Target, Vec<u64>>,
    /// Each element's completed transitions (Transitions 1 section 3).
    completed: HashMap<Target, Vec<Completed>>,
    /// Each target's effects, by their numbers.
    targeted: HashMap<Target, Vec<u64>>,
    /// The longhands each target's animations gave its last style: what
    /// a change of its style starts no transition for, as in the
    /// reference browser, since an animation's values hold them.
    styled: HashMap<Target, Vec<crate::animate::Longhand>>,
    /// The CSS events queued since they were last taken (M22.2).
    events: Vec<AnimationEvent>,
    /// The playback events (`finish`, `cancel`, `remove`) queued for the
    /// next animation frame to dispatch.
    playback: Vec<PlaybackEvent>,
    /// What the procedures owe the promises scripts hold, by animation.
    notices: Vec<(u64, Notice)>,
    /// The elements a script's call changed the animated values of, for
    /// the next restyle, and how many times one has been.
    touched: Vec<NodeId>,
    touches: u64,
    /// The animation whose effect stack `commitStyles()` reads (section
    /// 6.4): the effects above it in composite order give nothing while
    /// it is set.
    committing: Option<u64>,
    /// How many changes of style have looked for transitions to start.
    transition_generation: u64,
    /// Whether the store is lent to a script's run, whose style flushes
    /// the frame's styles do not see.
    in_script: bool,
    /// Whether the user asked for reduced motion (D76).
    reduced_motion: bool,
}

impl Default for Animations {
    /// None, at the time zero, the document's timeline the only one.
    fn default() -> Self {
        Animations::new()
    }
}

impl Animations {
    /// None, at the time zero.
    pub fn new() -> Self {
        Animations {
            time: 0.0,
            timelines: vec![Timeline::new(Kind::Document { origin: 0.0 })],
            next: 0,
            animations: BTreeMap::new(),
            effects: HashMap::new(),
            css: HashMap::new(),
            transitions: HashMap::new(),
            completed: HashMap::new(),
            targeted: HashMap::new(),
            styled: HashMap::new(),
            events: Vec::new(),
            playback: Vec::new(),
            notices: Vec::new(),
            touched: Vec::new(),
            touches: 0,
            committing: None,
            transition_generation: 0,
            in_script: false,
            reduced_motion: false,
        }
    }

    /// The timeline's time, in seconds.
    pub fn time(&self) -> f64 {
        self.time / 1000.0
    }

    /// The timeline's time, in milliseconds.
    pub fn time_ms(&self) -> f64 {
        self.time
    }

    /// Moves the timeline to `time`, in seconds: never back, and never
    /// to a time that is not a number.
    pub fn set_time(&mut self, time: f64) {
        let time = time * 1000.0;
        if time > self.time && time.is_finite() {
            self.time = time;
        }
    }

    /// How many animations there are.
    pub fn len(&self) -> usize {
        self.animations.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.animations.is_empty()
    }

    /// A new number.
    fn number(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// The current time of timeline `timeline`, `None` for none or an
    /// inactive one.
    fn timeline_time(&self, timeline: Option<usize>) -> Option<f64> {
        self.timelines.get(timeline?)?.current_time(self.time)
    }

    /// Whether timeline `timeline` is progress-based.
    fn progress_based(&self, timeline: Option<usize>) -> bool {
        timeline
            .and_then(|t| self.timelines.get(t))
            .is_some_and(|t| t.kind.progress_based())
    }

    /// The range of its progress-based timeline `animation` is attached
    /// to, as percentages of the timeline's (Web Animations 2 section
    /// 4.4.1.1): `None` on a document timeline, or none; the whole of it
    /// while the timeline is inactive.
    fn attachment(&self, animation: &Animation) -> Option<(f64, f64)> {
        let timeline = self.timelines.get(animation.timeline?)?;
        if !timeline.kind.progress_based() {
            return None;
        }
        Some(match &timeline.measured {
            Some(measured) => (
                measured.edge_percent(&animation.range[0], false),
                measured.edge_percent(&animation.range[1], true),
            ),
            // Unmeasured, a percentage of the whole is what it says.
            None => {
                let plain = |edge: &Edge<LengthPercentage>, default: f64| match edge {
                    Edge::At(None, LengthPercentage::Percent(p)) => f64::from(*p),
                    _ => default,
                };
                (
                    plain(&animation.range[0], 0.0),
                    plain(&animation.range[1], 100.0),
                )
            }
        })
    }

    /// The timing `effect` plays by in `animation`: as it is given, or on
    /// a progress-based timeline converted to proportions of the range
    /// the animation is attached to (Web Animations 2 section 4.5.3): an
    /// `auto` duration fills the range, its iterations sharing it, and
    /// times given keep their proportions, scaled to fill it.
    fn timing_in(&self, animation: Option<&Animation>, effect: &Effect) -> Timing {
        let Some((start, end)) = animation.and_then(|a| self.attachment(a)) else {
            return effect.timing.clone();
        };
        let length = (end - start).max(0.0);
        let mut timing = effect.timing.clone();
        if effect.duration_auto {
            timing.delay = 0.0;
            timing.end_delay = 0.0;
            timing.duration = if timing.iterations.is_finite() && timing.iterations > 0.0 {
                length / timing.iterations
            } else {
                0.0
            };
            return timing;
        }
        let total = timing.delay + timing.active_duration() + timing.end_delay;
        if total.is_finite() && total > 0.0 {
            let scale = length / total;
            timing.delay *= scale;
            timing.end_delay *= scale;
            timing.duration *= scale;
        } else {
            timing.delay = 0.0;
            timing.end_delay = 0.0;
            timing.duration = 0.0;
        }
        timing
    }

    /// What `animation`'s procedures need: its timeline's time, its
    /// effect's end, and the range of a progress-based timeline it is
    /// attached to.
    fn context(&self, animation: &Animation) -> Context {
        let end = animation
            .effect
            .and_then(|e| self.effects.get(&e))
            .map_or(0.0, |e| self.timing_in(Some(animation), e).end_time());
        Context {
            timeline: self.timeline_time(animation.timeline),
            end,
            range: self.attachment(animation),
        }
    }

    /// `animation`'s effect's progress at its current time, `None` when
    /// it has no effect or no current time.
    fn progress_of(&self, animation: &Animation) -> Option<Progress> {
        let effect = self.effects.get(&animation.effect?)?;
        let local = animation.player.current_time(&self.context(animation))?;
        Some(
            self.timing_in(Some(animation), effect)
                .progress_in(local, self.backwards(animation)),
        )
    }

    /// Whether `animation`'s phases are those of playing backwards, whose
    /// active interval takes its end and not its start: when it does,
    /// and on a progress-based timeline when it plays forwards at the
    /// timeline's end, where the effect's end is in it (Web Animations 2
    /// section 4.5.5, "at progress timeline boundary"), or backwards at
    /// its start.
    fn backwards(&self, animation: &Animation) -> bool {
        let backwards = animation.player.playback_rate < 0.0;
        if !self.progress_based(animation.timeline) {
            return backwards;
        }
        match self.timeline_time(animation.timeline) {
            Some(time) if !backwards && time >= 100.0 => true,
            Some(time) if backwards && time <= 0.0 => false,
            _ => backwards,
        }
    }

    /// Whether `animation`'s effect is current (section 4.5.2): in play,
    /// or yet to play in the direction it plays.
    fn is_current(&self, animation: &Animation) -> bool {
        let Some(progress) = self.progress_of(animation) else {
            // One waiting for its progress-based timeline to become active
            // is yet to play, as the reference browser has it.
            return self.progress_based(animation.timeline)
                && animation.player.play_state(&self.context(animation)) != PlayState::Idle;
        };
        let cx = self.context(animation);
        let rate = animation.player.playback_rate;
        (progress.phase == Phase::Active && animation.player.play_state(&cx) != PlayState::Finished)
            || (rate > 0.0 && progress.phase == Phase::Before)
            || (rate < 0.0 && progress.phase == Phase::After)
    }

    /// Whether `animation` is relevant (section 4.5.2): its effect
    /// current or in effect, and it not removed.
    fn is_relevant(&self, animation: &Animation) -> bool {
        animation.replace != ReplaceState::Removed
            && (self.is_current(animation)
                || self
                    .progress_of(animation)
                    .is_some_and(|p| p.active_time.is_some()))
    }

    /// Keeps what a procedure on animation `id` owes its promises: for
    /// the script that holds it, or carried out at once when none does,
    /// the finish notification steps among them.
    fn owe(&mut self, id: u64, notices: Vec<Notice>) {
        if notices.is_empty() {
            return;
        }
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        if animation.held {
            self.notices
                .extend(notices.into_iter().map(|notice| (id, notice)));
            return;
        }
        if notices
            .iter()
            .any(|n| matches!(n, Notice::FinishNotify { .. }))
        {
            let cx = self.context(animation);
            if let Some(animation) = self.animations.get_mut(&id) {
                animation.player.notify_finished(&cx);
            }
        }
    }

    /// Lists effect `effect` under its target.
    fn index_target(&mut self, effect: u64) {
        if let Some(target) = self.effects.get(&effect).and_then(|e| e.target) {
            let list = self.targeted.entry(target).or_default();
            if !list.contains(&effect) {
                list.push(effect);
            }
        }
    }

    /// Takes effect `effect` off its target's list.
    fn unindex_target(&mut self, effect: u64) {
        if let Some(target) = self.effects.get(&effect).and_then(|e| e.target)
            && let Some(list) = self.targeted.get_mut(&target)
        {
            list.retain(|&e| e != effect);
            if list.is_empty() {
                self.targeted.remove(&target);
            }
        }
    }

    /// Records that `effect`'s target's animated values may have changed.
    fn touch_effect(&mut self, effect: Option<u64>) {
        if let Some(target) = effect
            .and_then(|e| self.effects.get(&e))
            .and_then(|e| e.target)
        {
            self.touches += 1;
            if !self.touched.contains(&target.0) {
                self.touched.push(target.0);
            }
        }
    }

    /// Forgets animation `id` and its effect, unless a script holds the
    /// effect.
    fn forget(&mut self, id: u64) {
        let Some(animation) = self.animations.remove(&id) else {
            return;
        };
        if let Some(effect) = animation.effect {
            let held = self.effects.get(&effect).is_some_and(|e| e.held);
            if held {
                if let Some(e) = self.effects.get_mut(&effect) {
                    e.animation = None;
                }
            } else {
                self.unindex_target(effect);
                self.effects.remove(&effect);
            }
        }
    }

    /// Runs the pending tasks of every animation waiting for a frame,
    /// each ready at its timeline's time, the frame's: its current time
    /// goes on from what it showed.
    pub fn start_pending(&mut self) {
        let waiting: Vec<u64> = self
            .animations
            .iter()
            .filter(|(_, a)| a.player.pending())
            .map(|(&id, _)| id)
            .collect();
        for id in waiting {
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            let cx = self.context(animation);
            // An animation on no timeline, or an inactive one, waits.
            let Some(ready) = cx.timeline else {
                continue;
            };
            let mut notices = Vec::new();
            if let Some(animation) = self.animations.get_mut(&id) {
                animation.player.ready(ready, &cx, &mut notices);
            }
            self.owe(id, notices);
        }
    }

    /// The store is lent to a script's run, or given back from one: the
    /// animations its style flushes make are the frame's to see.
    pub fn set_in_script(&mut self, in_script: bool) {
        self.in_script = in_script;
    }

    /// The frame's styles have seen every animation there is.
    pub fn styles_seen(&mut self) {
        for animation in self.animations.values_mut() {
            animation.unseen = false;
        }
    }

    /// The animations' part of a frame's update of the animations (Web
    /// Animations 1 section 4.2, HTML's "update animations and send
    /// events"): the pending tasks of those ready run at the timeline's
    /// time, and every finished state updated, before the frame's
    /// callbacks run.
    pub fn update_timelines(&mut self) {
        self.align_start_times();
        self.start_pending();
        if self.reduced_motion {
            self.finish_playing();
        }
        self.update_finished();
    }

    /// Whether the user asked for reduced motion (D76, G30): from now on
    /// each frame finishes the animations and transitions playing.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    /// Finishes every animation playing forwards on a document timeline
    /// to an end it has (D76): it shows its end at once, its events
    /// still sent as its phase changes. One that never ends plays on.
    fn finish_playing(&mut self) {
        let ids: Vec<u64> = self.animations.keys().copied().collect();
        for id in ids {
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            let cx = self.context(animation);
            if self.progress_based(animation.timeline)
                || animation.player.pending()
                || animation.player.playback_rate <= 0.0
                || !cx.end.is_finite()
                || animation.player.play_state(&cx) != PlayState::Running
            {
                continue;
            }
            let mut notices = Vec::new();
            if let Some(animation) = self.animations.get_mut(&id) {
                let _ = animation.player.finish(&cx, &mut notices);
            }
            self.owe(id, notices);
            let effect = self.animations.get(&id).and_then(|a| a.effect);
            self.touch_effect(effect);
        }
    }

    /// Aligns the start time of each playing animation whose start time
    /// follows its progress-based timeline's range (Web Animations 2
    /// section 4.4.11.1, "auto-align start time"), the range having moved
    /// with the timeline's measure.
    fn align_start_times(&mut self) {
        let ids: Vec<u64> = self
            .animations
            .iter()
            .filter(|(_, a)| a.player.auto_align)
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            let cx = self.context(animation);
            if let Some(animation) = self.animations.get_mut(&id) {
                animation.player.align(&cx);
            }
        }
    }

    /// The timelines that follow a scroll, by their places, with what
    /// each follows: those the engine measures at each frame.
    pub fn progress_timelines(&self) -> Vec<(usize, TimelineKind)> {
        self.timelines
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind.progress_based())
            .map(|(i, t)| (i, t.kind.clone()))
            .collect()
    }

    /// What the engine measured of progress-based timeline `timeline` at
    /// the frame's start, `None` for an inactive one; whether it changed.
    pub fn set_measured(&mut self, timeline: usize, measured: Option<Measured>) -> bool {
        match self.timelines.get_mut(timeline) {
            Some(t) if t.measured != measured => {
                t.measured = measured;
                true
            }
            _ => false,
        }
    }

    /// What the engine last measured of timeline `timeline`.
    pub fn measured(&self, timeline: usize) -> Option<&Measured> {
        self.timelines.get(timeline)?.measured.as_ref()
    }

    /// What timeline `timeline` follows.
    pub fn timeline_kind(&self, timeline: usize) -> Option<&TimelineKind> {
        self.timelines.get(timeline).map(|t| &t.kind)
    }

    /// The place of a timeline of `kind`, made if there is none of it
    /// yet; the document's past [`MAX_TIMELINES`].
    pub fn timeline_of(&mut self, kind: TimelineKind) -> usize {
        if let Some(i) = self.timelines.iter().position(|t| t.kind == kind) {
            return i;
        }
        if self.timelines.len() > MAX_TIMELINES {
            return 0;
        }
        self.timelines.push(Timeline::new(kind));
        self.timelines.len() - 1
    }

    /// The timeline a CSS animation of the element `node` runs on, by
    /// its `animation-timeline` (Scroll-driven Animations 1 section 4.1):
    /// the document's, none, or a progress-based one of the element.
    fn css_timeline(
        &mut self,
        node: NodeId,
        value: &crate::computed::Timeline<LengthPercentage>,
    ) -> Option<usize> {
        use crate::computed::{Scroller, Timeline as Value};
        let kind = match value {
            Value::Auto => return Some(0),
            Value::None => return None,
            Value::Named(name) => Kind::Named {
                name: Arc::clone(name),
                from: node,
            },
            Value::Scroll { scroller, axis } => Kind::Scroll {
                source: match scroller {
                    Scroller::Nearest => TimelineSource::Nearest(node),
                    Scroller::Root => TimelineSource::Root,
                    Scroller::Own => TimelineSource::Own(node),
                },
                axis: *axis,
            },
            Value::View { axis, inset } => Kind::View {
                subject: Some(node),
                axis: *axis,
                inset: inset.clone(),
            },
        };
        Some(self.timeline_of(kind))
    }

    /// Updates every animation's finished state at the timeline's time
    /// (section 4.4.15), as moving the timeline does.
    fn update_finished(&mut self) {
        let ids: Vec<u64> = self.animations.keys().copied().collect();
        for id in ids {
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            let cx = self.context(animation);
            let mut notices = Vec::new();
            if let Some(animation) = self.animations.get_mut(&id) {
                animation
                    .player
                    .update_finished_state(false, false, &cx, &mut notices);
            }
            self.owe(id, notices);
        }
    }

    /// Samples every animation at the timeline's time: the elements whose
    /// animations or transitions give something other than at the last
    /// sample, a transition that ended among them, which a frame
    /// restyles, each once, in order.
    ///
    /// It is an animation frame: the finished states are updated, and
    /// each animation not waiting for one queues the events its change of
    /// phase since the last asks for (Animations 2 section 5.1,
    /// Transitions 2 section 5.1), the frame's by element, a transition's
    /// before an animation's as their composite order has them, each
    /// one's in the order of its table. The scripts' animations no script
    /// holds that are no longer relevant are forgotten.
    pub fn sample(&mut self) -> Vec<NodeId> {
        self.update_finished();
        let queued = self.events.len();
        self.report_transitions();
        let mut changed: Vec<NodeId> = self
            .end_transitions()
            .into_iter()
            .map(|(node, _)| node)
            .collect();
        changed.append(&mut self.touched);
        let mut events = Vec::new();
        let ids: Vec<u64> = self.animations.keys().copied().collect();
        let mut forgotten = Vec::new();
        for id in ids {
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            let target = animation
                .effect
                .and_then(|e| self.effects.get(&e))
                .and_then(|e| e.target);
            let mut progress = self.progress_of(animation);
            let mut timing = animation
                .effect
                .and_then(|e| self.effects.get(&e))
                .map(|e| e.timing.clone());
            // A CSS animation a script took the effect of plays an effect
            // of no duration, past whose end it is (Animations 2 section
            // 4.2): its events say it ended.
            if timing.is_none() && matches!(animation.class, Class::Css { .. }) {
                let none = Timing::default();
                let local = animation.player.current_time(&self.context(animation));
                progress = local.map(|local| none.progress(local));
                timing = Some(none);
            }
            let gone =
                animation.class == Class::Script && !animation.held && !self.is_relevant(animation);
            let owner = match &animation.class {
                Class::Css {
                    owner: Some(owner),
                    name,
                    ..
                } => Some((*owner, Arc::clone(name))),
                _ => None,
            };
            let pending = animation.player.pending();
            let time = self.time;
            let schedule =
                |delay: f64, elapsed: f64| scheduled(&animation.player, time, delay, elapsed);
            let mut queued = Vec::new();
            if let (Some((owner, name)), Some(timing)) = (&owner, &timing)
                && !pending
            {
                let reported = progress.map_or(Reported::Idle, |p| Reported::from(p.phase));
                let iteration = progress.and_then(|p| p.iteration);
                for (kind, elapsed) in animation_events(
                    timing,
                    animation.reported,
                    reported,
                    animation.reported_iteration,
                    iteration,
                ) {
                    queued.push(event(
                        (*owner, id),
                        name,
                        kind,
                        elapsed,
                        schedule(timing.delay, elapsed),
                    ));
                }
            }
            events.append(&mut queued);
            let Some(animation) = self.animations.get_mut(&id) else {
                continue;
            };
            if owner.is_some() && timing.is_some() && !pending {
                animation.reported = progress.map_or(Reported::Idle, |p| Reported::from(p.phase));
                animation.reported_iteration = progress.and_then(|p| p.iteration);
            }
            let now = Sampled::from(progress);
            if now != animation.last {
                animation.last = now;
                if let Some(target) = target {
                    changed.push(target.0);
                }
            }
            if gone {
                forgotten.push(id);
            }
        }
        for id in forgotten {
            if let Some(target) = self
                .animations
                .get(&id)
                .and_then(|a| a.effect)
                .and_then(|e| self.effects.get(&e))
                .and_then(|e| e.target)
            {
                changed.push(target.0);
            }
            self.forget(id);
        }
        self.events.extend(events);
        // Sorted by their scheduled times, then by element, an element's
        // own before its `::marker`'s, `::before`'s and `::after`'s, a
        // transition's before an animation's as their composite order has
        // them; by a key that borrows the name rather than clones it.
        fn order(e: &AnimationEvent) -> (f64, NodeId, u8, bool, &str) {
            let pseudo = match e.pseudo {
                None => 0,
                Some(PseudoElement::Marker) => 1,
                Some(PseudoElement::Before) => 2,
                Some(PseudoElement::After) => 3,
                Some(_) => 4,
            };
            let animation = !matches!(
                e.kind,
                EventKind::TransitionRun
                    | EventKind::TransitionStart
                    | EventKind::TransitionEnd
                    | EventKind::TransitionCancel
            );
            (e.scheduled, e.node, pseudo, animation, &*e.name)
        }
        self.events[queued..].sort_by(|a, b| {
            let (x, y) = (order(a), order(b));
            x.0.total_cmp(&y.0)
                .then_with(|| (x.1, x.2, x.3, x.4).cmp(&(y.1, y.2, y.3, y.4)))
        });
        changed.sort_unstable();
        changed.dedup();
        changed
    }

    /// Whether a frame is wanted now: an animation waits for one to
    /// start it, or plays in its active phase, its values changing with
    /// the time; or what a script holds waits for the frame's update.
    pub fn needs_frame(&self) -> bool {
        if !self.notices.is_empty() || !self.playback.is_empty() || !self.touched.is_empty() {
            return true;
        }
        self.animations.values().any(|animation| {
            if animation.player.pending() {
                return self.timeline_time(animation.timeline).is_some();
            }
            let cx = self.context(animation);
            // One on a progress-based timeline changes as it scrolls, not
            // as time goes.
            !self.progress_based(animation.timeline)
                && animation.player.play_state(&cx) == PlayState::Running
                && animation.player.playback_rate != 0.0
                && self
                    .progress_of(animation)
                    .is_some_and(|p| p.phase == Phase::Active)
        })
    }

    /// The timeline's time, in seconds, at which a frame is next wanted
    /// when none is now: the soonest end of a playing animation's delay,
    /// or of an effect's active interval past which its finished state
    /// changes; `None` when there is none to wait for.
    pub fn next_change(&self) -> Option<f64> {
        let mut soonest: Option<f64> = None;
        for animation in self.animations.values() {
            if self.progress_based(animation.timeline) {
                continue;
            }
            let cx = self.context(animation);
            let (Some(start), Some(timeline)) = (animation.player.start_time, cx.timeline) else {
                continue;
            };
            if animation.player.hold_time.is_some()
                || animation.player.play_state(&cx) != PlayState::Running
            {
                continue;
            }
            let rate = animation.player.playback_rate;
            let Some(effect) = animation.effect.and_then(|e| self.effects.get(&e)) else {
                continue;
            };
            if rate <= 0.0 {
                continue;
            }
            let timing = &effect.timing;
            let end = timing.end_time();
            let local = (timeline - start) * rate;
            // One in its active phase wants every frame already.
            if timing.phase(local) == Phase::Active {
                continue;
            }
            // The next boundary ahead: the end of the delay, or the end.
            let boundary = [timing.delay.min(end).max(0.0), end]
                .into_iter()
                .find(|&b| b > local);
            if let Some(boundary) = boundary {
                let at = start + boundary / rate;
                let origin = match animation.timeline.and_then(|t| self.timelines.get(t)) {
                    Some(Timeline {
                        kind: Kind::Document { origin },
                        ..
                    }) => *origin,
                    _ => 0.0,
                };
                let at = (at + origin) / 1000.0;
                soonest = Some(soonest.map_or(at, |s: f64| s.min(at)));
            }
        }
        soonest
    }

    /// Keeps the animations of the elements `keep` says, cancelling the
    /// CSS animations and transitions of the others: those of elements no
    /// longer in the document. A script's animation of such an element
    /// stays, its effect having nothing to apply to.
    pub fn retain(&mut self, mut keep: impl FnMut(NodeId) -> bool) {
        let gone: Vec<Target> = self
            .css
            .keys()
            .chain(self.transitions.keys())
            .filter(|target| !keep(target.0))
            .copied()
            .collect();
        for target in gone {
            if let Some(ids) = take(&mut self.css, &target) {
                for id in ids {
                    self.drop_css(id, target);
                }
            }
            self.cancel_transitions(target);
        }
        self.completed.retain(|&(node, _), _| keep(node));
        self.styled.retain(|&(node, _), _| keep(node));
    }

    /// Cancels CSS animation `id`, which its owner `target` no longer
    /// names: its cancel event queued, and it forgotten unless a script
    /// holds it, which then holds an idle animation of no owner.
    fn drop_css(&mut self, id: u64, target: Target) {
        let time = self.time;
        if let Some(animation) = self.animations.get(&id)
            && let Class::Css { name, .. } = &animation.class
        {
            let name = Arc::clone(name);
            let local = animation.player.current_time(&self.context(animation));
            let timing = animation
                .effect
                .and_then(|e| self.effects.get(&e))
                .map(|e| e.timing.clone());
            if let (Some(timing), Some(local)) = (timing, local)
                && let Some(elapsed) = cancelled(&timing, animation.reported, local)
            {
                self.events.push(event(
                    (target, id),
                    &name,
                    EventKind::AnimationCancel,
                    elapsed,
                    self.time,
                ));
            }
        }
        let _ = time;
        self.cancel_animation(id);
        if let Some(animation) = self.animations.get_mut(&id) {
            animation.reported = Reported::Idle;
            animation.reported_iteration = None;
            if let Class::Css { owner, .. } = &mut animation.class {
                *owner = None;
            }
            if !animation.held {
                self.forget(id);
            }
        }
    }

    /// Cancels animation `id` (section 4.4.16), its `cancel` event due if
    /// it was not idle.
    fn cancel_animation(&mut self, id: u64) {
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        let cx = self.context(animation);
        let effect = animation.effect;
        let mut notices = Vec::new();
        let due = self
            .animations
            .get_mut(&id)
            .is_some_and(|a| a.player.cancel(&cx, &mut notices));
        if due && self.animations.get(&id).is_some_and(|a| a.held) {
            self.playback.push(PlaybackEvent {
                animation: id,
                kind: PlaybackKind::Cancel,
                current_time: None,
                timeline_time: cx.timeline,
                scheduled: self.time,
                rank: self.event_rank(id),
            });
        }
        self.owe(id, notices);
        self.touch_effect(effect);
    }

    /// The longhands `target`'s animations gave its last style.
    pub(crate) fn animated_longhands(&self, target: Target) -> &[crate::animate::Longhand] {
        self.styled.get(&target).map_or(&[], Vec::as_slice)
    }

    /// Records the longhands `target`'s animations give its style now.
    pub(crate) fn set_animated_longhands(
        &mut self,
        target: Target,
        longhands: Vec<crate::animate::Longhand>,
    ) {
        if longhands.is_empty() {
            take(&mut self.styled, &target);
        } else {
            self.styled.insert(target, longhands);
        }
    }

    /// Whether `target` has a CSS animation, or an effect targets it.
    pub(crate) fn animates(&self, target: Target) -> bool {
        (!self.css.is_empty() && self.css.contains_key(&target))
            || (!self.targeted.is_empty() && self.targeted.contains_key(&target))
    }

    /// Matches `target`'s CSS animations to `wanted`, what its style asks
    /// for now, by Animations 1 section 2: the list is walked from its
    /// last item to its first, each taking the last animation of its
    /// name not taken yet, which keeps its playback and takes the item's
    /// timing and play state, but for what a script set of them; an item
    /// that finds none starts an animation, waiting for the next frame,
    /// or paused at zero when it is paused; an animation no item takes is
    /// cancelled.
    pub(crate) fn update(&mut self, target: Target, wanted: &[Wanted]) {
        let old = if wanted.is_empty() {
            match take(&mut self.css, &target) {
                Some(old) => old,
                None => return,
            }
        } else {
            take(&mut self.css, &target).unwrap_or_default()
        };
        let mut taken = vec![false; old.len()];
        let mut new = Vec::with_capacity(wanted.len());
        for item in wanted.iter().rev() {
            let found = (0..old.len()).rev().find(|&i| {
                !taken[i]
                    && self.animations.get(&old[i]).is_some_and(
                        |a| matches!(&a.class, Class::Css { name, .. } if *name == item.name),
                    )
            });
            let id = match found {
                Some(i) => {
                    taken[i] = true;
                    old[i]
                }
                None if self.animations.len() < MAX_ANIMATIONS => self.start_css(target, item),
                None => continue,
            };
            self.restyle_css(id, item);
            // Sampled at the timeline's time, which its style shows: the
            // frame's styles, which a script's flush does not change, so
            // that the next frame restyles what changed since them.
            let sampled = self.animations.get(&id).map(|a| self.progress_of(a).into());
            if !self.in_script
                && let (Some(animation), Some(sampled)) = (self.animations.get_mut(&id), sampled)
            {
                animation.last = sampled;
            }
            new.push(id);
        }
        for (&id, _) in old.iter().zip(&taken).filter(|(_, taken)| !**taken) {
            self.drop_css(id, target);
        }
        if !new.is_empty() {
            new.reverse();
            self.css.insert(target, new);
        }
    }

    /// A new CSS animation of `item` owned by `target`, played or paused
    /// as its style says: its number.
    fn start_css(&mut self, target: Target, item: &Wanted) -> u64 {
        let effect = self.number();
        self.effects.insert(
            effect,
            Effect {
                target: Some(target),
                pseudo: target.1,
                animation: None,
                timing: item.timing.clone(),
                duration_auto: item.duration_auto,
                fill_auto: false,
                overridden: 0,
                composite: item.composite,
                frames: Frames::Css {
                    name: Arc::clone(&item.name),
                    easing: item.easing.clone(),
                    longhands: Arc::clone(&item.longhands),
                },
                held: false,
            },
        );
        self.index_target(effect);
        let id = self.number();
        if let Some(e) = self.effects.get_mut(&effect) {
            e.animation = Some(id);
        }
        let timeline = self.css_timeline(target.0, &item.timeline);
        self.animations.insert(
            id,
            Animation {
                class: Class::Css {
                    owner: Some(target),
                    name: Arc::clone(&item.name),
                    scripted: false,
                    paused: item.paused,
                },
                player: Player::new(),
                effect: Some(effect),
                timeline,
                range: item.range.clone(),
                range_set: false,
                timeline_set: false,
                replace: ReplaceState::Active,
                last: Sampled::NOTHING,
                reported: Reported::Idle,
                reported_iteration: None,
                held: false,
                unseen: self.in_script,
            },
        );
        let Some(animation) = self.animations.get(&id) else {
            return id;
        };
        let cx = self.context(animation);
        let mut notices = Vec::new();
        if let Some(animation) = self.animations.get_mut(&id) {
            let _ = if item.paused {
                animation.player.pause(&cx, &mut notices)
            } else {
                animation.player.play(true, &cx, &mut notices)
            };
        }
        self.owe(id, notices);
        id
    }

    /// Gives CSS animation `id` what `item` asks of it now: the timing,
    /// keyframe easing and composite operation a script did not set, and
    /// a change of its play state, unless a script played or paused it.
    fn restyle_css(&mut self, id: u64, item: &Wanted) {
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        if let Some(effect) = animation.effect.and_then(|e| self.effects.get_mut(&e))
            && matches!(&effect.frames, Frames::Css { name, .. } if *name == item.name)
        {
            let set = effect.overridden;
            let timing = &mut effect.timing;
            let wanted = &item.timing;
            if set & SET_DELAY == 0 {
                timing.delay = wanted.delay;
            }
            if set & SET_DURATION == 0 {
                timing.duration = wanted.duration;
                effect.duration_auto = item.duration_auto;
            }
            if set & SET_ITERATIONS == 0 {
                timing.iterations = wanted.iterations;
            }
            if set & SET_DIRECTION == 0 {
                timing.direction = wanted.direction;
            }
            if set & SET_FILL == 0 {
                timing.fill = wanted.fill;
            }
            if set & SET_COMPOSITE == 0 {
                effect.composite = item.composite;
            }
            if set & SET_KEYFRAMES == 0
                && let Frames::Css {
                    easing, longhands, ..
                } = &mut effect.frames
            {
                *easing = item.easing.clone();
                *longhands = Arc::clone(&item.longhands);
            }
        }
        // Its timeline and its range, as its style names them now, the
        // range unless a script set it.
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        let (owner, range_set, timeline_set) = match &animation.class {
            Class::Css { owner, .. } => (*owner, animation.range_set, animation.timeline_set),
            _ => (None, true, true),
        };
        if let Some(owner) = owner {
            if !timeline_set {
                let timeline = self.css_timeline(owner.0, &item.timeline);
                let _ = self.set_timeline(id, timeline);
            }
            if !range_set && let Some(animation) = self.animations.get_mut(&id) {
                animation.range = item.range.clone();
            }
        }
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        let cx = self.context(animation);
        let Class::Css {
            scripted, paused, ..
        } = animation.class
        else {
            return;
        };
        // One a script cancelled stays idle whatever the property says.
        let idle = animation.player.play_state(&cx) == PlayState::Idle;
        let mut notices = Vec::new();
        if let Some(animation) = self.animations.get_mut(&id) {
            if !scripted && !idle && paused != item.paused {
                let _ = if item.paused {
                    animation.player.pause(&cx, &mut notices)
                } else {
                    animation.player.play(false, &cx, &mut notices)
                };
            }
            if let Class::Css { paused, .. } = &mut animation.class {
                *paused = item.paused;
            }
            // A change of its timing may finish it, or take it back.
            animation
                .player
                .update_finished_state(false, false, &cx, &mut notices);
        }
        self.owe(id, notices);
    }

    /// Finishes every transition running, as a script's `finish()` of
    /// each would (Web Animations 1 section 4.4.14): its current time put
    /// at its end, so that the next sample ends it, its element showing
    /// the value it was going to. Says whether there was one.
    pub fn finish_transitions(&mut self) -> bool {
        let time = self.time;
        let ids: Vec<u64> = self.transitions.values().flatten().copied().collect();
        for &id in &ids {
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            let end = self.context(animation).end;
            // The start that long before the time can round to a current
            // time short of the end; each step down an ulp lengthens it.
            let mut start = time - end;
            while time - start < end {
                start = start.next_down();
            }
            if let Some(animation) = self.animations.get_mut(&id) {
                let player = &mut animation.player;
                player.pending = None;
                player.hold_time = None;
                player.playback_rate = 1.0;
                player.pending_playback_rate = None;
                player.start_time = Some(start);
            }
        }
        !ids.is_empty()
    }

    /// The events queued since this was last asked, in the order they
    /// were queued, and none after.
    pub fn take_events(&mut self) -> Vec<AnimationEvent> {
        std::mem::take(&mut self.events)
    }

    /// The playback events queued for the scripts' animations, in the
    /// order they were queued, and none after.
    pub fn take_playback_events(&mut self) -> Vec<PlaybackEvent> {
        std::mem::take(&mut self.playback)
    }

    /// The elements a script's call changed the animated values of since
    /// the last frame, which a restyle takes, and a count that grows with
    /// each change, so that a restyle between frames takes them once.
    pub fn touched(&self) -> (&[NodeId], u64) {
        (&self.touched, self.touches)
    }

    /// Every element an animation, a transition or a script's effect
    /// acts on: what a flush between frames restyles to the timeline's
    /// time, the frame's values being of the frame's.
    pub fn targets(&self) -> Vec<NodeId> {
        let mut nodes: Vec<NodeId> = self
            .animations
            .values()
            .filter_map(|animation| match &animation.class {
                Class::Css {
                    owner: Some(owner), ..
                }
                | Class::Transition {
                    owner: Some(owner), ..
                } => Some(owner.0),
                _ => None,
            })
            .chain(
                self.effects
                    .values()
                    .filter_map(|effect| effect.target.map(|target| target.0)),
            )
            .collect();
        nodes.sort_unstable();
        nodes.dedup();
        nodes
    }

    /// Every CSS animation and transition running, sorted by element,
    /// kind and name: for the tests.
    pub fn running(&self) -> Vec<Running> {
        let mut running: Vec<Running> = Vec::new();
        for animation in self.animations.values() {
            let (owner, name, transition) = match &animation.class {
                Class::Css {
                    owner: Some(owner),
                    name,
                    ..
                } => (*owner, Arc::clone(name), false),
                Class::Transition {
                    owner: Some(owner),
                    property,
                    ..
                } => (*owner, property.name(), true),
                _ => continue,
            };
            let cx = self.context(animation);
            let current_time = animation.player.current_time(&cx).unwrap_or(0.0) / 1000.0;
            let phase = if animation.player.pending() {
                None
            } else {
                self.progress_of(animation).map(|p| p.phase)
            };
            let paused = animation.player.play_state(&cx) == PlayState::Paused;
            running.push(Running {
                node: owner.0,
                pseudo: owner.1,
                name,
                transition,
                phase,
                current_time,
                paused,
            });
        }
        running
            .sort_by(|a, b| (a.node, a.transition, &a.name).cmp(&(b.node, b.transition, &b.name)));
        running
    }

    /// The rank in composite order of animation `id`'s playback events:
    /// transitions, CSS animations, then the others, each kind in the
    /// order its animations were made.
    pub(crate) fn event_rank(&self, id: u64) -> (u8, u64) {
        let class = match self.animations.get(&id).map(|a| &a.class) {
            Some(Class::Transition { owner: Some(_), .. }) => 0,
            Some(Class::Css { owner: Some(_), .. }) => 1,
            _ => 2,
        };
        (class, id)
    }

    /// The rank of animation `id` in composite order among the effects of
    /// one target (section 5.4.2): transitions first, then CSS animations
    /// by their owners and their places in `animation-name`, then every
    /// other by its place in the global animation list.
    fn composite_rank(&self, id: u64) -> (u8, u64, u64) {
        let Some(animation) = self.animations.get(&id) else {
            return (3, 0, id);
        };
        match &animation.class {
            Class::Transition { owner: Some(_), .. } => (0, 0, id),
            Class::Css {
                owner: Some(owner), ..
            } => {
                let place = self
                    .css
                    .get(owner)
                    .and_then(|list| list.iter().position(|&a| a == id))
                    .unwrap_or(0);
                (1, owner.0.to_bits(), place as u64)
            }
            _ => (2, 0, id),
        }
    }

    /// What `target`'s keyframe effects give at the timeline's time, in
    /// composite order: every one but a transition's, whose values
    /// [`Animations::transitioning`] gives, and but those of animations
    /// removed by a later one.
    pub(crate) fn samples(&self, target: Target) -> Vec<Sample> {
        let Some(effects) = (!self.targeted.is_empty())
            .then(|| self.targeted.get(&target))
            .flatten()
        else {
            return Vec::new();
        };
        let mut found: Vec<((u8, u64, u64), Sample)> = Vec::new();
        for &e in effects {
            let Some(effect) = self.effects.get(&e) else {
                continue;
            };
            let Some(id) = effect.animation else {
                continue;
            };
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            if animation.replace == ReplaceState::Removed && self.committing != Some(id) {
                continue;
            }
            let (frames, easing) = match &effect.frames {
                Frames::Css { name, easing, .. } => {
                    (SampleFrames::Css(Arc::clone(name)), easing.clone())
                }
                Frames::Script(keyframes) => {
                    (SampleFrames::Script(Arc::clone(keyframes)), Easing::Linear)
                }
                Frames::Transition(_) => continue,
            };
            let progress = if self.committing == Some(id) {
                // The animation `commitStyles()` commits gives its value
                // as if it filled both ways (Web Animations 1 section 6.4).
                animation
                    .player
                    .current_time(&self.context(animation))
                    .map(|local| {
                        let both = Timing {
                            fill: AnimationFillMode::Both,
                            ..effect.timing.clone()
                        };
                        both.progress_in(local, animation.player.playback_rate < 0.0)
                    })
            } else {
                self.progress_of(animation)
            };
            let Some(progress) = progress else {
                continue;
            };
            if let Some(limit) = self.committing
                && self.composite_rank(id) > self.composite_rank(limit)
            {
                continue;
            }
            found.push((
                self.composite_rank(id),
                Sample {
                    frames,
                    easing,
                    composite: effect.composite,
                    progress,
                    current: self.is_current(animation),
                },
            ));
        }
        found.sort_by_key(|(rank, _)| *rank);
        found.into_iter().map(|(_, sample)| sample).collect()
    }

    /// The effect targets of every relevant animation and its owner's
    /// CSS ones: what [`Animations::relevant`] sorts.
    fn relevant_ids(&self) -> Vec<u64> {
        self.animations
            .iter()
            .filter(|(_, a)| self.is_relevant(a))
            .map(|(&id, _)| id)
            .collect()
    }
}

/// When on the timeline an event of an animation playing as `player`
/// does, the timeline's time being `time`: its effect's local time at the
/// start delay `delay` and the elapsed time `elapsed` reached at the
/// playback rate from the start time, no later than now; now for one not
/// playing.
fn scheduled(player: &Player, time: f64, delay: f64, elapsed: f64) -> f64 {
    match player.start_time {
        Some(start) if player.playback_rate != 0.0 && player.hold_time.is_none() => {
            let at = start + (delay + elapsed) / player.playback_rate;
            if at.is_finite() { at.min(time) } else { time }
        }
        _ => time,
    }
}

/// An event of `kind` from `target`'s animation or transition `name`,
/// its elapsed time in seconds from `elapsed` milliseconds, scheduled at
/// `scheduled` milliseconds.
fn event(
    (target, animation): (Target, u64),
    name: &Arc<str>,
    kind: EventKind,
    elapsed: f64,
    scheduled: f64,
) -> AnimationEvent {
    AnimationEvent {
        kind,
        node: target.0,
        pseudo: target.1,
        name: Arc::clone(name),
        elapsed: elapsed / 1000.0,
        animation,
        scheduled,
    }
}

#[cfg(test)]
mod tests;
