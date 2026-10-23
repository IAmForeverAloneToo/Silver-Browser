//! `PeriodicWave` (Web Audio 1.1 section 1.31): a period's waveform from
//! its Fourier coefficients, band-limited as the reference browser does
//! it, so that an oscillator's numbers are its: a table for each third of
//! an octave above the lowest fundamental its period can hold, each
//! keeping the partials below the Nyquist frequency at the top of its
//! band; an oscillator reads the two tables about its frequency and
//! mixes them by where it falls between; all scaled so that the table
//! with every partial peaks at 1 (probe p5's square, sawtooth and
//! triangle).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::fft::{Complex, fft};

/// The bands an octave is cut into.
const BANDS_PER_OCTAVE: f64 = 3.0;
const CENTS_PER_BAND: f64 = 1200.0 / BANDS_PER_OCTAVE;

/// The built-in shapes' waves, by shape and table size.
type Cache = Mutex<HashMap<(Shape, usize), Arc<PeriodicWave>>>;

/// The oscillators' built-in shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

/// A periodic wave's band-limited tables.
#[derive(Debug)]
pub struct PeriodicWave {
    size: usize,
    tables: Vec<Vec<f32>>,
    /// The lowest fundamental its period holds every partial of.
    lowest: f64,
}

/// The table size for a context's rate: the reference's.
fn table_size(rate: f32) -> usize {
    if rate <= 24_000.0 {
        2048
    } else if rate <= 88_200.0 {
        4096
    } else {
        16_384
    }
}

impl PeriodicWave {
    /// The wave of the coefficients `real` (cosines) and `imag` (sines),
    /// the first of each, the DC term, ignored; scaled to a peak of 1
    /// unless `normalize` is false.
    pub fn new(real: &[f32], imag: &[f32], normalize: bool, rate: f32) -> PeriodicWave {
        let size = table_size(rate);
        let max_partials = size / 2;
        let ranges = (BANDS_PER_OCTAVE * (size as f64).log2()).round() as usize;
        let given = real.len().max(imag.len());
        let mut tables = Vec::with_capacity(ranges);
        for range in 0..ranges {
            let culled = 2f64.powf(-(range as f64) * CENTS_PER_BAND / 1200.0);
            let partials = ((culled * max_partials as f64) as usize)
                .min(given.saturating_sub(1))
                .min(max_partials - 1);
            let mut data = vec![Complex::default(); size];
            for (n, slot) in data.iter_mut().enumerate().take(partials + 1).skip(1) {
                let a = f64::from(real.get(n).copied().unwrap_or(0.0));
                let b = f64::from(imag.get(n).copied().unwrap_or(0.0));
                *slot = Complex::new(a, -b);
            }
            fft(&mut data, true);
            tables.push(data.iter().map(|c| c.re as f32).collect::<Vec<f32>>());
        }
        if normalize && let Some(full) = tables.first() {
            let peak = full.iter().fold(0f32, |m, v| m.max(v.abs()));
            if peak > 0.0 {
                let scale = 1.0 / peak;
                for table in &mut tables {
                    for v in table.iter_mut() {
                        *v *= scale;
                    }
                }
            }
        }
        PeriodicWave {
            size,
            tables,
            lowest: f64::from(rate) / 2.0 / max_partials as f64,
        }
    }

    /// The built-in shape's wave at `rate`, made once.
    pub fn shape(shape: Shape, rate: f32) -> Arc<PeriodicWave> {
        static CACHE: OnceLock<Cache> = OnceLock::new();
        let size = table_size(rate);
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(map) = cache.lock()
            && let Some(wave) = map.get(&(shape, size))
        {
            return wave.clone();
        }
        let n = size / 2;
        let mut imag = vec![0.0f32; n];
        for (k, b) in imag.iter_mut().enumerate().skip(1) {
            let k_f = k as f64;
            let pi = std::f64::consts::PI;
            *b = match shape {
                Shape::Sine => {
                    if k == 1 {
                        1.0
                    } else {
                        0.0
                    }
                }
                Shape::Square => (2.0 / (k_f * pi) * (1.0 - (-1f64).powi(k as i32))) as f32,
                Shape::Sawtooth => ((-1f64).powi(k as i32 + 1) * 2.0 / (k_f * pi)) as f32,
                Shape::Triangle => (8.0 * (k_f * pi / 2.0).sin() / (pi * k_f).powi(2)) as f32,
            };
        }
        let wave = Arc::new(PeriodicWave::new(&[], &imag, true, rate));
        if let Ok(mut map) = cache.lock() {
            map.insert((shape, size), wave.clone());
        }
        wave
    }

    /// The wave at `phase` (in periods) for a fundamental of
    /// `frequency` Hz.
    pub fn sample(&self, phase: f64, frequency: f64) -> f32 {
        let ranges = self.tables.len();
        let ratio = if frequency.abs() > 0.0 {
            frequency.abs() / self.lowest
        } else {
            0.5
        };
        let cents = ratio.log2() * 1200.0;
        let pitch = (1.0 + cents / CENTS_PER_BAND).clamp(0.0, (ranges - 1) as f64);
        let higher = pitch.floor() as usize;
        let lower = (higher + 1).min(ranges - 1);
        let mix = (pitch - higher as f64) as f32;
        let at = phase.rem_euclid(1.0) * self.size as f64;
        let k = (at as usize).min(self.size - 1);
        let frac = (at - k as f64) as f32;
        let read = |table: &[f32]| {
            let a = table[k];
            let b = table[(k + 1) % self.size];
            a + (b - a) * frac
        };
        (1.0 - mix) * read(&self.tables[higher]) + mix * read(&self.tables[lower])
    }
}
