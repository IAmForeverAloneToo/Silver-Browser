//! The scheduled sources (Web Audio 1.1 section 1.2,
//! `AudioScheduledSourceNode`): `ConstantSourceNode`, `OscillatorNode`
//! and `AudioBufferSourceNode`, each sounding from its start until its
//! stop, its `ended` once it has stopped.

use std::sync::Arc;

use crate::graph::Quantum;
use crate::mix::{Bus, silence};
use crate::wave::{PeriodicWave, Shape};
use crate::{Buffer, QUANTUM};

/// When a source sounds: from its start (if started) until its stop.
#[derive(Debug, Clone, Default)]
pub struct Schedule {
    pub start: Option<f64>,
    pub stop: Option<f64>,
    /// Whether it has ended (its `ended` told).
    pub ended: bool,
}

impl Schedule {
    /// The frames of `quantum` it sounds in, and whether it ends in it.
    pub fn frames(&mut self, quantum: Quantum) -> (Vec<usize>, bool) {
        if self.ended {
            return (Vec::new(), false);
        }
        let Some(start) = self.start else {
            return (Vec::new(), false);
        };
        let frames: Vec<usize> = (0..QUANTUM)
            .filter(|&i| {
                let t = quantum.time(i);
                t >= start && self.stop.is_none_or(|stop| t < stop)
            })
            .collect();
        let end = quantum.time(QUANTUM);
        if self.stop.is_some_and(|stop| stop.max(start) < end) {
            self.ended = true;
            return (frames, true);
        }
        (frames, false)
    }
}

/// What an oscillator draws.
#[derive(Clone)]
pub enum Wave {
    Sine,
    Table(Arc<PeriodicWave>),
}

pub struct Oscillator {
    pub wave: Wave,
    pub schedule: Schedule,
    rate: f32,
    /// Where in its period it is, in periods.
    phase: f64,
}

impl Oscillator {
    pub fn new(rate: f32) -> Oscillator {
        Oscillator {
            wave: Wave::Sine,
            schedule: Schedule::default(),
            rate,
            phase: 0.0,
        }
    }

    /// Draws a built-in shape.
    pub fn set_shape(&mut self, shape: Shape) {
        self.wave = match shape {
            Shape::Sine => Wave::Sine,
            other => Wave::Table(PeriodicWave::shape(other, self.rate)),
        };
    }

    pub(crate) fn process(
        &mut self,
        quantum: Quantum,
        params: &[[f32; QUANTUM]],
    ) -> (Vec<Bus>, bool) {
        let (playing, ended) = self.schedule.frames(quantum);
        if playing.is_empty() {
            return (vec![silence(1)], ended);
        }
        let nyquist = f64::from(self.rate) / 2.0;
        let mut out = [0.0; QUANTUM];
        for i in playing {
            let frequency = f64::from(params[0][i]) * 2f64.powf(f64::from(params[1][i]) / 1200.0);
            let frequency = if frequency.is_finite() {
                frequency.clamp(-nyquist, nyquist)
            } else {
                0.0
            };
            out[i] = match &self.wave {
                Wave::Sine => (2.0 * std::f64::consts::PI * self.phase).sin() as f32,
                Wave::Table(table) => table.sample(self.phase, frequency),
            };
            self.phase = (self.phase + frequency / f64::from(self.rate)).rem_euclid(1.0);
        }
        (vec![vec![out]], ended)
    }
}

/// An `AudioBufferSourceNode`'s state.
#[derive(Default)]
pub struct BufferSource {
    pub buffer: Option<Buffer>,
    pub looping: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    pub schedule: Schedule,
    /// Where in the buffer it starts, in seconds, and how much of it it
    /// plays.
    pub offset: f64,
    pub duration: Option<f64>,
    /// Where in the buffer it is, in its frames; none before it starts.
    position: Option<f64>,
    /// The buffer's seconds it has played, for its `duration`.
    played: f64,
}

impl BufferSource {
    pub(crate) fn process(
        &mut self,
        quantum: Quantum,
        params: &[[f32; QUANTUM]],
    ) -> (Vec<Bus>, bool) {
        let (playing, mut ended) = self.schedule.frames(quantum);
        let Some(buffer) = self.buffer.clone() else {
            return (vec![silence(1)], ended);
        };
        if playing.is_empty() || buffer.channels.is_empty() {
            return (vec![silence(1)], ended);
        }
        let frames = buffer.frames();
        let length = frames as f64;
        // The computed playback rate, k-rate: the rate times the detune,
        // in the buffer's frames a frame of the context.
        let rate = f64::from(params[0][0]) * 2f64.powf(f64::from(params[1][0]) / 1200.0);
        let step = if rate.is_finite() {
            rate * f64::from(buffer.rate) / f64::from(quantum.rate)
        } else {
            0.0
        };
        let (mut loop_start, mut loop_end) = (
            self.loop_start * f64::from(buffer.rate),
            self.loop_end * f64::from(buffer.rate),
        );
        if !(self.loop_start < self.loop_end && self.loop_end > 0.0) || loop_end > length {
            loop_end = length;
        }
        if loop_start < 0.0 || loop_start >= loop_end {
            loop_start = 0.0;
        }
        let mut out: Bus = silence(buffer.channels.len());
        for i in playing {
            let position = *self
                .position
                .get_or_insert_with(|| (self.offset * f64::from(buffer.rate)).clamp(0.0, length));
            if !self.looping && (position >= length || position < 0.0 || frames == 0) {
                if !self.schedule.ended {
                    self.schedule.ended = true;
                    ended = true;
                }
                break;
            }
            if self.duration.is_some_and(|d| self.played >= d) {
                if !self.schedule.ended {
                    self.schedule.ended = true;
                    ended = true;
                }
                break;
            }
            let k = position.floor() as usize;
            let frac = (position - k as f64) as f32;
            for (channel, data) in out.iter_mut().zip(&buffer.channels) {
                let a = data.get(k).copied().unwrap_or(0.0);
                let b = if k + 1 < frames {
                    data[k + 1]
                } else if self.looping && frames > 0 {
                    data[loop_start as usize % frames]
                } else if k >= 1 {
                    // Past its last frame, the last step extended.
                    a + (a - data[k - 1])
                } else {
                    a
                };
                channel[i] = a + (b - a) * frac;
            }
            let mut next = position + step;
            self.played += step.abs() / f64::from(buffer.rate);
            if self.looping && loop_end > loop_start {
                let span = loop_end - loop_start;
                if next >= loop_end {
                    next = loop_start + (next - loop_end) % span;
                } else if next < loop_start && step < 0.0 {
                    next = loop_end - (loop_start - next) % span;
                }
            }
            self.position = Some(next);
        }
        (vec![out], ended)
    }
}
