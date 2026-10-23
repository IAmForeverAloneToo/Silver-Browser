//! A radix-2 complex FFT, in place: the oscillators' wave tables, the
//! analyser's spectrum and the convolver's partitions.

use std::f64::consts::PI;

/// A complex number.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub fn new(re: f64, im: f64) -> Complex {
        Complex { re, im }
    }

    fn mul(self, other: Complex) -> Complex {
        Complex::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }
}

/// Transforms `data` (its length a power of two) in place: forward
/// (`e^{-i...}`) or inverse (`e^{+i...}`), never scaled.
pub(crate) fn fft(data: &mut [Complex], inverse: bool) {
    let n = data.len();
    if n < 2 {
        return;
    }
    debug_assert!(n.is_power_of_two());
    // Bit-reversed order.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            data.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut size = 2;
    while size <= n {
        let angle = sign * 2.0 * PI / size as f64;
        let step = Complex::new(angle.cos(), angle.sin());
        for start in (0..n).step_by(size) {
            let mut w = Complex::new(1.0, 0.0);
            for k in 0..size / 2 {
                let a = data[start + k];
                let b = data[start + k + size / 2].mul(w);
                data[start + k] = Complex::new(a.re + b.re, a.im + b.im);
                data[start + k + size / 2] = Complex::new(a.re - b.re, a.im - b.im);
                w = w.mul(step);
            }
        }
        size *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transform_and_its_inverse_give_back_the_signal_scaled_by_its_length() {
        let n = 64;
        let signal: Vec<Complex> = (0..n)
            .map(|i| Complex::new((i as f64 * 0.37).sin(), (i as f64 * 0.11).cos()))
            .collect();
        let mut data = signal.clone();
        fft(&mut data, false);
        // Bin 0 is the sum.
        let sum = signal
            .iter()
            .fold((0.0, 0.0), |s, c| (s.0 + c.re, s.1 + c.im));
        assert!((data[0].re - sum.0).abs() < 1e-9 && (data[0].im - sum.1).abs() < 1e-9);
        fft(&mut data, true);
        for (a, b) in data.iter().zip(&signal) {
            assert!((a.re / n as f64 - b.re).abs() < 1e-9 && (a.im / n as f64 - b.im).abs() < 1e-9);
        }
    }
}
