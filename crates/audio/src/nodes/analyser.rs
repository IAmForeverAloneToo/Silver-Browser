//! `AnalyserNode` (Web Audio 1.1 section 1.8): what passes through it,
//! down-mixed to mono and kept, its time-domain data the latest
//! `fftSize` frames and its frequency data their Blackman-windowed
//! spectrum smoothed over time, in dB. The control thread reads them
//! through the shared state.

use std::sync::{Arc, Mutex};

use crate::fft::{Complex, fft};
use crate::mix::{Bus, Interpretation, mix_into, silence};
use crate::{MAX_FFT_SIZE, QUANTUM};

/// The analyser's state, shared with the control thread.
pub struct AnalyserData {
    pub fft_size: usize,
    pub smoothing: f64,
    pub min_db: f64,
    pub max_db: f64,
    /// The latest frames, a ring of `MAX_FFT_SIZE`.
    ring: Vec<f32>,
    at: usize,
    /// The smoothed magnitudes, by bin.
    smoothed: Vec<f64>,
    /// The quanta taken, and the one the frequency data was last made in.
    quanta: u64,
    made: Option<u64>,
}

impl Default for AnalyserData {
    fn default() -> Self {
        AnalyserData {
            fft_size: 2048,
            smoothing: 0.8,
            min_db: -100.0,
            max_db: -30.0,
            ring: vec![0.0; MAX_FFT_SIZE],
            at: 0,
            smoothed: Vec::new(),
            quanta: 0,
            made: None,
        }
    }
}

impl AnalyserData {
    /// The latest `fft_size` frames, oldest first.
    fn latest(&self) -> Vec<f32> {
        let n = self.fft_size;
        (0..n)
            .map(|i| self.ring[(self.at + MAX_FFT_SIZE - n + i) % MAX_FFT_SIZE])
            .collect()
    }

    /// `getFloatTimeDomainData()`.
    pub fn float_time(&self, out: &mut [f32]) {
        for (o, v) in out.iter_mut().zip(self.latest()) {
            *o = v;
        }
    }

    /// `getByteTimeDomainData()`.
    pub fn byte_time(&self, out: &mut [u8]) {
        for (o, v) in out.iter_mut().zip(self.latest()) {
            *o = (128.0 * (1.0 + v)).floor().clamp(0.0, 255.0) as u8;
        }
    }

    /// The spectrum, smoothed, made once a quantum however often asked.
    fn spectrum(&mut self) -> &[f64] {
        let bins = self.fft_size / 2;
        if self.made == Some(self.quanta) && self.smoothed.len() == bins {
            return &self.smoothed;
        }
        let n = self.fft_size;
        let latest = self.latest();
        let mut data: Vec<Complex> = latest
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = i as f64 / n as f64;
                let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * x).cos()
                    + 0.08 * (4.0 * std::f64::consts::PI * x).cos();
                Complex::new(f64::from(*v) * w, 0.0)
            })
            .collect();
        fft(&mut data, false);
        if self.smoothed.len() != bins {
            self.smoothed = vec![0.0; bins];
        }
        let tau = self.smoothing;
        for (s, c) in self.smoothed.iter_mut().zip(&data) {
            let magnitude = c.re.hypot(c.im) / n as f64;
            let next = tau * *s + (1.0 - tau) * magnitude;
            *s = if next.is_finite() { next } else { 0.0 };
        }
        self.made = Some(self.quanta);
        &self.smoothed
    }

    /// `getFloatFrequencyData()`, in dB.
    pub fn float_frequency(&mut self, out: &mut [f32]) {
        let spectrum = self.spectrum().to_vec();
        for (o, v) in out.iter_mut().zip(spectrum) {
            *o = (20.0 * v.log10()) as f32;
        }
    }

    /// `getByteFrequencyData()`.
    pub fn byte_frequency(&mut self, out: &mut [u8]) {
        let (min, max) = (self.min_db, self.max_db);
        let spectrum = self.spectrum().to_vec();
        for (o, v) in out.iter_mut().zip(spectrum) {
            let db = 20.0 * v.log10();
            let scaled = 255.0 / (max - min) * (db - min);
            *o = if scaled.is_finite() {
                scaled.floor().clamp(0.0, 255.0) as u8
            } else {
                0
            };
        }
    }
}

#[derive(Default)]
pub struct Analyser {
    pub shared: Arc<Mutex<AnalyserData>>,
}

impl Analyser {
    pub(crate) fn take(&mut self, bus: &Bus) {
        let mut mono = silence(1);
        mix_into(&mut mono, bus, Interpretation::Speakers);
        if let Ok(mut data) = self.shared.lock() {
            for v in mono[0].iter().take(QUANTUM) {
                let at = data.at;
                data.ring[at] = *v;
                data.at = (at + 1) % MAX_FFT_SIZE;
            }
            data.quanta += 1;
        }
    }
}
