//! CSS transitions (Transitions 1 §3, Transitions 2 §2.5 and §3; M22.1):
//! the running and completed transitions of a document, started,
//! replaced, reversed and cancelled at each change of an element's style
//! as §3 says, to the letter.
//!
//! A transition's values are computed values fixed when it starts: the
//! property's value in a style the transition keeps, shared by every
//! property one change of style starts, or a custom property's tokens. Its
//! effect is Web Animations' with the transition's delay, duration and
//! timing function, one iteration and a backwards fill, so that it shows
//! its start value through its delay; it waits for the next frame to
//! start, as a CSS animation does, and ends at its end time, when it
//! joins the completed transitions. A completed transition is kept while
//! the element's style before a change and after it still differ in its
//! property, which is when §3's rules ask of it, and forgotten after.

use std::sync::Arc;

use css::Token;

use super::events::{Reported, cancelled, transition_events};
use super::{
    AnimationEvent, Animations, Clock, EventKind, MAX_ANIMATIONS, Phase, Sampled, Target, Timing,
    clock_time, event,
};
use crate::animate::{self, AnimationType, Longhand, longhands_of};
use crate::cascade::canonical_name;
use crate::computed::{
    AnimationDirection, AnimationFillMode, ComputedStyle, TransitionBehavior, TransitionProperty,
};
use crate::easing::Easing;

/// What a transition transitions: a longhand, or a custom property by its
/// name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Property {
    Longhand(Longhand),
    Custom(Arc<str>),
}

impl Property {
    /// Its name, as a transition event gives it.
    pub(super) fn name(&self) -> Arc<str> {
        match self {
            Property::Longhand(longhand) => Arc::from(longhand.name()),
            Property::Custom(name) => Arc::clone(name),
        }
    }
}

/// A transitioning property's value: a computed style that holds it, for a
/// longhand, or a custom property's value, `None` for the
/// guaranteed-invalid value.
#[derive(Debug, Clone)]
pub(crate) enum Value {
    Style(Arc<ComputedStyle>),
    Custom(Option<Arc<[Token]>>),
}

impl Value {
    /// Whether `property` has the same value in this and `other`.
    fn same(&self, property: &Property, other: &Value) -> bool {
        match (self, other, property) {
            (Value::Style(a), Value::Style(b), Property::Longhand(longhand)) => {
                Arc::ptr_eq(a, b) || animate::same(*longhand, a, b)
            }
            (Value::Custom(a), Value::Custom(b), Property::Custom(_)) => a == b,
            _ => false,
        }
    }

    /// The value `p` of the way from this to `end`: a style holding it.
    fn towards(&self, property: &Property, end: &Value, p: f64) -> Value {
        match (self, end, property) {
            (Value::Style(a), Value::Style(b), Property::Longhand(longhand)) => {
                let mut style = ComputedStyle::clone(b);
                animate::blend(*longhand, a, b, p, &mut style);
                Value::Style(Arc::new(style))
            }
            _ => {
                if p < 0.5 {
                    self.clone()
                } else {
                    end.clone()
                }
            }
        }
    }
}

/// A running transition (Transitions 1 §3).
#[derive(Debug, Clone)]
pub(super) struct CssTransition {
    pub(super) property: Property,
    /// Its effect's timing: the transition's delay, duration and timing
    /// function, one iteration, filled backwards.
    pub(super) timing: Timing,
    pub(super) clock: Clock,
    start: Value,
    end: Value,
    /// The reversing-adjusted start value (§3.1).
    reversing_start: Value,
    /// The reversing shortening factor (§3.1).
    shortening: f64,
    pub(super) last: Sampled,
    /// Its phase at the last animation frame, for the events.
    reported: Reported,
}

impl CssTransition {
    /// Queues the events of its change of phase to the one at the
    /// timeline's time `time`.
    fn report(&mut self, events: &mut Vec<AnimationEvent>, target: Target, time: f64) {
        if matches!(self.clock, Clock::Pending { .. }) {
            return;
        }
        let now = Reported::from(self.timing.phase(clock_time(&self.clock, time)));
        let name = self.property.name();
        for (kind, elapsed) in transition_events(&self.timing, self.reported, now) {
            events.push(event(target, &name, kind, elapsed));
        }
        self.reported = now;
    }

    /// Queues its cancel event, it being cancelled at the timeline's time
    /// `time`, if one is due.
    pub(super) fn cancelled(&self, events: &mut Vec<AnimationEvent>, target: Target, time: f64) {
        let local = clock_time(&self.clock, time);
        if let Some(elapsed) = cancelled(&self.timing, self.reported, local) {
            events.push(event(
                target,
                &self.property.name(),
                EventKind::TransitionCancel,
                elapsed,
            ));
        }
    }
}

/// A completed transition: its property and end value.
#[derive(Debug, Clone)]
pub(super) struct Completed {
    pub(super) property: Property,
    end: Value,
}

/// A running transition's value at the timeline's time, for the cascade
/// to apply.
#[derive(Debug, Clone)]
pub(crate) struct Transitioning {
    pub(crate) property: Property,
    pub(crate) start: Value,
    pub(crate) end: Value,
    /// Its transformed progress, the timing function's output.
    pub(crate) progress: f64,
}

/// What the matching item of `transition-*` gives a property (§3).
#[derive(Debug, Clone, Copy)]
struct Matching<'m> {
    duration: f64,
    delay: f64,
    easing: &'m Easing,
    allow_discrete: bool,
}

impl Matching<'_> {
    /// The combined duration: the duration not below zero, and the delay.
    fn combined(&self) -> f64 {
        self.duration.max(0.0) + self.delay
    }

    /// The effect timing of a transition it starts, shortened by
    /// `shortening` (§3.1).
    fn timing(&self, shortening: f64) -> Timing {
        Timing {
            delay: if self.delay < 0.0 {
                self.delay * shortening
            } else {
                self.delay
            },
            duration: self.duration * shortening,
            iterations: 1.0,
            direction: AnimationDirection::Normal,
            fill: AnimationFillMode::Backwards,
            easing: self.easing.clone(),
        }
    }
}

/// Whether `property`'s values in `a` and `b` are transitionable
/// (Transitions 2 §2.1): they interpolate, or the change is discrete and
/// `allow-discrete` lets it transition.
fn transitionable(property: &Property, a: &Value, b: &Value, allow_discrete: bool) -> bool {
    match (property, a, b) {
        (Property::Longhand(longhand), Value::Style(a), Value::Style(b)) => {
            match longhand.animation_type() {
                AnimationType::NotAnimatable => false,
                AnimationType::Discrete => allow_discrete,
                AnimationType::ByComputedValue => {
                    allow_discrete || animate::interpolable(*longhand, a, b)
                }
            }
        }
        (Property::Custom(_), Value::Custom(_), Value::Custom(_)) => allow_discrete,
        _ => false,
    }
}

/// The value of `property` in `style`.
fn value_in(property: &Property, style: &Arc<ComputedStyle>) -> Value {
    match property {
        Property::Longhand(_) => Value::Style(Arc::clone(style)),
        Property::Custom(name) => {
            Value::Custom(style.custom.as_ref().and_then(|m| m.get(name)).cloned())
        }
    }
}

impl Animations {
    /// Transitions 1 §3 for `target` at a change of its style: `before` is
    /// its style as the last change left it, `None` when it has none — it
    /// was not rendered, or not in the document — and `after` its style
    /// after this change, with its animations' values and without its
    /// transitions'. A property `skip` names, one a CSS animation of the
    /// target animated before the change or animates after it, is left
    /// alone: no transition starts for it, as in the reference browser.
    pub(crate) fn update_transitions(
        &mut self,
        target: Target,
        before: Option<&ComputedStyle>,
        after: &ComputedStyle,
        skip: &dyn Fn(Longhand) -> bool,
    ) {
        let time = self.time;
        let has_any = (!self.transitions.is_empty() || !self.completed.is_empty())
            && (self.transitions.contains_key(&target) || self.completed.contains_key(&target));
        let Some(before) = before else {
            if has_any {
                self.cancel_transitions(target);
            }
            return;
        };
        let items: Vec<_> = after.motion().transitions().collect();
        if !has_any && items.iter().all(|t| t.duration.max(0.0) + t.delay <= 0.0) {
            return;
        }
        // The matching item of each property: the last that names it.
        let mut longhands: Vec<Option<usize>> = vec![None; Longhand::ALL.len()];
        let mut custom: Vec<(Arc<str>, usize)> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            match item.property {
                TransitionProperty::All => {
                    for &longhand in Longhand::ALL {
                        if longhand.is_animatable() {
                            longhands[longhand as usize] = Some(index);
                        }
                    }
                }
                TransitionProperty::Name(name) if crate::custom::is_custom_property_name(name) => {
                    custom.retain(|(n, _)| n != name);
                    custom.push((Arc::clone(name), index));
                }
                TransitionProperty::Name(name) => {
                    for longhand in longhands_of(canonical_name(name)) {
                        longhands[longhand as usize] = Some(index);
                    }
                }
            }
        }
        let matching = |property: &Property| {
            let index = match property {
                Property::Longhand(longhand) => longhands[*longhand as usize],
                Property::Custom(name) => custom.iter().find(|(n, _)| n == name).map(|&(_, i)| i),
            }?;
            let item = &items[index];
            Some(Matching {
                duration: item.duration,
                delay: item.delay,
                easing: item.timing_function,
                allow_discrete: item.behavior == TransitionBehavior::AllowDiscrete,
            })
        };
        // The properties to look at: those an item names, and those with a
        // running or completed transition.
        let mut properties: Vec<Property> = Longhand::ALL
            .iter()
            .filter(|&&l| longhands[l as usize].is_some())
            .map(|&l| Property::Longhand(l))
            .collect();
        properties.extend(
            custom
                .iter()
                .map(|(name, _)| Property::Custom(Arc::clone(name))),
        );
        for property in self
            .transitions
            .get(&target)
            .into_iter()
            .flatten()
            .map(|t| &t.property)
            .chain(
                self.completed
                    .get(&target)
                    .into_iter()
                    .flatten()
                    .map(|c| &c.property),
            )
        {
            if !properties.contains(property) {
                properties.push(property.clone());
            }
        }
        let mut before_style: Option<Arc<ComputedStyle>> = None;
        let mut after_style: Option<Arc<ComputedStyle>> = None;
        let mut running = self.transitions.remove(&target).unwrap_or_default();
        let mut completed = self.completed.remove(&target).unwrap_or_default();
        // A transition past its end time is completed, whether or not a
        // frame has said so yet: its events are queued as a frame's.
        let events = &mut self.events;
        running.retain_mut(|t| {
            let over = ended(t, time);
            if over {
                t.report(events, target, time);
                self.count -= 1;
                completed.retain(|c| c.property != t.property);
                completed.push(Completed {
                    property: t.property.clone(),
                    end: t.end.clone(),
                });
            }
            !over
        });
        for property in properties {
            if let Property::Longhand(longhand) = property
                && skip(longhand)
            {
                continue;
            }
            let differs = match &property {
                Property::Longhand(longhand) => !animate::same(*longhand, before, after),
                Property::Custom(name) => {
                    before.custom.as_ref().and_then(|m| m.get(name))
                        != after.custom.as_ref().and_then(|m| m.get(name))
                }
            };
            let matching = matching(&property);
            let at = running.iter().position(|t| t.property == property);
            let done = completed.iter().position(|c| c.property == property);
            if at.is_none() && done.is_none() && (!differs || matching.is_none()) {
                continue;
            }
            let before_value = value_in(
                &property,
                before_style.get_or_insert_with(|| Arc::new(before.clone())),
            );
            let after_value = value_in(
                &property,
                after_style.get_or_insert_with(|| Arc::new(after.clone())),
            );
            let start = match (at, matching) {
                (None, Some(matching))
                    if differs
                        && matching.combined() > 0.0
                        && transitionable(
                            &property,
                            &before_value,
                            &after_value,
                            matching.allow_discrete,
                        )
                        && done.is_none_or(|d| !completed[d].end.same(&property, &after_value)) =>
                {
                    Some((
                        matching.timing(1.0),
                        before_value.clone(),
                        before_value,
                        1.0,
                    ))
                }
                _ => None,
            };
            if let Some((timing, from, reversing_start, shortening)) = start {
                if let Some(d) = done {
                    completed.remove(d);
                }
                if self.count < MAX_ANIMATIONS {
                    self.count += 1;
                    running.push(new_transition(
                        property,
                        timing,
                        from,
                        after_value,
                        reversing_start,
                        shortening,
                        time,
                    ));
                }
                continue;
            }
            if let Some(d) = done
                && (matching.is_none()
                    || !completed[d].end.same(&property, &after_value)
                    || !differs)
            {
                completed.remove(d);
            }
            let Some(at) = at else {
                continue;
            };
            let Some(matching) = matching else {
                running.remove(at).cancelled(&mut self.events, target, time);
                self.count -= 1;
                continue;
            };
            if running[at].end.same(&property, &after_value) {
                continue;
            }
            // The running transition's end is no longer the property's
            // value: cancelled, replaced or reversed.
            let old = running.remove(at);
            old.cancelled(&mut self.events, target, time);
            self.count -= 1;
            let progress = old.timing.progress(clock_time(&old.clock, time));
            let p = progress.progress.unwrap_or(1.0);
            let current = old.start.towards(&property, &old.end, p);
            if current.same(&property, &after_value)
                || !transitionable(&property, &current, &after_value, matching.allow_discrete)
                || matching.combined() <= 0.0
            {
                continue;
            }
            let (timing, reversing_start, shortening) =
                if old.reversing_start.same(&property, &after_value) {
                    let shortening = (p * old.shortening + 1.0 - old.shortening)
                        .abs()
                        .clamp(0.0, 1.0);
                    (matching.timing(shortening), old.end.clone(), shortening)
                } else {
                    (matching.timing(1.0), current.clone(), 1.0)
                };
            self.count += 1;
            running.push(new_transition(
                property,
                timing,
                current,
                after_value,
                reversing_start,
                shortening,
                time,
            ));
        }
        if !running.is_empty() {
            self.transitions.insert(target, running);
        }
        if !completed.is_empty() {
            self.completed.insert(target, completed);
        }
    }

    /// Cancels `target`'s running transitions and forgets its completed
    /// ones: it is no longer rendered, or no longer in the document.
    pub(crate) fn cancel_transitions(&mut self, target: Target) {
        if let Some(running) = self.transitions.remove(&target) {
            self.count -= running.len();
            for transition in &running {
                transition.cancelled(&mut self.events, target, self.time);
            }
        }
        self.completed.remove(&target);
    }

    /// Queues the events of every running transition's change of phase
    /// since the last animation frame: this one.
    pub(super) fn report_transitions(&mut self) {
        let time = self.time;
        let events = &mut self.events;
        for (&target, running) in &mut self.transitions {
            for transition in running {
                transition.report(events, target, time);
            }
        }
    }

    /// Whether `target` has a running transition.
    pub(crate) fn transitions_of(&self, target: Target) -> bool {
        self.transitions.contains_key(&target)
    }

    /// How many completed transitions are kept, for the tests.
    #[cfg(test)]
    pub(crate) fn completed_count(&self) -> usize {
        self.completed.values().map(Vec::len).sum()
    }

    /// The values `target`'s running transitions give at the timeline's
    /// time, in no order that matters: one transition for a property.
    pub(crate) fn transitioning(&self, target: Target) -> Vec<Transitioning> {
        let time = self.time;
        self.transitions
            .get(&target)
            .map_or_else(Vec::new, |running| {
                running
                    .iter()
                    .filter_map(|t| {
                        let progress = t.timing.progress(clock_time(&t.clock, time));
                        Some(Transitioning {
                            property: t.property.clone(),
                            start: t.start.clone(),
                            end: t.end.clone(),
                            progress: progress.progress?,
                        })
                    })
                    .collect()
            })
    }

    /// Moves the transitions that have ended by the timeline's time to the
    /// completed ones: the targets they were on.
    pub(super) fn end_transitions(&mut self) -> Vec<Target> {
        let time = self.time;
        let mut targets = Vec::new();
        let mut count = self.count;
        for (&target, running) in &mut self.transitions {
            let before = running.len();
            let mut done = Vec::new();
            running.retain(|t| {
                let over = ended(t, time);
                if over {
                    done.push(Completed {
                        property: t.property.clone(),
                        end: t.end.clone(),
                    });
                }
                !over
            });
            if running.len() != before {
                count -= before - running.len();
                targets.push(target);
                let completed = self.completed.entry(target).or_default();
                for c in done {
                    completed.retain(|o| o.property != c.property);
                    completed.push(c);
                }
            }
        }
        self.count = count;
        self.transitions.retain(|_, running| !running.is_empty());
        targets
    }
}

/// Whether `transition` has ended by the timeline's time `time`: it
/// started, and its effect is past its active interval.
fn ended(transition: &CssTransition, time: f64) -> bool {
    matches!(transition.clock, Clock::Running { .. })
        && transition.timing.phase(clock_time(&transition.clock, time)) == Phase::After
}

/// A transition of `property` from `start` to `end`, waiting for the next
/// frame to start it.
fn new_transition(
    property: Property,
    timing: Timing,
    start: Value,
    end: Value,
    reversing_start: Value,
    shortening: f64,
    time: f64,
) -> CssTransition {
    let clock = Clock::Pending { from: 0.0 };
    let last = timing.progress(clock_time(&clock, time)).into();
    CssTransition {
        property,
        timing,
        clock,
        start,
        end,
        reversing_start,
        shortening,
        last,
        reported: Reported::Idle,
    }
}
