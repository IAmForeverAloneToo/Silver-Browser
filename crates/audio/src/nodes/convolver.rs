//! `ConvolverNode` (Web Audio 1.1 section 1.17): the input convolved with
//! an impulse response of 1, 2 or 4 channels, directly when it is short
//! and by uniformly partitioned FFT convolution (overlap-save, a quantum a
//! partition) when not; scaled, unless `normalize` was false when the
//! response was given, by the reference browser's RMS normalization
//! (probe p5: its -58 dB calibration at 44.1 kHz).

use crate::fft::{Complex, fft};
use crate::mix::{Bus, silence};
use crate::{AudioData, QUANTUM};

/// The longest response convolved directly.
const DIRECT: usize = 256;
const N: usize = 2 * QUANTUM;

/// One response channel's partitions' spectra.
struct Partitions(Vec<Vec<Complex>>);

impl Partitions {
    fn new(response: &[f32]) -> Partitions {
        Partitions(
            response
                .chunks(QUANTUM)
                .map(|chunk| {
                    let mut data = vec![Complex::default(); N];
                    for (d, v) in data.iter_mut().zip(chunk) {
                        d.re = f64::from(*v);
                    }
                    fft(&mut data, false);
                    data
                })
                .collect(),
        )
    }
}

/// One input channel's convolution state.
#[derive(Default)]
struct Line {
    /// Direct: the last inputs, newest first.
    recent: Vec<f32>,
    /// Partitioned: the previous block, and the input spectra, newest
    /// first.
    previous: Vec<f32>,
    spectra: Vec<Vec<Complex>>,
}

impl Line {
    fn direct(&mut self, response: &[f32], input: &[f32; QUANTUM]) -> [f32; QUANTUM] {
        let n = response.len();
        if self.recent.len() != n {
            self.recent = vec![0.0; n];
        }
        let mut out = [0.0; QUANTUM];
        for (o, &x) in out.iter_mut().zip(input) {
            self.recent.rotate_right(1);
            self.recent[0] = x;
            *o = self.recent.iter().zip(response).map(|(a, b)| a * b).sum();
        }
        out
    }

    fn partitioned(&mut self, partitions: &Partitions, input: &[f32; QUANTUM]) -> [f32; QUANTUM] {
        if self.previous.len() != QUANTUM {
            self.previous = vec![0.0; QUANTUM];
        }
        let mut data = vec![Complex::default(); N];
        for (d, v) in data
            .iter_mut()
            .zip(self.previous.iter().chain(input.iter()))
        {
            d.re = f64::from(*v);
        }
        self.previous.copy_from_slice(input);
        fft(&mut data, false);
        self.spectra.insert(0, data);
        self.spectra.truncate(partitions.0.len());
        let mut sum = vec![Complex::default(); N];
        for (x, h) in self.spectra.iter().zip(&partitions.0) {
            for ((s, a), b) in sum.iter_mut().zip(x).zip(h) {
                s.re += a.re * b.re - a.im * b.im;
                s.im += a.re * b.im + a.im * b.re;
            }
        }
        fft(&mut sum, true);
        let mut out = [0.0; QUANTUM];
        for (o, v) in out.iter_mut().zip(&sum[QUANTUM..]) {
            *o = (v.re / N as f64) as f32;
        }
        out
    }
}

/// A response, scaled, with its partitions when long.
struct Response {
    channels: Vec<Vec<f32>>,
    partitions: Vec<Partitions>,
}

#[derive(Default)]
pub struct Convolver {
    response: Option<Response>,
    /// A line per (input channel, response channel) pair used.
    lines: Vec<Line>,
}

/// The reference's normalization of a response: the inverse of its RMS
/// power, calibrated to -58 dB at 44.1 kHz, halved for a true-stereo
/// (four channel) response.
pub fn normalization(response: &AudioData) -> f32 {
    let channels = response.channels.len().max(1);
    let length = response.frames().max(1);
    let power: f64 = response
        .channels
        .iter()
        .flatten()
        .map(|v| f64::from(*v) * f64::from(*v))
        .sum();
    let mut power = (power / (channels * length) as f64).sqrt();
    if !power.is_finite() || power < 0.000125 {
        power = 0.000125;
    }
    let mut scale = 1.0 / power * 10f64.powf(-58.0 * 0.05);
    if response.rate > 0.0 {
        scale *= 44_100.0 / f64::from(response.rate);
    }
    if channels == 4 {
        scale *= 0.5;
    }
    scale as f32
}

impl Convolver {
    /// Takes `response` (1, 2 or 4 channels), normalized if `normalize`.
    pub fn set_response(&mut self, response: Option<&AudioData>, normalize: bool) {
        self.lines.clear();
        self.response = response.map(|r| {
            let scale = if normalize { normalization(r) } else { 1.0 };
            let channels: Vec<Vec<f32>> = r
                .channels
                .iter()
                .map(|c| c.iter().map(|v| v * scale).collect())
                .collect();
            let partitions = if r.frames() > DIRECT {
                channels.iter().map(|c| Partitions::new(c)).collect()
            } else {
                Vec::new()
            };
            Response {
                channels,
                partitions,
            }
        });
    }

    fn convolve(&mut self, line: usize, response: usize, input: &[f32; QUANTUM]) -> [f32; QUANTUM] {
        let Some(r) = &self.response else {
            return [0.0; QUANTUM];
        };
        if self.lines.len() <= line {
            self.lines.resize_with(line + 1, Line::default);
        }
        let line = &mut self.lines[line];
        if r.partitions.is_empty() {
            line.direct(&r.channels[response], input)
        } else {
            line.partitioned(&r.partitions[response], input)
        }
    }

    pub(crate) fn process(&mut self, input: &Bus) -> Bus {
        let Some(response) = &self.response else {
            return silence(1);
        };
        let ir = response.channels.len();
        let left = input[0];
        let right = input.get(1).copied().unwrap_or(left);
        let add = |a: [f32; QUANTUM], b: [f32; QUANTUM]| {
            let mut s = a;
            for (x, y) in s.iter_mut().zip(b) {
                *x += y;
            }
            s
        };
        match (input.len(), ir) {
            (1, 1) => vec![self.convolve(0, 0, &left)],
            (_, 1) => vec![self.convolve(0, 0, &left), self.convolve(1, 0, &right)],
            (1, 2) => vec![self.convolve(0, 0, &left), self.convolve(1, 1, &left)],
            (_, 2) => vec![self.convolve(0, 0, &left), self.convolve(1, 1, &right)],
            (_, 4) => {
                let l = add(self.convolve(0, 0, &left), self.convolve(2, 2, &right));
                let r = add(self.convolve(1, 1, &left), self.convolve(3, 3, &right));
                vec![l, r]
            }
            _ => silence(2),
        }
    }
}
