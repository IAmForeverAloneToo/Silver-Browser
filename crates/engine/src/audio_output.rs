//! The sound's way out (M40 step 4, D37, `docs/design/audio-video.md`):
//! a process's mixer writes its sound to an [`AudioSink`], 48 kHz stereo
//! floats, and keeps a little ahead of what the sink says it played. In a
//! renderer the sink sends the browser its blocks; the browser's
//! [`AudioOutput`] (the shell's: WASAPI on Windows) gives each renderer a
//! sink of its own and mixes them to the device. Without a device the
//! sink plays at the system's clock.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

/// The rate of what a mixer writes, in frames a second.
pub const OUTPUT_RATE: u32 = 48_000;
/// Its channels, interleaved: left, right.
pub const OUTPUT_CHANNELS: usize = 2;
/// The most frames one block carries; a renderer's longer block is its
/// lie.
pub const MAX_AUDIO_BLOCK_FRAMES: usize = 64 * audio::QUANTUM;
/// The most frames a sink keeps queued; past it, a block is dropped.
pub const MAX_AUDIO_QUEUE_FRAMES: usize = OUTPUT_RATE as usize;

/// Where a mixer's sound goes.
pub trait AudioSink: Send + Sync {
    /// Queues `samples`, interleaved stereo frames at [`OUTPUT_RATE`]. An
    /// empty block asks for news of what was played.
    fn write(&self, samples: &[f32]);
    /// The frames of what was written that were played so far.
    fn played(&self) -> u64;
}

/// The browser's audio output: a sink for each renderer, mixed to the
/// device.
pub trait AudioOutput: Send + Sync {
    fn source(&self) -> Arc<dyn AudioSink>;
}

/// A sink that plays nothing, at the system's clock: what it was given
/// counts as played as time passes.
pub struct ClockSink {
    state: Mutex<ClockState>,
}

struct ClockState {
    queued: u64,
    played: u64,
    last: Instant,
}

impl ClockSink {
    pub fn new() -> ClockSink {
        ClockSink {
            state: Mutex::new(ClockState {
                queued: 0,
                played: 0,
                last: Instant::now(),
            }),
        }
    }

    fn advance(state: &mut ClockState) {
        let now = Instant::now();
        let due = (now.duration_since(state.last).as_secs_f64() * f64::from(OUTPUT_RATE)) as u64;
        if due == 0 {
            return;
        }
        let taken = due.min(state.queued);
        state.queued -= taken;
        state.played += taken;
        state.last = now;
    }
}

impl Default for ClockSink {
    fn default() -> Self {
        ClockSink::new()
    }
}

impl AudioSink for ClockSink {
    fn write(&self, samples: &[f32]) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        ClockSink::advance(&mut state);
        let frames = (samples.len() / OUTPUT_CHANNELS) as u64;
        if state.queued + frames <= MAX_AUDIO_QUEUE_FRAMES as u64 {
            state.queued += frames;
        }
    }

    fn played(&self) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        ClockSink::advance(&mut state);
        state.played
    }
}

/// An output with no device: each source a [`ClockSink`].
pub struct ClockOutput;

impl AudioOutput for ClockOutput {
    fn source(&self) -> Arc<dyn AudioSink> {
        Arc::new(ClockSink::new())
    }
}

/// Where this process's mixer writes from now: the browser's output in
/// one process. A renderer's is the browser, set by the renderer itself.
pub fn set_audio_sink(sink: Arc<dyn AudioSink>) {
    crate::webaudio::mixer::set_sink(sink);
}

/// `samples` made safe to play: what is not a number silent, the rest
/// within [-1, 1].
pub fn clamp_samples(samples: &mut [f32]) {
    for s in samples {
        *s = if s.is_finite() {
            s.clamp(-1.0, 1.0)
        } else {
            0.0
        };
    }
}
