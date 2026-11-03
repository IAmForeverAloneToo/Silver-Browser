//! The Web Animations API's calls (D75): what the bindings' `Animation`,
//! `KeyframeEffect`, `CSSAnimation`, `CSSTransition` and
//! `DocumentTimeline` objects ask of the document's animations, each a
//! JSON op answered with JSON. Every answer carries the notices the
//! store's procedures owe the promises the script holds, which the
//! bindings carry out before they return. The store is the document's,
//! lent for the run (`scripts::Lent`); the ops that read what styles say
//! bring them up to date first, as the reference browser flushes style
//! before it answers.

use devtools::Json;
use dom::NodeId;
use js::Handle;
use style::animation::player::Notice;
use style::animation::{
    EffectInput, KeyframesSource, ScriptError, TimingInput, TransitionValue, pseudo_element,
    pseudo_element_name,
};
use style::animation::{TimelineKind, TimelineSource};
use style::computed::{Axis, Composite, Edge, Inset, LengthPercentage};
use style::keyframes::{ScriptKeyframe, ScriptKeyframes, parse_easing, script_declaration};
use style::selector_matching::PseudoElement;
use style::{CascadeContext, ComputedStyle, Recascade};

use super::DocumentHost;

/// Why an op failed: the exception's name and its message.
type Failure = (&'static str, String);

/// The store's refusal as an op's failure.
fn refused(error: ScriptError) -> Failure {
    (error.name(), error.message())
}

/// A number as the bindings read it: a finite one as a number, the
/// others as the strings JavaScript writes them, which JSON cannot.
fn number(value: f64) -> Json {
    if value.is_finite() {
        Json::Number(value)
    } else if value.is_nan() {
        Json::from("NaN")
    } else if value > 0.0 {
        Json::from("Infinity")
    } else {
        Json::from("-Infinity")
    }
}

/// The axis an op names, `block` where it names none.
fn op_axis(op: &Json) -> Result<Axis, Failure> {
    match op.get("axis").and_then(Json::as_str) {
        None => Ok(Axis::Block),
        Some(word) => Axis::from_name(word).ok_or_else(|| type_error("The axis is not valid.")),
    }
}

/// A time, or `null`.
fn time(value: Option<f64>) -> Json {
    value.map_or(Json::Null, number)
}

/// A number the bindings wrote: a number, or a string for one JSON
/// cannot hold.
fn read_number(value: &Json) -> Option<f64> {
    match value {
        Json::Number(n) => Some(*n),
        Json::String(s) => match s.as_str() {
            "Infinity" => Some(f64::INFINITY),
            "-Infinity" => Some(f64::NEG_INFINITY),
            "NaN" => Some(f64::NAN),
            _ => None,
        },
        _ => None,
    }
}

fn type_error(message: impl Into<String>) -> Failure {
    ("TypeError", message.into())
}

/// The pseudo-element's selector, as `pseudoElement` gives it.
fn pseudo_name(pseudo: Option<PseudoElement>) -> Json {
    pseudo
        .and_then(pseudo_element_name)
        .map_or(Json::Null, Json::from)
}

/// The notice as the bindings name it.
fn notice_name(notice: Notice) -> &'static str {
    match notice {
        Notice::ReadyNew => "readyNew",
        Notice::ReadyResolved => "readyResolved",
        Notice::ReadyRejected => "readyRejected",
        Notice::FinishedNew => "finishedNew",
        Notice::FinishedRejected => "finishedRejected",
        Notice::FinishNotify { sync: true } => "notifySync",
        Notice::FinishNotify { sync: false } => "notify",
    }
}

/// The notices as the bindings take them: number and name pairs.
pub(crate) fn notices_json(notices: Vec<(u64, Notice)>) -> Json {
    Json::Array(
        notices
            .into_iter()
            .map(|(id, notice)| {
                Json::Array(vec![
                    Json::Number(id as f64),
                    Json::from(notice_name(notice)),
                ])
            })
            .collect(),
    )
}

/// The timing members a script gave, as the store takes them.
fn timing_input(value: Option<&Json>) -> TimingInput {
    let Some(value) = value else {
        return TimingInput::default();
    };
    let get = |name: &str| value.get(name);
    TimingInput {
        delay: get("delay").and_then(read_number),
        end_delay: get("endDelay").and_then(read_number),
        fill: get("fill").and_then(Json::as_str).map(str::to_owned),
        iteration_start: get("iterationStart").and_then(read_number),
        iterations: get("iterations").and_then(read_number),
        duration: get("duration").map(|d| match d.as_str() {
            Some("auto") => None,
            _ => Some(read_number(d).unwrap_or(f64::NAN)),
        }),
        direction: get("direction").and_then(Json::as_str).map(str::to_owned),
        easing: get("easing").and_then(Json::as_str).map(str::to_owned),
    }
}

/// The keyframes the bindings processed, checked and made the store's:
/// a `TypeError` for an easing that does not parse, a composite operation
/// that is none, or offsets out of range or of order.
fn keyframes_input(value: Option<&Json>) -> Result<ScriptKeyframes, Failure> {
    let mut given = Vec::new();
    for frame in value.and_then(Json::as_array).unwrap_or_default() {
        let offset = frame.get("offset").and_then(read_number);
        let text = frame
            .get("easing")
            .and_then(Json::as_str)
            .unwrap_or("linear");
        let easing = parse_easing(text)
            .ok_or_else(|| type_error(format!("'{text}' is not a valid value for easing")))?;
        let composite = match frame.get("composite").and_then(Json::as_str) {
            None | Some("auto") => None,
            Some(word) => Some(
                Composite::from_name(word)
                    .ok_or_else(|| type_error(format!("'{word}' is not a composite operation.")))?,
            ),
        };
        let mut declarations: Vec<css::Declaration> = Vec::new();
        for pair in frame
            .get("values")
            .and_then(Json::as_array)
            .unwrap_or_default()
        {
            let (Some(name), Some(value)) = (
                pair.as_array()
                    .and_then(|p| p.first())
                    .and_then(Json::as_str),
                pair.as_array()
                    .and_then(|p| p.get(1))
                    .and_then(Json::as_str),
            ) else {
                continue;
            };
            if let Some(declaration) = script_declaration(name, value) {
                declarations.retain(|d| d.name != declaration.name);
                declarations.push(declaration);
            }
        }
        given.push(ScriptKeyframe {
            offset,
            easing,
            composite,
            declarations,
        });
    }
    ScriptKeyframes::new(given).map_err(|error| type_error(error.0))
}

/// What the bindings' `getKeyframes()` gives of one keyframe: its offset
/// (`None` for none), its computed offset, its easing and composite
/// operation, and its properties' values by CSS name.
struct KeyframeOut {
    offset: Option<f64>,
    computed: f64,
    easing: String,
    composite: &'static str,
    values: Vec<(String, String)>,
}

impl KeyframeOut {
    fn json(self) -> Json {
        Json::object([
            ("offset", time(self.offset)),
            ("computedOffset", number(self.computed)),
            ("easing", Json::from(self.easing)),
            ("composite", Json::from(self.composite)),
            (
                "values",
                Json::Array(
                    self.values
                        .into_iter()
                        .map(|(name, value)| Json::Array(vec![Json::from(name), Json::from(value)]))
                        .collect(),
                ),
            ),
        ])
    }
}

/// A script keyframe's values as the reference browser writes them back:
/// a property's specified value serialized, a custom property's tokens.
fn script_values(keyframe: &ScriptKeyframe) -> Vec<(String, String)> {
    keyframe
        .declarations
        .iter()
        .map(|d| {
            let value = if d.name.starts_with("--") {
                style::serialize::tokens(&d.value)
            } else {
                style::serialize::specified(&d.value)
            };
            (d.name.clone(), value)
        })
        .collect()
}

impl DocumentHost {
    /// The Web Animations API's call `request` (D75), as JSON: its result
    /// or its error, and the notices the store owes the promises the
    /// script holds.
    pub(super) fn animation_call(&mut self, request: &str) -> String {
        let touches = self.animations.borrow().touched().1;
        let answer = Json::parse(request)
            .map_err(|_| type_error("a malformed animation call"))
            .and_then(|op| self.animation_op(&op));
        // A change of what an animation gives its target is a change of
        // the layout scripts ask of, which is kept by the document's
        // version alone.
        if self.animations.borrow().touched().1 != touches {
            self.laid_out = None;
        }
        let notices = notices_json(self.animations.get_mut().take_notices());
        match answer {
            Ok(result) => Json::object([("result", result), ("notices", notices)]).write(),
            Err((name, message)) => Json::object([
                (
                    "error",
                    Json::object([("name", Json::from(name)), ("message", Json::from(message))]),
                ),
                ("notices", notices),
            ])
            .write(),
        }
    }

    /// The element a handle names, and the pseudo-element a selector
    /// names, as an effect's target.
    fn animation_target(
        &self,
        target: Option<&Json>,
        pseudo: Option<&Json>,
    ) -> Result<(Option<NodeId>, Option<PseudoElement>), Failure> {
        let pseudo = pseudo_element(pseudo.and_then(Json::as_str)).map_err(refused)?;
        let Some(handle) = target.and_then(Json::as_f64) else {
            return Ok((None, pseudo));
        };
        let node = NodeId::from_bits(handle as u64)
            .ok_or_else(|| type_error("The target is not an element."))?;
        Ok((Some(node), pseudo))
    }

    /// Animation op `op`'s result.
    fn animation_op(&mut self, op: &Json) -> Result<Json, Failure> {
        let name = op.get("op").and_then(Json::as_str).unwrap_or_default();
        let id = op.get("id").and_then(Json::as_u64).unwrap_or(0);
        let effect = op.get("effect").and_then(Json::as_u64).unwrap_or(0);
        // What styles say comes first where the reference browser
        // flushes them: for the document's animations, for what a CSS
        // animation or transition is, which the changes before the call
        // may have started, changed or cancelled, and for the values
        // `commitStyles()` writes. A script's own animation's call
        // produces no style change event (section 5.7).
        let css = {
            let animations = self.animations.borrow();
            let of_animation = |id: u64| animations.state(id).is_some_and(|s| s.kind != "script");
            match name {
                "animations" | "animation.commit" => true,
                _ if name.starts_with("animation.") => of_animation(id),
                _ if name.starts_with("effect.") => animations
                    .effect_animation(effect)
                    .is_some_and(of_animation),
                _ => false,
            }
        };
        if css {
            let _ = self.styles_now();
        }
        let store = |r: Result<(), ScriptError>| r.map(|()| Json::Null).map_err(refused);
        match name {
            "properties" => Ok(Json::Array(
                animatable_properties()
                    .into_iter()
                    .map(Json::from)
                    .collect(),
            )),
            "offset" => {
                let text = op.get("text").and_then(Json::as_str).unwrap_or_default();
                style::keyframes::parse_offset(text)
                    .map(Json::from)
                    .ok_or_else(|| type_error(format!("'{text}' is not a valid offset.")))
            }
            "easings" => {
                for text in op
                    .get("easings")
                    .and_then(Json::as_array)
                    .unwrap_or_default()
                {
                    let text = text.as_str().unwrap_or_default();
                    if parse_easing(text).is_none() {
                        return Err(type_error(format!(
                            "'{text}' is not a valid value for easing"
                        )));
                    }
                }
                Ok(Json::Null)
            }
            "effect.create" | "animate" => {
                let (target, pseudo) = self.animation_target(op.get("target"), op.get("pseudo"))?;
                let keyframes = keyframes_input(op.get("keyframes"))?;
                let composite = op
                    .get("composite")
                    .and_then(Json::as_str)
                    .and_then(Composite::from_name)
                    .unwrap_or_default();
                let timing = timing_input(op.get("timing"));
                let animations = self.animations.get_mut();
                let effect = animations
                    .create_effect(EffectInput {
                        target,
                        pseudo,
                        keyframes,
                        timing,
                        composite,
                    })
                    .map_err(refused)?;
                if name == "effect.create" {
                    return Ok(Json::from(effect));
                }
                let timeline = op
                    .get("timeline")
                    .and_then(Json::as_u64)
                    .map(|t| t as usize);
                let animation = animations
                    .create_animation(Some(effect), timeline)
                    .ok_or_else(|| ("NotSupportedError", "Too many animations.".to_owned()))?;
                Ok(Json::object([
                    ("effect", Json::from(effect)),
                    ("animation", Json::from(animation)),
                ]))
            }
            "effect.copy" => {
                let keyframes = self.keyframes_of(effect)?;
                let given = keyframes
                    .into_iter()
                    .map(|k| ScriptKeyframe {
                        offset: k.offset,
                        easing: parse_easing(&k.easing).unwrap_or_default(),
                        composite: Composite::from_name(k.composite),
                        declarations: k
                            .values
                            .iter()
                            .filter_map(|(n, v)| script_declaration(n, v))
                            .collect(),
                    })
                    .collect();
                let keyframes = ScriptKeyframes::new(given).map_err(|e| type_error(e.0))?;
                self.animations
                    .get_mut()
                    .copy_effect(effect, keyframes)
                    .map(Json::from)
                    .ok_or_else(|| type_error("The effect is gone."))
            }
            "effect.hold" => Ok(Json::Bool(self.animations.get_mut().hold_effect(effect))),
            "effect.release" => {
                self.animations.get_mut().release_effect(effect);
                Ok(Json::Null)
            }
            "effect.timing" | "effect.computed" => {
                let animations = self.animations.borrow();
                let Some((timing, duration_auto, fill_auto)) = animations.timing(effect) else {
                    return Ok(Json::Null);
                };
                let fill = if fill_auto {
                    "auto"
                } else {
                    fill_name(timing.fill)
                };
                let mut members = vec![
                    ("delay", number(timing.delay)),
                    ("endDelay", number(timing.end_delay)),
                    ("fill", Json::from(fill)),
                    ("iterationStart", number(timing.iteration_start)),
                    ("iterations", number(timing.iterations)),
                    (
                        "duration",
                        if duration_auto && name == "effect.timing" {
                            Json::from("auto")
                        } else {
                            number(timing.duration)
                        },
                    ),
                    ("direction", Json::from(direction_name(timing.direction))),
                    (
                        "easing",
                        Json::from(style::serialize::easing(&timing.easing)),
                    ),
                ];
                if name == "effect.computed" {
                    // On a progress-based timeline, the timing it plays by:
                    // proportions of the range it is attached to.
                    let computed = animations
                        .computed_timing(effect)
                        .expect("its timing was there");
                    if computed.progress_based {
                        members[5] = ("duration", number(computed.timing.duration));
                    }
                    let computed = animations
                        .computed_timing(effect)
                        .expect("its timing was there");
                    // A computed timing's fill is the one it takes.
                    members[2] = ("fill", Json::from(fill_name(timing.fill)));
                    members.extend([
                        // An effect on a document timeline starts at its
                        // start (Web Animations 2's computed start time).
                        ("startTime", number(0.0)),
                        ("endTime", number(computed.end_time)),
                        ("activeDuration", number(computed.active_duration)),
                        ("localTime", time(computed.local_time)),
                        ("progress", time(computed.progress)),
                        ("currentIteration", time(computed.current_iteration)),
                        ("progressBased", Json::Bool(computed.progress_based)),
                    ]);
                }
                Ok(Json::object(members))
            }
            "effect.update" => store(
                self.animations
                    .get_mut()
                    .update_timing(effect, &timing_input(op.get("timing"))),
            ),
            "effect.target" => {
                let target = self.animations.borrow().effect_target(effect);
                let pseudo = self.animations.borrow().effect_pseudo(effect);
                Ok(Json::object([
                    (
                        "target",
                        target
                            .filter(|(node, _)| self.document.contains(*node))
                            .map_or(Json::Null, |(node, _)| Json::from(node.to_bits())),
                    ),
                    ("pseudo", pseudo_name(pseudo)),
                ]))
            }
            "effect.setTarget" => {
                let (target, pseudo) = self.animation_target(op.get("target"), op.get("pseudo"))?;
                self.animations
                    .get_mut()
                    .set_effect_target(effect, target, pseudo);
                Ok(Json::Null)
            }
            "effect.keyframes" => Ok(Json::Array(
                self.keyframes_of(effect)?
                    .into_iter()
                    .map(KeyframeOut::json)
                    .collect(),
            )),
            "effect.setKeyframes" => {
                let keyframes = keyframes_input(op.get("keyframes"))?;
                self.animations.get_mut().set_keyframes(effect, keyframes);
                Ok(Json::Null)
            }
            "effect.composite" => Ok(Json::from(
                self.animations
                    .borrow()
                    .composite(effect)
                    .unwrap_or_default()
                    .name(),
            )),
            "effect.setComposite" => {
                if let Some(composite) = op
                    .get("composite")
                    .and_then(Json::as_str)
                    .and_then(Composite::from_name)
                {
                    self.animations.get_mut().set_composite(effect, composite);
                }
                Ok(Json::Null)
            }
            "animation.create" => {
                let effect = op.get("effect").and_then(Json::as_u64);
                let timeline = op
                    .get("timeline")
                    .and_then(Json::as_u64)
                    .map(|t| t as usize);
                self.animations
                    .get_mut()
                    .create_animation(effect, timeline)
                    .map(Json::from)
                    .ok_or_else(|| ("NotSupportedError", "Too many animations.".to_owned()))
            }
            "animation.hold" => {
                if !self.animations.get_mut().hold(id) {
                    return Ok(Json::Null);
                }
                Ok(self.animation_state(id))
            }
            "animation.state" => Ok(self.animation_state(id)),
            "animation.release" => {
                self.animations.get_mut().release(id);
                Ok(Json::Null)
            }
            "animation.play" => store(self.animations.get_mut().play(id)),
            "animation.pause" => store(self.animations.get_mut().pause(id)),
            "animation.finish" => store(self.animations.get_mut().finish(id)),
            "animation.cancel" => store(self.animations.get_mut().cancel(id)),
            "animation.reverse" => store(self.animations.get_mut().reverse(id)),
            "animation.persist" => {
                self.animations.get_mut().persist(id);
                Ok(Json::Null)
            }
            "animation.rate" => {
                let rate = op.get("rate").and_then(read_number).unwrap_or(1.0);
                store(self.animations.get_mut().set_playback_rate(id, rate))
            }
            "animation.updateRate" => {
                let rate = op.get("rate").and_then(read_number).unwrap_or(1.0);
                store(self.animations.get_mut().update_playback_rate(id, rate))
            }
            "animation.currentTime" => {
                let at = op.get("time").and_then(read_number);
                store(self.animations.get_mut().set_current_time(id, at))
            }
            "animation.startTime" => {
                let at = op.get("time").and_then(read_number);
                store(self.animations.get_mut().set_start_time(id, at))
            }
            "animation.timeline" => {
                let timeline = op
                    .get("timeline")
                    .and_then(Json::as_u64)
                    .map(|t| t as usize);
                store(self.animations.get_mut().set_script_timeline(id, timeline))
            }
            "animation.effect" => {
                let effect = op.get("effect").and_then(Json::as_u64);
                self.animations.get_mut().set_effect(id, effect);
                Ok(Json::Null)
            }
            "animation.notify" => Ok(Json::Bool(self.animations.get_mut().notify_finished(id))),
            "animation.commit" => self.commit_styles(id),
            "animations" => {
                let target = op
                    .get("target")
                    .and_then(Json::as_f64)
                    .and_then(|h| self.node(Handle(h as u64)).ok());
                let subtree = op.get("subtree").and_then(Json::as_bool).unwrap_or(false);
                // A pseudo-element named: its animations alone.
                let pseudo = match op.get("pseudo").and_then(Json::as_str) {
                    Some(text) => Some(pseudo_element(Some(text)).map_err(refused)?),
                    None => None,
                };
                Ok(Json::Array(
                    self.relevant_animations(target, subtree, pseudo)
                        .into_iter()
                        .map(Json::from)
                        .collect(),
                ))
            }
            "timeline.create" => {
                let origin = op.get("origin").and_then(read_number).unwrap_or(0.0);
                Ok(Json::from(
                    self.animations.get_mut().create_timeline(origin) as u64,
                ))
            }
            "timeline.time" => {
                let timeline = op.get("timeline").and_then(Json::as_u64).unwrap_or(0) as usize;
                Ok(time(
                    self.animations.borrow().timeline_current_time(timeline),
                ))
            }
            // The `ScrollTimeline` constructor (Scroll-driven Animations 1
            // section 2.2): its source an element or none.
            "timeline.scroll" => {
                let source = self.op_node(op.get("source"));
                let axis = op_axis(op)?;
                let made =
                    self.animations
                        .get_mut()
                        .create_progress_timeline(TimelineKind::Scroll {
                            source: TimelineSource::Element(source),
                            axis,
                        });
                self.measure_timeline(made);
                Ok(Json::from(made as u64))
            }
            // The `ViewTimeline` constructor (section 3.2), its inset as
            // `view-timeline-inset` takes one.
            "timeline.view" => {
                let subject = self.op_node(op.get("subject"));
                let axis = op_axis(op)?;
                let inset = match op.get("inset").and_then(Json::as_str) {
                    Some(text) => style::cascade::timelines::computed_insets(&css::tokenize(text))
                        .ok_or_else(|| type_error("The inset is not valid."))?,
                    None => [Inset::Auto, Inset::Auto],
                };
                let made = self
                    .animations
                    .get_mut()
                    .create_progress_timeline(TimelineKind::View {
                        subject,
                        axis,
                        inset,
                    });
                self.measure_timeline(made);
                Ok(Json::from(made as u64))
            }
            // What a timeline is: its kind, and a progress-based one's
            // source, subject, axis and the offsets of its whole range.
            "timeline.info" => {
                let timeline = op.get("timeline").and_then(Json::as_u64).unwrap_or(0) as usize;
                // One no frame has measured yet, a style's named timeline
                // among them, is measured now, as the reference browser
                // resolves it with the style.
                let unmeasured = {
                    let animations = self.animations.borrow();
                    animations
                        .timeline_kind(timeline)
                        .is_some_and(|k| k.progress_based())
                        && animations.measured(timeline).is_none()
                };
                if unmeasured {
                    self.measure_timeline(timeline);
                }
                let animations = self.animations.borrow();
                let Some(kind) = animations.timeline_kind(timeline) else {
                    return Ok(Json::Null);
                };
                let measured = animations.measured(timeline);
                let node = |n: Option<NodeId>| {
                    n.filter(|n| self.document.contains(*n))
                        .map_or(Json::Null, |n| Json::from(n.to_bits()))
                };
                let root = self.document.document_element();
                let (name, axis, source, subject) = match kind {
                    TimelineKind::Document { .. } => ("document", Axis::Block, None, None),
                    TimelineKind::Scroll { source, axis } => (
                        "scroll",
                        *axis,
                        match source {
                            TimelineSource::Element(element) => *element,
                            // The viewport's is the document's scrolling
                            // element, which stands for it before a frame
                            // has measured a scroll container nearer.
                            TimelineSource::Root => root,
                            TimelineSource::Nearest(_) => {
                                measured.map_or(root, |m| m.scroller.or(root))
                            }
                            TimelineSource::Own(node) => {
                                measured.map_or(Some(*node), |m| m.scroller.or(root))
                            }
                        },
                        None,
                    ),
                    TimelineKind::View { subject, axis, .. } => (
                        "view",
                        *axis,
                        measured.and_then(|m| m.scroller.or(root)),
                        *subject,
                    ),
                    TimelineKind::Named { .. } => match measured {
                        Some(m) if m.view => ("view", m.axis, m.scroller.or(root), m.subject),
                        Some(m) => ("scroll", m.axis, m.scroller.or(root), None),
                        None => ("none", Axis::Block, None, None),
                    },
                };
                let cover = measured.map(|m| m.ranges[0]);
                Ok(Json::object([
                    ("kind", Json::from(name)),
                    (
                        "named",
                        Json::Bool(matches!(kind, TimelineKind::Named { .. })),
                    ),
                    ("axis", Json::from(axis.name())),
                    ("source", node(source)),
                    ("subject", node(subject)),
                    ("start", time(cover.map(|c| c.0))),
                    ("end", time(cover.map(|c| c.1))),
                ]))
            }
            // `rangeStart` and `rangeEnd` set (Web Animations 2 section
            // 6.4), each as text.
            "animation.range" => {
                let edge =
                    |key: &str, end: bool| -> Result<Option<Edge<LengthPercentage>>, Failure> {
                        match op.get(key).and_then(Json::as_str) {
                            None => Ok(None),
                            Some(text) => {
                                style::cascade::timelines::computed_edge(&css::tokenize(text), end)
                                    .map(Some)
                                    .ok_or_else(|| type_error("The range is not valid."))
                            }
                        }
                    };
                let (start, end) = (edge("start", false)?, edge("end", true)?);
                self.animations.get_mut().set_range(id, start, end);
                Ok(Json::Null)
            }
            other => Err(type_error(format!("'{other}' is no animation call"))),
        }
    }

    /// Measures progress-based timeline `index` as a script made it, in
    /// the layout as it is now: a timeline has its time from when it is
    /// made, each frame then measuring it again (D75).
    fn measure_timeline(&mut self, index: usize) {
        let Some(kind) = self.animations.borrow().timeline_kind(index).cloned() else {
            return;
        };
        let (scroll_y, offsets) = self.scroll_state();
        if self.layout_now().is_none() {
            return;
        }
        let Some((_, layout)) = self.laid_out.as_ref() else {
            return;
        };
        let scene = crate::timelines::Scene {
            document: &self.document,
            styles: self.computed.as_ref().map(|cached| &cached.styles),
            layout,
            viewport: (self.window.viewport.0 as f32, self.window.viewport.1 as f32),
            scroll: (scroll_y, offsets.viewport_x),
            offsets: &offsets,
        };
        let measured = scene.measure(&kind);
        self.animations.get_mut().set_measured(index, measured);
    }

    /// The element an op's handle names, `None` for none or a node gone.
    fn op_node(&self, value: Option<&Json>) -> Option<NodeId> {
        let handle = value.and_then(Json::as_f64)?;
        self.node(Handle(handle as u64)).ok()
    }

    /// What animation `id` is, as its object reads it; `null` for none.
    fn animation_state(&self, id: u64) -> Json {
        let animations = self.animations.borrow();
        let Some(state) = animations.state(id) else {
            return Json::Null;
        };
        // Web Animations 2's overall progress: the current time through
        // the effect's end, in [0, 1].
        let end = state
            .effect
            .and_then(|e| animations.computed_timing(e))
            .map(|t| t.end_time);
        let overall = match (state.current_time, end) {
            (Some(current), Some(end)) if state.timeline.is_some() => Some(if end == 0.0 {
                if current < 0.0 { 0.0 } else { 1.0 }
            } else {
                (current / end).clamp(0.0, 1.0)
            }),
            _ => None,
        };
        Json::object([
            ("playState", Json::from(state.play_state.name())),
            ("pending", Json::Bool(state.pending)),
            ("currentTime", time(state.current_time)),
            ("startTime", time(state.start_time)),
            ("playbackRate", number(state.playback_rate)),
            ("replaceState", Json::from(state.replace_state.name())),
            (
                "timeline",
                state.timeline.map_or(Json::Null, |t| Json::from(t as u64)),
            ),
            ("effect", state.effect.map_or(Json::Null, Json::from)),
            ("readyPending", Json::Bool(state.ready_pending)),
            ("finishedResolved", Json::Bool(state.finished_resolved)),
            ("kind", Json::from(state.kind)),
            (
                "name",
                Json::from(state.name.as_deref().unwrap_or_default()),
            ),
            ("progress", Json::Bool(state.progress)),
            ("rangeStart", Json::from(state.range[0].text(false))),
            ("rangeEnd", Json::from(state.range[1].text(true))),
            ("overallProgress", time(overall)),
        ])
    }

    /// The relevant animations of the document, of `target`, or of
    /// `target` and the elements and pseudo-elements under it, in
    /// composite order (`getAnimations()`, Web Animations 1 section 6.9
    /// and 6.10): an element's own, without `subtree`, are its own alone.
    fn relevant_animations(
        &self,
        target: Option<NodeId>,
        subtree: bool,
        pseudo: Option<Option<PseudoElement>>,
    ) -> Vec<u64> {
        let document = &self.document;
        // In shadow-including tree order (D68); the document's own
        // animations are those of its tree, a shadow root's or an
        // element's subtree's those of its tree alone, none crossing into
        // a shadow tree below.
        let order: std::collections::HashMap<NodeId, usize> = document
            .shadow_including_subtree(document.root())
            .into_iter()
            .enumerate()
            .map(|(i, node)| (node, i))
            .collect();
        let root = document.root();
        let wanted = |(node, of): style::animation::Target| match target {
            None => order.contains_key(&node) && document.root_of(node) == root,
            Some(element) if pseudo.is_some() => node == element && Some(of) == pseudo,
            Some(element) if !subtree => node == element && of.is_none(),
            Some(element) => {
                order.contains_key(&node)
                    && (node == element || document.ancestors(node).any(|a| a == element))
            }
        };
        self.animations.borrow().relevant(wanted, |node| {
            order.get(&node).copied().unwrap_or(usize::MAX)
        })
    }

    /// The keyframes `getKeyframes()` gives of effect `effect`: a
    /// script's as given, a CSS animation's from its `@keyframes` rule
    /// with the implicit keyframes at 0% and 100% of the element's own
    /// values, a transition's two values.
    fn keyframes_of(&mut self, effect: u64) -> Result<Vec<KeyframeOut>, Failure> {
        let source = self.animations.borrow().keyframes_source(effect);
        let target = self.animations.borrow().effect_target(effect);
        let Some(source) = source else {
            return Ok(Vec::new());
        };
        match source {
            KeyframesSource::Script(keyframes) => Ok(keyframes
                .given
                .iter()
                .zip(&keyframes.computed_offsets)
                .map(|(k, &computed)| KeyframeOut {
                    offset: k.offset,
                    computed,
                    easing: style::serialize::easing(&k.easing),
                    composite: k.composite.map_or("auto", Composite::name),
                    values: script_values(k),
                })
                .collect()),
            KeyframesSource::Transition {
                property,
                longhand,
                start,
                end,
            } => {
                let value = |value: &TransitionValue| match (value, longhand) {
                    (TransitionValue::Style(style), Some(longhand)) => {
                        style::serialize::computed_value(style, longhand.name()).unwrap_or_default()
                    }
                    (TransitionValue::Custom(Some(tokens)), _) => style::serialize::tokens(tokens),
                    _ => String::new(),
                };
                Ok([(0.0, &start), (1.0, &end)]
                    .into_iter()
                    .map(|(offset, v)| KeyframeOut {
                        offset: Some(offset),
                        computed: offset,
                        easing: "linear".to_owned(),
                        composite: "auto",
                        values: vec![(property.to_string(), value(v))],
                    })
                    .collect())
            }
            KeyframesSource::Css { name, easing } => {
                let base = target.and_then(|(node, _)| self.base_style(node));
                let Some(computed) = self.computed.as_ref() else {
                    return Ok(Vec::new());
                };
                let Some(frames) = computed.sheets.keyframes_named(&name, &self.window.media)
                else {
                    return Ok(Vec::new());
                };
                let base = base.unwrap_or_default();
                let default_easing = style::serialize::easing(&easing);
                let mut out: Vec<KeyframeOut> = frames
                    .iter()
                    .map(|k| KeyframeOut {
                        offset: Some(k.offset),
                        computed: k.offset,
                        easing: k
                            .easing
                            .as_ref()
                            .map_or_else(|| default_easing.clone(), style::serialize::easing),
                        composite: k.composite.map_or("auto", Composite::name),
                        values: style::cascade::declared_values(&base, &k.declarations),
                    })
                    // A keyframe that animates nothing is none (an easing
                    // or a property not animatable alone).
                    .filter(|k| !k.values.is_empty())
                    .collect();
                // The properties no keyframe at an end sets take the
                // element's values there (Animations 2 section 3.2).
                let mut properties: Vec<String> = Vec::new();
                for k in &out {
                    for (property, _) in &k.values {
                        if !properties.contains(property) {
                            properties.push(property.clone());
                        }
                    }
                }
                for end in [0.0, 1.0] {
                    let missing: Vec<(String, String)> = properties
                        .iter()
                        .filter(|p| {
                            !out.iter()
                                .any(|k| k.computed == end && k.values.iter().any(|(n, _)| n == *p))
                        })
                        .map(|p| {
                            let value = if p.starts_with("--") {
                                base.custom
                                    .as_ref()
                                    .and_then(|m| m.get(p))
                                    .map(|t| style::serialize::tokens(t))
                                    .unwrap_or_default()
                            } else {
                                style::serialize::computed_value(&base, p).unwrap_or_default()
                            };
                            (p.clone(), value)
                        })
                        .collect();
                    if missing.is_empty() {
                        continue;
                    }
                    let place = out.iter().position(|k| {
                        k.computed == end && k.easing == default_easing && k.composite == "auto"
                    });
                    match place {
                        Some(at) => out[at].values.extend(missing),
                        None => {
                            let frame = KeyframeOut {
                                offset: Some(end),
                                computed: end,
                                easing: default_easing.clone(),
                                composite: "replace",
                                values: missing,
                            };
                            if end == 0.0 {
                                out.insert(0, frame);
                            } else {
                                out.push(frame);
                            }
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    /// `node`'s style without its animations and transitions: what an
    /// implicit keyframe takes.
    fn base_style(&mut self, node: NodeId) -> Option<ComputedStyle> {
        self.recascaded(node, false)
    }

    /// `node`'s style cascaded again from its rules and its parent's
    /// style, with the animations the store's limit lets through when
    /// `animated`, without any when not.
    fn recascaded(&mut self, node: NodeId, animated: bool) -> Option<ComputedStyle> {
        self.styles_now()?;
        let page = self.page.as_ref()?;
        let computed = self.computed.as_ref()?;
        let context = CascadeContext {
            viewport_width: self.window.viewport.0 as f32,
            viewport_height: self.window.viewport.1 as f32,
            fonts: None,
            language: page.language(),
            media: self.window.media,
            animations: animated.then_some(&self.animations),
            inline_styles: Some(page),
            frame_margins: self.window.frame_margins,
            top_layer: None,
        };
        let parent = self
            .document
            .parent(node)
            .and_then(|p| computed.styles.get(p))
            .cloned()
            .unwrap_or_default();
        let recascade = computed
            .sheets
            .recascader(&self.document, &computed.styles, &context);
        Some(recascade.element(node, &parent))
    }

    /// `commitStyles()` (Web Animations 1 section 6.4): the values animation
    /// `id`'s effect gives its target now, written into the target's
    /// `style` attribute, as the properties the bindings then set: their
    /// names and values. A `NoModificationAllowedError` for a target that
    /// is a pseudo-element or none, an `InvalidStateError` for one not
    /// rendered.
    fn commit_styles(&mut self, id: u64) -> Result<Json, Failure> {
        let animations = self.animations.borrow();
        let effect = animations.state(id).and_then(|s| s.effect);
        let target = effect.and_then(|e| animations.effect_target(e));
        let source = effect.and_then(|e| animations.keyframes_source(e));
        drop(animations);
        // An element with a `style` attribute: HTML's, SVG's or MathML's.
        let styled = |node: NodeId| {
            self.document.element_name(node).is_some_and(|(ns, _)| {
                matches!(
                    ns,
                    dom::Namespace::Html | dom::Namespace::Svg | dom::Namespace::MathMl
                )
            })
        };
        let Some((node, None)) = target.filter(|&(node, _)| styled(node)) else {
            return Err((
                "NoModificationAllowedError",
                "Animation target is not an element with a style attribute.".to_owned(),
            ));
        };
        let properties: Vec<String> = match source {
            Some(KeyframesSource::Script(keyframes)) => keyframes
                .longhands()
                .iter()
                .map(|l| l.name().to_owned())
                .chain(keyframes.custom().iter().map(|c| c.to_string()))
                .collect(),
            Some(KeyframesSource::Transition { property, .. }) => vec![property.to_string()],
            Some(KeyframesSource::Css { .. }) => self
                .keyframes_of(effect.unwrap_or(0))?
                .into_iter()
                .flat_map(|k| k.values.into_iter().map(|(n, _)| n))
                .collect(),
            None => Vec::new(),
        };
        let connected = self.document.contains(node) && self.connected(node);
        let mut chain: Vec<NodeId> = vec![node];
        chain.extend(
            self.document
                .ancestors(node)
                .filter(|&a| self.document.is_element(a)),
        );
        let Some(styles) = self.styles_now() else {
            return Err((
                "InvalidStateError",
                "Target element is not rendered.".to_owned(),
            ));
        };
        // Rendered: neither it nor an ancestor is `display: none`.
        let rendered = connected
            && chain.iter().all(|&n| {
                styles
                    .get(n)
                    .is_some_and(|s| s.display != style::computed::Display::None)
            });
        if !rendered {
            return Err((
                "InvalidStateError",
                "Target element is not rendered.".to_owned(),
            ));
        }
        // The effect stack up to and including the animation.
        self.animations.get_mut().commit_limit(Some(id));
        let style = self.recascaded(node, true);
        self.animations.get_mut().commit_limit(None);
        let style = style.unwrap_or_default();
        let mut pairs = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for property in properties {
            if seen.contains(&property) {
                continue;
            }
            seen.push(property.clone());
            let value = if property.starts_with("--") {
                style
                    .custom
                    .as_ref()
                    .and_then(|m| m.get(&property))
                    .map(|t| style::serialize::tokens(t))
            } else {
                style::serialize::computed_value(&style, &property)
            };
            if let Some(value) = value {
                pairs.push(Json::Array(vec![Json::from(property), Json::from(value)]));
            }
        }
        Ok(Json::object([
            ("target", Json::from(node.to_bits())),
            ("values", Json::Array(pairs)),
        ]))
    }
}

/// Every property a keyframe may name (Web Animations 1 section 6.6.1,
/// "animatable properties"): the animatable longhands and the shorthands
/// of them, by their CSS names.
fn animatable_properties() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = style::animate::Longhand::ALL
        .iter()
        .filter(|l| l.is_animatable())
        .map(|l| l.name())
        .collect();
    names.extend(style::animate::SHORTHANDS.iter().copied().filter(|name| {
        style::animate::longhands_of(name)
            .iter()
            .any(|l| l.is_animatable())
    }));
    names
}

/// A fill mode's keyword.
fn fill_name(fill: style::computed::AnimationFillMode) -> &'static str {
    use style::computed::AnimationFillMode as F;
    match fill {
        F::None => "none",
        F::Forwards => "forwards",
        F::Backwards => "backwards",
        F::Both => "both",
    }
}

/// A playback direction's keyword.
fn direction_name(direction: style::computed::AnimationDirection) -> &'static str {
    use style::computed::AnimationDirection as D;
    match direction {
        D::Normal => "normal",
        D::Reverse => "reverse",
        D::Alternate => "alternate",
        D::AlternateReverse => "alternate-reverse",
    }
}
