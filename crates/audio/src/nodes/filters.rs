//! `BiquadFilterNode` (Web Audio 1.1 section 1.13, its coefficients the
//! Audio EQ Cookbook's as the standard writes them) and `IIRFilterNode`
//! (section 1.20), each with its frequency response.

use crate::QUANTUM;
use crate::graph::Quantum;
use crate::mix::Bus;

/// A biquad's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BiquadType {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Lowshelf,
    Highshelf,
    Peaking,
    Notch,
    Allpass,
}

/// Normalized coefficients: b0, b1, b2, a1, a2 (a0 divided out).
type Coefficients = [f64; 5];

/// The coefficients for `kind` at `frequency` Hz (detune applied), `q`,
/// `gain` dB, at `rate`.
pub fn coefficients(
    kind: BiquadType,
    frequency: f64,
    q: f64,
    gain: f64,
    rate: f64,
) -> Coefficients {
    let nyquist = rate / 2.0;
    let f0 = frequency.clamp(0.0, nyquist);
    let w0 = 2.0 * std::f64::consts::PI * f0 / rate;
    let (sin, cos) = w0.sin_cos();
    let a = 10f64.powf(gain / 40.0);
    let alpha_q = sin / (2.0 * q);
    let alpha_q_db = sin / (2.0 * 10f64.powf(q / 20.0));
    // The shelf slope S is 1, as the standard has it.
    let slope = 1.0;
    let alpha_s = sin / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt();
    let sqrt_a = a.sqrt();
    let pass = [1.0, 0.0, 0.0, 0.0, 0.0];
    let zero = [0.0, 0.0, 0.0, 0.0, 0.0];
    // The edges the formulas divide by zero at, as the reference treats
    // them.
    match kind {
        BiquadType::Lowpass if f0 >= nyquist => return pass,
        BiquadType::Lowpass if f0 <= 0.0 => return zero,
        BiquadType::Highpass if f0 >= nyquist => return zero,
        BiquadType::Highpass if f0 <= 0.0 => return pass,
        BiquadType::Bandpass | BiquadType::Notch | BiquadType::Allpass | BiquadType::Peaking
            if f0 <= 0.0 || f0 >= nyquist =>
        {
            return match kind {
                BiquadType::Bandpass => zero,
                BiquadType::Peaking => pass,
                _ => pass,
            };
        }
        _ => {}
    }
    let (b0, b1, b2, a0, a1, a2) = match kind {
        BiquadType::Lowpass => (
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha_q_db,
            -2.0 * cos,
            1.0 - alpha_q_db,
        ),
        BiquadType::Highpass => (
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha_q_db,
            -2.0 * cos,
            1.0 - alpha_q_db,
        ),
        BiquadType::Bandpass => (
            alpha_q,
            0.0,
            -alpha_q,
            1.0 + alpha_q,
            -2.0 * cos,
            1.0 - alpha_q,
        ),
        BiquadType::Notch => (
            1.0,
            -2.0 * cos,
            1.0,
            1.0 + alpha_q,
            -2.0 * cos,
            1.0 - alpha_q,
        ),
        BiquadType::Allpass => (
            1.0 - alpha_q,
            -2.0 * cos,
            1.0 + alpha_q,
            1.0 + alpha_q,
            -2.0 * cos,
            1.0 - alpha_q,
        ),
        BiquadType::Peaking => (
            1.0 + alpha_q * a,
            -2.0 * cos,
            1.0 - alpha_q * a,
            1.0 + alpha_q / a,
            -2.0 * cos,
            1.0 - alpha_q / a,
        ),
        BiquadType::Lowshelf => (
            a * ((a + 1.0) - (a - 1.0) * cos + 2.0 * alpha_s * sqrt_a),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - 2.0 * alpha_s * sqrt_a),
            (a + 1.0) + (a - 1.0) * cos + 2.0 * alpha_s * sqrt_a,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - 2.0 * alpha_s * sqrt_a,
        ),
        BiquadType::Highshelf => (
            a * ((a + 1.0) + (a - 1.0) * cos + 2.0 * alpha_s * sqrt_a),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - 2.0 * alpha_s * sqrt_a),
            (a + 1.0) - (a - 1.0) * cos + 2.0 * alpha_s * sqrt_a,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - 2.0 * alpha_s * sqrt_a,
        ),
    };
    let c = [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
    if c.iter().all(|v| v.is_finite()) {
        c
    } else {
        pass
    }
}

/// The response of normalized coefficients at `frequency` Hz: its
/// magnitude and phase.
fn response(b: &[f64], a: &[f64], frequency: f64, rate: f64) -> (f32, f32) {
    let w = 2.0 * std::f64::consts::PI * frequency / rate;
    let at_w = |c: &[f64]| {
        c.iter().enumerate().fold((0.0, 0.0), |(re, im), (k, v)| {
            let (s, co) = (-(k as f64) * w).sin_cos();
            (re + v * co, im + v * s)
        })
    };
    let (nr, ni) = at_w(b);
    let (dr, di) = at_w(a);
    let den = dr * dr + di * di;
    let re = (nr * dr + ni * di) / den;
    let im = (ni * dr - nr * di) / den;
    ((re.hypot(im)) as f32, im.atan2(re) as f32)
}

/// The biquad's state: its type, and each channel's two inputs and two
/// outputs before.
#[derive(Default)]
pub struct Biquad {
    pub kind: BiquadType,
    history: Vec<[f64; 4]>,
}

impl Biquad {
    pub(crate) fn process(
        &mut self,
        quantum: Quantum,
        input: &Bus,
        params: &[[f32; QUANTUM]],
    ) -> Bus {
        let rate = f64::from(quantum.rate);
        self.history.resize(input.len(), [0.0; 4]);
        // The params as a-rate: the coefficients again wherever they
        // change.
        let mut c = [0.0; 5];
        let mut last = None;
        let mut out = input.clone();
        for i in 0..QUANTUM {
            let key = (params[0][i], params[1][i], params[2][i], params[3][i]);
            if last != Some(key) {
                let frequency = f64::from(key.0) * 2f64.powf(f64::from(key.1) / 1200.0);
                c = coefficients(
                    self.kind,
                    frequency,
                    f64::from(key.2),
                    f64::from(key.3),
                    rate,
                );
                last = Some(key);
            }
            for (ch, channel) in out.iter_mut().enumerate() {
                let h = &mut self.history[ch];
                let x = f64::from(channel[i]);
                let y = c[0] * x + c[1] * h[0] + c[2] * h[1] - c[3] * h[2] - c[4] * h[3];
                *h = [x, h[0], y, h[2]];
                channel[i] = y as f32;
            }
        }
        // A state gone non-finite starts over.
        for h in &mut self.history {
            if h.iter().any(|v| !v.is_finite()) {
                *h = [0.0; 4];
            }
        }
        out
    }

    /// `getFrequencyResponse()` for the params' current values.
    pub fn frequency_response(
        &self,
        frequency: f64,
        detune: f64,
        q: f64,
        gain: f64,
        rate: f64,
        at: &[f32],
    ) -> Vec<(f32, f32)> {
        let f0 = frequency * 2f64.powf(detune / 1200.0);
        let c = coefficients(self.kind, f0, q, gain, rate);
        let nyquist = rate / 2.0;
        at.iter()
            .map(|&f| {
                let f = f64::from(f);
                if !(0.0..=nyquist).contains(&f) {
                    return (f32::NAN, f32::NAN);
                }
                response(&c[..3], &[1.0, c[3], c[4]], f, rate)
            })
            .collect()
    }
}

/// An IIR filter: its coefficients, normalized, and each channel's
/// inputs and outputs before.
pub struct Iir {
    b: Vec<f64>,
    a: Vec<f64>,
    history: Vec<(Vec<f64>, Vec<f64>)>,
}

impl Iir {
    /// `feedback[0]` is not zero (the bindings check).
    pub fn new(feedforward: Vec<f64>, feedback: Vec<f64>) -> Iir {
        let a0 = feedback
            .first()
            .copied()
            .filter(|v| *v != 0.0)
            .unwrap_or(1.0);
        Iir {
            b: feedforward.iter().map(|v| v / a0).collect(),
            a: feedback.iter().map(|v| v / a0).collect(),
            history: Vec::new(),
        }
    }

    pub(crate) fn process(&mut self, input: &Bus) -> Bus {
        let (nb, na) = (self.b.len(), self.a.len());
        self.history
            .resize(input.len(), (vec![0.0; nb], vec![0.0; na]));
        let mut out = input.clone();
        for (channel, (xs, ys)) in out.iter_mut().zip(&mut self.history) {
            for sample in channel.iter_mut() {
                xs.rotate_right(1);
                xs[0] = f64::from(*sample);
                let mut y: f64 = self.b.iter().zip(xs.iter()).map(|(b, x)| b * x).sum();
                y -= self
                    .a
                    .iter()
                    .skip(1)
                    .zip(ys.iter())
                    .map(|(a, y)| a * y)
                    .sum::<f64>();
                if !y.is_finite() {
                    y = 0.0;
                }
                ys.rotate_right(1);
                if !ys.is_empty() {
                    ys[0] = y;
                }
                *sample = y as f32;
            }
        }
        out
    }

    /// `getFrequencyResponse()`.
    pub fn frequency_response(&self, rate: f64, at: &[f32]) -> Vec<(f32, f32)> {
        let nyquist = rate / 2.0;
        at.iter()
            .map(|&f| {
                let f = f64::from(f);
                if !(0.0..=nyquist).contains(&f) {
                    return (f32::NAN, f32::NAN);
                }
                response(&self.b, &self.a, f, rate)
            })
            .collect()
    }
}
