//! An `OfflineAudioContext`'s rendering (Web Audio 1.1 section 1.3): a
//! thread that renders its graph as fast as it can into the buffer
//! `startRendering()` resolves with, stopping at each `suspend()` frame
//! until the page resumes it, and at each `ScriptProcessorNode` buffer
//! filled until the page answers it, as the reference browser waits for
//! the page offline.

use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex};

use audio::nodes::script::Filled;
use audio::{Graph, NodeId, QUANTUM};

/// What the rendering tells the document.
pub(crate) enum Report {
    /// It stopped at a suspension, at that frame.
    Suspended(u64),
    /// Sources ended.
    Ended(Vec<NodeId>),
    /// `ScriptProcessorNode`s' buffers filled, which the page answers.
    Filled(Vec<(NodeId, Filled)>),
    /// It rendered every frame: the channels.
    Done(Vec<Vec<f32>>),
}

#[derive(Default)]
struct Shared {
    /// The frames it stops at, each a quantum's first.
    suspends: BTreeSet<u64>,
    /// Whether it is stopped, waiting.
    paused: bool,
    /// The document went: it stops.
    cancelled: bool,
    /// The buffers handed to the page not yet answered.
    owed: usize,
    /// It rendered every frame, or stopped.
    finished: bool,
}

type Pair = Arc<(Mutex<Shared>, Condvar)>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An offline context's rendering, before and while it runs.
pub(crate) struct Offline {
    pub(crate) length: usize,
    shared: Pair,
    pub(crate) started: bool,
}

impl Offline {
    pub(crate) fn new(length: usize) -> Offline {
        Offline {
            length,
            shared: Arc::default(),
            started: false,
        }
    }

    /// Adds a suspension at `frame`; false if there is one there.
    pub(crate) fn suspend_at(&self, frame: u64) -> bool {
        lock(&self.shared.0).suspends.insert(frame)
    }

    /// Whether it renders now: started, not finished, not stopped at a
    /// suspension.
    pub(crate) fn busy(&self) -> bool {
        let shared = lock(&self.shared.0);
        self.started && !shared.finished && !shared.paused
    }

    /// The page answered a buffer it was handed.
    pub(crate) fn answered(&self) {
        let (shared, wake) = &*self.shared;
        let mut shared = lock(shared);
        shared.owed = shared.owed.saturating_sub(1);
        wake.notify_all();
    }

    /// Lets a stopped rendering go on.
    pub(crate) fn resume(&self) -> bool {
        let (shared, wake) = &*self.shared;
        let mut shared = lock(shared);
        let was = shared.paused;
        shared.paused = false;
        wake.notify_all();
        was
    }

    /// Renders `graph` (of `channels` channels) on a thread of its own,
    /// telling `report` as it goes.
    pub(crate) fn start(
        &mut self,
        graph: Arc<Mutex<Graph>>,
        channels: usize,
        report: Box<dyn Fn(Report) + Send>,
    ) {
        self.started = true;
        let shared = self.shared.clone();
        let length = self.length;
        let _ = std::thread::Builder::new()
            .name("offline audio".to_owned())
            .spawn(move || {
                render(&graph, channels, length, &shared, &*report);
                lock(&shared.0).finished = true;
            });
    }
}

impl Drop for Offline {
    fn drop(&mut self) {
        let (shared, wake) = &*self.shared;
        lock(shared).cancelled = true;
        wake.notify_all();
    }
}

fn render(
    graph: &Mutex<Graph>,
    channels: usize,
    length: usize,
    shared: &Pair,
    report: &dyn Fn(Report),
) {
    let mut out: Vec<Vec<f32>> = (0..channels).map(|_| Vec::with_capacity(length)).collect();
    let mut frame = 0u64;
    while (frame as usize) < length {
        {
            let (state, wake) = &**shared;
            let mut state = lock(state);
            if state.cancelled {
                return;
            }
            if state.suspends.remove(&frame) {
                state.paused = true;
                drop(state);
                report(Report::Suspended(frame));
                let mut state = lock(&shared.0);
                while state.paused && !state.cancelled {
                    state = wake
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                if state.cancelled {
                    return;
                }
            }
        }
        let mut g = lock(graph);
        let bus = g.render();
        let ended = g.take_ended();
        let filled = g.take_filled();
        drop(g);
        let take = (length - frame as usize).min(QUANTUM);
        for (channel, data) in out
            .iter_mut()
            .zip(bus.iter().chain(std::iter::repeat(&[0.0; QUANTUM])))
        {
            channel.extend_from_slice(&data[..take]);
        }
        if !ended.is_empty() {
            report(Report::Ended(ended));
        }
        if !filled.is_empty() {
            let (state, wake) = &**shared;
            lock(state).owed += filled.len();
            report(Report::Filled(filled));
            let mut state = lock(state);
            while state.owed > 0 && !state.cancelled {
                state = wake
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if state.cancelled {
                return;
            }
        }
        frame += QUANTUM as u64;
    }
    report(Report::Done(out));
}
