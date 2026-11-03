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
//! start, as a CSS animation does, and ends when its animation finishes,
//! when it joins the completed transitions. A completed transition is
//! kept while the element's style before a change and after it still
//! differ in its property, which is when section 3's rules ask of it, and
//! forgotten after. Each is an animation of the document's store, by its
//! number, which a script reaches as a `CSSTransition` (D75): its own
//! property's values live in its effect's frames.

use std::sync::Arc;

use css::Token;

use super::events::{Reported, cancelled, transition_events};
use super::player::{PlayState, Player};
use super::{
    Animation, Animations, Class, Effect, EventKind, Frames, MAX_ANIMATIONS, ReplaceState, Sampled,
    Target, Timing, event, scheduled, take,
};
use crate::animate::{self, AnimationType, Longhand, longhands_of};
use crate::cascade::canonical_name;
use crate::computed::{
    AnimationFillMode, Composite, ComputedStyle, Edge, TransitionBehavior, TransitionProperty,
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

/// What a transition's keyframe effect runs between (Transitions 1 section 3):
/// its start and end values, and its reversing-adjusted start value and
/// shortening factor (section 3.1).
#[derive(Debug, Clone)]
pub(crate) struct TransitionValues {
    pub(crate) start: Value,
    pub(crate) end: Value,
    reversing_start: Value,
    shortening: f64,
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
    /// `shortening` (section 3.1), in milliseconds.
    fn timing(&self, shortening: f64) -> Timing {
        Timing {
            delay: if self.delay < 0.0 {
                self.delay * shortening
            } else {
                self.delay
            } * 1000.0,
            duration: self.duration * shortening * 1000.0,
            fill: AnimationFillMode::Backwards,
            easing: self.easing.clone(),
            ..Timing::default()
        }
    }
}

/// Whether `property`'s values in `a` and `b` are transitionable
/// (Transitions 2 §2.1): they interpolate, or the change is discrete and
/// `allow-discrete` lets it transition. A custom property's interpolate
/// where it is `typed`, registered with a syntax (D73), and they are of
/// one type.
fn transitionable(
    property: &Property,
    a: &Value,
    b: &Value,
    allow_discrete: bool,
    typed: bool,
) -> bool {
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
        (Property::Custom(_), Value::Custom(a), Value::Custom(b)) => {
            allow_discrete
                || (typed
                    && matches!((a, b), (Some(a), Some(b))
                        if crate::property::interpolate(a, b, 0.5).is_some()))
        }
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
    /// Transition `id`'s property, values and timing, if it is one.
    fn transition(&self, id: u64) -> Option<(&Property, &TransitionValues, &Timing)> {
        let animation = self.animations.get(&id)?;
        let Class::Transition { property, .. } = &animation.class else {
            return None;
        };
        let effect = self.effects.get(&animation.effect?)?;
        let Frames::Transition(values) = &effect.frames else {
            return None;
        };
        Some((property, values, &effect.timing))
    }

    /// The property transition `id` transitions.
    fn property_of(&self, id: u64) -> Option<&Property> {
        match &self.animations.get(&id)?.class {
            Class::Transition { property, .. } => Some(property),
            _ => None,
        }
    }

    /// Whether transition `id` has ended by the timeline's time: it
    /// started, and it is finished.
    fn ended(&self, id: u64) -> bool {
        self.animations.get(&id).is_some_and(|animation| {
            !animation.player.pending()
                && animation.player.play_state(&self.context(animation)) == PlayState::Finished
        })
    }

    /// Queues the events of transition `id`'s change of phase since the
    /// last animation frame, owned by `target`.
    fn report_transition(&mut self, id: u64, target: Target) {
        let Some(animation) = self.animations.get(&id) else {
            return;
        };
        if animation.player.pending() {
            return;
        }
        let now = self
            .progress_of(animation)
            .map_or(Reported::Idle, |p| Reported::from(p.phase));
        let before = animation.reported;
        let player = animation.player.clone();
        let Some((property, _, timing)) = self.transition(id) else {
            return;
        };
        // Its name made only for an event: a frame samples every running
        // transition twice (M26.13).
        let mut name = None;
        let mut queued = Vec::new();
        for (kind, elapsed) in transition_events(timing, before, now) {
            let name = name.get_or_insert_with(|| property.name());
            let at = scheduled(&player, self.time, timing.delay, elapsed);
            queued.push(event((target, id), name, kind, elapsed, at));
        }
        self.events.extend(queued);
        if let Some(animation) = self.animations.get_mut(&id) {
            animation.reported = now;
        }
    }

    /// Cancels transition `id`, owned by `target` until now: its cancel
    /// event queued if one is due, it forgotten unless a script holds it.
    pub(super) fn cancel_transition(&mut self, id: u64, target: Target) {
        if let Some(animation) = self.animations.get(&id)
            && let Some(local) = animation.player.current_time(&self.context(animation))
            && let Some((property, _, timing)) = self.transition(id)
            && let Some(elapsed) = cancelled(timing, animation.reported, local)
        {
            let name = property.name();
            self.events.push(event(
                (target, id),
                &name,
                EventKind::TransitionCancel,
                elapsed,
                self.time,
            ));
        }
        self.cancel_animation(id);
        self.disown_transition(id);
    }

    /// Transition `id` no longer runs on its element: no owner, and
    /// forgotten unless a script holds it.
    fn disown_transition(&mut self, id: u64) {
        let Some(animation) = self.animations.get_mut(&id) else {
            return;
        };
        animation.reported = Reported::Idle;
        if let Class::Transition { owner, .. } = &mut animation.class {
            *owner = None;
        }
        if !animation.held {
            self.forget(id);
        }
    }

    /// Transition `id` as a completed transition: its property and end
    /// value.
    pub(super) fn completed_of(&self, id: u64) -> Option<Completed> {
        let (property, values, _) = self.transition(id)?;
        Some(Completed {
            property: property.clone(),
            end: values.end.clone(),
        })
    }

    /// Transition `id`, ended: completed, its end value kept among
    /// `completed`.
    fn complete_transition(&mut self, id: u64, completed: &mut Vec<Completed>) {
        if let Some((property, values, _)) = self.transition(id) {
            let done = Completed {
                property: property.clone(),
                end: values.end.clone(),
            };
            completed.retain(|c| c.property != done.property);
            completed.push(done);
        }
        self.disown_transition(id);
    }

    /// A transition of `property` on `target` from `values`' start to
    /// its end, waiting for the next frame to start it: its number.
    fn start_transition(
        &mut self,
        target: Target,
        property: Property,
        timing: Timing,
        values: TransitionValues,
    ) -> u64 {
        let effect = self.number();
        self.effects.insert(
            effect,
            Effect {
                target: Some(target),
                pseudo: target.1,
                animation: None,
                timing,
                duration_auto: false,
                fill_auto: false,
                overridden: 0,
                composite: Composite::Replace,
                frames: Frames::Transition(Box::new(values)),
                held: false,
            },
        );
        self.index_target(effect);
        let id = self.number();
        if let Some(e) = self.effects.get_mut(&effect) {
            e.animation = Some(id);
        }
        let mut animation = Animation {
            class: Class::Transition {
                owner: Some(target),
                property,
                generation: self.transition_generation,
            },
            player: Player::new(),
            effect: Some(effect),
            timeline: Some(0),
            range: [Edge::Normal, Edge::Normal],
            range_set: false,
            timeline_set: false,
            replace: ReplaceState::Active,
            last: Sampled::NOTHING,
            reported: Reported::Idle,
            reported_iteration: None,
            held: false,
            unseen: self.in_script,
        };
        let cx = self.context(&animation);
        let mut notices = Vec::new();
        let _ = animation.player.play(true, &cx, &mut notices);
        // What the frame's styles show of it; nothing when a script's
        // flush made it, which the frame's styles do not hold.
        if !self.in_script {
            animation.last = self.progress_of(&animation).into();
        }
        self.animations.insert(id, animation);
        id
    }

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
        let has_any = (!self.transitions.is_empty() || !self.completed.is_empty())
            && (self.transitions.contains_key(&target) || self.completed.contains_key(&target));
        let Some(before) = before else {
            if has_any {
                // One a script's style flush started, which the frame's
                // styles have not seen, stays: the flush saw the element
                // rendered.
                let running = take(&mut self.transitions, &target).unwrap_or_default();
                let (kept, gone): (Vec<u64>, Vec<u64>) = running
                    .into_iter()
                    .partition(|id| self.animations.get(id).is_some_and(|a| a.unseen));
                for id in gone {
                    self.cancel_transition(id, target);
                }
                if kept.is_empty() {
                    take(&mut self.completed, &target);
                } else {
                    self.transitions.insert(target, kept);
                }
            }
            return;
        };
        if !has_any
            && after
                .motion()
                .transitions()
                .all(|t| t.duration.max(0.0) + t.delay <= 0.0)
        {
            return;
        }
        let items: Vec<_> = after.motion().transitions().collect();
        // The transitions this change starts are of one generation.
        self.transition_generation += 1;
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
        let mut running = self.transitions.remove(&target).unwrap_or_default();
        let mut completed = self.completed.remove(&target).unwrap_or_default();
        // The longhands with a running or completed transition: those
        // looked for among them, the rest only when they change.
        let mut tracked = vec![false; Longhand::ALL.len()];
        let kept: Vec<Property> = running
            .iter()
            .filter_map(|&id| self.property_of(id).cloned())
            .chain(completed.iter().map(|c| c.property.clone()))
            .collect();
        for property in kept {
            let named = match &property {
                Property::Longhand(longhand) => {
                    let seen =
                        longhands[*longhand as usize].is_some() || tracked[*longhand as usize];
                    tracked[*longhand as usize] = true;
                    seen
                }
                Property::Custom(_) => properties.contains(&property),
            };
            if !named {
                properties.push(property);
            }
        }
        let mut before_style: Option<Arc<ComputedStyle>> = None;
        let mut after_style: Option<Arc<ComputedStyle>> = None;
        // A transition past its end time is completed, whether or not a
        // frame has said so yet: its events are queued as a frame's.
        let over: Vec<u64> = running
            .iter()
            .copied()
            .filter(|&id| self.ended(id))
            .collect();
        running.retain(|id| !over.contains(id));
        for id in over {
            self.report_transition(id, target);
            self.complete_transition(id, &mut completed);
        }
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
            let tracked = match &property {
                Property::Longhand(longhand) => tracked[*longhand as usize],
                Property::Custom(_) => true,
            };
            if !tracked && !differs {
                continue;
            }
            let matching = matching(&property);
            let at = running
                .iter()
                .position(|&id| self.property_of(id) == Some(&property));
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
            let typed = match &property {
                Property::Custom(name) => after.custom.as_ref().is_some_and(|m| m.is_typed(name)),
                Property::Longhand(_) => false,
            };
            let start = match (at, matching) {
                (None, Some(matching))
                    if differs
                        && matching.combined() > 0.0
                        && transitionable(
                            &property,
                            &before_value,
                            &after_value,
                            matching.allow_discrete,
                            typed,
                        )
                        && done.is_none_or(|d| !completed[d].end.same(&property, &after_value)) =>
                {
                    Some(matching.timing(1.0))
                }
                _ => None,
            };
            if let Some(timing) = start {
                if let Some(d) = done {
                    completed.remove(d);
                }
                if self.animations.len() < MAX_ANIMATIONS {
                    let values = TransitionValues {
                        start: before_value.clone(),
                        end: after_value,
                        reversing_start: before_value,
                        shortening: 1.0,
                    };
                    running.push(self.start_transition(target, property, timing, values));
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
                let id = running.remove(at);
                self.cancel_transition(id, target);
                continue;
            };
            let old = running[at];
            let Some((_, values, _)) = self.transition(old) else {
                running.remove(at);
                continue;
            };
            if values.end.same(&property, &after_value) {
                continue;
            }
            // The running transition's end is no longer the property's
            // value: cancelled, replaced or reversed.
            let p = self
                .animations
                .get(&old)
                .and_then(|a| self.progress_of(a))
                .and_then(|progress| progress.progress)
                .unwrap_or(1.0);
            let current = values.start.towards(&property, &values.end, p);
            let reversing = values
                .reversing_start
                .same(&property, &after_value)
                .then(|| (values.end.clone(), values.shortening));
            running.remove(at);
            self.cancel_transition(old, target);
            if current.same(&property, &after_value)
                || !transitionable(
                    &property,
                    &current,
                    &after_value,
                    matching.allow_discrete,
                    typed,
                )
                || matching.combined() <= 0.0
            {
                continue;
            }
            let (timing, reversing_start, shortening) = match reversing {
                Some((old_end, old_shortening)) => {
                    let shortening = (p * old_shortening + 1.0 - old_shortening)
                        .abs()
                        .clamp(0.0, 1.0);
                    (matching.timing(shortening), old_end, shortening)
                }
                None => (matching.timing(1.0), current.clone(), 1.0),
            };
            let values = TransitionValues {
                start: current,
                end: after_value,
                reversing_start,
                shortening,
            };
            running.push(self.start_transition(target, property, timing, values));
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
        if let Some(running) = take(&mut self.transitions, &target) {
            for id in running {
                self.cancel_transition(id, target);
            }
        }
        take(&mut self.completed, &target);
    }

    /// Queues the events of every running transition's change of phase
    /// since the last animation frame: this one.
    pub(super) fn report_transitions(&mut self) {
        let running: Vec<(Target, u64)> = self
            .transitions
            .iter()
            .flat_map(|(&target, ids)| ids.iter().map(move |&id| (target, id)))
            .collect();
        for (target, id) in running {
            self.report_transition(id, target);
        }
    }

    /// Whether `target` has a running transition, or a transition's
    /// effect a script played again after its element let it go.
    pub(crate) fn transitions_of(&self, target: Target) -> bool {
        (!self.transitions.is_empty() && self.transitions.contains_key(&target))
            || (!self.targeted.is_empty()
                && self.targeted.get(&target).is_some_and(|effects| {
                    effects.iter().any(|e| {
                        self.effects
                            .get(e)
                            .is_some_and(|e| matches!(e.frames, Frames::Transition(_)))
                    })
                }))
    }

    /// How many completed transitions are kept, for the tests.
    #[cfg(test)]
    pub(crate) fn completed_count(&self) -> usize {
        self.completed.values().map(Vec::len).sum()
    }

    /// The values `target`'s running transitions give at the timeline's
    /// time, in no order that matters: one transition for a property.
    pub(crate) fn transitioning(&self, target: Target) -> Vec<Transitioning> {
        // Its running transitions, and those its element let go of that a
        // script holds and plays: every transition's effect on it.
        let mut ids: Vec<u64> = self.transitions.get(&target).cloned().unwrap_or_default();
        for effect in self.targeted.get(&target).into_iter().flatten() {
            if let Some(e) = self.effects.get(effect)
                && matches!(e.frames, Frames::Transition(_))
                && let Some(id) = e.animation
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        ids.iter()
            .filter_map(|&id| {
                let animation = self.animations.get(&id)?;
                if animation.replace == ReplaceState::Removed {
                    return None;
                }
                let (property, values, _) = self.transition(id)?;
                let progress = self.progress_of(animation)?;
                Some(Transitioning {
                    property: property.clone(),
                    start: values.start.clone(),
                    end: values.end.clone(),
                    progress: progress.progress?,
                })
            })
            .collect()
    }

    /// Moves the transitions that have ended by the timeline's time to the
    /// completed ones: the targets they were on.
    pub(super) fn end_transitions(&mut self) -> Vec<Target> {
        let mut targets = Vec::new();
        let all: Vec<Target> = self.transitions.keys().copied().collect();
        for target in all {
            let Some(mut running) = self.transitions.remove(&target) else {
                continue;
            };
            let over: Vec<u64> = running
                .iter()
                .copied()
                .filter(|&id| self.ended(id))
                .collect();
            if !over.is_empty() {
                targets.push(target);
                running.retain(|id| !over.contains(id));
                let mut completed = self.completed.remove(&target).unwrap_or_default();
                for id in over {
                    self.complete_transition(id, &mut completed);
                }
                self.completed.insert(target, completed);
            }
            if !running.is_empty() {
                self.transitions.insert(target, running);
            }
        }
        targets
    }
}
