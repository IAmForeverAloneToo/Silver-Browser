use super::*;

fn timing(delay: f64, duration: f64, iterations: f64) -> Timing {
    Timing {
        delay,
        duration,
        iterations,
        ..Timing::default()
    }
}

#[test]
fn phases_and_active_times_follow_the_fill_mode() {
    let mut t = timing(1.0, 2.0, 1.0);
    assert_eq!(t.phase(0.5), Phase::Before);
    assert_eq!(t.phase(1.0), Phase::Active);
    assert_eq!(t.phase(2.999), Phase::Active);
    assert_eq!(t.phase(3.0), Phase::After);
    // Played backwards, a boundary belongs to the phase it leaves.
    assert_eq!(t.phase_in(1.0, true), Phase::Before);
    assert_eq!(t.phase_in(3.0, true), Phase::Active);
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
    // The end delay lengthens the end time, not the active interval.
    let mut ending = timing(1.0, 2.0, 1.0);
    ending.end_delay = 2.0;
    assert_eq!(ending.end_time(), 5.0);
    assert_eq!(ending.phase(3.5), Phase::After);
    ending.end_delay = -10.0;
    assert_eq!(ending.end_time(), 0.0);
    assert_eq!(ending.phase(0.0), Phase::After);
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
fn the_iteration_start_shifts_the_progress_and_the_iteration() {
    let mut t = timing(0.0, 1.0, 2.0);
    t.iteration_start = 0.5;
    t.fill = AnimationFillMode::Both;
    assert_eq!(t.progress(0.0).progress, Some(0.5));
    assert_eq!(t.progress(0.75).iteration, Some(1.0));
    assert_eq!(t.progress(0.75).progress, Some(0.25));
    // It ends half way through its third iteration.
    let end = t.progress(2.0);
    assert_eq!((end.progress, end.iteration), (Some(0.5), Some(2.0)));
    // An infinite count's after phase: the start's fraction.
    let mut forever = timing(0.0, 0.0, f64::INFINITY);
    forever.iteration_start = 2.25;
    forever.fill = AnimationFillMode::Forwards;
    assert_eq!(forever.progress(1.0).progress, Some(0.25));
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
    // Its end is one of an iteration (section 4.7.4), the infinite one.
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
            delay: delay * 1000.0,
            duration: 1000.0,
            ..Timing::default()
        },
        easing: Easing::Linear,
        paused,
        composite: Composite::Replace,
        longhands: Arc::from(Vec::new()),
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

/// The current times of `target`'s CSS animations, by name, in
/// seconds.
fn times(animations: &Animations, target: Target) -> Vec<(String, f64)> {
    animations.css.get(&target).map_or_else(Vec::new, |list| {
        list.iter()
            .map(|id| {
                let state = animations.state(*id).expect("there");
                (
                    state.name.expect("a name").to_string(),
                    state.current_time.unwrap_or(f64::NAN) / 1000.0,
                )
            })
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
    // Paused, it holds the time of the frame its pause waited for; played
    // again, it goes on from there once a frame starts it.
    animations.update(target, &[wanted("a", 0.0, true)]);
    animations.set_time(12.5);
    animations.start_pending();
    animations.set_time(20.0);
    assert_eq!(times(&animations, target), vec![("a".into(), 0.5)]);
    assert!(!animations.needs_frame());
    animations.update(target, &[wanted("a", 0.0, false)]);
    animations.set_time(21.0);
    animations.start_pending();
    animations.set_time(21.25);
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
    // Reordered, each keeps its playback and takes its new place.
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
    assert!(animations.effects.is_empty() && animations.targeted.is_empty());
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
    assert_eq!(animations.create_animation(None, Some(0)), None);
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

/// An effect of `keyframes` declarations of opacity on `target`.
fn opacity_effect(animations: &mut Animations, target: Target, duration: f64) -> u64 {
    let keyframes = crate::keyframes::ScriptKeyframes::new(vec![
        crate::keyframes::ScriptKeyframe {
            offset: None,
            easing: Easing::Linear,
            composite: None,
            declarations: vec![crate::keyframes::script_declaration("opacity", "0").unwrap()],
        },
        crate::keyframes::ScriptKeyframe {
            offset: None,
            easing: Easing::Linear,
            composite: None,
            declarations: vec![crate::keyframes::script_declaration("opacity", "1").unwrap()],
        },
    ])
    .unwrap();
    animations
        .create_effect(EffectInput {
            target: Some(target.0),
            pseudo: target.1,
            keyframes,
            timing: TimingInput {
                duration: Some(Some(duration)),
                fill: Some("forwards".into()),
                ..TimingInput::default()
            },
            composite: Composite::Replace,
        })
        .unwrap()
}

#[test]
fn a_scripts_animation_plays_from_the_next_frame_and_owes_its_promises() {
    let target = (node(0), None);
    let mut animations = Animations::new();
    animations.set_time(1.0);
    let effect = opacity_effect(&mut animations, target, 1000.0);
    let id = animations.create_animation(Some(effect), Some(0)).unwrap();
    assert_eq!(animations.state(id).unwrap().play_state, PlayState::Idle);
    animations.play(id).unwrap();
    assert_eq!(animations.take_notices(), vec![(id, Notice::ReadyNew)]);
    assert!(animations.state(id).unwrap().pending);
    // Its value at once is its start's.
    assert_eq!(animations.samples(target)[0].progress.progress, Some(0.0));
    assert!(animations.needs_frame());
    animations.set_time(2.0);
    animations.start_pending();
    assert_eq!(animations.take_notices(), vec![(id, Notice::ReadyResolved)]);
    assert_eq!(animations.state(id).unwrap().start_time, Some(2000.0));
    animations.set_time(2.5);
    assert_eq!(animations.samples(target)[0].progress.progress, Some(0.5));
    // Finished, its notification is due in a microtask, which queues
    // its `finish` event.
    animations.set_time(3.5);
    animations.sample();
    assert_eq!(
        animations.take_notices(),
        vec![(id, Notice::FinishNotify { sync: false })]
    );
    assert!(animations.notify_finished(id));
    assert!(!animations.notify_finished(id));
    let events = animations.take_playback_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, PlaybackKind::Finish);
    assert_eq!(events[0].current_time, Some(1000.0));
    assert_eq!(events[0].timeline_time, Some(3500.0));
    // Filling forwards, it is still relevant to its target.
    assert_eq!(animations.relevant(|t| t == target, |_| 0), vec![id]);
    // Cancelled: idle, its finished promise rejected, its `cancel` event
    // queued, and nothing applied.
    animations.cancel(id).unwrap();
    assert_eq!(
        animations.take_notices(),
        vec![(id, Notice::FinishedRejected)]
    );
    assert_eq!(
        animations.take_playback_events()[0].kind,
        PlaybackKind::Cancel
    );
    assert!(animations.samples(target).is_empty());
    assert!(animations.relevant(|_| true, |_| 0).is_empty());
}

#[test]
fn a_later_filling_animation_of_the_same_properties_removes_an_earlier() {
    let target = (node(0), None);
    let mut animations = Animations::new();
    let first = opacity_effect(&mut animations, target, 100.0);
    let a = animations.create_animation(Some(first), Some(0)).unwrap();
    let second = opacity_effect(&mut animations, target, 100.0);
    let b = animations.create_animation(Some(second), Some(0)).unwrap();
    let third = opacity_effect(&mut animations, target, 100.0);
    let c = animations.create_animation(Some(third), Some(0)).unwrap();
    for id in [a, b, c] {
        animations.play(id).unwrap();
    }
    animations.persist(b);
    animations.start_pending();
    animations.set_time(1.0);
    animations.sample();
    animations.remove_replaced();
    let state = |id| animations.state(id).unwrap().replace_state;
    assert_eq!(state(a), ReplaceState::Removed);
    assert_eq!(state(b), ReplaceState::Persisted);
    assert_eq!(state(c), ReplaceState::Active);
    let events = animations.take_playback_events();
    assert!(
        events
            .iter()
            .any(|e| e.animation == a && e.kind == PlaybackKind::Remove)
    );
    // The removed one gives no value; the others do, in order.
    assert_eq!(animations.samples(target).len(), 2);
}

#[test]
fn a_css_animation_a_script_paused_keeps_paused_and_its_timing_set_stays() {
    let target = (node(0), None);
    let mut animations = Animations::new();
    animations.update(target, &[wanted("a", 0.0, false)]);
    animations.start_pending();
    let id = animations.css[&target][0];
    animations.pause(id).unwrap();
    animations.set_time(0.25);
    animations.start_pending();
    // The style says paused, then running: the script's pause stands.
    animations.update(target, &[wanted("a", 0.0, true)]);
    animations.update(target, &[wanted("a", 0.0, false)]);
    animations.set_time(0.5);
    animations.start_pending();
    assert_eq!(times(&animations, target), vec![("a".into(), 0.25)]);
    // A duration a script set is not the style's.
    let effect = animations.state(id).unwrap().effect.unwrap();
    animations
        .update_timing(
            effect,
            &TimingInput {
                duration: Some(Some(5000.0)),
                ..TimingInput::default()
            },
        )
        .unwrap();
    animations.update(target, &[wanted("a", 0.0, false)]);
    assert_eq!(animations.timing(effect).unwrap().0.duration, 5000.0);
    // Cancelled by the script, it is idle but still its element's.
    animations.cancel(id).unwrap();
    animations.update(target, &[wanted("a", 0.0, false)]);
    assert_eq!(animations.state(id).unwrap().play_state, PlayState::Idle);
    assert_eq!(animations.css[&target], vec![id]);
}

#[test]
fn timing_a_script_gives_is_checked() {
    let target = (node(0), None);
    let mut animations = Animations::new();
    let effect = opacity_effect(&mut animations, target, 100.0);
    let bad = |input: TimingInput| animations.clone().update_timing(effect, &input).is_err();
    assert!(bad(TimingInput {
        iteration_start: Some(-1.0),
        ..TimingInput::default()
    }));
    assert!(bad(TimingInput {
        iterations: Some(f64::NAN),
        ..TimingInput::default()
    }));
    assert!(bad(TimingInput {
        duration: Some(Some(-1.0)),
        ..TimingInput::default()
    }));
    assert!(bad(TimingInput {
        easing: Some("jumpy".into()),
        ..TimingInput::default()
    }));
    assert!(bad(TimingInput {
        easing: Some("ease, linear".into()),
        ..TimingInput::default()
    }));
    assert!(!bad(TimingInput {
        iterations: Some(f64::INFINITY),
        easing: Some("steps(2, start)".into()),
        ..TimingInput::default()
    }));
    // Nothing changes when one member is refused.
    let mut copy = animations.clone();
    let _ = copy.update_timing(
        effect,
        &TimingInput {
            delay: Some(5.0),
            duration: Some(Some(-1.0)),
            ..TimingInput::default()
        },
    );
    assert_eq!(copy.timing(effect).unwrap().0.delay, 0.0);
}

#[test]
fn script_keyframes_compute_their_missing_offsets_and_check_their_order() {
    use crate::keyframes::{ScriptKeyframe, ScriptKeyframes};
    let frame = |offset: Option<f64>| ScriptKeyframe {
        offset,
        easing: Easing::Linear,
        composite: None,
        declarations: Vec::new(),
    };
    let offsets = |given: Vec<Option<f64>>| {
        ScriptKeyframes::new(given.into_iter().map(frame).collect()).map(|k| k.computed_offsets)
    };
    assert_eq!(offsets(vec![None]), Ok(vec![1.0]));
    assert_eq!(offsets(vec![None, None, None]), Ok(vec![0.0, 0.5, 1.0]));
    assert_eq!(
        offsets(vec![None, Some(0.6), None, None]),
        Ok(vec![0.0, 0.6, 0.8, 1.0])
    );
    assert!(offsets(vec![Some(0.5), Some(0.25)]).is_err());
    assert!(offsets(vec![Some(1.5)]).is_err());
    // Values a property does not take are dropped where they are read.
    assert!(crate::keyframes::script_declaration("left", "invalid").is_none());
    assert!(crate::keyframes::script_declaration("left", "10px; top: 0").is_none());
    assert!(crate::keyframes::script_declaration("animation-name", "x").is_none());
    assert!(crate::keyframes::script_declaration("--x", "anything").is_some());
    assert!(crate::keyframes::script_declaration("margin", "1px 2px").is_some());
}
