//! Sample rate conversion: a windowed sinc, its kernel tabulated at
//! [`PHASES`] fractional positions and interpolated between them, cut off
//! below the lower rate's Nyquist frequency. For `decodeAudioData()` into
//! a context of another rate, a media element's sound to the output's,
//! and a playback rate.

use std::f64::consts::PI;

/// Half the kernel's width, in input frames at the lower rate.
const HALF: usize = 16;
/// The kernel's tabulated positions between two input frames.
const PHASES: usize = 256;

/// A converter of interleaved sound from one rate to another, fed in
/// pieces.
pub struct Resampler {
    channels: usize,
    /// Input frames per output frame.
    step: f64,
    /// Half the kernel's width in input frames.
    reach: usize,
    /// The kernel: `taps * PHASES + 1` values, by position.
    table: Vec<f32>,
    /// The cutoff as a fraction of the input's Nyquist frequency.
    cutoff: f64,
    /// Input not yet wholly used, interleaved; `reach` frames of zeros
    /// before the first.
    history: Vec<f32>,
    /// The outputs made, and the input frames dropped from `history`:
    /// each output's position is worked out from them afresh, so that
    /// the result is the same however the input is cut.
    made: u64,
    dropped: u64,
}

fn blackman(x: f64) -> f64 {
    // `x` from -1 to 1.
    let t = (x + 1.0) / 2.0;
    0.42 - 0.5 * (2.0 * PI * t).cos() + 0.08 * (4.0 * PI * t).cos()
}

impl Resampler {
    /// From `from` to `to` frames a second; also any step, for a rate's
    /// change (`to` = `from` / rate).
    pub fn new(from: f64, to: f64, channels: usize) -> Resampler {
        let step = from / to;
        let cutoff = (to / from).min(1.0) * 0.97;
        let reach = ((HALF as f64) / cutoff).ceil() as usize;
        let span = reach * PHASES;
        let mut table = Vec::with_capacity(span + 2);
        for i in 0..=span + 1 {
            let x = i as f64 / PHASES as f64;
            let sinc = if x == 0.0 {
                1.0
            } else {
                (PI * x * cutoff).sin() / (PI * x * cutoff)
            };
            let window = if x >= reach as f64 {
                0.0
            } else {
                blackman(x / reach as f64)
            };
            table.push((sinc * window * cutoff) as f32);
        }
        Resampler {
            channels: channels.max(1),
            step,
            reach,
            table,
            cutoff,
            history: vec![0.0; reach * channels.max(1)],
            made: 0,
            dropped: 0,
        }
    }

    fn kernel(&self, distance: f64) -> f32 {
        let at = distance.abs() * PHASES as f64;
        let i = at as usize;
        if i + 1 >= self.table.len() {
            return 0.0;
        }
        let frac = (at - i as f64) as f32;
        self.table[i] + (self.table[i + 1] - self.table[i]) * frac
    }

    /// Converts `input` (interleaved), giving the output frames it makes
    /// possible; some of its last frames wait for what follows.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.history.extend_from_slice(input);
        let frames = self.history.len() / self.channels;
        let mut out = Vec::new();
        loop {
            let position = self.position();
            if position + self.reach as f64 >= frames as f64 {
                break;
            }
            let center = position.floor() as isize;
            let frac = position - center as f64;
            let lo = center - self.reach as isize + 1;
            let hi = center + self.reach as isize;
            for ch in 0..self.channels {
                let mut sum = 0.0f32;
                for k in lo..=hi {
                    if k < 0 || k as usize >= frames {
                        continue;
                    }
                    let distance = (k - center) as f64 - frac;
                    sum += self.history[k as usize * self.channels + ch] * self.kernel(distance);
                }
                out.push(sum);
            }
            self.made += 1;
        }
        // Drop what no output will need again.
        let keep_from = (self.position().floor() as usize).saturating_sub(self.reach);
        if keep_from > 0 {
            self.history.drain(..keep_from * self.channels);
            self.dropped += keep_from as u64;
        }
        out
    }

    /// The next output's position in `history`, in frames.
    fn position(&self) -> f64 {
        self.made as f64 * self.step + self.reach as f64 - self.dropped as f64
    }

    /// The output still due once the input has ended.
    pub fn flush(&mut self) -> Vec<f32> {
        let zeros = vec![0.0; (self.reach + 1) * self.channels];
        self.process(&zeros)
    }

    /// The cutoff, as a fraction of the input's Nyquist frequency.
    pub fn cutoff(&self) -> f64 {
        self.cutoff
    }
}

/// `samples` (interleaved, `channels` of them) from `from` Hz to `to` Hz,
/// whole: `floor(frames * to / from)` frames.
pub fn resample(samples: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
    let channels = channels.max(1);
    if from == to {
        return samples.to_vec();
    }
    let frames = samples.len() / channels;
    let want = (frames as u128 * u128::from(to) / u128::from(from.max(1))) as usize;
    let mut resampler = Resampler::new(f64::from(from), f64::from(to), channels);
    let mut out = resampler.process(samples);
    out.extend(resampler.flush());
    out.truncate(want * channels);
    out.resize(want * channels, 0.0);
    out
}
