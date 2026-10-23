//! The Web Audio API's rendering (M40, `docs/design/audio-video.md`): a
//! graph of nodes rendered in quanta of [`QUANTUM`] frames, their channels
//! mixed by the standard's rules, their params automated, as Web Audio
//! API 1.1 writes them. A leaf crate: the engine owns the contexts, their
//! threads and their bindings; this crate is given a graph's changes and
//! asked for its sound.

#![forbid(unsafe_code)]

mod fft;
mod graph;
mod mix;
pub mod nodes;
mod param;
mod wave;

#[cfg(test)]
mod tests;

use std::sync::Arc;

pub use graph::{Graph, Node, NodeId, Quantum};
pub use mix::{Bus, Interpretation, Mode, computed_channels, mix_into, silence};
pub use nodes::Kind;
pub use param::{Event, Overlap, Param, Rate};
pub use wave::PeriodicWave;

/// The frames a graph renders at a time: the render quantum.
pub const QUANTUM: usize = 128;
/// The most channels a node, a buffer or a context may have.
pub const MAX_CHANNELS: usize = 32;
/// The most nodes a graph holds.
pub const MAX_NODES: usize = 100_000;
/// The longest a `DelayNode` may delay, in seconds: the standard's.
pub const MAX_DELAY_SECONDS: f64 = 180.0;
/// The most events an `AudioParam`'s timeline keeps; past them, those
/// wholly past are forgotten first, and then a new one is refused.
pub const MAX_PARAM_EVENTS: usize = 10_000;
/// The longest impulse response a `ConvolverNode` takes, in frames.
pub const MAX_CONVOLVER_FRAMES: usize = 480_000;
/// The largest `fftSize` an `AnalyserNode` takes: the standard's.
pub const MAX_FFT_SIZE: usize = 32_768;

/// An `AudioBuffer`'s samples: its rate and its channels' frames.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioData {
    pub rate: f32,
    pub channels: Vec<Vec<f32>>,
}

impl AudioData {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

/// An `AudioBuffer` shared between the control thread and the rendering
/// one: what a node plays is a copy taken when it is given it, as the
/// standard's "acquire the content" says.
pub type Buffer = Arc<AudioData>;
