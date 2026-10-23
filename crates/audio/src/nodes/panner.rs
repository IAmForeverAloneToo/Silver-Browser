//! `StereoPannerNode` (Web Audio 1.1 section 1.30) and `PannerNode`
//! (section 1.27) with the equal-power model, its distance models and
//! cone, and `AudioListener` (section 1.28).

use crate::QUANTUM;
use crate::graph::Quantum;
use crate::mix::{Bus, silence};
use crate::param::{Param, Rate};

use std::f64::consts::FRAC_PI_2;

/// The listener's params: position x, y, z; forward x, y, z; up x, y, z.
pub fn listener_params() -> Vec<Param> {
    let f = f32::MAX;
    [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0]
        .into_iter()
        .map(|v| Param::new(v, -f, f, Rate::A))
        .collect()
}

/// Equal power: `input` (mono or stereo) panned by `x` from 0 (left) to 1
/// (right) for a mono input, or as the standard's stereo steps say.
fn equal_power(left: f32, right: Option<f32>, pan: f64) -> (f32, f32) {
    match right {
        None => {
            let x = (pan + 1.0) / 2.0;
            let (gl, gr) = ((x * FRAC_PI_2).cos() as f32, (x * FRAC_PI_2).sin() as f32);
            (left * gl, left * gr)
        }
        Some(right) => {
            if pan <= 0.0 {
                let x = pan + 1.0;
                let (gl, gr) = ((x * FRAC_PI_2).cos() as f32, (x * FRAC_PI_2).sin() as f32);
                (left + right * gl, right * gr)
            } else {
                let x = pan;
                let (gl, gr) = ((x * FRAC_PI_2).cos() as f32, (x * FRAC_PI_2).sin() as f32);
                (left * gl, right + left * gr)
            }
        }
    }
}

/// The stereo panner's quantum: `pan` per frame.
pub(crate) fn stereo(input: &Bus, pan: &[f32; QUANTUM]) -> Bus {
    let mut out = silence(2);
    for i in 0..QUANTUM {
        let right = (input.len() > 1).then(|| input[1][i]);
        let (l, r) = equal_power(input[0][i], right, f64::from(pan[i]).clamp(-1.0, 1.0));
        out[0][i] = l;
        out[1][i] = r;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistanceModel {
    Linear,
    #[default]
    Inverse,
    Exponential,
}

/// A panner's attributes.
#[derive(Debug, Clone)]
pub struct Panner {
    pub distance_model: DistanceModel,
    pub ref_distance: f64,
    pub max_distance: f64,
    pub rolloff: f64,
    pub cone_inner: f64,
    pub cone_outer: f64,
    pub cone_outer_gain: f64,
}

impl Default for Panner {
    fn default() -> Self {
        Panner {
            distance_model: DistanceModel::Inverse,
            ref_distance: 1.0,
            max_distance: 10_000.0,
            rolloff: 1.0,
            cone_inner: 360.0,
            cone_outer: 360.0,
            cone_outer_gain: 0.0,
        }
    }
}

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn normalize(a: V3) -> Option<V3> {
    let l = length(a);
    (l > 0.0 && l.is_finite()).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

impl Panner {
    /// The azimuth of `source` heard by a listener at `listener` facing
    /// `forward` with `up` (the standard's "azimuth and elevation").
    fn azimuth(source: V3, listener: V3, forward: V3, up: V3) -> f64 {
        let Some(to_source) = normalize(sub(source, listener)) else {
            return 0.0;
        };
        let (Some(forward), Some(right)) = (normalize(forward), normalize(cross(forward, up)))
        else {
            return 0.0;
        };
        let up = cross(right, forward);
        let projection = dot(to_source, up);
        let Some(projected) = normalize(sub(
            to_source,
            [up[0] * projection, up[1] * projection, up[2] * projection],
        )) else {
            return 0.0;
        };
        let mut azimuth = dot(projected, right).clamp(-1.0, 1.0).acos().to_degrees();
        if dot(projected, forward) < 0.0 {
            azimuth = 360.0 - azimuth;
        }
        if (0.0..=270.0).contains(&azimuth) {
            90.0 - azimuth
        } else {
            450.0 - azimuth
        }
    }

    fn distance_gain(&self, distance: f64) -> f64 {
        let r = self.ref_distance;
        match self.distance_model {
            DistanceModel::Linear => {
                let max = self.max_distance.max(r);
                if max == r {
                    return 1.0 - self.rolloff.min(1.0);
                }
                1.0 - self.rolloff.min(1.0) * (distance.clamp(r, max) - r) / (max - r)
            }
            DistanceModel::Inverse => {
                if r == 0.0 {
                    return 0.0;
                }
                r / (r + self.rolloff * (distance.max(r) - r))
            }
            DistanceModel::Exponential => {
                if r == 0.0 {
                    return 0.0;
                }
                (distance.max(r) / r).powf(-self.rolloff)
            }
        }
    }

    fn cone_gain(&self, source: V3, orientation: V3, listener: V3) -> f64 {
        if self.cone_inner == 360.0 && self.cone_outer == 360.0 {
            return 1.0;
        }
        let (Some(orientation), Some(to_listener)) =
            (normalize(orientation), normalize(sub(listener, source)))
        else {
            return 1.0;
        };
        let angle = dot(to_listener, orientation)
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
            .abs();
        let (inner, outer) = (self.cone_inner.abs() / 2.0, self.cone_outer.abs() / 2.0);
        if angle <= inner {
            1.0
        } else if angle >= outer {
            self.cone_outer_gain
        } else {
            let x = (angle - inner) / (outer - inner);
            (1.0 - x) + self.cone_outer_gain * x
        }
    }

    pub(crate) fn process(
        &self,
        input: &Bus,
        params: &[[f32; QUANTUM]],
        listener: &[Param],
        quantum: Quantum,
    ) -> Bus {
        let mut out = silence(2);
        let mut listener_values = [[0.0f32; QUANTUM]; 9];
        for (values, param) in listener_values.iter_mut().zip(listener) {
            param.computed(quantum.start, quantum.rate, None, values);
        }
        for i in 0..QUANTUM {
            let p = |k: usize| f64::from(params[k][i]);
            let l = |k: usize| f64::from(listener_values[k][i]);
            let source = [p(0), p(1), p(2)];
            let orientation = [p(3), p(4), p(5)];
            let position = [l(0), l(1), l(2)];
            let forward = [l(3), l(4), l(5)];
            let up = [l(6), l(7), l(8)];
            let mut azimuth = Panner::azimuth(source, position, forward, up);
            azimuth = azimuth.clamp(-180.0, 180.0);
            if azimuth < -90.0 {
                azimuth = -180.0 - azimuth;
            } else if azimuth > 90.0 {
                azimuth = 180.0 - azimuth;
            }
            let gain = (self.distance_gain(length(sub(source, position)))
                * self.cone_gain(source, orientation, position)) as f32;
            let right = (input.len() > 1).then(|| input[1][i]);
            let pan = match right {
                None => azimuth / 90.0,
                Some(_) => azimuth / 90.0,
            };
            let (left, right) = equal_power(input[0][i], right, pan);
            out[0][i] = left * gain;
            out[1][i] = right * gain;
        }
        out
    }
}
