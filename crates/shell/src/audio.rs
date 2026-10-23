//! The browser's audio output (M40 step 4, D37,
//! `docs/design/audio-video.md`): a source for each renderer (or for this
//! process's own mixer), each a queue of 48 kHz stereo frames, mixed on a
//! thread of the output's to the device: WASAPI's default device on
//! Windows (the platform module), which converts to its own format.
//! Without one, or until one comes back, the sources are drained at the
//! system's clock, so that what plays keeps time. The device is let go
//! when nothing has played for `IDLE`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use engine::audio_output::{MAX_AUDIO_QUEUE_FRAMES, OUTPUT_CHANNELS, OUTPUT_RATE};
use engine::{AudioOutput, AudioSink};

/// How long the output plays silence before it lets the device go.
const IDLE: Duration = Duration::from_secs(2);

/// How long the output waits before it tries a device that failed again.
const RETRY: Duration = Duration::from_secs(1);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The output: its sources, and its thread, started by the first sound.
pub(crate) struct Output {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    sources: Mutex<Vec<Weak<Source>>>,
    /// Whether a source has sound, and the output's thread waits for it.
    sound: Mutex<bool>,
    woken: Condvar,
    started: std::sync::OnceLock<()>,
}

/// One renderer's sound, queued.
struct Source {
    queue: Mutex<VecDeque<f32>>,
    played: AtomicU64,
    shared: Weak<Shared>,
}

impl Output {
    pub(crate) fn new() -> Arc<Output> {
        Arc::new(Output {
            shared: Arc::default(),
        })
    }
}

/// The browser's one output, every context's.
pub(crate) fn output() -> Arc<Output> {
    static OUTPUT: std::sync::OnceLock<Arc<Output>> = std::sync::OnceLock::new();
    OUTPUT.get_or_init(Output::new).clone()
}

impl AudioOutput for Output {
    fn source(&self) -> Arc<dyn AudioSink> {
        let source = Arc::new(Source {
            queue: Mutex::default(),
            played: AtomicU64::new(0),
            shared: Arc::downgrade(&self.shared),
        });
        let mut sources = lock(&self.shared.sources);
        sources.retain(|s| s.strong_count() > 0);
        sources.push(Arc::downgrade(&source));
        source
    }
}

impl AudioSink for Source {
    fn write(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        {
            let mut queue = lock(&self.queue);
            if queue.len() + samples.len() > MAX_AUDIO_QUEUE_FRAMES * OUTPUT_CHANNELS {
                return;
            }
            queue.extend(samples);
        }
        if let Some(shared) = self.shared.upgrade() {
            let started = shared.clone();
            shared.started.get_or_init(|| {
                let _ = std::thread::Builder::new()
                    .name("audio output".to_owned())
                    .spawn(move || run(&started));
            });
            *lock(&shared.sound) = true;
            shared.woken.notify_all();
        }
    }

    fn played(&self) -> u64 {
        self.played.load(Ordering::Relaxed)
    }
}

/// Mixes the sources' next frames into `out`, interleaved stereo; whether
/// any had sound.
fn fill(shared: &Shared, out: &mut [f32]) -> bool {
    out.fill(0.0);
    let sources: Vec<Arc<Source>> = lock(&shared.sources)
        .iter()
        .filter_map(Weak::upgrade)
        .collect();
    let mut any = false;
    for source in sources {
        let mut queue = lock(&source.queue);
        let n = queue.len().min(out.len()) / OUTPUT_CHANNELS * OUTPUT_CHANNELS;
        for (o, s) in out.iter_mut().zip(queue.drain(..n)) {
            *o += s;
        }
        drop(queue);
        source
            .played
            .fetch_add((n / OUTPUT_CHANNELS) as u64, Ordering::Relaxed);
        any |= n > 0;
    }
    engine::audio_output::clamp_samples(out);
    any
}

/// Plays `shared`'s sources until `until`, or until silent for `IDLE`, at
/// the system's clock: what a device would have played, taken.
fn drain_at_clock(shared: &Shared, until: Instant) {
    let tick = Duration::from_millis(10);
    let mut last = Instant::now();
    let mut heard = Instant::now();
    let mut scratch = Vec::new();
    loop {
        std::thread::sleep(tick);
        let now = Instant::now();
        let frames = (now.duration_since(last).as_secs_f64() * f64::from(OUTPUT_RATE)) as usize;
        if frames == 0 {
            continue;
        }
        last = now;
        scratch.resize(frames * OUTPUT_CHANNELS, 0.0);
        if fill(shared, &mut scratch) {
            heard = now;
        }
        if now.duration_since(heard) >= IDLE || now >= until {
            return;
        }
    }
}

fn run(shared: &Shared) {
    loop {
        {
            let mut sound = lock(&shared.sound);
            while !*sound {
                sound = shared
                    .woken
                    .wait(sound)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            *sound = false;
        }
        let mut heard = Instant::now();
        let played = crate::platform::play_audio(&mut |out: &mut [f32]| {
            if fill(shared, out) {
                heard = Instant::now();
            }
            heard.elapsed() < IDLE
        });
        if played.is_err() {
            // No device: the clock plays a while, and while there is
            // sound the device is tried again.
            drain_at_clock(shared, Instant::now() + RETRY);
            let queued = lock(&shared.sources)
                .iter()
                .filter_map(Weak::upgrade)
                .any(|s| !lock(&s.queue).is_empty());
            if queued {
                *lock(&shared.sound) = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The output plays what each source gives, mixed, and says what was
    /// played: by the device where there is one, else at the clock.
    #[test]
    fn the_output_plays_its_sources_and_says_what_was_played() {
        let output = Output::new();
        let (a, b) = (output.source(), output.source());
        let tone: Vec<f32> = (0..4800 * 2)
            .map(|i| ((i / 2) as f32 * 0.05).sin() * 0.1)
            .collect();
        a.write(&tone);
        b.write(&vec![0.0; 4800 * 2]);
        let start = Instant::now();
        while (a.played() < 4800 || b.played() < 4800) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!((a.played(), b.played()), (4800, 4800));
        // A tenth of a second of sound takes about that long to play.
        assert!(
            start.elapsed() >= Duration::from_millis(60),
            "{:?}",
            start.elapsed()
        );
        // A block past the queue's bound is dropped whole.
        let c = output.source();
        c.write(&vec![0.0; (MAX_AUDIO_QUEUE_FRAMES + 1) * OUTPUT_CHANNELS]);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(c.played(), 0);
    }

    #[test]
    fn mixing_adds_the_sources_and_keeps_within_full_scale() {
        let shared = Arc::new(Shared::default());
        let make = |samples: &[f32]| {
            let source = Arc::new(Source {
                queue: Mutex::new(samples.iter().copied().collect()),
                played: AtomicU64::new(0),
                shared: Arc::downgrade(&shared),
            });
            lock(&shared.sources).push(Arc::downgrade(&source));
            source
        };
        let a = make(&[0.75, -0.5, 0.75, 0.75]);
        let b = make(&[0.5, 0.25]);
        let mut out = [9.0; 6];
        assert!(fill(&shared, &mut out));
        assert_eq!(out, [1.0, -0.25, 0.75, 0.75, 0.0, 0.0]);
        assert_eq!((a.played(), b.played()), (2, 1));
        assert!(!fill(&shared, &mut out));
    }
}
