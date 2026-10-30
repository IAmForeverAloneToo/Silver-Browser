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

/// What a context's rendering tells its document: the sources that
/// ended, and the `ScriptProcessorNode`s' buffers filled.
pub(crate) type Ended =
    Box<dyn Fn(Vec<audio::NodeId>, Vec<(audio::NodeId, audio::nodes::script::Filled)>) + Send>;

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
    /// The media elements' sounds (M40 step 6), mixed as they play.
    elements: Mutex<HashMap<u64, Weak<crate::playback::ElementSound>>>,
    sink: Mutex<Arc<dyn AudioSink>>,
    started: OnceLock<()>,
}

fn mixer() -> &'static Mixer {
    static MIXER: OnceLock<Mixer> = OnceLock::new();
    MIXER.get_or_init(|| Mixer {
        live: Mutex::default(),
        elements: Mutex::default(),
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
        lock(&mixer().elements).remove(&self.0);
    }
}

/// Mixes a media element's sound from now, while it plays.
pub(crate) fn add_element(sound: &Arc<crate::playback::ElementSound>) -> Running {
    let key = next_key();
    lock(&mixer().elements).insert(key, Arc::downgrade(sound));
    start();
    Running(key)
}

fn next_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn start() {
    mixer().started.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("audio mixer".to_owned())
            .spawn(run_mixer);
    });
}

/// Starts rendering `graph` from now; `ended` hears its sources end.
pub(crate) fn run(graph: &Arc<Mutex<Graph>>, ended: Ended) -> Running {
    let key = next_key();
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
    start();
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

/// Adds `frames` frames of every running context's sound into `out`, in
/// the order they started running: a context that gives its sound to
/// another's stream source before the one that took the stream renders
/// it, which then reads this wake's sound, not a gap.
fn render_all(live: &mut HashMap<u64, Live>, frames: usize, out: &mut [f32]) {
    let mut keys: Vec<u64> = live.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        let Some(entry) = live.get_mut(&key) else {
            continue;
        };
        let Some(graph) = entry.graph.upgrade() else {
            continue;
        };
        let mut graph = lock(&graph);
        render(entry, &mut graph, frames, out);
        let ended = graph.take_ended();
        let filled = graph.take_filled();
        drop(graph);
        if !ended.is_empty() || !filled.is_empty() {
            (entry.ended)(ended, filled);
        }
    }
}

/// Adds a wake's `frames` frames of the elements' and the contexts' sound
/// into `out`: the elements' first, so that a graph that took one's
/// sound renders this wake's, however many frames the wake has, rather
/// than running dry on the last wake's.
fn mix_wake(
    live: &mut HashMap<u64, Live>,
    elements: &[Arc<crate::playback::ElementSound>],
    frames: usize,
    out: &mut [f32],
) {
    for sound in elements {
        if sound.playing.load(Ordering::Relaxed) {
            sound.take_into(out);
        }
    }
    render_all(live, frames, out);
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
    let elements: Vec<_> = {
        let mut elements = lock(&mixer().elements);
        elements.retain(|_, sound| sound.strong_count() > 0);
        elements.values().filter_map(Weak::upgrade).collect()
    };
    let played = sink.played();
    // A sink that played past what it was given (it ran dry, or nothing
    // ran) is written to from where it is; so is a new sink.
    if played < written.played {
        written.written = played;
    }
    written.played = played;
    written.written = written.written.max(played);
    if live.is_empty() && elements.is_empty() {
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
    mix_wake(&mut live, &elements, frames, &mut out);
    crate::audio_output::clamp_samples(&mut out);
    sink.write(&out);
    written.written += frames as u64;
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio::nodes::{self, external::Tap};

    #[test]
    fn a_streams_writer_renders_before_its_reader() {
        // Pairs of contexts at the output's rate, each writer started
        // before its reader: the writer's constant 0.25 goes to its tap,
        // which the reader plays. Rendered in the order they started,
        // every reader hears the wake's sound in that same wake.
        const PAIRS: usize = 16;
        let mut graphs = Vec::new();
        let mut live = HashMap::new();
        let mut add = |live: &mut HashMap<u64, Live>, key: u64, graph: Graph| {
            let graph = Arc::new(Mutex::new(graph));
            live.insert(
                key,
                Live {
                    graph: Arc::downgrade(&graph),
                    ended: Box::new(|_, _| {}),
                    resampler: None,
                    made: VecDeque::new(),
                },
            );
            graphs.push(graph);
        };
        for pair in 0..PAIRS as u64 {
            let tap = Tap::default();
            let mut writer = Graph::new(OUTPUT_RATE_F32, 1);
            let c = writer.add(nodes::constant()).expect("a node");
            let node = writer.node_mut(c).expect("the node");
            node.params[0].value = 0.25;
            if let audio::Kind::Constant(schedule) = &mut node.kind {
                schedule.start = Some(0.0);
            }
            let t = writer.add(nodes::tap(tap.clone())).expect("a node");
            writer.connect(c, 0, t, 0);
            let mut reader = Graph::new(OUTPUT_RATE_F32, 1);
            let source = reader.add(nodes::external(tap.reader())).expect("a node");
            reader.connect(source, 0, 0, 0);
            // The writer's key the lower, as it started first.
            add(&mut live, 2 * pair + 1, writer);
            add(&mut live, 2 * pair + 2, reader);
        }
        let frames = 8 * QUANTUM;
        let mut out = vec![0.0; frames * OUTPUT_CHANNELS];
        render_all(&mut live, frames, &mut out);
        let want = 0.25 * PAIRS as f32;
        assert!(
            out.iter().all(|s| (s - want).abs() < 1e-5),
            "{:?}",
            &out[..8]
        );
    }

    #[test]
    fn an_elements_sound_reaches_its_graph_in_the_same_wake() {
        // A wake longer than the feed holds: the element's frames are
        // given before the graph renders, so the graph plays all of them.
        let sound = Arc::new(crate::playback::ElementSound::default());
        let external = nodes::external::External {
            rate: OUTPUT_RATE_F32,
            ..Default::default()
        };
        assert!(sound.take_by(external.clone()));
        sound.set_gain(1.0);
        sound.playing.store(true, Ordering::Relaxed);
        let frames = 16 * QUANTUM;
        sound
            .queue
            .lock()
            .unwrap()
            .extend(std::iter::repeat_n(0.5, frames * OUTPUT_CHANNELS));
        let mut graph = Graph::new(OUTPUT_RATE_F32, 2);
        let source = graph.add(nodes::external(external)).expect("a node");
        graph.connect(source, 0, 0, 0);
        let graph = Arc::new(Mutex::new(graph));
        let mut live = HashMap::from([(
            1,
            Live {
                graph: Arc::downgrade(&graph),
                ended: Box::new(|_, _| {}),
                resampler: None,
                made: VecDeque::new(),
            },
        )]);
        let mut out = vec![0.0; frames * OUTPUT_CHANNELS];
        mix_wake(&mut live, &[sound], frames, &mut out);
        assert!(
            out.iter().all(|s| (s - 0.5).abs() < 1e-6),
            "{:?}",
            &out[..8]
        );
    }
}
