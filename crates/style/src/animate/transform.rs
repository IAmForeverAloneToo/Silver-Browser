//! Interpolating transforms (M22): the `transform` list by
//! css-transforms-1 §9 and css-transforms-2 §14, and the individual
//! `translate`, `rotate` and `scale` by css-transforms-2 §5.
//!
//! Two lists are made as long as each other by the identity functions
//! of the longer one's; from their start, pairs of functions of one
//! primitive interpolate as that primitive — the translations, the
//! scales and the rotations each one, the three skews each their own,
//! as the reference browser has them; a rotation about two different
//! axes, two `matrix()`es, two `matrix3d()`s and two `perspective()`s
//! as matrices — and at the first pair that is not, the rest of each list
//! becomes one matrix and the two interpolate by decomposition
//! (`crate::transform::Decomposed`). A matrix that does not decompose
//! makes the whole interpolation discrete (css-transforms-1 §11). A rest
//! with a translation's percentage in it waits for its box:
//! [`TransformFunction::Interpolated`] keeps the two rests and the
//! progress, nested at most [`MAX_MIX_DEPTH`] deep, past which the
//! interpolation is discrete.

use std::sync::Arc;

use super::{length, length_percentage, lerp};
use crate::cascade::MAX_LENGTH_PX;
use crate::computed::{LengthPercentage, Rotate, TransformFunction, TransformMix, Translate};
use crate::transform::{Matrix, flattened_2d, interpolate_matrices, list_matrix};

/// How deep interpolated transforms may nest inside one another: a
/// transition interrupted during one starts from it.
pub const MAX_MIX_DEPTH: usize = 8;

/// The identity function of `function`'s primitive (css-transforms-1
/// §9): what a shorter list is extended by.
fn identity_of(function: &TransformFunction) -> TransformFunction {
    match function {
        TransformFunction::Translate(..) => {
            TransformFunction::Translate(LengthPercentage::Px(0.0), LengthPercentage::Px(0.0), 0.0)
        }
        TransformFunction::Scale(..) => TransformFunction::Scale(1.0, 1.0, 1.0),
        TransformFunction::Rotate { x, y, z, .. } => TransformFunction::Rotate {
            x: *x,
            y: *y,
            z: *z,
            angle: 0.0,
        },
        TransformFunction::Skew(..) => TransformFunction::Skew(0.0, 0.0),
        TransformFunction::SkewX(_) => TransformFunction::SkewX(0.0),
        TransformFunction::SkewY(_) => TransformFunction::SkewY(0.0),
        TransformFunction::Matrix(_) => TransformFunction::Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        TransformFunction::Matrix3d(_) => TransformFunction::Matrix3d(Matrix::IDENTITY.0),
        TransformFunction::Perspective(_) => TransformFunction::Perspective(None),
        TransformFunction::Interpolated(_) if !function.is_3d() => {
            TransformFunction::Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
        }
        TransformFunction::Interpolated(_) => TransformFunction::Matrix3d(Matrix::IDENTITY.0),
    }
}

/// The function a matrix interpolated is: `matrix()` when both ends
/// were 2D, `matrix3d()` otherwise.
fn function_of(m: &Matrix, two_d: bool) -> TransformFunction {
    if two_d {
        let c = flattened_2d(m).0;
        TransformFunction::Matrix([c[0], c[1], c[4], c[5], c[12], c[13]])
    } else {
        TransformFunction::Matrix3d(m.0)
    }
}

/// A number of a transform function interpolated, within the magnitude
/// the parser keeps them to.
fn factor(a: f32, b: f32, p: f64) -> f32 {
    let v = lerp(f64::from(a), f64::from(b), p);
    if v.is_nan() {
        return a;
    }
    v.clamp(-f64::from(MAX_LENGTH_PX), f64::from(MAX_LENGTH_PX)) as f32
}

/// A rotation's axis made a unit vector, or `None` when it has no
/// length.
fn unit(x: f32, y: f32, z: f32) -> Option<[f64; 3]> {
    let (x, y, z) = (f64::from(x), f64::from(y), f64::from(z));
    let length = (x * x + y * y + z * z).sqrt();
    (length > 0.0 && length.is_finite()).then(|| [x / length, y / length, z / length])
}

/// Whether two rotations turn about one axis.
fn same_axis(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6)
}

/// How two functions at one place interpolate.
enum Pair {
    /// As their primitive, or as matrices of their own.
    Done(TransformFunction),
    /// Not of one primitive: the rests of the lists meet as matrices.
    Different,
    /// As matrices that do not decompose: the whole list is discrete.
    Discrete,
}

fn pair(a: &TransformFunction, b: &TransformFunction, p: f64) -> Pair {
    use TransformFunction::*;
    let matrices = |a: &TransformFunction, b: &TransformFunction| {
        let (ma, mb) = (a.matrix(0.0, 0.0), b.matrix(0.0, 0.0));
        match interpolate_matrices(&ma, &mb, p) {
            Some(m) => Pair::Done(function_of(&m, ma.is_2d() && mb.is_2d())),
            None => Pair::Discrete,
        }
    };
    match (a, b) {
        (Translate(ax, ay, az), Translate(bx, by, bz)) => match (
            length_percentage(ax, bx, p, false),
            length_percentage(ay, by, p, false),
        ) {
            (Some(x), Some(y)) => Pair::Done(Translate(x, y, length(*az, *bz, p, false))),
            _ => Pair::Discrete,
        },
        (Scale(ax, ay, az), Scale(bx, by, bz)) => Pair::Done(Scale(
            factor(*ax, *bx, p),
            factor(*ay, *by, p),
            factor(*az, *bz, p),
        )),
        (Skew(ax, ay), Skew(bx, by)) => Pair::Done(Skew(factor(*ax, *bx, p), factor(*ay, *by, p))),
        (SkewX(a), SkewX(b)) => Pair::Done(SkewX(factor(*a, *b, p))),
        (SkewY(a), SkewY(b)) => Pair::Done(SkewY(factor(*a, *b, p))),
        (
            Rotate {
                x: ax,
                y: ay,
                z: az,
                angle: a_angle,
            },
            Rotate {
                x: bx,
                y: by,
                z: bz,
                angle: b_angle,
            },
        ) => {
            // css-transforms-2 §14: one axis, or an angle of zero, turns
            // by the angles; two axes and two turns, by matrices.
            let a_turns = *a_angle != 0.0 && unit(*ax, *ay, *az).is_some();
            let b_turns = *b_angle != 0.0 && unit(*bx, *by, *bz).is_some();
            let one_axis = match (unit(*ax, *ay, *az), unit(*bx, *by, *bz)) {
                (Some(u), Some(v)) => same_axis(u, v),
                _ => false,
            };
            if !(a_turns && b_turns) || one_axis {
                let (x, y, z) = if a_turns {
                    (*ax, *ay, *az)
                } else if b_turns {
                    (*bx, *by, *bz)
                } else {
                    (0.0, 0.0, 1.0)
                };
                Pair::Done(Rotate {
                    x,
                    y,
                    z,
                    angle: factor(*a_angle, *b_angle, p),
                })
            } else {
                matrices(a, b)
            }
        }
        // Of one name only: `matrix()` and `matrix3d()` share no
        // primitive (css-transforms-2 §12.3, §14).
        (Matrix(_), Matrix(_)) | (Matrix3d(_), Matrix3d(_)) | (Perspective(_), Perspective(_)) => {
            matrices(a, b)
        }
        _ => Pair::Different,
    }
}

/// Whether a list's matrix depends on its reference box: a translation
/// by a percentage, or an interpolation still waiting for one.
fn needs_box(list: &[TransformFunction]) -> bool {
    list.iter().any(|function| match function {
        TransformFunction::Translate(x, y, _) => {
            !matches!(x, LengthPercentage::Px(_)) || !matches!(y, LengthPercentage::Px(_))
        }
        TransformFunction::Interpolated(_) => true,
        _ => false,
    })
}

/// How deep interpolated transforms nest in a list.
fn mix_depth(list: &[TransformFunction]) -> usize {
    list.iter()
        .map(|function| match function {
            TransformFunction::Interpolated(mix) => {
                1 + mix_depth(&mix.from).max(mix_depth(&mix.to))
            }
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

/// The rests of two lists interpolated as matrices: now when neither
/// needs its box, or kept for the box otherwise.
fn rests(a: &[TransformFunction], b: &[TransformFunction], p: f64) -> Option<TransformFunction> {
    if needs_box(a) || needs_box(b) {
        if 1 + mix_depth(a).max(mix_depth(b)) > MAX_MIX_DEPTH {
            return None;
        }
        return Some(TransformFunction::Interpolated(Arc::new(TransformMix {
            from: a.to_vec(),
            to: b.to_vec(),
            progress: p,
        })));
    }
    let (ma, mb) = (list_matrix(a, 0.0, 0.0), list_matrix(b, 0.0, 0.0));
    let m = interpolate_matrices(&ma, &mb, p)?;
    Some(function_of(&m, ma.is_2d() && mb.is_2d()))
}

/// The `transform` list `p` of the way from `a` to `b`
/// (css-transforms-1 §9), or `None` when the interpolation is discrete.
pub fn lists(
    a: &[TransformFunction],
    b: &[TransformFunction],
    p: f64,
) -> Option<Vec<TransformFunction>> {
    let n = a.len().max(b.len());
    let extended = |list: &[TransformFunction], longer: &[TransformFunction]| {
        let mut out = list.to_vec();
        out.extend(longer[list.len()..].iter().map(identity_of));
        out
    };
    let (a, b) = if a.len() < n {
        (extended(a, b), b.to_vec())
    } else {
        (a.to_vec(), extended(b, a))
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        match pair(&a[i], &b[i], p) {
            Pair::Done(function) => out.push(function),
            Pair::Discrete => return None,
            Pair::Different => {
                out.push(rests(&a[i..], &b[i..], p)?);
                break;
            }
        }
    }
    Some(out)
}

/// `translate` interpolated (css-transforms-2 §5): `none` is a
/// translation by nothing, and two `none`s are `none`.
pub fn translate(
    a: &Option<Translate>,
    b: &Option<Translate>,
    p: f64,
) -> Option<Option<Translate>> {
    if a.is_none() && b.is_none() {
        return Some(None);
    }
    let zero = Translate {
        x: LengthPercentage::Px(0.0),
        y: LengthPercentage::Px(0.0),
        z: 0.0,
    };
    let (a, b) = (a.as_ref().unwrap_or(&zero), b.as_ref().unwrap_or(&zero));
    Some(Some(Translate {
        x: length_percentage(&a.x, &b.x, p, false)?,
        y: length_percentage(&a.y, &b.y, p, false)?,
        z: length(a.z, b.z, p, false),
    }))
}

/// A rotation as a unit quaternion `x, y, z, w`.
fn quaternion(r: &Rotate) -> [f64; 4] {
    let Some([x, y, z]) = unit(r.x, r.y, r.z) else {
        return [0.0, 0.0, 0.0, 1.0];
    };
    let half = f64::from(r.angle).to_radians() / 2.0;
    let s = half.sin();
    [x * s, y * s, z * s, half.cos()]
}

/// `rotate` interpolated (css-transforms-2 §5): `none` turns by nothing
/// about the other's axis, and two `none`s are `none`; one axis, or an
/// angle of zero, turns by the angles, the turns kept; two axes and two
/// turns, by spherical interpolation of their quaternions, as
/// `rotate: x 90deg` to `rotate: y 90deg` half way is 70.5288° about
/// `0.707107 0.707107 0` in the reference browser.
pub fn rotate(a: &Option<Rotate>, b: &Option<Rotate>, p: f64) -> Option<Option<Rotate>> {
    let (a, b) = match (a, b) {
        (None, None) => return Some(None),
        (Some(a), None) => (*a, Rotate { angle: 0.0, ..*a }),
        (None, Some(b)) => (Rotate { angle: 0.0, ..*b }, *b),
        (Some(a), Some(b)) => (*a, *b),
    };
    let a_turns = a.angle != 0.0 && unit(a.x, a.y, a.z).is_some();
    let b_turns = b.angle != 0.0 && unit(b.x, b.y, b.z).is_some();
    let one_axis = match (unit(a.x, a.y, a.z), unit(b.x, b.y, b.z)) {
        (Some(u), Some(v)) => same_axis(u, v),
        _ => false,
    };
    if !(a_turns && b_turns) || one_axis {
        let axis = if a_turns { a } else { b };
        return Some(Some(Rotate {
            angle: factor(a.angle, b.angle, p),
            ..axis
        }));
    }
    let (qa, qb) = (quaternion(&a), quaternion(&b));
    let product = (0..4).map(|i| qa[i] * qb[i]).sum::<f64>().clamp(-1.0, 1.0);
    let q = if product.abs() == 1.0 {
        qa
    } else {
        let theta = product.acos();
        let w = (p * theta).sin() / (1.0 - product * product).sqrt();
        let s = (p * theta).cos() - product * w;
        std::array::from_fn(|i| qa[i] * s + qb[i] * w)
    };
    let w = q[3].clamp(-1.0, 1.0);
    let angle = 2.0 * w.acos();
    let s = (angle / 2.0).sin();
    let (x, y, z) = if s.abs() < 1e-12 {
        (0.0, 0.0, 1.0)
    } else {
        (q[0] / s, q[1] / s, q[2] / s)
    };
    Some(Some(Rotate {
        x: x as f32,
        y: y as f32,
        z: z as f32,
        angle: angle.to_degrees() as f32,
    }))
}

/// `scale` interpolated (css-transforms-2 §5): `none` is a scale by one,
/// and two `none`s are `none`.
pub fn scale(a: &Option<[f32; 3]>, b: &Option<[f32; 3]>, p: f64) -> Option<[f32; 3]> {
    if a.is_none() && b.is_none() {
        return None;
    }
    let one = [1.0; 3];
    let (a, b) = (a.unwrap_or(one), b.unwrap_or(one));
    Some(std::array::from_fn(|i| factor(a[i], b[i], p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn percent_translation() -> Vec<TransformFunction> {
        vec![
            TransformFunction::Translate(
                LengthPercentage::Percent(50.0),
                LengthPercentage::Px(0.0),
                0.0,
            ),
            TransformFunction::Scale(2.0, 2.0, 1.0),
        ]
    }

    #[test]
    fn interpolations_waiting_for_their_box_nest_only_so_deep() {
        // A transition interrupted during one starts from it: each
        // interpolation holds the last, until the bound.
        let turn = vec![TransformFunction::Rotate {
            x: 0.0,
            y: 0.0,
            z: 1.0,
            angle: 90.0,
        }];
        let mut value = percent_translation();
        let mut steps = 0;
        while let Some(next) = lists(&value, &turn, 0.5) {
            value = next;
            steps += 1;
            assert!(steps <= MAX_MIX_DEPTH, "{steps}");
            // And it ends at the other list's matrix whatever the depth.
            assert!(mix_depth(&value) <= MAX_MIX_DEPTH);
        }
        assert_eq!(steps, MAX_MIX_DEPTH);
        // The other way, from the list to one: at its end, its matrix.
        let mix = lists(&percent_translation(), &turn, 1.0).expect("interpolates");
        let m = list_matrix(&mix, 100.0, 100.0);
        let want = list_matrix(&turn, 100.0, 100.0);
        assert!(m.0.iter().zip(want.0).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn a_shorter_list_takes_the_identity_of_the_longer_ones_functions() {
        let longer = vec![
            TransformFunction::Scale(3.0, 3.0, 1.0),
            TransformFunction::Rotate {
                x: 0.0,
                y: 0.0,
                z: 1.0,
                angle: 100.0,
            },
        ];
        let shorter = vec![TransformFunction::Scale(1.0, 1.0, 1.0)];
        assert_eq!(
            lists(&shorter, &longer, 0.5),
            Some(vec![
                TransformFunction::Scale(2.0, 2.0, 1.0),
                TransformFunction::Rotate {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                    angle: 50.0
                }
            ])
        );
        assert_eq!(lists(&[], &[], 0.5), Some(vec![]));
        // A rotation of nothing takes the other's axis; two of nothing,
        // z.
        let still = Rotate {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            angle: 0.0,
        };
        let turn = Rotate {
            x: 0.0,
            y: 1.0,
            z: 0.0,
            angle: 40.0,
        };
        assert_eq!(
            rotate(&Some(still), &Some(turn), 0.5),
            Some(Some(Rotate {
                angle: 20.0,
                ..turn
            }))
        );
        assert_eq!(rotate(&None, &None, 0.5), Some(None));
    }
}
