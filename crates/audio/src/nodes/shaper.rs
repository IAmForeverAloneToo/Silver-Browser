//! `WaveShaperNode` (Web Audio 1.1 section 1.32): each sample mapped
//! through its curve, interpolated as the standard says; with 2x or 4x
//! oversampling, shaped at that rate between a windowed-sinc up-sampler
//! and down-sampler.

use crate::QUANTUM;
use crate::mix::Bus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Oversample {
    #[default]
    None,
    X2,
    X4,
}

/// Half a half-band filter's taps.
const TAPS: usize = 16;

/// A 2x half-band low-pass's taps (a windowed sinc cut at a quarter of
/// the doubled rate), `2 * TAPS + 1` of them.
fn halfband() -> Vec<f32> {
    let n = 2 * TAPS + 1;
    (0..n)
        .map(|i| {
            let x = i as f64 - TAPS as f64;
            let sinc = if x == 0.0 {
                0.5
            } else {
                (std::f64::consts::PI * x / 2.0).sin() / (std::f64::consts::PI * x)
            };
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos()
                + 0.08 * (4.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos();
            (2.0 * sinc * w) as f32
        })
        .collect()
}

/// A 2x up-sampler and down-sampler with their histories.
#[derive(Default, Clone)]
struct Stage {
    up: Vec<f32>,
    down: Vec<f32>,
}

#[derive(Default)]
pub struct Shaper {
    pub curve: Option<Vec<f32>>,
    pub oversample: Oversample,
    /// Each channel's stages.
    stages: Vec<Vec<Stage>>,
    taps: Vec<f32>,
}

/// `x` through `curve`.
pub fn shape(curve: &[f32], x: f32) -> f32 {
    let n = curve.len();
    if n == 0 {
        return x;
    }
    if n == 1 {
        return curve[0];
    }
    let v = (n - 1) as f32 / 2.0 * (x + 1.0);
    if !v.is_finite() || v <= 0.0 {
        return curve[0];
    }
    if v >= (n - 1) as f32 {
        return curve[n - 1];
    }
    let k = v.floor() as usize;
    let f = v - k as f32;
    curve[k] + f * (curve[k + 1] - curve[k])
}

fn filter(history: &mut Vec<f32>, taps: &[f32], x: f32) -> f32 {
    if history.len() != taps.len() {
        *history = vec![0.0; taps.len()];
    }
    history.rotate_right(1);
    history[0] = x;
    history.iter().zip(taps).map(|(h, t)| h * t).sum()
}

impl Shaper {
    pub(crate) fn process(&mut self, input: &Bus) -> Bus {
        let Some(curve) = self.curve.clone() else {
            return input.clone();
        };
        let factor = match self.oversample {
            Oversample::None => {
                let mut out = input.clone();
                for channel in &mut out {
                    for s in channel.iter_mut() {
                        *s = shape(&curve, *s);
                    }
                }
                return out;
            }
            Oversample::X2 => 1,
            Oversample::X4 => 2,
        };
        if self.taps.is_empty() {
            self.taps = halfband();
        }
        self.stages.resize(input.len(), Vec::new());
        let mut out = input.clone();
        for (channel, stages) in out.iter_mut().zip(&mut self.stages) {
            stages.resize(factor, Stage::default());
            let mut signal: Vec<f32> = channel.to_vec();
            for stage in stages.iter_mut() {
                let mut up = Vec::with_capacity(signal.len() * 2);
                for &x in &signal {
                    up.push(filter(&mut stage.up, &self.taps, x * 2.0));
                    up.push(filter(&mut stage.up, &self.taps, 0.0));
                }
                signal = up;
            }
            for s in &mut signal {
                *s = shape(&curve, *s);
            }
            for stage in stages.iter_mut().rev() {
                let mut down = Vec::with_capacity(signal.len() / 2);
                for pair in signal.chunks(2) {
                    let y = filter(&mut stage.down, &self.taps, pair[0]);
                    let _ = filter(&mut stage.down, &self.taps, pair[1]);
                    down.push(y);
                }
                signal = down;
            }
            channel.copy_from_slice(&signal[..QUANTUM]);
        }
        out
    }
}
