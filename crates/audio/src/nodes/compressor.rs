//! `DynamicsCompressorNode` (Web Audio 1.1 section 1.19): the input
//! delayed 6 ms (its look-ahead) and scaled by a gain that follows the
//! compression curve's reduction of the input's level, falling at the
//! attack's pace and rising at the release's, then made up by
//! (1 / curve(1)) ^ 0.6. The curve is linear below the threshold, bends
//! over the knee, and above it keeps the ratio. The reference browser's
//! kernel is its own refinement of this (adaptive release, a different
//! knee): its numbers are not this one's (probe p5).

use crate::QUANTUM;
use crate::mix::{Bus, silence};

/// The look-ahead, in seconds.
const PRE_DELAY: f64 = 0.006;

pub struct Compressor {
    rate: f32,
    /// The gain applied, in dB (0 or less).
    gain_db: f64,
    /// The look-ahead's ring, each channel's.
    delay: Vec<Vec<f32>>,
    at: usize,
    /// The last quantum's reduction, for the `reduction` attribute.
    pub reduction: f32,
}

impl Compressor {
    pub fn new(rate: f32) -> Compressor {
        Compressor {
            rate,
            gain_db: 0.0,
            delay: Vec::new(),
            at: 0,
            reduction: 0.0,
        }
    }

    /// The curve: an input level in dB to an output level in dB.
    fn curve(x: f64, threshold: f64, knee: f64, ratio: f64) -> f64 {
        if x <= threshold {
            return x;
        }
        if knee > 0.0 && x < threshold + knee {
            // A quadratic bend from slope 1 at the threshold to slope
            // 1/ratio at its end.
            let d = x - threshold;
            return x + (1.0 / ratio - 1.0) * d * d / (2.0 * knee);
        }
        let knee_end = threshold + knee + (1.0 / ratio - 1.0) * knee / 2.0;
        knee_end + (x - threshold - knee) / ratio
    }

    pub(crate) fn process(&mut self, input: &Bus, params: &[[f32; QUANTUM]]) -> Bus {
        let [threshold, knee, ratio, attack, release] =
            [0, 1, 2, 3, 4].map(|i| f64::from(params[i][0]));
        let rate = f64::from(self.rate);
        let length = ((PRE_DELAY * rate).round() as usize).max(1);
        if self.delay.len() != input.len() || self.delay.first().is_some_and(|d| d.len() != length)
        {
            self.delay = vec![vec![0.0; length]; input.len()];
            self.at = 0;
        }
        let attack_k = (-1.0 / (attack.max(1e-4) * rate)).exp();
        let release_k = (-1.0 / (release.max(1e-4) * rate)).exp();
        let makeup_db = -0.6 * Compressor::curve(0.0, threshold, knee, ratio.max(1.0));
        let mut out = silence(input.len());
        for i in 0..QUANTUM {
            let level = input.iter().map(|c| c[i].abs()).fold(0f32, f32::max);
            let x = if level > 0.0 {
                20.0 * f64::from(level).log10()
            } else {
                -1000.0
            };
            let target = Compressor::curve(x, threshold, knee, ratio.max(1.0)) - x;
            let k = if target < self.gain_db {
                attack_k
            } else {
                release_k
            };
            self.gain_db = target + (self.gain_db - target) * k;
            let gain = 10f64.powf((self.gain_db + makeup_db) / 20.0) as f32;
            for (ch, channel) in input.iter().enumerate() {
                let delayed = self.delay[ch][self.at];
                self.delay[ch][self.at] = channel[i];
                out[ch][i] = delayed * gain;
            }
            self.at = (self.at + 1) % length;
        }
        self.reduction = self.gain_db as f32;
        out
    }
}
