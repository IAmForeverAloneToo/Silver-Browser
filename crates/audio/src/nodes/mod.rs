//! The nodes: what each does with its inputs and params each quantum, and
//! each one's defaults (its inputs and outputs, channel rules and params)
//! as the standard's tables give them.

pub mod analyser;
pub mod compressor;
pub mod convolver;
pub mod delay;
pub mod external;
pub mod filters;
pub mod panner;
pub mod shaper;
pub mod sources;

use crate::QUANTUM;
use crate::graph::{Node, Quantum};
use crate::mix::{Bus, Interpretation, Mode, silence};
use crate::param::{Param, Rate};

/// What a node is given besides its inputs and params.
pub struct Context<'a> {
    pub quantum: Quantum,
    pub listener: &'a [Param],
    pub channel_count: usize,
}

/// What a node does.
pub enum Kind {
    Destination,
    Gain,
    Constant(sources::Schedule),
    Oscillator(sources::Oscillator),
    BufferSource(sources::BufferSource),
    Biquad(filters::Biquad),
    Iir(filters::Iir),
    Delay(delay::Delay),
    StereoPanner,
    Panner(panner::Panner),
    Splitter,
    Merger,
    Shaper(shaper::Shaper),
    Compressor(compressor::Compressor),
    Convolver(convolver::Convolver),
    Analyser(analyser::Analyser),
    External(external::External),
}

/// The largest single-precision float: the standard's "most positive
/// single float", most params' bound.
const F: f32 = f32::MAX;

/// A destination of `channels` channels.
pub fn destination(channels: usize) -> Node {
    Node::new(
        Kind::Destination,
        1,
        1,
        channels,
        Mode::Explicit,
        Interpretation::Speakers,
        vec![],
    )
}

pub fn gain() -> Node {
    Node::new(
        Kind::Gain,
        1,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![Param::new(1.0, -F, F, Rate::A)],
    )
}

pub fn constant() -> Node {
    Node::new(
        Kind::Constant(sources::Schedule::default()),
        0,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![Param::new(1.0, -F, F, Rate::A)],
    )
}

/// An oscillator at `rate`: params frequency and detune.
pub fn oscillator(rate: f32) -> Node {
    let nyquist = rate / 2.0;
    let detune = 1200.0 * F.log2();
    Node::new(
        Kind::Oscillator(sources::Oscillator::new(rate)),
        0,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![
            Param::new(440.0, -nyquist, nyquist, Rate::A),
            Param::new(0.0, -detune, detune, Rate::A),
        ],
    )
}

/// An `AudioBufferSourceNode`: params playbackRate and detune, k-rate.
pub fn buffer_source() -> Node {
    Node::new(
        Kind::BufferSource(sources::BufferSource::default()),
        0,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![
            Param::new(1.0, -F, F, Rate::K),
            Param::new(0.0, -F, F, Rate::K),
        ],
    )
}

/// A biquad at `rate`: params frequency, detune, Q and gain.
pub fn biquad(rate: f32) -> Node {
    let nyquist = rate / 2.0;
    let detune = 1200.0 * F.log2();
    let gain = 40.0 * F.log10();
    Node::new(
        Kind::Biquad(filters::Biquad::default()),
        1,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![
            Param::new(350.0, 0.0, nyquist, Rate::A),
            Param::new(0.0, -detune, detune, Rate::A),
            Param::new(1.0, -F, F, Rate::A),
            Param::new(0.0, -F, gain, Rate::A),
        ],
    )
}

/// An IIR filter of `feedforward` and `feedback` coefficients.
pub fn iir(feedforward: Vec<f64>, feedback: Vec<f64>) -> Node {
    Node::new(
        Kind::Iir(filters::Iir::new(feedforward, feedback)),
        1,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![],
    )
}

/// A delay of at most `max` seconds at `rate`: param delayTime.
pub fn delay(max: f64, rate: f32) -> Node {
    Node::new(
        Kind::Delay(delay::Delay::new(max, rate)),
        1,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![Param::new(0.0, 0.0, max as f32, Rate::A)],
    )
}

pub fn stereo_panner() -> Node {
    Node::new(
        Kind::StereoPanner,
        1,
        1,
        2,
        Mode::ClampedMax,
        Interpretation::Speakers,
        vec![Param::new(0.0, -1.0, 1.0, Rate::A)],
    )
}

/// A panner: params position x, y, z and orientation x, y, z.
pub fn panner() -> Node {
    Node::new(
        Kind::Panner(panner::Panner::default()),
        1,
        1,
        2,
        Mode::ClampedMax,
        Interpretation::Speakers,
        vec![
            Param::new(0.0, -F, F, Rate::A),
            Param::new(0.0, -F, F, Rate::A),
            Param::new(0.0, -F, F, Rate::A),
            Param::new(1.0, -F, F, Rate::A),
            Param::new(0.0, -F, F, Rate::A),
            Param::new(0.0, -F, F, Rate::A),
        ],
    )
}

pub fn splitter(outputs: usize) -> Node {
    Node::new(
        Kind::Splitter,
        1,
        outputs,
        outputs,
        Mode::Explicit,
        Interpretation::Discrete,
        vec![],
    )
}

pub fn merger(inputs: usize) -> Node {
    Node::new(
        Kind::Merger,
        inputs,
        1,
        1,
        Mode::Explicit,
        Interpretation::Speakers,
        vec![],
    )
}

pub fn shaper() -> Node {
    Node::new(
        Kind::Shaper(shaper::Shaper::default()),
        1,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![],
    )
}

/// A compressor: params threshold, knee, ratio, attack and release.
pub fn compressor(rate: f32) -> Node {
    Node::new(
        Kind::Compressor(compressor::Compressor::new(rate)),
        1,
        1,
        2,
        Mode::ClampedMax,
        Interpretation::Speakers,
        vec![
            Param::new(-24.0, -100.0, 0.0, Rate::K),
            Param::new(30.0, 0.0, 40.0, Rate::K),
            Param::new(12.0, 1.0, 20.0, Rate::K),
            Param::new(0.003, 0.0, 1.0, Rate::K),
            Param::new(0.25, 0.0, 1.0, Rate::K),
        ],
    )
}

pub fn convolver() -> Node {
    Node::new(
        Kind::Convolver(convolver::Convolver::default()),
        1,
        1,
        2,
        Mode::ClampedMax,
        Interpretation::Speakers,
        vec![],
    )
}

pub fn analyser() -> Node {
    Node::new(
        Kind::Analyser(analyser::Analyser::default()),
        1,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![],
    )
}

/// A node whose sound comes from outside the graph: a media element's.
pub fn external(feed: external::External) -> Node {
    Node::new(
        Kind::External(feed),
        0,
        1,
        2,
        Mode::Max,
        Interpretation::Speakers,
        vec![],
    )
}

impl Kind {
    /// Processes one quantum: the outputs, and whether a source ended in
    /// it.
    pub(crate) fn process(
        &mut self,
        context: &Context<'_>,
        inputs: &[Bus],
        params: &[[f32; QUANTUM]],
    ) -> (Vec<Bus>, bool) {
        let input = || inputs.first().cloned().unwrap_or_else(|| silence(1));
        match self {
            Kind::Destination => {
                let mut out = silence(context.channel_count);
                if let Some(bus) = inputs.first() {
                    crate::mix::mix_into(&mut out, bus, Interpretation::Speakers);
                }
                (vec![out], false)
            }
            Kind::Gain => {
                let mut bus = input();
                for channel in &mut bus {
                    for (s, g) in channel.iter_mut().zip(&params[0]) {
                        *s *= g;
                    }
                }
                (vec![bus], false)
            }
            Kind::Constant(schedule) => {
                let mut out = [0.0; QUANTUM];
                let (playing, ended) = schedule.frames(context.quantum);
                if playing.is_empty() {
                    return (vec![silence(1)], ended);
                }
                for i in playing {
                    out[i] = params[0][i];
                }
                (vec![vec![out]], ended)
            }
            Kind::Oscillator(oscillator) => oscillator.process(context.quantum, params),
            Kind::BufferSource(source) => source.process(context.quantum, params),
            Kind::Biquad(biquad) => (
                vec![biquad.process(context.quantum, &input(), params)],
                false,
            ),
            Kind::Iir(iir) => (vec![iir.process(&input())], false),
            Kind::Delay(delay) => {
                delay.write(&input());
                (vec![delay.read(context.quantum, &params[0], false)], false)
            }
            Kind::StereoPanner => (vec![panner::stereo(&input(), &params[0])], false),
            Kind::Panner(panner) => (
                vec![panner.process(&input(), params, context.listener, context.quantum)],
                false,
            ),
            Kind::Splitter => {
                let bus = input();
                let outputs = context.channel_count;
                (
                    (0..outputs)
                        .map(|i| vec![bus.get(i).copied().unwrap_or([0.0; QUANTUM])])
                        .collect(),
                    false,
                )
            }
            Kind::Merger => {
                let out: Bus = inputs.iter().map(|bus| bus[0]).collect();
                (vec![out], false)
            }
            Kind::Shaper(shaper) => (vec![shaper.process(&input())], false),
            Kind::Compressor(compressor) => (vec![compressor.process(&input(), params)], false),
            Kind::Convolver(convolver) => (vec![convolver.process(&input())], false),
            Kind::Analyser(analyser) => {
                let bus = input();
                analyser.take(&bus);
                (vec![bus], false)
            }
            Kind::External(feed) => (vec![feed.take()], false),
        }
    }

    /// A delay in a cycle: what it delayed, before this quantum's input.
    pub(crate) fn read_delayed(
        &mut self,
        context: &Context<'_>,
        params: &[[f32; QUANTUM]],
    ) -> (Vec<Bus>, bool) {
        match self {
            Kind::Delay(delay) => (vec![delay.read(context.quantum, &params[0], true)], false),
            _ => (vec![silence(1)], false),
        }
    }
}
