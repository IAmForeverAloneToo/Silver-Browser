//! `DelayNode` (Web Audio 1.1 section 1.18): a ring of what came, read at
//! each frame's delay with linear interpolation; in a cycle, read before
//! the quantum's input is written, its delay at least a quantum.

use crate::QUANTUM;
use crate::graph::Quantum;
use crate::mix::{Bus, silence};

pub struct Delay {
    /// Each channel's ring, allocated as channels come.
    rings: Vec<Vec<f32>>,
    size: usize,
    /// Frames written so far.
    written: u64,
}

impl Delay {
    /// A delay of at most `max` seconds at `rate`.
    pub fn new(max: f64, rate: f32) -> Delay {
        let frames = (max * f64::from(rate)).ceil().max(1.0) as usize;
        Delay {
            rings: Vec::new(),
            size: frames + 2 * QUANTUM + 1,
            written: 0,
        }
    }

    /// Takes a quantum of input.
    pub(crate) fn write(&mut self, input: &Bus) {
        if self.rings.len() < input.len() {
            self.rings.resize(input.len(), vec![0.0; self.size]);
        }
        for (ring, channel) in self
            .rings
            .iter_mut()
            .zip(input.iter().chain(std::iter::repeat(&[0.0; QUANTUM])))
        {
            for (i, sample) in channel.iter().enumerate() {
                ring[((self.written + i as u64) % self.size as u64) as usize] = *sample;
            }
        }
        self.written += QUANTUM as u64;
    }

    /// A quantum of what it delayed by `times` seconds; `cyclic` when the
    /// quantum's input is not written yet.
    pub(crate) fn read(&self, quantum: Quantum, times: &[f32; QUANTUM], cyclic: bool) -> Bus {
        if self.rings.is_empty() {
            return silence(1);
        }
        let base = if cyclic {
            self.written
        } else {
            self.written.saturating_sub(QUANTUM as u64)
        };
        let min = if cyclic { QUANTUM as f64 } else { 0.0 };
        let max = (self.size - 2 * QUANTUM) as f64;
        let mut out = silence(self.rings.len());
        for i in 0..QUANTUM {
            // In single precision, as the reference: 0.01 s at 48 kHz is
            // then 480 frames, not 479.99999.
            let delay = f64::from(times[i] * quantum.rate).clamp(min, max);
            let at = (base + i as u64) as f64 - delay;
            if at < 0.0 {
                continue;
            }
            let k = at.floor() as u64;
            let frac = (at - k as f64) as f32;
            for (channel, ring) in out.iter_mut().zip(&self.rings) {
                let a = ring[(k % self.size as u64) as usize];
                let b = if frac > 0.0 {
                    ring[((k + 1) % self.size as u64) as usize]
                } else {
                    a
                };
                channel[i] = a + (b - a) * frac;
            }
        }
        out
    }
}
