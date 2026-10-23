//! The process's mixer (D37): one thread that renders every running
//! `AudioContext`'s graph, mixes them to the output's stereo at its rate
//! and writes the mix to the process's sink, keeping `AHEAD` ahead of
//! what the sink says it played. A context of another rate is resampled
//! on its way. Its clock is the sink's.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use audio::{Graph, Interpretation, QUANTUM};

use crate::audio_output::{AudioSink, ClockSink, OUTPUT_CHANNELS, OUTPUT_RATE};

/// The rate a context renders at when its page names none: the
/// output's.
pub(crate) const OUTPUT_RATE_F32: f32 = OUTPUT_RATE as f32;

/// How often the mixer wakes.
const TICK: Duration = Duration::from_millis(10);

/// How far ahead of what was played the mixer writes: 30 ms.
const AHEAD: u64 = OUTPUT_RATE as u64 * 30 / 1000;

/// The most quanta the mixer writes in one wake; a sink further behind
/// is let go of what it missed.
const MAX_QUANTA_A_TICK: u64 = 64;

/// What a context's rendering tells its document: the sources that ended.
pub(crate) type Ended = Box<dyn Fn(Vec<audio::NodeId>) + Send>;

/// A running context, as the mixer keeps it.
struct Live {
    graph: Weak<Mutex<Graph>>,
    ended: Ended,
    /// For a context not at the output's rate: its resampler and what it
    /// made not yet mixed, interleaved stereo.
    resampler: Option<media::Resampler>,
    made: VecDeque<f32>,
}

struct Mixer {
    live: Mutex<HashMap<u64, Live>>,
    sink: Mutex<Arc<dyn AudioSink>>,
    started: OnceLock<()>,
}

fn mixer() -> &'static Mixer {
    static MIXER: OnceLock<Mixer> = OnceLock::new();
    MIXER.get_or_init(|| Mixer {
        live: Mutex::default(),
        sink: Mutex::new(Arc::new(ClockSink::new())),
        started: OnceLock::new(),
    })
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Where the mixer writes from now.
pub(crate) fn set_sink(sink: Arc<dyn AudioSink>) {
    *lock(&mixer().sink) = sink;
}

/// A context's place in the mixer: it renders while this is held.
pub(crate) struct Running(u64);

impl Drop for Running {
    fn drop(&mut self) {
        lock(&mixer().live).remove(&self.0);
    }
}

/// Starts rendering `graph` from now; `ended` hears its sources end.
pub(crate) fn run(graph: &Arc<Mutex<Graph>>, ended: Ended) -> Running {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let key = NEXT.fetch_add(1, Ordering::Relaxed);
    let rate = lock(graph).rate;
    let resampler = (rate != OUTPUT_RATE_F32)
        .then(|| media::Resampler::new(f64::from(rate), f64::from(OUTPUT_RATE), OUTPUT_CHANNELS));
    lock(&mixer().live).insert(
        key,
        Live {
            graph: Arc::downgrade(graph),
            ended,
            resampler,
            made: VecDeque::new(),
        },
    );
    mixer().started.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("audio mixer".to_owned())
            .spawn(run_mixer);
    });
    Running(key)
}

/// One quantum of `graph`, mixed to the output's channels and
/// interleaved.
fn stereo(graph: &mut Graph) -> [f32; QUANTUM * OUTPUT_CHANNELS] {
    let bus = graph.render();
    let mut both = audio::silence(OUTPUT_CHANNELS);
    audio::mix_into(&mut both, &bus, Interpretation::Speakers);
    let mut out = [0.0; QUANTUM * OUTPUT_CHANNELS];
    for (i, frame) in out
        .as_chunks_mut::<OUTPUT_CHANNELS>()
        .0
        .iter_mut()
        .enumerate()
    {
        for (c, sample) in frame.iter_mut().enumerate() {
            *sample = both[c][i];
        }
    }
    out
}

/// Adds `frames` frames of `entry`'s sound into `into`.
fn render(entry: &mut Live, graph: &mut Graph, frames: usize, into: &mut [f32]) {
    let want = frames * OUTPUT_CHANNELS;
    match &mut entry.resampler {
        None => {
            for chunk in into.chunks_mut(QUANTUM * OUTPUT_CHANNELS) {
                let quantum = stereo(graph);
                for (o, s) in chunk.iter_mut().zip(quantum) {
                    *o += s;
                }
            }
        }
        Some(resampler) => {
            while entry.made.len() < want {
                let quantum = stereo(graph);
                entry.made.extend(resampler.process(&quantum));
            }
            for o in into.iter_mut().take(want) {
                *o += entry.made.pop_front().unwrap_or(0.0);
            }
        }
    }
}

/// What the mixer wrote so far, and what its sink said it played last,
/// in frames.
#[derive(Default)]
struct Written {
    written: u64,
    played: u64,
}

fn run_mixer() {
    let mut written = Written::default();
    loop {
        std::thread::sleep(TICK);
        mix_once(&mut written);
    }
}

fn mix_once(written: &mut Written) {
    let sink = lock(&mixer().sink).clone();
    let mut live = lock(&mixer().live);
    live.retain(|_, entry| entry.graph.strong_count() > 0);
    let played = sink.played();
    // A sink that played past what it was given (it ran dry, or nothing
    // ran) is written to from where it is; so is a new sink.
    if played < written.played {
        written.written = played;
    }
    written.played = played;
    written.written = written.written.max(played);
    if live.is_empty() {
        return;
    }
    let ahead = written.written - played;
    let quanta = AHEAD
        .saturating_sub(ahead)
        .div_ceil(QUANTUM as u64)
        .min(MAX_QUANTA_A_TICK);
    if quanta == 0 {
        // Nothing due: a sink that learns what was played by asking is
        // asked.
        sink.write(&[]);
        return;
    }
    let frames = quanta as usize * QUANTUM;
    let mut out = vec![0.0f32; frames * OUTPUT_CHANNELS];
    for entry in live.values_mut() {
        let Some(graph) = entry.graph.upgrade() else {
            continue;
        };
        let mut graph = lock(&graph);
        render(entry, &mut graph, frames, &mut out);
        let ended = graph.take_ended();
        drop(graph);
        if !ended.is_empty() {
            (entry.ended)(ended);
        }
    }
    crate::audio_output::clamp_samples(&mut out);
    sink.write(&out);
    written.written += frames as u64;
}
