//! The timing model of Web Animations (Web Animations Level 1 §4.6,
//! §4.7, §5.3.4; M22) for the two kinds of animation CSS makes: one
//! timeline, a playback rate of one, an iteration start of zero and
//! `replace` composition.
//!
//! [`Timing`] is an effect's timing: its delay, iteration duration,
//! iteration count, direction, fill mode and easing. Given a local time
//! it gives the effect's [`Progress`]: its phase, its active time as the
//! fill mode allows one, its current iteration, and its transformed
//! progress with the before flag. [`interval`] then finds the keyframes
//! on either side of that progress among one property's keyframes
//! (§5.3.4).
//!
//! Times are seconds in an `f64`. An infinite iteration count makes the
//! active duration infinite; a zero iteration duration or count makes
//! it zero; no step computes an infinity less an infinity or zero times
//! an infinity, so that every result is a number or `None`.
//!
//! [`Animations`] is what a document runs (M22.2, M22.3): its timeline's
//! time, which the caller moves on and which never goes back, and each
//! target's CSS animations with their clocks. The cascade keeps them as
//! it computes styles — an element's `animation-name` matched to the
//! animations it had (Animations 1 §2), each new one waiting for the
//! next frame to start it — and reads their progress at the timeline's
//! time; a frame moves the timeline on, starts what waits, and asks
//! which targets' animations give something new, to be restyled.

use std::collections::HashMap;
use std::sync::Arc;

use dom::NodeId;

use crate::computed::{AnimationDirection, AnimationFillMode};
use crate::easing::Easing;
use crate::selector_matching::PseudoElement;

/// An animation effect's timing.
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    /// The start delay, in seconds, negative to start part way.
    pub delay: f64,
    /// The iteration duration, in seconds, not below zero.
    pub duration: f64,
    /// The iteration count, not below zero, infinite for `infinite`.
    pub iterations: f64,
    /// The playback direction.
    pub direction: AnimationDirection,
    /// The fill mode.
    pub fill: AnimationFillMode,
    /// The effect's easing: a transition's timing function; linear for
    /// a CSS animation, whose timing function is its keyframes'.
    pub easing: Easing,
}

/// Where an effect's local time falls (§4.6.6).
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
    /// The before flag (§4.7.7): set going forwards in the before phase.
    pub before: bool,
}

impl Timing {
    /// The active duration (§4.7.1): the iteration duration times the
    /// count, zero when either is.
    pub fn active_duration(&self) -> f64 {
        if self.duration == 0.0 || self.iterations == 0.0 {
            0.0
        } else {
            self.duration * self.iterations
        }
    }

    /// The end time: the delay and the active duration together, not
    /// below zero (§4.6.1).
    pub fn end_time(&self) -> f64 {
        (self.delay + self.active_duration()).max(0.0)
    }

    /// The phase at `local` (§4.6.6), the playback going forwards.
    pub fn phase(&self, local: f64) -> Phase {
        let end = self.end_time();
        let before_active = self.delay.min(end).max(0.0);
        let active_after = (self.delay + self.active_duration()).min(end).max(0.0);
        if local < before_active {
            Phase::Before
        } else if local >= active_after {
            Phase::After
        } else {
            Phase::Active
        }
    }

    /// What the effect gives at the local time `local` (§4.7).
    pub fn progress(&self, local: f64) -> Progress {
        let phase = self.phase(local);
        let active_duration = self.active_duration();
        let backwards = matches!(
            self.fill,
            AnimationFillMode::Backwards | AnimationFillMode::Both
        );
        let forwards = matches!(
            self.fill,
            AnimationFillMode::Forwards | AnimationFillMode::Both
        );
        // §4.7.2.
        let active_time = match phase {
            Phase::Before => backwards.then(|| (local - self.delay).max(0.0)),
            Phase::Active => Some(local - self.delay),
            Phase::After => forwards.then(|| (local - self.delay).min(active_duration).max(0.0)),
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
        // §4.7.3: the overall progress.
        let overall = if self.duration == 0.0 {
            if phase == Phase::Before {
                0.0
            } else {
                self.iterations
            }
        } else {
            active_time / self.duration
        };
        // §4.7.4: the simple iteration progress, the end of an iteration
        // being one of it rather than zero of the next.
        let mut simple = if overall.is_infinite() {
            0.0
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
        // §4.7.5: the current iteration.
        let iteration = if phase == Phase::After && self.iterations.is_infinite() {
            f64::INFINITY
        } else if simple == 1.0 {
            overall.floor() - 1.0
        } else {
            overall.floor()
        };
        // §4.7.6: the direction.
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
        // §4.7.7: the before flag and the easing.
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

/// The keyframes around a progress among one property's (§5.3.4).
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
/// falls in (§5.3.4): beyond either end, the end's first or last
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

/// The most animations and transitions a document runs together: past it
/// a new one does not start. No page meets it; one built to exhaust the
/// engine does.
pub const MAX_ANIMATIONS: usize = 65_536;

/// What an animation animates: an element, or its `::before`, `::after`
/// or `::marker`.
pub type Target = (NodeId, Option<PseudoElement>);

/// Where an animation's current time comes from (Web Animations §4.4).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Clock {
    /// Playing, its start time not resolved yet: it shows the time `from`
    /// until the next frame starts it from there, as the pending play
    /// task resolves at the next animation frame (§4.4.4).
    Pending { from: f64 },
    /// Playing since the timeline's time `start`.
    Running { start: f64 },
    /// Paused at the time `at`, its hold time.
    Held { at: f64 },
}

/// A CSS animation (Animations 1 §2): the `@keyframes` name it runs, the
/// timing the element's properties give it, and its clock.
#[derive(Debug, Clone, PartialEq)]
struct CssAnimation {
    name: Arc<str>,
    /// Its effect's timing, its easing linear: an animation's timing
    /// function is its keyframes'.
    timing: Timing,
    /// The easing of a keyframe that declares none: the animation's
    /// `animation-timing-function`.
    easing: Easing,
    clock: Clock,
    /// What the last sample gave, for the next frame to compare with.
    last: Sampled,
}

/// What a sample gives that a style is computed from.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sampled {
    phase: Phase,
    progress: Option<f64>,
    before: bool,
}

impl From<Progress> for Sampled {
    fn from(progress: Progress) -> Self {
        Sampled {
            phase: progress.phase,
            progress: progress.progress,
            before: progress.before,
        }
    }
}

/// An animation an element's style asks for (Animations 1 §2): an item
/// of its `animation-name` that names keyframes in force, with the other
/// properties' items at its place.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Wanted {
    pub(crate) name: Arc<str>,
    pub(crate) timing: Timing,
    pub(crate) easing: Easing,
    pub(crate) paused: bool,
}

/// One of a target's animations at the timeline's time: the keyframes
/// it runs, its default keyframe easing and what its effect gives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sample<'a> {
    pub(crate) name: &'a str,
    pub(crate) easing: &'a Easing,
    pub(crate) progress: Progress,
}

/// The animations a document runs, and its timeline (M22.2, M22.3).
#[derive(Debug, Clone, Default)]
pub struct Animations {
    /// The timeline's time, in seconds from its origin.
    time: f64,
    /// Each target's CSS animations, in its `animation-name`'s order,
    /// the last of which wins a property they share.
    css: HashMap<Target, Vec<CssAnimation>>,
    /// How many there are together, at most [`MAX_ANIMATIONS`].
    count: usize,
}

impl Animations {
    /// None, at the time zero.
    pub fn new() -> Self {
        Animations::default()
    }

    /// The timeline's time, in seconds.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Moves the timeline to `time`, in seconds: never back, and never
    /// to a time that is not a number.
    pub fn set_time(&mut self, time: f64) {
        if time > self.time && time.is_finite() {
            self.time = time;
        }
    }

    /// How many animations there are.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Starts every animation waiting for a frame at the timeline's time,
    /// the frame's: its current time goes on from what it showed.
    pub fn start_pending(&mut self) {
        let time = self.time;
        for animation in self.css.values_mut().flatten() {
            if let Clock::Pending { from } = animation.clock {
                animation.clock = Clock::Running { start: time - from };
            }
        }
    }

    /// Samples every animation at the timeline's time: the elements
    /// whose animations give something other than at the last sample,
    /// which a frame restyles, each once, in order.
    pub fn sample(&mut self) -> Vec<NodeId> {
        let time = self.time;
        let mut changed = Vec::new();
        for (&(node, _), animations) in &mut self.css {
            for animation in animations {
                let now = Sampled::from(animation.timing.progress(current_time(animation, time)));
                if now != animation.last {
                    animation.last = now;
                    changed.push(node);
                }
            }
        }
        changed.sort_unstable();
        changed.dedup();
        changed
    }

    /// Whether a frame is wanted now: an animation waits for one to
    /// start it, or plays in its active phase, its values changing with
    /// the time.
    pub fn needs_frame(&self) -> bool {
        let time = self.time;
        self.css.values().flatten().any(|a| match a.clock {
            Clock::Pending { .. } => true,
            Clock::Running { start } => a.timing.phase(time - start) == Phase::Active,
            Clock::Held { .. } => false,
        })
    }

    /// The timeline's time at which a frame is next wanted when none is
    /// now: the soonest end of a playing animation's delay; `None` when
    /// no playing animation is in its delay.
    pub fn next_change(&self) -> Option<f64> {
        let time = self.time;
        self.css
            .values()
            .flatten()
            .filter_map(|a| match a.clock {
                Clock::Running { start } if a.timing.phase(time - start) == Phase::Before => {
                    let end = a.timing.end_time();
                    Some(start + a.timing.delay.min(end).max(0.0))
                }
                _ => None,
            })
            .min_by(f64::total_cmp)
    }

    /// Keeps the animations of the elements `keep` says, cancelling the
    /// others': those of elements no longer in the document.
    pub fn retain(&mut self, mut keep: impl FnMut(NodeId) -> bool) {
        let count = &mut self.count;
        self.css.retain(|&(node, _), animations| {
            let kept = keep(node);
            if !kept {
                *count -= animations.len();
            }
            kept
        });
    }

    /// Whether `target` has an animation.
    pub(crate) fn animates(&self, target: Target) -> bool {
        self.css.contains_key(&target)
    }

    /// Matches `target`'s animations to `wanted`, what its style asks
    /// for now, by Animations 1 §2: the list is walked from its last item
    /// to its first, each taking the last animation of its name not
    /// taken yet, which keeps its clock and takes the item's timing and
    /// play state; an item that finds none starts an animation, waiting
    /// for the next frame, or held at zero when it is paused; an
    /// animation no item takes is cancelled. Each is sampled at the
    /// timeline's time.
    pub(crate) fn update(&mut self, target: Target, wanted: &[Wanted]) {
        let old = if wanted.is_empty() {
            match self.css.remove(&target) {
                Some(old) => old,
                None => return,
            }
        } else {
            self.css.remove(&target).unwrap_or_default()
        };
        let time = self.time;
        let mut taken = vec![false; old.len()];
        let mut new = Vec::with_capacity(wanted.len());
        for item in wanted.iter().rev() {
            let found = (0..old.len())
                .rev()
                .find(|&i| !taken[i] && old[i].name == item.name);
            let mut animation = match found {
                Some(i) => {
                    taken[i] = true;
                    old[i].clone()
                }
                None if self.count < MAX_ANIMATIONS => {
                    self.count += 1;
                    CssAnimation {
                        name: Arc::clone(&item.name),
                        timing: item.timing.clone(),
                        easing: item.easing.clone(),
                        // Held at zero below when it is paused.
                        clock: Clock::Pending { from: 0.0 },
                        last: Sampled {
                            phase: Phase::Before,
                            progress: None,
                            before: false,
                        },
                    }
                }
                None => continue,
            };
            animation.timing = item.timing.clone();
            animation.easing = item.easing.clone();
            animation.clock = match (animation.clock, item.paused) {
                (Clock::Running { start }, true) => Clock::Held { at: time - start },
                (Clock::Pending { from }, true) => Clock::Held { at: from },
                (Clock::Held { at }, false) => Clock::Pending { from: at },
                (clock, _) => clock,
            };
            animation.last = animation
                .timing
                .progress(current_time(&animation, time))
                .into();
            new.push(animation);
        }
        self.count -= taken.iter().filter(|&&t| !t).count();
        if !new.is_empty() {
            new.reverse();
            self.css.insert(target, new);
        }
    }

    /// What `target`'s animations give at the timeline's time, in their
    /// order.
    pub(crate) fn samples(&self, target: Target) -> Vec<Sample<'_>> {
        let time = self.time;
        self.css.get(&target).map_or_else(Vec::new, |animations| {
            animations
                .iter()
                .map(|a| Sample {
                    name: &a.name,
                    easing: &a.easing,
                    progress: a.timing.progress(current_time(a, time)),
                })
                .collect()
        })
    }
}

/// `animation`'s current time when the timeline's is `time`.
fn current_time(animation: &CssAnimation, time: f64) -> f64 {
    match animation.clock {
        Clock::Pending { from } => from,
        Clock::Running { start } => time - start,
        Clock::Held { at } => at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(delay: f64, duration: f64, iterations: f64) -> Timing {
        Timing {
            delay,
            duration,
            iterations,
            direction: AnimationDirection::Normal,
            fill: AnimationFillMode::None,
            easing: Easing::Linear,
        }
    }

    #[test]
    fn phases_and_active_times_follow_the_fill_mode() {
        let mut t = timing(1.0, 2.0, 1.0);
        assert_eq!(t.phase(0.5), Phase::Before);
        assert_eq!(t.phase(1.0), Phase::Active);
        assert_eq!(t.phase(2.999), Phase::Active);
        assert_eq!(t.phase(3.0), Phase::After);
        // No fill: nothing outside the active interval.
        assert_eq!(t.progress(0.5).progress, None);
        assert_eq!(t.progress(3.5).progress, None);
        assert_eq!(t.progress(2.0).progress, Some(0.5));
        // Backwards: the start in the delay; forwards: the end after.
        t.fill = AnimationFillMode::Backwards;
        assert_eq!(t.progress(0.5).progress, Some(0.0));
        assert!(t.progress(0.5).before);
        t.fill = AnimationFillMode::Both;
        assert_eq!(t.progress(4.0).progress, Some(1.0));
        assert_eq!(t.progress(4.0).active_time, Some(2.0));
        // A negative delay starts part way, as `animation: m 1s paused
        // -0.25s` shows a quarter in the reference browser.
        let late = timing(-0.25, 1.0, 1.0);
        assert_eq!(late.progress(0.0).progress, Some(0.25));
        assert_eq!(late.end_time(), 0.75);
        // A delay past the whole: in its after phase from the start.
        let gone = timing(-5.0, 1.0, 2.0);
        assert_eq!(gone.phase(0.0), Phase::After);
    }

    #[test]
    fn iterations_end_at_one_and_alternate() {
        let mut t = timing(0.0, 1.0, 3.0);
        t.fill = AnimationFillMode::Forwards;
        // The end of the last iteration is one of it, not zero of the
        // next.
        let end = t.progress(3.0);
        assert_eq!((end.progress, end.iteration), (Some(1.0), Some(2.0)));
        assert_eq!(t.progress(1.25).iteration, Some(1.0));
        assert_eq!(t.progress(1.0).progress, Some(0.0));
        // A fractional count ends part way.
        t.iterations = 2.5;
        assert_eq!(t.progress(10.0).progress, Some(0.5));
        assert_eq!(t.progress(10.0).iteration, Some(2.0));
        // Directions.
        t.iterations = 4.0;
        let at = |direction, local| {
            let mut t = t.clone();
            t.direction = direction;
            t.progress(local).progress.unwrap()
        };
        assert_eq!(at(AnimationDirection::Reverse, 0.25), 0.75);
        assert_eq!(at(AnimationDirection::Alternate, 0.25), 0.25);
        assert_eq!(at(AnimationDirection::Alternate, 1.25), 0.75);
        assert_eq!(at(AnimationDirection::AlternateReverse, 0.25), 0.75);
        assert_eq!(at(AnimationDirection::AlternateReverse, 1.25), 0.25);
        // An even count alternated ends at its start.
        assert_eq!(at(AnimationDirection::Alternate, 10.0), 0.0);
        // Infinite: never after, its active duration infinite.
        let forever = timing(0.0, 1.0, f64::INFINITY);
        assert_eq!(forever.end_time(), f64::INFINITY);
        assert_eq!(forever.progress(1e9 + 0.5).progress, Some(0.5));
        assert_eq!(forever.phase(1e300), Phase::Active);
    }

    #[test]
    fn a_zero_duration_jumps_to_its_end() {
        let mut t = timing(1.0, 0.0, 2.0);
        t.fill = AnimationFillMode::Both;
        assert_eq!(t.active_duration(), 0.0);
        assert_eq!(t.progress(0.5).progress, Some(0.0));
        assert_eq!(t.progress(1.0).phase, Phase::After);
        assert_eq!(t.progress(1.0).progress, Some(1.0));
        assert_eq!(t.progress(1.0).iteration, Some(1.0));
        // Zero times infinity is not computed.
        let mut never = timing(0.0, 0.0, f64::INFINITY);
        never.fill = AnimationFillMode::Forwards;
        assert_eq!(never.active_duration(), 0.0);
        let p = never.progress(1.0);
        // Its end is one of an iteration (§4.7.4), the infinite one.
        assert_eq!((p.progress, p.iteration), (Some(1.0), Some(f64::INFINITY)));
    }

    #[test]
    fn the_effect_easing_takes_the_before_flag() {
        // `steps(4, jump-start)` in a backwards-filled delay is at its
        // bottom step, as the reference browser shows it.
        let mut t = timing(1.0, 1.0, 1.0);
        t.fill = AnimationFillMode::Backwards;
        t.easing = Easing::Steps {
            count: 4,
            position: crate::easing::StepPosition::JumpStart,
        };
        assert_eq!(t.progress(0.5).progress, Some(0.0));
        assert_eq!(t.progress(1.0).progress, Some(0.25));
        // Reversed, the flag is set in the after phase.
        let mut back = t.clone();
        back.direction = AnimationDirection::Reverse;
        back.fill = AnimationFillMode::Forwards;
        let after = back.progress(5.0);
        assert!(after.before);
    }

    fn wanted(name: &str, delay: f64, paused: bool) -> Wanted {
        Wanted {
            name: Arc::from(name),
            timing: Timing {
                delay,
                duration: 1.0,
                iterations: 1.0,
                direction: AnimationDirection::Normal,
                fill: AnimationFillMode::None,
                easing: Easing::Linear,
            },
            easing: Easing::Linear,
            paused,
        }
    }

    fn node(index: usize) -> NodeId {
        let mut document = dom::Document::new();
        let mut last = document.root();
        for _ in 0..=index {
            last = document.create_element("div").expect("room");
        }
        last
    }

    /// The current times of `target`'s animations, by name.
    fn times(animations: &Animations, target: Target) -> Vec<(String, f64)> {
        animations.css.get(&target).map_or_else(Vec::new, |list| {
            list.iter()
                .map(|a| (a.name.to_string(), current_time(a, animations.time)))
                .collect()
        })
    }

    #[test]
    fn animations_wait_for_a_frame_and_hold_while_paused() {
        let target = (node(0), None);
        let mut animations = Animations::new();
        animations.set_time(10.0);
        animations.update(target, &[wanted("a", 0.0, false)]);
        // Made between frames, it shows its start until the next frame.
        assert!(animations.needs_frame());
        animations.set_time(12.0);
        assert_eq!(times(&animations, target), vec![("a".into(), 0.0)]);
        animations.start_pending();
        animations.set_time(12.25);
        assert_eq!(times(&animations, target), vec![("a".into(), 0.25)]);
        // Paused, it holds its time; played again, it goes on from there
        // once a frame starts it.
        animations.update(target, &[wanted("a", 0.0, true)]);
        animations.set_time(20.0);
        assert_eq!(times(&animations, target), vec![("a".into(), 0.25)]);
        assert!(!animations.needs_frame());
        animations.update(target, &[wanted("a", 0.0, false)]);
        animations.set_time(21.0);
        animations.start_pending();
        animations.set_time(21.5);
        assert_eq!(times(&animations, target), vec![("a".into(), 0.75)]);
        // One made paused holds zero.
        let other = (node(1), Some(PseudoElement::Before));
        animations.update(other, &[wanted("b", -0.5, true)]);
        animations.set_time(30.0);
        animations.start_pending();
        animations.set_time(30.5);
        assert_eq!(times(&animations, other), vec![("b".into(), 0.0)]);
        assert_eq!(animations.samples(other)[0].progress.progress, Some(0.5));
        // The timeline never goes back, nor to a time that is not one.
        animations.set_time(1.0);
        animations.set_time(f64::NAN);
        animations.set_time(f64::INFINITY);
        assert_eq!(animations.time(), 30.5);
    }

    #[test]
    fn a_changed_name_list_keeps_the_animations_it_still_names() {
        let target = (node(0), None);
        let mut animations = Animations::new();
        animations.update(target, &[wanted("a", 0.0, false), wanted("b", 0.0, false)]);
        animations.start_pending();
        animations.set_time(0.5);
        animations.update(target, &[wanted("c", 0.0, false)]);
        animations.update(target, &[]);
        assert!(animations.is_empty());
        animations.update(target, &[wanted("a", 0.0, false), wanted("b", 0.0, false)]);
        animations.start_pending();
        animations.set_time(0.75);
        // Reordered, each keeps its clock and takes its new place.
        animations.update(target, &[wanted("b", 0.0, false), wanted("a", 0.0, false)]);
        assert_eq!(
            times(&animations, target),
            vec![("b".into(), 0.25), ("a".into(), 0.25)]
        );
        // A name twice: the last takes the animation, the first is new.
        animations.update(target, &[wanted("a", 0.0, false), wanted("a", 0.0, false)]);
        assert_eq!(
            times(&animations, target),
            vec![("a".into(), 0.0), ("a".into(), 0.25)]
        );
        assert_eq!(animations.len(), 2);
        // A name dropped is cancelled.
        animations.update(target, &[wanted("a", 0.0, false)]);
        assert_eq!(times(&animations, target), vec![("a".into(), 0.25)]);
        assert_eq!(animations.len(), 1);
        animations.retain(|_| false);
        assert!(animations.is_empty() && animations.css.is_empty());
    }

    #[test]
    fn a_frame_restyles_the_targets_whose_animations_moved_and_waits_out_delays() {
        let (a, b) = ((node(0), None), (node(1), None));
        let mut animations = Animations::new();
        animations.update(a, &[wanted("a", 0.0, false)]);
        animations.start_pending();
        animations.set_time(0.25);
        animations.update(b, &[wanted("b", 2.0, false)]);
        animations.start_pending();
        // `a` moved on since it was made; nothing since then.
        assert_eq!(animations.sample(), vec![a.0]);
        assert_eq!(animations.sample(), Vec::<NodeId>::new());
        animations.set_time(0.5);
        // `a` plays; `b` waits out its delay with nothing to show.
        assert_eq!(animations.sample(), vec![a.0]);
        assert!(animations.needs_frame());
        // `b` started at 0.25.
        assert_eq!(animations.next_change(), Some(2.25));
        animations.set_time(1.5);
        // `a` ended: once more, for its end.
        assert_eq!(animations.sample(), vec![a.0]);
        assert!(!animations.needs_frame());
        assert_eq!(animations.sample(), Vec::<NodeId>::new());
        animations.set_time(2.25);
        assert_eq!(animations.sample(), vec![b.0]);
        assert!(animations.needs_frame());
        assert_eq!(animations.next_change(), None);
    }

    #[test]
    fn no_more_than_the_limit_run() {
        let mut animations = Animations::new();
        let mut document = dom::Document::new();
        let names: Vec<Wanted> = (0..64)
            .map(|i| wanted(&format!("n{i}"), 0.0, false))
            .collect();
        for _ in 0..MAX_ANIMATIONS / 64 {
            let node = document.create_element("div").expect("room");
            animations.update((node, None), &names);
        }
        assert_eq!(animations.len(), MAX_ANIMATIONS);
        let node = document.create_element("div").expect("room");
        animations.update((node, None), &names);
        assert_eq!(animations.len(), MAX_ANIMATIONS);
        assert!(!animations.animates((node, None)));
    }

    #[test]
    fn intervals_follow_web_animations() {
        let offsets = [0.0, 0.5, 1.0];
        assert_eq!(
            interval(&offsets, 0.25),
            Interval::Between {
                from: 0,
                to: 1,
                distance: 0.5
            }
        );
        assert_eq!(
            interval(&offsets, 0.5),
            Interval::Between {
                from: 1,
                to: 2,
                distance: 0.0
            }
        );
        assert_eq!(
            interval(&offsets, 1.0),
            Interval::Between {
                from: 1,
                to: 2,
                distance: 1.0
            }
        );
        // Past the ends: the end two, extrapolated.
        assert_eq!(
            interval(&offsets, -0.5),
            Interval::Between {
                from: 0,
                to: 1,
                distance: -1.0
            }
        );
        assert_eq!(
            interval(&offsets, 1.5),
            Interval::Between {
                from: 1,
                to: 2,
                distance: 2.0
            }
        );
        // Two keyframes at an end: the first or the last alone past it.
        let jumps = [0.0, 0.0, 1.0, 1.0];
        assert_eq!(interval(&jumps, -0.1), Interval::At(0));
        assert_eq!(interval(&jumps, 1.0), Interval::At(3));
        assert_eq!(
            interval(&jumps, 0.0),
            Interval::Between {
                from: 1,
                to: 2,
                distance: 0.0
            }
        );
        // Two at one inner offset: the later from there.
        let step = [0.0, 0.5, 0.5, 1.0];
        assert_eq!(
            interval(&step, 0.5),
            Interval::Between {
                from: 2,
                to: 3,
                distance: 0.0
            }
        );
    }
}
