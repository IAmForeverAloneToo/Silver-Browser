//! What scripts do to a document's animations (Web Animations 1 section
//! 6, D75): the operations behind the bindings' `Animation`,
//! `KeyframeEffect`, `CSSAnimation`, `CSSTransition` and
//! `DocumentTimeline` objects, each of which is a view of an animation's
//! or an effect's number in the store. A call reads or changes the
//! store; what a change owes the promises the script holds is kept as
//! notices for the bindings to take, and the playback events it queues
//! wait for the next animation frame.
//!
//! An operation on a CSS animation that a script makes sticks: a play or
//! a pause leaves `animation-play-state` no say over it, and a timing
//! member, the keyframes or the composite operation set on its effect
//! leave the matching `animation-*` property none over that member
//! (Animations 2 section 4.2). One on a CSS transition or animation that
//! cancels it, or a new effect, ends its owner's hold on it, as
//! Transitions 2 section 6.1 and Animations 2 section 4.1 say.

use std::collections::HashMap;
use std::sync::Arc;

use dom::NodeId;

use super::events::Reported;
use super::player::{Notice, PlayState, Player, Refusal};
use super::{
    Animation, Animations, Class, Effect, Frames, MAX_ANIMATIONS, MAX_TIMELINES, Phase,
    PlaybackEvent, PlaybackKind, ReplaceState, SET_COMPOSITE, SET_DELAY, SET_DIRECTION,
    SET_DURATION, SET_EASING, SET_END_DELAY, SET_FILL, SET_ITERATION_START, SET_ITERATIONS,
    SET_KEYFRAMES, Sampled, Target, Timing, Value, cancelled, event,
};
use crate::animate::Longhand;
use crate::computed::{AnimationDirection, AnimationFillMode, Composite};
use crate::easing::Easing;
use crate::keyframes::{ScriptKeyframes, parse_easing};
use crate::selector_matching::PseudoElement;

/// An animation's place in composite order among a document's (section
/// 5.4.2): its kind, its owner's place in tree order and its
/// pseudo-element, its place among its kind's, and a transition's
/// property.
type CompositeKey = (u8, usize, u8, u64, Arc<str>);

/// A replaceable animation of a target (section 5.5): its number, and
/// the longhands and custom properties its effect animates.
type Replaceable = (u64, Vec<Longhand>, Vec<Arc<str>>);

/// Why an operation refused: the exception the script gets, with its
/// message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    /// A `TypeError`.
    Type(String),
    /// A `DOMException` named `InvalidStateError`.
    InvalidState(String),
    /// A `DOMException` named `SyntaxError`: a pseudo-element selector
    /// that names none an effect can target.
    Syntax(String),
}

impl From<Refusal> for ScriptError {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::InvalidState => {
                ScriptError::InvalidState("The animation's effect has an infinite end.".into())
            }
            Refusal::Type => ScriptError::Type("currentTime may not be changed to null.".into()),
        }
    }
}

/// The members of an `OptionalEffectTiming` a script gave, already
/// converted as Web IDL says: `None` for a member it left out. A duration
/// of `Some(None)` is `auto`; the fill and the direction are their
/// keywords.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimingInput {
    pub delay: Option<f64>,
    pub end_delay: Option<f64>,
    pub fill: Option<String>,
    pub iteration_start: Option<f64>,
    pub iterations: Option<f64>,
    pub duration: Option<Option<f64>>,
    pub direction: Option<String>,
    pub easing: Option<String>,
}

/// What a `KeyframeEffect` is made of: its target element and the
/// pseudo-element of it, which may be named without one.
#[derive(Debug, Clone)]
pub struct EffectInput {
    pub target: Option<NodeId>,
    pub pseudo: Option<PseudoElement>,
    pub keyframes: ScriptKeyframes,
    pub timing: TimingInput,
    pub composite: Composite,
}

/// What an effect's keyframes are, for `getKeyframes`: a script's, a CSS
/// animation's `@keyframes` rule's by name with the easing of a keyframe
/// that declares none, or a CSS transition's two values of its property.
#[derive(Debug, Clone)]
pub enum KeyframesSource {
    Script(Arc<ScriptKeyframes>),
    Css {
        name: Arc<str>,
        easing: Easing,
    },
    Transition {
        /// The property's name.
        property: Arc<str>,
        /// Its longhand; `None` for a custom property.
        longhand: Option<Longhand>,
        /// The start and end values: styles holding a longhand's, or a
        /// custom property's tokens.
        start: TransitionValue,
        end: TransitionValue,
    },
}

/// A transition's value, for `getKeyframes`.
#[derive(Debug, Clone)]
pub enum TransitionValue {
    Style(Arc<crate::ComputedStyle>),
    Custom(Option<Arc<[css::Token]>>),
}

impl From<&Value> for TransitionValue {
    fn from(value: &Value) -> Self {
        match value {
            Value::Style(style) => TransitionValue::Style(Arc::clone(style)),
            Value::Custom(tokens) => TransitionValue::Custom(tokens.clone()),
        }
    }
}

/// An effect's timing as `getComputedTiming` gives it (section 6.7.2),
/// times in milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedTiming {
    pub timing: Timing,
    pub end_time: f64,
    pub active_duration: f64,
    pub local_time: Option<f64>,
    pub progress: Option<f64>,
    pub current_iteration: Option<f64>,
}

/// What an animation is, as its object reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationState {
    pub play_state: PlayState,
    pub pending: bool,
    pub current_time: Option<f64>,
    pub start_time: Option<f64>,
    pub playback_rate: f64,
    pub replace_state: ReplaceState,
    /// Its timeline's place, `None` for none.
    pub timeline: Option<usize>,
    /// Its effect's number.
    pub effect: Option<u64>,
    /// Whether its ready promise waits, and its finished promise is
    /// resolved.
    pub ready_pending: bool,
    pub finished_resolved: bool,
    /// What kind it is: `script`, `css` or `transition`.
    pub kind: &'static str,
    /// A CSS animation's name or a transition's property.
    pub name: Option<Arc<str>>,
    /// Its owning element while it has one.
    pub owner: Option<Target>,
}

/// The pseudo-element a script names (Web Animations 1 section 6.6): a
/// tree-abiding or highlight pseudo-element the engine knows, ASCII
/// case-insensitively, Selectors 2's single colon taken for the four it
/// had; `Ok(None)` for none, a `SyntaxError` for any other.
pub fn pseudo_element(text: Option<&str>) -> Result<Option<PseudoElement>, ScriptError> {
    let Some(text) = text else {
        return Ok(None);
    };
    match text.to_ascii_lowercase().as_str() {
        "::before" | ":before" => Ok(Some(PseudoElement::Before)),
        "::after" | ":after" => Ok(Some(PseudoElement::After)),
        "::marker" => Ok(Some(PseudoElement::Marker)),
        "::first-line" | ":first-line" => Ok(Some(PseudoElement::FirstLine)),
        "::first-letter" | ":first-letter" => Ok(Some(PseudoElement::FirstLetter)),
        "::placeholder" => Ok(Some(PseudoElement::Placeholder)),
        "::selection" => Ok(Some(PseudoElement::Selection)),
        _ => Err(ScriptError::Syntax(text.to_owned())),
    }
}

/// A pseudo-element's selector as `pseudoElement` gives it.
pub fn pseudo_element_name(pseudo: PseudoElement) -> Option<&'static str> {
    match pseudo {
        PseudoElement::Before => Some("::before"),
        PseudoElement::After => Some("::after"),
        PseudoElement::Marker => Some("::marker"),
        PseudoElement::FirstLine => Some("::first-line"),
        PseudoElement::FirstLetter => Some("::first-letter"),
        PseudoElement::Placeholder => Some("::placeholder"),
        PseudoElement::Selection => Some("::selection"),
        PseudoElement::DetailsContent => Some("::details-content"),
        PseudoElement::FileSelectorButton => Some("::file-selector-button"),
        PseudoElement::Picker => Some("::picker(select)"),
        PseudoElement::PickerIcon => Some("::picker-icon"),
        PseudoElement::Checkmark => Some("::checkmark"),
        PseudoElement::Nested | PseudoElement::WebkitPrefixed => None,
    }
}

impl ScriptError {
    /// Its exception's name.
    pub fn name(&self) -> &'static str {
        match self {
            ScriptError::Type(_) => "TypeError",
            ScriptError::InvalidState(_) => "InvalidStateError",
            ScriptError::Syntax(_) => "SyntaxError",
        }
    }

    /// Its message.
    pub fn message(&self) -> String {
        match self {
            ScriptError::Type(message) | ScriptError::InvalidState(message) => message.clone(),
            ScriptError::Syntax(text) => format!("'{text}' is not a valid pseudo-element."),
        }
    }
}

/// `timing` updated by `input` (section 6.7.1, "update the timing
/// properties of an animation effect"): a `TypeError` for an iteration
/// start or count below zero, a duration below zero or not a number, or
/// an easing that does not parse, nothing being changed then. The
/// members set, as bits.
fn updated(
    timing: &mut Timing,
    duration_auto: &mut bool,
    fill_auto: &mut bool,
    input: &TimingInput,
) -> Result<u16, ScriptError> {
    if input
        .iteration_start
        .is_some_and(|v| v < 0.0 || !v.is_finite())
    {
        return Err(ScriptError::Type(
            "iterationStart must be non-negative.".into(),
        ));
    }
    if input.iterations.is_some_and(|v| v < 0.0 || v.is_nan()) {
        return Err(ScriptError::Type(
            "iterationCount must be non-negative.".into(),
        ));
    }
    if let Some(Some(duration)) = input.duration
        && (duration < 0.0 || duration.is_nan())
    {
        return Err(ScriptError::Type(
            "duration must be non-negative or auto.".into(),
        ));
    }
    let easing = match &input.easing {
        Some(text) => Some(parse_easing(text).ok_or_else(|| {
            ScriptError::Type(format!("'{text}' is not a valid value for easing"))
        })?),
        None => None,
    };
    let fill = match input.fill.as_deref() {
        None => None,
        Some("auto") => Some((AnimationFillMode::None, true)),
        Some("none") => Some((AnimationFillMode::None, false)),
        Some("forwards") => Some((AnimationFillMode::Forwards, false)),
        Some("backwards") => Some((AnimationFillMode::Backwards, false)),
        Some("both") => Some((AnimationFillMode::Both, false)),
        Some(other) => return Err(ScriptError::Type(format!("'{other}' is not a fill mode."))),
    };
    let direction = match input.direction.as_deref() {
        None => None,
        Some("normal") => Some(AnimationDirection::Normal),
        Some("reverse") => Some(AnimationDirection::Reverse),
        Some("alternate") => Some(AnimationDirection::Alternate),
        Some("alternate-reverse") => Some(AnimationDirection::AlternateReverse),
        Some(other) => {
            return Err(ScriptError::Type(format!(
                "'{other}' is not a playback direction."
            )));
        }
    };
    let mut set = 0;
    if let Some(delay) = input.delay {
        timing.delay = delay;
        set |= SET_DELAY;
    }
    if let Some(end_delay) = input.end_delay {
        timing.end_delay = end_delay;
        set |= SET_END_DELAY;
    }
    if let Some((fill, auto)) = fill {
        timing.fill = fill;
        *fill_auto = auto;
        set |= SET_FILL;
    }
    if let Some(start) = input.iteration_start {
        timing.iteration_start = start;
        set |= SET_ITERATION_START;
    }
    if let Some(iterations) = input.iterations {
        timing.iterations = iterations;
        set |= SET_ITERATIONS;
    }
    if let Some(duration) = input.duration {
        timing.duration = duration.unwrap_or(0.0);
        *duration_auto = duration.is_none();
        set |= SET_DURATION;
    }
    if let Some(direction) = direction {
        timing.direction = direction;
        set |= SET_DIRECTION;
    }
    if let Some(easing) = easing {
        timing.easing = easing;
        set |= SET_EASING;
    }
    Ok(set)
}

impl Animations {
    /// A new keyframe effect (section 6.6, the `KeyframeEffect`
    /// constructor): its number.
    pub fn create_effect(&mut self, input: EffectInput) -> Result<u64, ScriptError> {
        let mut timing = Timing::default();
        let (mut duration_auto, mut fill_auto) = (true, true);
        updated(
            &mut timing,
            &mut duration_auto,
            &mut fill_auto,
            &input.timing,
        )?;
        let id = self.number();
        self.effects.insert(
            id,
            Effect {
                target: input.target.map(|node| (node, input.pseudo)),
                pseudo: input.pseudo,
                animation: None,
                timing,
                duration_auto,
                fill_auto,
                overridden: 0,
                composite: input.composite,
                frames: Frames::Script(Arc::new(input.keyframes)),
                held: true,
            },
        );
        self.index_target(id);
        Ok(id)
    }

    /// A copy of effect `effect` (the `KeyframeEffect` constructor given
    /// an effect): its target, timing, composite operation and keyframes,
    /// no animation. A CSS animation's keyframes are copied as the
    /// keyframes `keyframes` reads them; `None` for no such effect.
    pub fn copy_effect(&mut self, effect: u64, keyframes: ScriptKeyframes) -> Option<u64> {
        let source = self.effects.get(&effect)?;
        let copy = Effect {
            target: source.target,
            pseudo: source.pseudo,
            animation: None,
            timing: source.timing.clone(),
            duration_auto: source.duration_auto,
            fill_auto: source.fill_auto,
            overridden: 0,
            composite: source.composite,
            frames: match &source.frames {
                Frames::Script(frames) => Frames::Script(Arc::clone(frames)),
                _ => Frames::Script(Arc::new(keyframes)),
            },
            held: true,
        };
        let id = self.number();
        self.effects.insert(id, copy);
        self.index_target(id);
        Some(id)
    }

    /// A new animation (section 6.4, the `Animation` constructor) of
    /// effect `effect` on timeline `timeline`: its number, which a
    /// script holds. `None` past [`MAX_ANIMATIONS`].
    pub fn create_animation(
        &mut self,
        effect: Option<u64>,
        timeline: Option<usize>,
    ) -> Option<u64> {
        if self.animations.len() >= MAX_ANIMATIONS {
            return None;
        }
        let id = self.number();
        self.animations.insert(
            id,
            Animation {
                class: Class::Script,
                player: Player::new(),
                effect: None,
                timeline: timeline.filter(|&t| t < self.origins.len()),
                replace: ReplaceState::Active,
                last: Sampled::NOTHING,
                reported: Reported::Idle,
                reported_iteration: None,
                held: true,
                unseen: false,
            },
        );
        self.set_effect(id, effect);
        Some(id)
    }

    /// A new document timeline whose origin is `origin` milliseconds on
    /// the document's (the `DocumentTimeline` constructor): its place.
    pub fn create_timeline(&mut self, origin: f64) -> usize {
        if self.origins.len() > MAX_TIMELINES {
            return 0;
        }
        self.origins.push(origin);
        self.origins.len() - 1
    }

    /// The current time of timeline `timeline`, in milliseconds.
    pub fn timeline_current_time(&self, timeline: usize) -> Option<f64> {
        self.timeline_time(Some(timeline))
    }

    /// A script holds an object for animation `id`: from now on it hears
    /// its notices and its playback events. Says whether there is one.
    pub fn hold(&mut self, id: u64) -> bool {
        match self.animations.get_mut(&id) {
            Some(animation) => {
                animation.held = true;
                true
            }
            None => false,
        }
    }

    /// A script holds an object for effect `effect`.
    pub fn hold_effect(&mut self, effect: u64) -> bool {
        match self.effects.get_mut(&effect) {
            Some(effect) => {
                effect.held = true;
                true
            }
            None => false,
        }
    }

    /// The script let go of animation `id`'s object: it is forgotten
    /// once no longer relevant, as one no script holds is.
    pub fn release(&mut self, id: u64) {
        if let Some(animation) = self.animations.get_mut(&id) {
            animation.held = false;
            let gone = match &animation.class {
                Class::Script => !self.is_relevant(self.animations.get(&id).expect("there")),
                Class::Css { owner, .. } | Class::Transition { owner, .. } => owner.is_none(),
            };
            if gone {
                self.forget(id);
            }
        }
    }

    /// The script let go of effect `effect`'s object: it is forgotten
    /// with its animation, or now when it has none.
    pub fn release_effect(&mut self, effect: u64) {
        if let Some(e) = self.effects.get_mut(&effect) {
            e.held = false;
            if e.animation.is_none() {
                self.unindex_target(effect);
                self.effects.remove(&effect);
            }
        }
    }

    /// What the procedures owe the promises scripts hold, in order, and
    /// none after.
    pub fn take_notices(&mut self) -> Vec<(u64, Notice)> {
        std::mem::take(&mut self.notices)
    }

    /// Whether notices or playback events wait for the scripts.
    pub fn owes_scripts(&self) -> bool {
        !self.notices.is_empty() || !self.playback.is_empty()
    }

    /// What animation `id` is.
    pub fn state(&self, id: u64) -> Option<AnimationState> {
        let animation = self.animations.get(&id)?;
        let cx = self.context(animation);
        let (kind, name, owner) = match &animation.class {
            Class::Script => ("script", None, None),
            Class::Css { owner, name, .. } => ("css", Some(Arc::clone(name)), *owner),
            Class::Transition {
                owner, property, ..
            } => ("transition", Some(property.name()), *owner),
        };
        Some(AnimationState {
            play_state: animation.player.play_state(&cx),
            pending: animation.player.pending(),
            current_time: animation.player.current_time(&cx),
            start_time: animation.player.start_time,
            playback_rate: animation.player.playback_rate,
            replace_state: animation.replace,
            timeline: animation.timeline,
            effect: animation.effect,
            ready_pending: animation.player.ready_pending(),
            finished_resolved: animation.player.finished_resolved(),
            kind,
            name,
            owner,
        })
    }

    /// Runs `f` on animation `id`'s playback with its context, keeping
    /// what it owes, and records that its target's values may change.
    fn with_player<T>(
        &mut self,
        id: u64,
        f: impl FnOnce(&mut Player, &super::Context, &mut Vec<Notice>) -> Result<T, Refusal>,
    ) -> Result<T, ScriptError> {
        let animation = self
            .animations
            .get(&id)
            .ok_or_else(|| ScriptError::InvalidState("The animation is gone.".into()))?;
        let cx = self.context(animation);
        let effect = animation.effect;
        let mut notices = Vec::new();
        let result = {
            let animation = self.animations.get_mut(&id).expect("looked up above");
            f(&mut animation.player, &cx, &mut notices)
        };
        self.owe(id, notices);
        self.touch_effect(effect);
        result.map_err(ScriptError::from)
    }

    /// A script played, paused, reversed or set the start time of CSS
    /// animation `id`, the call not refused (`done`): its play state is
    /// the script's from now on (Animations 2 section 4.2).
    fn script_plays<T>(&mut self, id: u64, done: Result<T, ScriptError>) -> Result<T, ScriptError> {
        if done.is_ok()
            && let Some(Animation {
                class: Class::Css { scripted, .. },
                ..
            }) = self.animations.get_mut(&id)
        {
            *scripted = true;
        }
        done
    }

    /// `play()` (section 4.4.11, auto-rewinding).
    pub fn play(&mut self, id: u64) -> Result<(), ScriptError> {
        let done = self.with_player(id, |player, cx, notices| player.play(true, cx, notices));
        self.script_plays(id, done)
    }

    /// `pause()` (section 4.4.13).
    pub fn pause(&mut self, id: u64) -> Result<(), ScriptError> {
        let done = self.with_player(id, |player, cx, notices| player.pause(cx, notices));
        self.script_plays(id, done)
    }

    /// `finish()` (section 4.4.14).
    pub fn finish(&mut self, id: u64) -> Result<(), ScriptError> {
        self.with_player(id, |player, cx, notices| player.finish(cx, notices))
    }

    /// Whether animation `id` is paused, as `animation-play-state` reads
    /// it: paused or pausing.
    fn paused(&self, id: u64) -> bool {
        self.animations
            .get(&id)
            .is_some_and(|a| a.player.play_state(&self.context(a)) == PlayState::Paused)
    }

    /// `reverse()` (section 4.4.18): a CSS animation's play state is the
    /// script's if it played it.
    pub fn reverse(&mut self, id: u64) -> Result<(), ScriptError> {
        let paused = self.paused(id);
        let done = self.with_player(id, |player, cx, notices| player.reverse(cx, notices));
        if paused == self.paused(id) {
            return done;
        }
        self.script_plays(id, done)
    }

    /// `updatePlaybackRate()` (section 4.4.17.1).
    pub fn update_playback_rate(&mut self, id: u64, rate: f64) -> Result<(), ScriptError> {
        self.with_player(id, |player, cx, notices| {
            player.update_playback_rate(rate, cx, notices);
            Ok(())
        })
    }

    /// `playbackRate`'s setter (section 4.4.17).
    pub fn set_playback_rate(&mut self, id: u64, rate: f64) -> Result<(), ScriptError> {
        self.with_player(id, |player, cx, notices| {
            player.set_playback_rate(rate, cx, notices);
            Ok(())
        })
    }

    /// `currentTime`'s setter (section 4.4.9).
    pub fn set_current_time(&mut self, id: u64, time: Option<f64>) -> Result<(), ScriptError> {
        self.with_player(id, |player, cx, notices| {
            player.set_current_time(time, cx, notices)
        })
    }

    /// `startTime`'s setter (section 4.4.10): a CSS animation's play state
    /// is the script's if it played or paused it.
    pub fn set_start_time(&mut self, id: u64, time: Option<f64>) -> Result<(), ScriptError> {
        let paused = self.paused(id);
        let done = self.with_player(id, |player, cx, notices| {
            player.set_start_time(time, cx, notices);
            Ok(())
        });
        if paused == self.paused(id) {
            return done;
        }
        self.script_plays(id, done)
    }

    /// `cancel()` (section 4.4.16): a CSS animation's or transition's
    /// cancel event queued too, and a transition no longer its element's.
    pub fn cancel(&mut self, id: u64) -> Result<(), ScriptError> {
        let Some(animation) = self.animations.get(&id) else {
            return Ok(());
        };
        let cx = self.context(animation);
        if animation.player.play_state(&cx) == PlayState::Idle {
            return Ok(());
        }
        let local = animation.player.current_time(&cx);
        let timing = animation
            .effect
            .and_then(|e| self.effects.get(&e))
            .map(|e| e.timing.clone());
        match animation.class.clone() {
            Class::Css {
                owner: Some(owner),
                name,
                ..
            } => {
                if let (Some(timing), Some(local)) = (&timing, local)
                    && let Some(elapsed) = cancelled(timing, animation.reported, local)
                {
                    self.events.push(event(
                        (owner, id),
                        &name,
                        super::EventKind::AnimationCancel,
                        elapsed,
                        self.time,
                    ));
                }
                self.cancel_animation(id);
                if let Some(animation) = self.animations.get_mut(&id) {
                    animation.reported = Reported::Idle;
                    animation.reported_iteration = None;
                }
            }
            Class::Transition {
                owner: Some(owner), ..
            } => {
                if let Some(list) = self.transitions.get_mut(&owner) {
                    list.retain(|&t| t != id);
                    if list.is_empty() {
                        self.transitions.remove(&owner);
                    }
                }
                // Its end value kept as a completed transition's, so that
                // the change of style it leaves starts none again.
                let mut completed = self.completed.remove(&owner).unwrap_or_default();
                if let Some(done) = self.completed_of(id) {
                    completed.retain(|c| c.property != done.property);
                    completed.push(done);
                }
                self.completed.insert(owner, completed);
                self.cancel_transition(id, owner);
            }
            _ => self.cancel_animation(id),
        }
        Ok(())
    }

    /// `persist()` (section 6.4): a removed animation's effect is in the
    /// stack again.
    pub fn persist(&mut self, id: u64) {
        let Some(animation) = self.animations.get_mut(&id) else {
            return;
        };
        animation.replace = ReplaceState::Persisted;
        let effect = animation.effect;
        self.touch_effect(effect);
    }

    /// The finish notification steps of animation `id` (section 4.4.15),
    /// a microtask's or at once: when it is still finished, its finished
    /// promise is resolved and its `finish` event queued. Whether it was.
    pub fn notify_finished(&mut self, id: u64) -> bool {
        let Some(animation) = self.animations.get(&id) else {
            return false;
        };
        let cx = self.context(animation);
        let resolved = self
            .animations
            .get_mut(&id)
            .is_some_and(|a| a.player.notify_finished(&cx));
        if resolved {
            let player = self.animations.get(&id).map(|a| a.player.clone());
            let current = player.as_ref().and_then(|p| p.current_time(&cx));
            // When its current time reached the end it finished at.
            let reached = if player.as_ref().is_some_and(|p| p.playback_rate < 0.0) {
                0.0
            } else {
                cx.end
            };
            let at = player.map_or(self.time, |p| super::scheduled(&p, self.time, reached, 0.0));
            self.playback.push(PlaybackEvent {
                animation: id,
                kind: PlaybackKind::Finish,
                current_time: current,
                timeline_time: cx.timeline,
                scheduled: at,
                rank: self.event_rank(id),
            });
        }
        resolved
    }

    /// `timeline`'s setter (section 4.4.1): animation `id` on timeline
    /// `timeline`, its current time kept where it can be.
    pub fn set_timeline(&mut self, id: u64, timeline: Option<usize>) -> Result<(), ScriptError> {
        let timeline = timeline.filter(|&t| t < self.origins.len());
        let Some(animation) = self.animations.get(&id) else {
            return Ok(());
        };
        if animation.timeline == timeline {
            return Ok(());
        }
        let effect = animation.effect;
        if let Some(animation) = self.animations.get_mut(&id) {
            animation.timeline = timeline;
        }
        // A resolved start time is kept, and the hold time let go of: the
        // animation runs on the new timeline from where its start time
        // puts it, or has no current time on none.
        let Some(animation) = self.animations.get(&id) else {
            return Ok(());
        };
        let cx = self.context(animation);
        let mut notices = Vec::new();
        if let Some(animation) = self.animations.get_mut(&id) {
            let player = &mut animation.player;
            if player.start_time.is_some() {
                player.hold_time = None;
            }
            player.update_finished_state(false, false, &cx, &mut notices);
        }
        self.owe(id, notices);
        self.touch_effect(effect);
        Ok(())
    }

    /// `effect`'s setter (section 4.4.3): animation `id`'s effect is
    /// `effect`, taken from the animation it had, if any.
    pub fn set_effect(&mut self, id: u64, effect: Option<u64>) {
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        if animation.effect == effect {
            return;
        }
        let old = animation.effect;
        let effect = effect.filter(|e| self.effects.contains_key(e));
        if let Some(effect) = effect
            && let Some(previous) = self.effects.get(&effect).and_then(|e| e.animation)
            && previous != id
        {
            self.set_effect(previous, None);
        }
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        // The current time is kept: a paused or idle animation holds it,
        // a running one keeps its start time, which is the same.
        let pending = animation.player.pending();
        if let Some(old) = old {
            self.touch_effect(Some(old));
            let held = self.effects.get(&old).is_some_and(|e| e.held);
            if held {
                if let Some(e) = self.effects.get_mut(&old) {
                    e.animation = None;
                }
            } else {
                self.unindex_target(old);
                self.effects.remove(&old);
            }
        }
        if let Some(animation) = self.animations.get_mut(&id) {
            animation.effect = effect;
            animation.last = Sampled::NOTHING;
        }
        if let Some(effect) = effect
            && let Some(e) = self.effects.get_mut(&effect)
        {
            e.animation = Some(id);
        }
        self.touch_effect(effect);
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        let cx = self.context(animation);
        let mut notices = Vec::new();
        if let Some(animation) = self.animations.get_mut(&id) {
            if pending && animation.player.hold_time.is_none() {
                // A pending play task keeps waiting, as it would.
            }
            animation
                .player
                .update_finished_state(false, false, &cx, &mut notices);
        }
        self.owe(id, notices);
    }

    /// Effect `effect`'s target, its pseudo-element among it.
    pub fn effect_target(&self, effect: u64) -> Option<Target> {
        self.effects.get(&effect)?.target
    }

    /// The pseudo-element effect `effect` names, with its element or
    /// without one.
    pub fn effect_pseudo(&self, effect: u64) -> Option<PseudoElement> {
        self.effects.get(&effect)?.pseudo
    }

    /// The animation effect `effect` is associated with.
    pub fn effect_animation(&self, effect: u64) -> Option<u64> {
        self.effects.get(&effect)?.animation
    }

    /// `target`'s setter and `pseudoElement`'s (section 6.6): the element
    /// `node`, and the pseudo-element `pseudo` of it.
    pub fn set_effect_target(
        &mut self,
        effect: u64,
        node: Option<NodeId>,
        pseudo: Option<PseudoElement>,
    ) {
        if !self.effects.contains_key(&effect) {
            return;
        }
        self.touch_effect(Some(effect));
        self.unindex_target(effect);
        if let Some(e) = self.effects.get_mut(&effect) {
            e.target = node.map(|node| (node, pseudo));
            e.pseudo = pseudo;
        }
        self.index_target(effect);
        self.touch_effect(Some(effect));
    }

    /// `getTiming()` (section 6.7.1): the timing, and whether the
    /// duration and the fill are `auto`.
    pub fn timing(&self, effect: u64) -> Option<(Timing, bool, bool)> {
        let e = self.effects.get(&effect)?;
        Some((e.timing.clone(), e.duration_auto, e.fill_auto))
    }

    /// `updateTiming()` (section 6.7.1): a CSS animation's members set
    /// stay the script's.
    pub fn update_timing(&mut self, effect: u64, input: &TimingInput) -> Result<(), ScriptError> {
        let Some(e) = self.effects.get_mut(&effect) else {
            return Ok(());
        };
        let set = updated(&mut e.timing, &mut e.duration_auto, &mut e.fill_auto, input)?;
        e.overridden |= set;
        let animation = e.animation;
        self.touch_effect(Some(effect));
        if let Some(id) = animation
            && let Some(a) = self.animations.get(&id)
        {
            let cx = self.context(a);
            let mut notices = Vec::new();
            if let Some(a) = self.animations.get_mut(&id) {
                a.player
                    .update_finished_state(false, false, &cx, &mut notices);
            }
            self.owe(id, notices);
        }
        Ok(())
    }

    /// `getComputedTiming()` (section 6.7.2).
    pub fn computed_timing(&self, effect: u64) -> Option<ComputedTiming> {
        let e = self.effects.get(&effect)?;
        let animation = e.animation.and_then(|a| self.animations.get(&a));
        let local = animation.and_then(|a| a.player.current_time(&self.context(a)));
        let backwards = animation.is_some_and(|a| a.player.playback_rate < 0.0);
        let progress = local.map(|local| e.timing.progress_in(local, backwards));
        Some(ComputedTiming {
            timing: e.timing.clone(),
            end_time: e.timing.end_time(),
            active_duration: e.timing.active_duration(),
            local_time: local,
            progress: progress.and_then(|p| p.progress),
            current_iteration: progress.and_then(|p| p.iteration),
        })
    }

    /// The phase of effect `effect` at its local time, for the tests and
    /// the bindings' checks; `None` without one.
    pub fn effect_phase(&self, effect: u64) -> Option<Phase> {
        let e = self.effects.get(&effect)?;
        let animation = self.animations.get(&e.animation?)?;
        self.progress_of(animation).map(|p| p.phase)
    }

    /// What `getKeyframes()` reads of effect `effect`.
    pub fn keyframes_source(&self, effect: u64) -> Option<KeyframesSource> {
        let e = self.effects.get(&effect)?;
        Some(match &e.frames {
            Frames::Script(keyframes) => KeyframesSource::Script(Arc::clone(keyframes)),
            Frames::Css { name, easing, .. } => KeyframesSource::Css {
                name: Arc::clone(name),
                easing: easing.clone(),
            },
            Frames::Transition(values) => {
                let property = e
                    .animation
                    .and_then(|a| self.animations.get(&a))
                    .and_then(|a| match &a.class {
                        Class::Transition { property, .. } => Some(property.clone()),
                        _ => None,
                    })?;
                let longhand = match &property {
                    super::Property::Longhand(longhand) => Some(*longhand),
                    super::Property::Custom(_) => None,
                };
                KeyframesSource::Transition {
                    property: property.name(),
                    longhand,
                    start: TransitionValue::from(&values.start),
                    end: TransitionValue::from(&values.end),
                }
            }
        })
    }

    /// `setKeyframes()` (section 6.6): a CSS animation's keyframes stay
    /// the script's.
    pub fn set_keyframes(&mut self, effect: u64, keyframes: ScriptKeyframes) {
        self.touch_effect(Some(effect));
        if let Some(e) = self.effects.get_mut(&effect) {
            e.frames = Frames::Script(Arc::new(keyframes));
            e.overridden |= SET_KEYFRAMES;
        }
    }

    /// The composite operation of effect `effect`.
    pub fn composite(&self, effect: u64) -> Option<Composite> {
        Some(self.effects.get(&effect)?.composite)
    }

    /// `composite`'s setter: a CSS animation's stays the script's.
    pub fn set_composite(&mut self, effect: u64, composite: Composite) {
        self.touch_effect(Some(effect));
        if let Some(e) = self.effects.get_mut(&effect) {
            e.composite = composite;
            e.overridden |= SET_COMPOSITE;
        }
    }

    /// The relevant animations (section 4.5.2) of the effect targets
    /// `wanted` takes, in composite order (section 5.4.2):
    /// transitions by their owners in tree order, `order` giving each
    /// element's place, then by when they were made and by property;
    /// then CSS animations by their owners and their places in
    /// `animation-name`; then the others by their places in the global
    /// animation list. An owner's own animations come before those of its
    /// `::marker`, `::before` and `::after`, in that order.
    pub fn relevant(
        &self,
        wanted: impl Fn(Target) -> bool,
        order: impl Fn(NodeId) -> usize,
    ) -> Vec<u64> {
        let pseudo_rank = |pseudo: Option<PseudoElement>| match pseudo {
            None => 0,
            Some(PseudoElement::Marker) => 1,
            Some(PseudoElement::Before) => 2,
            Some(PseudoElement::After) => 3,
            Some(_) => 4,
        };
        let mut found: Vec<(CompositeKey, u64)> = Vec::new();
        for id in self.relevant_ids() {
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            let Some(target) = animation
                .effect
                .and_then(|e| self.effects.get(&e))
                .and_then(|e| e.target)
            else {
                continue;
            };
            if !wanted(target) {
                continue;
            }
            let key = match &animation.class {
                Class::Transition {
                    owner: Some(owner),
                    property,
                    generation,
                } => (
                    0,
                    order(owner.0),
                    pseudo_rank(owner.1),
                    *generation,
                    property.name(),
                ),
                Class::Css {
                    owner: Some(owner), ..
                } => {
                    let place = self
                        .css
                        .get(owner)
                        .and_then(|list| list.iter().position(|&a| a == id))
                        .unwrap_or(0);
                    (
                        1,
                        order(owner.0),
                        pseudo_rank(owner.1),
                        place as u64,
                        Arc::from(""),
                    )
                }
                _ => (2, 0, 0, id, Arc::from("")),
            };
            found.push((key, id));
        }
        found.sort();
        found.into_iter().map(|(_, id)| id).collect()
    }

    /// Removes the replaced animations (section 5.5): every finished,
    /// filling animation of a script whose every property a later such
    /// animation of the same target animates is removed, its `remove`
    /// event queued.
    pub fn remove_replaced(&mut self) {
        // The replaceable animations by target, in composite order.
        let mut by_target: HashMap<Target, Vec<Replaceable>> = HashMap::new();
        for (&id, animation) in &self.animations {
            let owned = matches!(
                &animation.class,
                Class::Css { owner: Some(_), .. } | Class::Transition { owner: Some(_), .. }
            );
            if owned || animation.replace == ReplaceState::Removed || animation.timeline.is_none() {
                continue;
            }
            let cx = self.context(animation);
            if animation.player.play_state(&cx) != PlayState::Finished {
                continue;
            }
            let Some(effect) = animation.effect.and_then(|e| self.effects.get(&e)) else {
                continue;
            };
            let Some(target) = effect.target else {
                continue;
            };
            if self
                .progress_of(animation)
                .is_none_or(|p| p.active_time.is_none())
            {
                continue;
            }
            // A CSS animation or transition no longer tied to its element
            // replaces and is replaced as a script's does.
            let (longhands, custom) = match &effect.frames {
                Frames::Script(keyframes) => {
                    (keyframes.longhands().to_vec(), keyframes.custom().to_vec())
                }
                Frames::Css { longhands, .. } => (longhands.to_vec(), Vec::new()),
                Frames::Transition(_) => match &animation.class {
                    Class::Transition {
                        property: super::Property::Longhand(longhand),
                        ..
                    } => (vec![*longhand], Vec::new()),
                    Class::Transition {
                        property: super::Property::Custom(name),
                        ..
                    } => (Vec::new(), vec![Arc::clone(name)]),
                    _ => continue,
                },
            };
            by_target
                .entry(target)
                .or_default()
                .push((id, longhands, custom));
        }
        let mut removed = Vec::new();
        for list in by_target.values_mut() {
            list.sort_by_key(|(id, _, _)| self.composite_rank(*id));
            for (i, (id, longhands, custom)) in list.iter().enumerate() {
                if self
                    .animations
                    .get(id)
                    .is_none_or(|a| a.replace != ReplaceState::Active)
                {
                    continue;
                }
                let later = &list[i + 1..];
                let covered = longhands
                    .iter()
                    .all(|l| later.iter().any(|(_, ls, _)| ls.contains(l)))
                    && custom
                        .iter()
                        .all(|c| later.iter().any(|(_, _, cs)| cs.contains(c)));
                if covered {
                    removed.push(*id);
                }
            }
        }
        removed.sort_unstable();
        for id in removed {
            let Some(animation) = self.animations.get(&id) else {
                continue;
            };
            let cx = self.context(animation);
            let current = animation.player.current_time(&cx);
            let held = animation.held;
            let effect = animation.effect;
            if let Some(animation) = self.animations.get_mut(&id) {
                animation.replace = ReplaceState::Removed;
            }
            self.touch_effect(effect);
            if held {
                self.playback.push(PlaybackEvent {
                    animation: id,
                    kind: PlaybackKind::Remove,
                    current_time: current,
                    timeline_time: cx.timeline,
                    scheduled: self.time,
                    rank: self.event_rank(id),
                });
            } else {
                self.forget(id);
            }
        }
    }

    /// Whether effect `effect` is current (section 4.5.2).
    pub fn effect_is_current(&self, effect: u64) -> bool {
        self.effects
            .get(&effect)
            .and_then(|e| e.animation)
            .and_then(|a| self.animations.get(&a))
            .is_some_and(|a| self.is_current(a))
    }

    /// Limits the effect stack the cascade reads to the animations below
    /// and including animation `id` in composite order, as
    /// `commitStyles()` reads it (section 6.4), until it is set back to
    /// `None`.
    pub fn commit_limit(&mut self, id: Option<u64>) {
        self.committing = id;
    }
}
