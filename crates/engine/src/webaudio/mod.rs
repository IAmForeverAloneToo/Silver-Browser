//! Web Audio in the engine (M40 step 3, `docs/design/audio-video.md`):
//! a document's contexts, their graphs (the `audio` crate's) and what the
//! bootstrap's objects ask of them through one native, `native.audio`,
//! whose operations are JSON with any samples as bytes after it. An
//! `AudioContext` renders on the renderer's mixer while it runs, an
//! `OfflineAudioContext` on a thread of its own; `decodeAudioData()`
//! decodes on a thread of its own. What they tell the document comes
//! back through its inbox as `Task::Audio`.

pub(crate) mod mixer;
pub(crate) mod offline;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use audio::nodes::{self, Kind, filters::BiquadType, panner::DistanceModel, shaper::Oversample};
use audio::{AudioData, Buffer, Event, Graph, Interpretation, Mode, NodeId, PeriodicWave, Rate};
use devtools::Json;

use crate::scripts::workers::{Inbox, WorkerMessage};

/// The most bytes of samples a document's audio keeps in the engine
/// (its contexts' buffers, decoded or rendered, and those given to
/// nodes): past it, a new one is refused.
pub(crate) const MAX_AUDIO_BUFFER_BYTES: usize = 512 << 20;
/// The most live contexts a document has; past them, a new one is
/// refused.
pub(crate) const MAX_AUDIO_CONTEXTS: usize = 32;
/// The most bytes `decodeAudioData()` decodes to.
pub(crate) const MAX_DECODE_AUDIO_BYTES: usize = 512 << 20;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An operation's failure, as the bootstrap throws it.
struct Failure {
    name: &'static str,
    message: String,
}

fn fail<T>(name: &'static str, message: impl Into<String>) -> Result<T, Failure> {
    Err(Failure {
        name,
        message: message.into(),
    })
}

/// Samples as little-endian `f32` bytes, channel after channel.
fn samples_to_bytes(channels: &[Vec<f32>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(channels.iter().map(Vec::len).sum::<usize>() * 4);
    for channel in channels {
        for v in channel {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// `bytes` as `count` channels of `length` frames.
fn bytes_to_samples(bytes: &[u8], count: usize, length: usize) -> Option<Vec<Vec<f32>>> {
    if bytes.len() != count.checked_mul(length)?.checked_mul(4)? {
        return None;
    }
    Some(
        floats(bytes)
            .chunks(length.max(1))
            .take(count)
            .map(<[f32]>::to_vec)
            .collect(),
    )
}

/// Little-endian `f32` bytes as samples.
fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Suspended,
    Running,
    Closed,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Suspended => "suspended",
            State::Running => "running",
            State::Closed => "closed",
        }
    }
}

/// What a context's threads tell its document.
#[derive(Clone)]
struct Teller {
    inbox: Inbox,
    context: u32,
}

impl Teller {
    fn tell(&self, members: Vec<(&str, Json)>, bytes: Vec<u8>) {
        let mut all = vec![("ctx", Json::from(self.context))];
        all.extend(members);
        let _ = self
            .inbox
            .send(WorkerMessage::Audio(Json::object(all).write(), bytes));
    }

    /// The context is in `state` now: the promises waiting for it
    /// settle, and its `statechange` fires if it changed.
    fn state(&self, state: State) {
        self.tell(
            vec![
                ("type", Json::from("state")),
                ("state", Json::from(state.name())),
            ],
            Vec::new(),
        );
    }

    fn ended(&self, nodes: &[NodeId]) {
        for node in nodes {
            self.tell(
                vec![("type", Json::from("ended")), ("node", Json::from(*node))],
                Vec::new(),
            );
        }
    }
}

struct Context {
    graph: Arc<Mutex<Graph>>,
    rate: f32,
    channels: usize,
    state: State,
    offline: Option<offline::Offline>,
    running: Option<mixer::Running>,
    /// A `resume()` waiting for the user's activation.
    waiting: bool,
    /// The buffers the engine keeps a copy of, by the id the bootstrap
    /// knows them by (decoded or rendered here, or given to a node).
    buffers: Arc<Mutex<HashMap<u32, Buffer>>>,
    waves: HashMap<u32, Arc<PeriodicWave>>,
    next: u32,
    teller: Teller,
}

impl Context {
    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn run(&mut self) {
        if self.running.is_none() {
            let teller = self.teller.clone();
            self.running = Some(mixer::run(
                &self.graph,
                Box::new(move |ended| teller.ended(&ended)),
            ));
        }
        self.state = State::Running;
    }
}

/// A document's Web Audio.
#[derive(Default)]
pub(crate) struct WebAudio {
    contexts: HashMap<u32, Context>,
    next: u32,
}

/// What a call needs of its document: its inbox, and whether the user
/// has activated it (sticky activation).
pub(crate) struct Env {
    pub(crate) inbox: Inbox,
    pub(crate) activated: bool,
    /// The sound of the media element a `MediaElementAudioSourceNode`
    /// being made takes.
    pub(crate) element: Option<Arc<crate::playback::ElementSound>>,
}

fn number(op: &Json, name: &str) -> Option<f64> {
    op.get(name).and_then(Json::as_f64)
}

fn whole(op: &Json, name: &str) -> Result<u32, Failure> {
    number(op, name)
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= f64::from(u32::MAX))
        .map(|v| v as u32)
        .ok_or_else(|| Failure {
            name: "TypeError",
            message: format!("'{name}' is missing"),
        })
}

fn text<'a>(op: &'a Json, name: &str) -> &'a str {
    op.get(name).and_then(Json::as_str).unwrap_or("")
}

impl WebAudio {
    /// Whether an `AudioContext` is open: its document is not kept by
    /// the back-forward cache, so that its sound stops as it goes.
    pub(crate) fn sounding(&self) -> bool {
        self.contexts
            .values()
            .any(|c| c.offline.is_none() && c.state != State::Closed)
    }

    /// Whether a context waits for the user's activation to start.
    pub(crate) fn waiting(&self) -> bool {
        self.contexts.values().any(|c| c.waiting)
    }

    /// The contexts waiting for activation, started: the user activated
    /// the document. Each tells its document it runs.
    pub(crate) fn activated(&mut self) {
        for context in self.contexts.values_mut().filter(|c| c.waiting) {
            context.waiting = false;
            context.run();
            context.teller.state(State::Running);
        }
    }

    /// One operation of `native.audio`: its answer, `{"result": ...}` or
    /// `{"error": {"name", "message"}}`, and the bytes it gives.
    pub(crate) fn call(&mut self, text: &str, bytes: Vec<u8>, env: &Env) -> (String, Vec<u8>) {
        let answer = Json::parse(text)
            .map_err(|_| Failure {
                name: "TypeError",
                message: "a malformed operation".to_owned(),
            })
            .and_then(|op| self.operate(&op, bytes, env));
        match answer {
            Ok((result, bytes)) => (Json::object([("result", result)]).write(), bytes),
            Err(e) => (
                Json::object([(
                    "error",
                    Json::object([
                        ("name", Json::from(e.name)),
                        ("message", Json::from(e.message)),
                    ]),
                )])
                .write(),
                Vec::new(),
            ),
        }
    }

    fn context(&mut self, op: &Json) -> Result<&mut Context, Failure> {
        let id = whole(op, "ctx")?;
        self.contexts.get_mut(&id).ok_or_else(|| Failure {
            name: "InvalidStateError",
            message: "the context is gone".to_owned(),
        })
    }

    fn operate(
        &mut self,
        op: &Json,
        bytes: Vec<u8>,
        env: &Env,
    ) -> Result<(Json, Vec<u8>), Failure> {
        let name = text(op, "op");
        if name == "newContext" {
            return self.new_context(op, env);
        }
        let activated = env.activated;
        let context = self.context(op)?;
        let empty = || Ok((Json::Null, Vec::new()));
        match name {
            "newNode" => new_node(context, op, env.element.as_ref()),
            "connect" | "connectParam" => {
                let (from, output, to, input) = (
                    whole(op, "from")?,
                    whole(op, "output")?,
                    whole(op, "to")?,
                    whole(op, "input")?,
                );
                let mut graph = lock(&context.graph);
                let ok = if name == "connect" {
                    graph.connect(from, output as usize, to, input as usize)
                } else {
                    graph.connect_param(from, output as usize, to, input as usize)
                };
                if !ok {
                    return fail(
                        "IndexSizeError",
                        "the output or input index is out of range",
                    );
                }
                empty()
            }
            "disconnect" => {
                let from = whole(op, "from")?;
                let output = number(op, "output").map(|v| v as usize);
                let to = number(op, "to").map(|v| v as NodeId);
                let input = number(op, "input").map(|v| v as usize);
                let param = op.get("param").and_then(Json::as_bool).unwrap_or(false);
                let removed = lock(&context.graph).disconnect(from, output, to, input, param);
                Ok((Json::from(removed as u64), Vec::new()))
            }
            "channels" => {
                let node = whole(op, "node")?;
                let mut graph = lock(&context.graph);
                let Some(n) = graph.node_mut(node) else {
                    return empty();
                };
                if let Some(count) = number(op, "count") {
                    n.channel_count = count as usize;
                }
                match text(op, "mode") {
                    "max" => n.mode = Mode::Max,
                    "clamped-max" => n.mode = Mode::ClampedMax,
                    "explicit" => n.mode = Mode::Explicit,
                    _ => {}
                }
                match text(op, "interpretation") {
                    "speakers" => n.interpretation = Interpretation::Speakers,
                    "discrete" => n.interpretation = Interpretation::Discrete,
                    _ => {}
                }
                empty()
            }
            "param" => param_op(context, op, &bytes),
            "set" => set_op(context, op),
            "start" | "stop" => {
                let node = whole(op, "node")?;
                let when = number(op, "when").unwrap_or(0.0);
                let mut graph = lock(&context.graph);
                let Some(n) = graph.node_mut(node) else {
                    return empty();
                };
                let schedule = match &mut n.kind {
                    Kind::Constant(s) => s,
                    Kind::Oscillator(o) => &mut o.schedule,
                    Kind::BufferSource(b) => {
                        if name == "start" {
                            b.offset = number(op, "offset").unwrap_or(0.0).max(0.0);
                            b.duration = number(op, "duration").filter(|d| d.is_finite());
                        }
                        &mut b.schedule
                    }
                    _ => return empty(),
                };
                if name == "start" {
                    schedule.start = Some(when);
                } else {
                    schedule.stop = Some(when);
                }
                empty()
            }
            "buffer" => buffer_op(context, op, bytes),
            "wave" => {
                let n = whole(op, "length")? as usize;
                let all = floats(&bytes);
                if all.len() != 2 * n {
                    return fail("TypeError", "the wave's coefficients");
                }
                let normalize = !op
                    .get("disableNormalization")
                    .and_then(Json::as_bool)
                    .unwrap_or(false);
                let wave = Arc::new(PeriodicWave::new(
                    &all[..n],
                    &all[n..],
                    normalize,
                    context.rate,
                ));
                let id = context.id();
                context.waves.insert(id, wave);
                Ok((Json::from(id), Vec::new()))
            }
            "time" => Ok((Json::from(lock(&context.graph).current_time()), Vec::new())),
            "resume" => {
                if context.state == State::Closed {
                    return fail("InvalidStateError", "Cannot resume a closed AudioContext.");
                }
                // An offline context's rendering stops at its suspensions
                // on its own thread: resuming lets it go on.
                if let Some(offline) = &context.offline {
                    if !offline.started {
                        return fail(
                            "InvalidStateError",
                            "cannot resume an offline context that has not started",
                        );
                    }
                    offline.resume();
                    context.state = State::Running;
                    context.teller.state(State::Running);
                    return Ok((Json::from("running"), Vec::new()));
                }
                if context.state == State::Running {
                    return Ok((Json::from("running"), Vec::new()));
                }
                if !activated {
                    context.waiting = true;
                    return Ok((Json::from("waiting"), Vec::new()));
                }
                context.run();
                context.teller.state(State::Running);
                Ok((Json::from("running"), Vec::new()))
            }
            "suspend" => {
                if context.state == State::Closed {
                    return fail("InvalidStateError", "Cannot suspend a closed AudioContext.");
                }
                if let Some(offline) = &context.offline {
                    let time = number(op, "time").unwrap_or(0.0);
                    let rate = f64::from(context.rate);
                    let frame = ((time * rate) / audio::QUANTUM as f64).ceil() as u64
                        * audio::QUANTUM as u64;
                    let now = lock(&context.graph).frame();
                    if frame as usize >= offline.length {
                        return fail(
                            "InvalidStateError",
                            format!(
                                "cannot schedule a suspend at frame {frame} ({time} seconds) because it is greater than or equal to the total render duration of {} frames",
                                offline.length
                            ),
                        );
                    }
                    if offline.started && frame < now {
                        return fail(
                            "InvalidStateError",
                            format!(
                                "cannot schedule a suspend at frame {frame} ({time} seconds) because it is earlier than the current frame of {now} ({} seconds)",
                                now as f64 / rate
                            ),
                        );
                    }
                    if !offline.suspend_at(frame) {
                        return fail(
                            "InvalidStateError",
                            format!(
                                "cannot schedule more than one suspend at frame {frame} ({time} seconds)"
                            ),
                        );
                    }
                    return Ok((Json::from(frame), Vec::new()));
                }
                context.running = None;
                context.waiting = false;
                context.state = State::Suspended;
                context.teller.state(State::Suspended);
                Ok((Json::from("suspended"), Vec::new()))
            }
            "close" => {
                context.running = None;
                context.waiting = false;
                context.offline = None;
                context.state = State::Closed;
                context.teller.state(State::Closed);
                Ok((Json::from("closed"), Vec::new()))
            }
            "render" => {
                let Some(offline) = context.offline.as_mut() else {
                    return fail("InvalidStateError", "not an offline context");
                };
                if offline.started {
                    return fail(
                        "InvalidStateError",
                        "cannot call startRendering more than once",
                    );
                }
                // Running is told before the rendering starts: a short
                // render's suspension or end, told from its thread, comes
                // after it (macOS's CI saw them first, M41).
                context.state = State::Running;
                context.teller.state(State::Running);
                let Some(offline) = context.offline.as_mut() else {
                    return fail("InvalidStateError", "not an offline context");
                };
                let teller = context.teller.clone();
                let buffers = context.buffers.clone();
                let rate = context.rate;
                let id = {
                    context.next += 1;
                    context.next
                };
                offline.start(
                    context.graph.clone(),
                    context.channels,
                    Box::new(move |report| match report {
                        offline::Report::Suspended(frame) => teller.tell(
                            vec![
                                ("type", Json::from("suspended")),
                                ("frame", Json::from(frame)),
                            ],
                            Vec::new(),
                        ),
                        offline::Report::Ended(nodes) => teller.ended(&nodes),
                        offline::Report::Done(channels) => {
                            let length = channels.first().map_or(0, Vec::len);
                            let bytes = samples_to_bytes(&channels);
                            lock(&buffers).insert(
                                id,
                                Arc::new(AudioData {
                                    rate,
                                    channels: channels.clone(),
                                }),
                            );
                            teller.tell(
                                vec![
                                    ("type", Json::from("rendered")),
                                    ("id", Json::from(id)),
                                    ("rate", Json::from(f64::from(rate))),
                                    ("channels", Json::from(channels.len() as u64)),
                                    ("length", Json::from(length as u64)),
                                ],
                                bytes,
                            );
                        }
                    }),
                );
                empty()
            }
            "decode" => {
                let promise = whole(op, "promise")?;
                let teller = context.teller.clone();
                let buffers = context.buffers.clone();
                let rate = context.rate;
                let id = context.id();
                let _ = std::thread::Builder::new()
                    .name("decodeAudioData".to_owned())
                    .spawn(move || {
                        let decoded = media::decode_all(&bytes, MAX_DECODE_AUDIO_BYTES / 4);
                        match decoded {
                            Ok((from, count, samples)) => {
                                let count = usize::from(count.max(1));
                                let resampled = media::resample(&samples, count, from, rate as u32);
                                let length = resampled.len() / count;
                                let channels: Vec<Vec<f32>> = (0..count)
                                    .map(|c| {
                                        resampled.iter().skip(c).step_by(count).copied().collect()
                                    })
                                    .collect();
                                let bytes = samples_to_bytes(&channels);
                                lock(&buffers).insert(id, Arc::new(AudioData { rate, channels }));
                                teller.tell(
                                    vec![
                                        ("type", Json::from("decoded")),
                                        ("promise", Json::from(promise)),
                                        ("id", Json::from(id)),
                                        ("rate", Json::from(f64::from(rate))),
                                        ("channels", Json::from(count as u64)),
                                        ("length", Json::from(length as u64)),
                                    ],
                                    bytes,
                                );
                            }
                            Err(_) => teller.tell(
                                vec![
                                    ("type", Json::from("decodeError")),
                                    ("promise", Json::from(promise)),
                                ],
                                Vec::new(),
                            ),
                        }
                    });
                empty()
            }
            "release" => {
                let id = whole(op, "id")?;
                lock(&context.buffers).remove(&id);
                empty()
            }
            "analyser" => analyser_op(context, op),
            "response" => response_op(context, op, &bytes),
            "reduction" => {
                let node = whole(op, "node")?;
                let graph = lock(&context.graph);
                let reduction = match graph.node(node).map(|n| &n.kind) {
                    Some(Kind::Compressor(c)) => c.reduction,
                    _ => 0.0,
                };
                Ok((Json::from(f64::from(reduction)), Vec::new()))
            }
            other => fail("TypeError", format!("no operation '{other}'")),
        }
    }

    fn new_context(&mut self, op: &Json, env: &Env) -> Result<(Json, Vec<u8>), Failure> {
        let live = self
            .contexts
            .values()
            .filter(|c| c.state != State::Closed)
            .count();
        if live >= MAX_AUDIO_CONTEXTS {
            return fail("NotSupportedError", "too many audio contexts");
        }
        let offline = op.get("offline").and_then(Json::as_bool).unwrap_or(false);
        let rate = number(op, "rate").map_or(mixer::OUTPUT_RATE_F32, |r| r as f32);
        let channels = number(op, "channels")
            .map_or(2, |c| c as usize)
            .clamp(1, audio::MAX_CHANNELS);
        self.next += 1;
        let id = self.next;
        let length = number(op, "length").map_or(0, |l| l as usize);
        if offline && length.saturating_mul(channels).saturating_mul(4) > MAX_AUDIO_BUFFER_BYTES {
            return fail(
                "NotSupportedError",
                "the rendered buffer would be too large",
            );
        }
        let mut context = Context {
            graph: Arc::new(Mutex::new(Graph::new(rate, channels))),
            rate,
            channels,
            state: State::Suspended,
            offline: offline.then(|| offline::Offline::new(length)),
            running: None,
            waiting: false,
            buffers: Arc::default(),
            waves: HashMap::new(),
            next: 0,
            teller: Teller {
                inbox: env.inbox.clone(),
                context: id,
            },
        };
        // A realtime context made once the user has activated the page
        // runs at once, but is `suspended` until the task that tells it
        // so, the standard's control message (WPT, M40 step 9); before,
        // it waits for `resume()` and an activation.
        if !offline && env.activated {
            context.run();
            context.teller.state(State::Running);
        }
        let state = State::Suspended.name();
        self.contexts.insert(id, context);
        Ok((
            Json::object([
                ("ctx", Json::from(id)),
                ("rate", Json::from(f64::from(rate))),
                ("state", Json::from(state)),
                ("baseLatency", Json::from(0.01)),
            ]),
            Vec::new(),
        ))
    }
}

/// Creates a node: its number, its shape and its params' ranges.
fn new_node(
    context: &mut Context,
    op: &Json,
    element: Option<&Arc<crate::playback::ElementSound>>,
) -> Result<(Json, Vec<u8>), Failure> {
    let rate = context.rate;
    let node = match text(op, "kind") {
        "gain" => nodes::gain(),
        "constant" => nodes::constant(),
        "oscillator" => nodes::oscillator(rate),
        "bufferSource" => nodes::buffer_source(),
        "biquad" => nodes::biquad(rate),
        "iir" => {
            let list = |name: &str| -> Vec<f64> {
                op.get(name)
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_f64).collect())
                    .unwrap_or_default()
            };
            nodes::iir(list("feedforward"), list("feedback"))
        }
        "delay" => nodes::delay(
            number(op, "max")
                .unwrap_or(1.0)
                .clamp(1e-6, audio::MAX_DELAY_SECONDS),
            rate,
        ),
        "stereoPanner" => nodes::stereo_panner(),
        "panner" => nodes::panner(),
        "splitter" => nodes::splitter(
            number(op, "count")
                .map_or(6, |c| c as usize)
                .clamp(1, audio::MAX_CHANNELS),
        ),
        "merger" => nodes::merger(
            number(op, "count")
                .map_or(6, |c| c as usize)
                .clamp(1, audio::MAX_CHANNELS),
        ),
        "shaper" => nodes::shaper(),
        "compressor" => nodes::compressor(rate),
        "convolver" => nodes::convolver(),
        "analyser" => nodes::analyser(),
        // An element's sound, taken out of the output into the graph, for
        // good.
        "mediaElementSource" => {
            let Some(element) = element else {
                return fail("TypeError", "no media element");
            };
            let mut feed = element
                .feed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if feed.is_some() {
                return fail(
                    "InvalidStateError",
                    "HTMLMediaElement already connected previously to a different MediaElementSourceNode.",
                );
            }
            let external = nodes::external::External::default();
            *feed = Some(external.clone());
            nodes::external(external)
        }
        "destination" => {
            let graph = lock(&context.graph);
            let n = graph.node(0).map_or(2, |n| n.channel_count);
            return Ok((
                describe(0, 1, 1, n, "explicit", "speakers", &[]),
                Vec::new(),
            ));
        }
        other => return fail("TypeError", format!("no node '{other}'")),
    };
    let (inputs, outputs, count, mode, interpretation) = (
        node.inputs,
        node.outputs,
        node.channel_count,
        node.mode,
        node.interpretation,
    );
    let params: Vec<(f32, f32, f32, Rate)> = node
        .params
        .iter()
        .map(|p| (p.default, p.min, p.max, p.rate))
        .collect();
    let Some(id) = lock(&context.graph).add(node) else {
        return fail("NotSupportedError", "too many audio nodes");
    };
    let mode = match mode {
        Mode::Max => "max",
        Mode::ClampedMax => "clamped-max",
        Mode::Explicit => "explicit",
    };
    let interpretation = match interpretation {
        Interpretation::Speakers => "speakers",
        Interpretation::Discrete => "discrete",
    };
    Ok((
        describe(id, inputs, outputs, count, mode, interpretation, &params),
        Vec::new(),
    ))
}

fn describe(
    id: NodeId,
    inputs: usize,
    outputs: usize,
    count: usize,
    mode: &str,
    interpretation: &str,
    params: &[(f32, f32, f32, Rate)],
) -> Json {
    Json::object([
        ("node", Json::from(id)),
        ("inputs", Json::from(inputs as u64)),
        ("outputs", Json::from(outputs as u64)),
        ("count", Json::from(count as u64)),
        ("mode", Json::from(mode)),
        ("interpretation", Json::from(interpretation)),
        (
            "params",
            Json::Array(
                params
                    .iter()
                    .map(|(d, min, max, rate)| {
                        Json::object([
                            ("default", Json::from(f64::from(*d))),
                            ("min", Json::from(f64::from(*min))),
                            ("max", Json::from(f64::from(*max))),
                            (
                                "rate",
                                Json::from(if *rate == Rate::A { "a-rate" } else { "k-rate" }),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// A param's value, its rate, or an automation call.
fn param_op(context: &mut Context, op: &Json, bytes: &[u8]) -> Result<(Json, Vec<u8>), Failure> {
    let (node, index) = (whole(op, "node")?, whole(op, "param")? as usize);
    let listener = op.get("listener").and_then(Json::as_bool).unwrap_or(false);
    let mut graph = lock(&context.graph);
    let now = graph.current_time();
    let param = if listener {
        graph.listener.get_mut(index)
    } else {
        graph.node_mut(node).and_then(|n| n.params.get_mut(index))
    };
    let Some(param) = param else {
        return fail("InvalidStateError", "no such param");
    };
    let time = number(op, "time").unwrap_or(0.0);
    let value = number(op, "value").unwrap_or(0.0) as f32;
    let overlap = |_| Failure {
        name: "NotSupportedError",
        message: "the event overlaps a value curve".to_owned(),
    };
    match text(op, "what") {
        "get" => return Ok((Json::from(f64::from(param.value_at(now))), Vec::new())),
        "value" => {
            param.value = value;
            param.cancel(now);
            param
                .schedule(Event::SetValue { value, time: now }, now)
                .map_err(overlap)?;
        }
        "rate" => {
            param.rate = if text(op, "rate") == "k-rate" {
                Rate::K
            } else {
                Rate::A
            }
        }
        "setValueAtTime" => param
            .schedule(Event::SetValue { value, time }, now)
            .map_err(overlap)?,
        "linearRampToValueAtTime" => param
            .schedule(Event::LinearRamp { value, time }, now)
            .map_err(overlap)?,
        "exponentialRampToValueAtTime" => param
            .schedule(Event::ExponentialRamp { value, time }, now)
            .map_err(overlap)?,
        "setTargetAtTime" => param
            .schedule(
                Event::SetTarget {
                    target: value,
                    time,
                    constant: number(op, "constant").unwrap_or(0.0),
                },
                now,
            )
            .map_err(overlap)?,
        "setValueCurveAtTime" => param
            .schedule(
                Event::SetValueCurve {
                    values: floats(bytes).into(),
                    time,
                    duration: number(op, "duration").unwrap_or(0.0),
                },
                now,
            )
            .map_err(overlap)?,
        "cancelScheduledValues" => param.cancel(time),
        "cancelAndHoldAtTime" => param.cancel_and_hold(time, context.rate),
        other => return fail("TypeError", format!("no param call '{other}'")),
    }
    if param.events().len() > audio::MAX_PARAM_EVENTS {
        param.forget_before(now);
        if param.events().len() > audio::MAX_PARAM_EVENTS {
            param.cancel(time);
            return fail("NotSupportedError", "too many automation events");
        }
    }
    Ok((Json::Null, Vec::new()))
}

/// A node's attribute set.
fn set_op(context: &mut Context, op: &Json) -> Result<(Json, Vec<u8>), Failure> {
    let node = whole(op, "node")?;
    let what = text(op, "what");
    let value = op.get("value");
    let v = value.and_then(Json::as_f64).unwrap_or(0.0);
    let s = value.and_then(Json::as_str).unwrap_or("");
    let wave = whole(op, "wave")
        .ok()
        .and_then(|w| context.waves.get(&w).cloned());
    let mut graph = lock(&context.graph);
    let Some(n) = graph.node_mut(node) else {
        return Ok((Json::Null, Vec::new()));
    };
    match (&mut n.kind, what) {
        (Kind::Oscillator(o), "type") => match s {
            "sine" => o.set_shape(audio::Shape::Sine),
            "square" => o.set_shape(audio::Shape::Square),
            "sawtooth" => o.set_shape(audio::Shape::Sawtooth),
            "triangle" => o.set_shape(audio::Shape::Triangle),
            _ => {}
        },
        (Kind::Oscillator(o), "wave") => {
            if let Some(wave) = wave {
                o.wave = nodes::sources::Wave::Table(wave);
            }
        }
        (Kind::BufferSource(b), "loop") => {
            b.looping = value.and_then(Json::as_bool).unwrap_or(false)
        }
        (Kind::BufferSource(b), "loopStart") => b.loop_start = v,
        (Kind::BufferSource(b), "loopEnd") => b.loop_end = v,
        (Kind::Biquad(b), "type") => {
            b.kind = match s {
                "highpass" => BiquadType::Highpass,
                "bandpass" => BiquadType::Bandpass,
                "lowshelf" => BiquadType::Lowshelf,
                "highshelf" => BiquadType::Highshelf,
                "peaking" => BiquadType::Peaking,
                "notch" => BiquadType::Notch,
                "allpass" => BiquadType::Allpass,
                _ => BiquadType::Lowpass,
            }
        }
        (Kind::Shaper(w), "oversample") => {
            w.oversample = match s {
                "2x" => Oversample::X2,
                "4x" => Oversample::X4,
                _ => Oversample::None,
            }
        }
        (Kind::Shaper(w), "curve") => {
            w.curve = op.get("curve").and_then(Json::as_array).map(|a| {
                a.iter()
                    .filter_map(Json::as_f64)
                    .map(|v| v as f32)
                    .collect()
            })
        }
        (Kind::Panner(p), _) => match what {
            "distanceModel" => {
                p.distance_model = match s {
                    "linear" => DistanceModel::Linear,
                    "exponential" => DistanceModel::Exponential,
                    _ => DistanceModel::Inverse,
                }
            }
            "refDistance" => p.ref_distance = v,
            "maxDistance" => p.max_distance = v,
            "rolloffFactor" => p.rolloff = v,
            "coneInnerAngle" => p.cone_inner = v,
            "coneOuterAngle" => p.cone_outer = v,
            "coneOuterGain" => p.cone_outer_gain = v,
            _ => {}
        },
        (Kind::Analyser(a), _) => {
            let mut data = lock(&a.shared);
            match what {
                "fftSize" => data.fft_size = (v as usize).clamp(32, audio::MAX_FFT_SIZE),
                "smoothingTimeConstant" => data.smoothing = v,
                "minDecibels" => data.min_db = v,
                "maxDecibels" => data.max_db = v,
                _ => {}
            }
        }
        _ => {}
    }
    Ok((Json::Null, Vec::new()))
}

/// A buffer given to a buffer source or a convolver: by the id of one
/// the engine keeps, or its samples.
fn buffer_op(context: &mut Context, op: &Json, bytes: Vec<u8>) -> Result<(Json, Vec<u8>), Failure> {
    let node = whole(op, "node")?;
    let buffer = if op.get("none").and_then(Json::as_bool).unwrap_or(false) {
        None
    } else if let Some(id) = number(op, "id") {
        match lock(&context.buffers).get(&(id as u32)) {
            Some(buffer) => Some(buffer.clone()),
            None => return Ok((Json::from("unknown"), Vec::new())),
        }
    } else {
        let count = whole(op, "channels")? as usize;
        let length = whole(op, "length")? as usize;
        if bytes.len() > MAX_AUDIO_BUFFER_BYTES {
            return fail("NotSupportedError", "the buffer is too large");
        }
        let Some(channels) = bytes_to_samples(&bytes, count, length) else {
            return fail("TypeError", "the buffer's samples");
        };
        let rate = number(op, "rate").unwrap_or(f64::from(context.rate)) as f32;
        let buffer = Arc::new(AudioData { rate, channels });
        let id = context.id();
        lock(&context.buffers).insert(id, buffer.clone());
        Some(buffer)
    };
    let normalize = op.get("normalize").and_then(Json::as_bool).unwrap_or(true);
    let mut graph = lock(&context.graph);
    match graph.node_mut(node).map(|n| &mut n.kind) {
        Some(Kind::BufferSource(b)) => b.buffer = buffer,
        Some(Kind::Convolver(c)) => {
            if buffer
                .as_ref()
                .is_some_and(|b| b.frames() > audio::MAX_CONVOLVER_FRAMES)
            {
                return fail("NotSupportedError", "the impulse response is too long");
            }
            c.set_response(buffer.as_deref(), normalize);
        }
        _ => {}
    }
    Ok((Json::from("taken"), Vec::new()))
}

/// An analyser's data, as bytes: floats, or bytes.
fn analyser_op(context: &mut Context, op: &Json) -> Result<(Json, Vec<u8>), Failure> {
    let node = whole(op, "node")?;
    let length = whole(op, "length")? as usize;
    let graph = lock(&context.graph);
    let Some(Kind::Analyser(a)) = graph.node(node).map(|n| &n.kind) else {
        return Ok((Json::Null, Vec::new()));
    };
    let mut data = lock(&a.shared);
    let bytes = match text(op, "what") {
        "floatFrequency" | "floatTime" => {
            let mut out = vec![0.0f32; length];
            if text(op, "what") == "floatFrequency" {
                data.float_frequency(&mut out);
            } else {
                data.float_time(&mut out);
            }
            out.iter().flat_map(|v| v.to_le_bytes()).collect()
        }
        what => {
            let mut out = vec![0u8; length];
            if what == "byteFrequency" {
                data.byte_frequency(&mut out);
            } else {
                data.byte_time(&mut out);
            }
            out
        }
    };
    Ok((Json::Null, bytes))
}

/// `getFrequencyResponse()`: magnitudes then phases, as floats.
fn response_op(context: &mut Context, op: &Json, bytes: &[u8]) -> Result<(Json, Vec<u8>), Failure> {
    let node = whole(op, "node")?;
    let at = floats(bytes);
    let rate = f64::from(context.rate);
    let graph = lock(&context.graph);
    let now = graph.current_time();
    let Some(n) = graph.node(node) else {
        return Ok((Json::Null, Vec::new()));
    };
    let pairs = match &n.kind {
        Kind::Biquad(b) => {
            let p = |i: usize| f64::from(n.params[i].value_at(now));
            b.frequency_response(p(0), p(1), p(2), p(3), rate, &at)
        }
        Kind::Iir(f) => f.frequency_response(rate, &at),
        _ => Vec::new(),
    };
    let mut out: Vec<u8> = pairs.iter().flat_map(|(m, _)| m.to_le_bytes()).collect();
    out.extend(pairs.iter().flat_map(|(_, p)| p.to_le_bytes()));
    Ok((Json::Null, out))
}
