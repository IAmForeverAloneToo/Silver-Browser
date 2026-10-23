//! Channels up-mixed and down-mixed (Web Audio 1.1 section 1.4.3,
//! "Channel Up-Mixing and Down-Mixing"): an input's connections each mixed
//! to the input's computed number of channels and summed.

use crate::QUANTUM;

/// A quantum of sound: one buffer of [`QUANTUM`] frames a channel.
pub type Bus = Vec<[f32; QUANTUM]>;

/// A bus of `channels` silent channels.
pub fn silence(channels: usize) -> Bus {
    vec![[0.0; QUANTUM]; channels.max(1)]
}

/// How a node's input decides its number of channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Max,
    ClampedMax,
    Explicit,
}

/// How channels are mixed between counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpretation {
    Speakers,
    Discrete,
}

/// An input's computed number of channels for connections of `counts`
/// channels: the most of them, at most `count`, or `count`.
pub fn computed_channels(counts: impl Iterator<Item = usize>, count: usize, mode: Mode) -> usize {
    let most = counts.max().unwrap_or(1).max(1);
    match mode {
        Mode::Max => most,
        Mode::ClampedMax => most.min(count),
        Mode::Explicit => count,
    }
}

const SQRT_HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Adds `from` mixed to `into`'s number of channels into `into`.
pub fn mix_into(into: &mut Bus, from: &Bus, interpretation: Interpretation) {
    let (n, m) = (from.len(), into.len());
    if n == m {
        for (o, i) in into.iter_mut().zip(from) {
            for (a, b) in o.iter_mut().zip(i) {
                *a += b;
            }
        }
        return;
    }
    if interpretation == Interpretation::Speakers && speakers(n, m, into, from) {
        return;
    }
    // Discrete: the channels both have, the rest silent or dropped.
    for (o, i) in into.iter_mut().zip(from) {
        for (a, b) in o.iter_mut().zip(i) {
            *a += b;
        }
    }
}

/// The speaker layouts' mixes: mono, stereo, quad and 5.1 (L R C LFE SL
/// SR). False for counts with no layout.
fn speakers(n: usize, m: usize, into: &mut Bus, from: &Bus) -> bool {
    let add = |o: &mut [f32; QUANTUM], k: f32, i: &[f32; QUANTUM]| {
        for (a, b) in o.iter_mut().zip(i) {
            *a += k * b;
        }
    };
    match (n, m) {
        // Up.
        (1, 2) => {
            add(&mut into[0], 1.0, &from[0]);
            add(&mut into[1], 1.0, &from[0]);
        }
        (1, 4) => {
            add(&mut into[0], 1.0, &from[0]);
            add(&mut into[1], 1.0, &from[0]);
        }
        (1, 6) => add(&mut into[2], 1.0, &from[0]),
        (2, 4) | (2, 6) => {
            add(&mut into[0], 1.0, &from[0]);
            add(&mut into[1], 1.0, &from[1]);
        }
        (4, 6) => {
            add(&mut into[0], 1.0, &from[0]);
            add(&mut into[1], 1.0, &from[1]);
            add(&mut into[4], 1.0, &from[2]);
            add(&mut into[5], 1.0, &from[3]);
        }
        // Down.
        (2, 1) => {
            add(&mut into[0], 0.5, &from[0]);
            add(&mut into[0], 0.5, &from[1]);
        }
        (4, 1) => {
            for channel in from.iter().take(4) {
                add(&mut into[0], 0.25, channel);
            }
        }
        (6, 1) => {
            add(&mut into[0], SQRT_HALF, &from[0]);
            add(&mut into[0], SQRT_HALF, &from[1]);
            add(&mut into[0], 1.0, &from[2]);
            add(&mut into[0], 0.5, &from[4]);
            add(&mut into[0], 0.5, &from[5]);
        }
        (4, 2) => {
            add(&mut into[0], 0.5, &from[0]);
            add(&mut into[0], 0.5, &from[2]);
            add(&mut into[1], 0.5, &from[1]);
            add(&mut into[1], 0.5, &from[3]);
        }
        (6, 2) => {
            add(&mut into[0], 1.0, &from[0]);
            add(&mut into[0], SQRT_HALF, &from[2]);
            add(&mut into[0], SQRT_HALF, &from[4]);
            add(&mut into[1], 1.0, &from[1]);
            add(&mut into[1], SQRT_HALF, &from[2]);
            add(&mut into[1], SQRT_HALF, &from[5]);
        }
        (6, 4) => {
            add(&mut into[0], 1.0, &from[0]);
            add(&mut into[0], SQRT_HALF, &from[2]);
            add(&mut into[1], 1.0, &from[1]);
            add(&mut into[1], SQRT_HALF, &from[2]);
            add(&mut into[2], 1.0, &from[4]);
            add(&mut into[3], 1.0, &from[5]);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(values: &[f32]) -> Bus {
        values.iter().map(|v| [*v; QUANTUM]).collect()
    }

    #[test]
    fn speakers_mix_by_the_standards_matrices_and_discrete_by_index() {
        let cases: [(&[f32], usize, &[f32]); 8] = [
            (&[0.5], 2, &[0.5, 0.5]),
            (&[0.5], 6, &[0.0, 0.0, 0.5, 0.0, 0.0, 0.0]),
            (&[1.0, 0.0], 1, &[0.5]),
            (&[1.0, 1.0, 1.0, 1.0], 1, &[1.0]),
            (&[1.0, 2.0, 3.0, 4.0], 2, &[2.0, 3.0]),
            (&[1.0, 1.0, 1.0, 9.0, 1.0, 1.0], 1, &[SQRT_HALF * 2.0 + 2.0]),
            (
                &[1.0, 1.0, 1.0, 9.0, 1.0, 1.0],
                2,
                &[1.0 + 2.0 * SQRT_HALF, 1.0 + 2.0 * SQRT_HALF],
            ),
            (&[1.0, 2.0, 3.0], 2, &[1.0, 2.0]),
        ];
        for (from, m, want) in cases {
            let mut into = silence(m);
            mix_into(&mut into, &bus(from), Interpretation::Speakers);
            let got: Vec<f32> = into.iter().map(|c| c[0]).collect();
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() < 1e-6, "{from:?} to {m}: {got:?}");
            }
        }
        let mut into = silence(2);
        mix_into(&mut into, &bus(&[0.5]), Interpretation::Discrete);
        assert_eq!((into[0][0], into[1][0]), (0.5, 0.0));
        assert_eq!(
            computed_channels([1, 6].into_iter(), 2, Mode::ClampedMax),
            2
        );
        assert_eq!(computed_channels([].into_iter(), 2, Mode::Max), 1);
    }
}
