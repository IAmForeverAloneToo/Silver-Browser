//! The renderer's mixer (D37): one thread that renders every running
//! `AudioContext`'s graph as time passes, a quantum at a time, and hands
//! their mixed sound to the process's sink. Until the browser's audio
//! output (M40 step 4) the sink plays nothing and the clock is the
//! system's.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use audio::{Graph, QUANTUM};

/// The rate a context renders at when its page names none: the
/// output's.
pub(crate) const OUTPUT_RATE: f32 = 48_000.0;

/// How often the mixer wakes.
const TICK: Duration = Duration::from_millis(10);

/// The most quanta a context renders in one wake: a context that fell
/// behind (the machine busy) skips ahead rather than racing to catch up.
const MAX_QUANTA_A_TICK: u64 = 64;

/// What a context's rendering tells its document: the sources that ended.
pub(crate) type Ended = Box<dyn Fn(Vec<audio::NodeId>) + Send>;

/// A running context, as the mixer keeps it.
struct Live {
    graph: Weak<Mutex<Graph>>,
    ended: Ended,
    /// When it started running, and its frame then.
    since: Instant,
    frame: u64,
}

#[derive(Default)]
struct Mixer {
    live: Mutex<HashMap<u64, Live>>,
    started: OnceLock<()>,
}

fn mixer() -> &'static Mixer {
    static MIXER: OnceLock<Mixer> = OnceLock::new();
    MIXER.get_or_init(Mixer::default)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    let frame = lock(graph).frame();
    lock(&mixer().live).insert(
        key,
        Live {
            graph: Arc::downgrade(graph),
            ended,
            since: Instant::now(),
            frame,
        },
    );
    mixer().started.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("audio mixer".to_owned())
            .spawn(run_mixer);
    });
    Running(key)
}

fn run_mixer() {
    loop {
        std::thread::sleep(TICK);
        let mut live = lock(&mixer().live);
        live.retain(|_, entry| entry.graph.strong_count() > 0);
        for entry in live.values_mut() {
            let Some(graph) = entry.graph.upgrade() else {
                continue;
            };
            let mut graph = lock(&graph);
            let rate = f64::from(graph.rate);
            let due = entry.frame + (entry.since.elapsed().as_secs_f64() * rate) as u64;
            let mut quanta = 0;
            while graph.frame() + QUANTUM as u64 <= due {
                // The sound itself goes nowhere until the output exists.
                let _ = graph.render();
                quanta += 1;
                if quanta >= MAX_QUANTA_A_TICK {
                    // Behind: the clock starts again from here.
                    entry.since = Instant::now();
                    entry.frame = graph.frame();
                    break;
                }
            }
            let ended = graph.take_ended();
            drop(graph);
            if !ended.is_empty() {
                (entry.ended)(ended);
            }
        }
    }
}
