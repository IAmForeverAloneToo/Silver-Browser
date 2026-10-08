//! Easing functions (CSS Easing 1 and 2, M22): what a transition's or an
//! animation's `*-timing-function` computes to, and the output progress
//! each gives for an input progress.
//!
//! The cascade parses them (`cascade::motion`); this module holds their
//! computed values and evaluates them: the identity `linear`; a cubic
//! Bézier, its curve solved for its x and extended past [0, 1] along its
//! tangents at the ends (Easing 1 §2.2.1); a step function, with the
//! before flag (§2.3.1); and `linear()`'s control points, canonicalized
//! when the value is computed and interpolated, or extrapolated along
//! the first or last two points, between them (Easing 2 §2.1).
//!
//! An input is any real number: chained easing functions, an
//! animation's and then a keyframe's, give a keyframe's easing what the
//! animation's made of its input, which may overshoot. The output is
//! kept within [`MAX_PROGRESS`](crate::easing::MAX_PROGRESS) of zero
//! and is never `NaN`, so that what interpolates by it stays finite
//! however the numbers were written.

use std::sync::Arc;

/// The most an output progress may be away from zero, either way. A
/// curve that rises past it still draws the same on any screen: every
/// interpolated value is clamped to its property's range long before.
pub const MAX_PROGRESS: f64 = 1e9;

/// How close to the input a cubic Bézier's curve is solved for its x.
const BEZIER_EPSILON: f64 = 1e-7;

/// An easing function's computed value.
#[derive(Debug, Clone, PartialEq)]
pub enum Easing {
    /// `linear`, the identity.
    Linear,
    /// A cubic Bézier through (0, 0) and (1, 1) with the control points
    /// (x1, y1) and (x2, y2), the x values in [0, 1]: `ease`, `ease-in`,
    /// `ease-out`, `ease-in-out` and `cubic-bezier()`.
    CubicBezier { x1: f64, y1: f64, x2: f64, y2: f64 },
    /// `steps()`, `step-start` and `step-end`: `count` steps, at least
    /// one, at least two at [`StepPosition::JumpNone`].
    Steps { count: u32, position: StepPosition },
    /// `linear()`, its control points canonicalized (Easing 2 §2.1): an
    /// input progress and its output each, the inputs never decreasing
    /// along the list, two points at least.
    Points(Arc<[(f64, f64)]>),
}

/// Where a step function's steps rise (Easing 1 §2.3). `start` is
/// [`StepPosition::JumpStart`] and `end` [`StepPosition::JumpEnd`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepPosition {
    /// The first rise at the input 0.
    JumpStart,
    /// The last rise at the input 1, the default.
    JumpEnd,
    /// Every rise inside (0, 1).
    JumpNone,
    /// A rise at 0 and one at 1.
    JumpBoth,
}

impl Easing {
    /// `ease`, the initial value of both timing functions.
    pub const EASE: Easing = Easing::CubicBezier {
        x1: 0.25,
        y1: 0.1,
        x2: 0.25,
        y2: 1.0,
    };
    /// `ease-in`.
    pub const EASE_IN: Easing = Easing::CubicBezier {
        x1: 0.42,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
    /// `ease-out`.
    pub const EASE_OUT: Easing = Easing::CubicBezier {
        x1: 0.0,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };
    /// `ease-in-out`.
    pub const EASE_IN_OUT: Easing = Easing::CubicBezier {
        x1: 0.42,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };

    /// The output progress for `input`, `before` being the before flag
    /// (Easing 1 §2.3.1: the animation has yet to reach its interval),
    /// which only a step function and the first point of `linear()`
    /// heed. Within [`MAX_PROGRESS`] of zero, never `NaN`.
    pub fn output(&self, input: f64, before: bool) -> f64 {
        let input = if input.is_nan() { 0.0 } else { input };
        let output = match self {
            Easing::Linear => input,
            Easing::CubicBezier { x1, y1, x2, y2 } => bezier(*x1, *y1, *x2, *y2, input),
            Easing::Steps { count, position } => steps(*count, *position, input, before),
            Easing::Points(points) => points_output(points, input, before),
        };
        if output.is_nan() {
            0.0
        } else {
            output.clamp(-MAX_PROGRESS, MAX_PROGRESS)
        }
    }
}

impl Default for Easing {
    fn default() -> Self {
        Easing::EASE
    }
}

/// A cubic Bézier easing function's output for `x` (Easing 1 §2.2.1).
fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    if x < 0.0 {
        // Along the tangent at P0: through P1, or P2 where P1 is on the
        // y axis, or flat where both are.
        let slope = if x1 > 0.0 {
            y1 / x1
        } else if x2 > 0.0 {
            y2 / x2
        } else {
            0.0
        };
        return slope * x;
    }
    if x > 1.0 {
        // Along the tangent at P3: through P2, or P1 where P2 is on the
        // line x = 1, or flat where both are.
        let slope = if x2 < 1.0 {
            (y2 - 1.0) / (x2 - 1.0)
        } else if x1 < 1.0 {
            (y1 - 1.0) / (x1 - 1.0)
        } else {
            0.0
        };
        return 1.0 + slope * (x - 1.0);
    }
    let curve = Curve::new(x1, y1, x2, y2);
    curve.y(curve.solve_x(x))
}

/// A cubic Bézier through (0, 0) and (1, 1) in polynomial form: x(t) =
/// ((ax t + bx) t + cx) t, and y(t) likewise.
struct Curve {
    ax: f64,
    bx: f64,
    cx: f64,
    ay: f64,
    by: f64,
    cy: f64,
}

impl Curve {
    fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Curve {
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        Curve {
            ax: 1.0 - cx - bx,
            bx,
            cx,
            ay: 1.0 - cy - by,
            by,
            cy,
        }
    }

    fn x(&self, t: f64) -> f64 {
        ((self.ax * t + self.bx) * t + self.cx) * t
    }

    fn y(&self, t: f64) -> f64 {
        ((self.ay * t + self.by) * t + self.cy) * t
    }

    fn dx(&self, t: f64) -> f64 {
        (3.0 * self.ax * t + 2.0 * self.bx) * t + self.cx
    }

    /// The parameter t in [0, 1] whose x is `x`, itself in [0, 1]: by
    /// Newton's method from t = x, which converges in a few steps for
    /// any curve whose slope there is not flat, and by bisection, which
    /// always does, where it does not. x(t) never decreases on [0, 1],
    /// the control points' x values being in it.
    fn solve_x(&self, x: f64) -> f64 {
        let mut t = x;
        for _ in 0..8 {
            let error = self.x(t) - x;
            if error.abs() < BEZIER_EPSILON {
                return t;
            }
            let slope = self.dx(t);
            if slope.abs() < 1e-6 {
                break;
            }
            t -= error / slope;
        }
        let (mut low, mut high) = (0.0, 1.0);
        t = x;
        // Fifty halvings take the interval below 10⁻¹⁵.
        for _ in 0..50 {
            let value = self.x(t);
            if (value - x).abs() < BEZIER_EPSILON {
                break;
            }
            if value < x {
                low = t;
            } else {
                high = t;
            }
            t = (low + high) / 2.0;
        }
        t
    }
}

/// A step easing function's output for `input` (Easing 1 §2.3.1).
fn steps(count: u32, position: StepPosition, input: f64, before: bool) -> f64 {
    let count = f64::from(count);
    let scaled = input * count;
    let mut step = scaled.floor();
    if matches!(position, StepPosition::JumpStart | StepPosition::JumpBoth) {
        step += 1.0;
    }
    if before && scaled % 1.0 == 0.0 {
        step -= 1.0;
    }
    if input >= 0.0 && step < 0.0 {
        step = 0.0;
    }
    let jumps = match position {
        StepPosition::JumpStart | StepPosition::JumpEnd => count,
        StepPosition::JumpNone => count - 1.0,
        StepPosition::JumpBoth => count + 1.0,
    };
    if input <= 1.0 && step > jumps {
        step = jumps;
    }
    step / jumps
}

/// `linear()`'s output for `input` (Easing 2 §2.1.2).
fn points_output(points: &[(f64, f64)], input: f64, before: bool) -> f64 {
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return input;
    };
    if points.len() == 1 {
        return first.1;
    }
    if input == first.0 && before {
        return first.1;
    }
    if let Some(&(_, output)) = points.iter().rev().find(|(at, _)| *at == input) {
        return output;
    }
    let (a, b) = if input < first.0 {
        let (a, b) = (points[0], points[1]);
        if a.0 == b.0 {
            return a.1;
        }
        (a, b)
    } else if input > last.0 {
        let (a, b) = (points[points.len() - 2], last);
        if a.0 == b.0 {
            return b.1;
        }
        (a, b)
    } else {
        // The last point before the input and the first after it: the
        // input lies strictly between two inputs of the list.
        let after = points
            .iter()
            .position(|(at, _)| *at > input)
            .expect("the input is below the last point's");
        (points[after - 1], points[after])
    };
    a.1 + (b.1 - a.1) * (input - a.0) / (b.0 - a.0)
}

/// Canonicalizes `linear()`'s control points (Easing 2 §2.1), `written`
/// being each point's output and its input where one was written, into
/// [`Easing::Points`]' inputs and outputs. The first point without an
/// input takes 0 and the last 1; an input below an earlier one takes the
/// largest before it; and each run of points still without one is
/// spaced evenly between its neighbours'.
pub fn canonical_points(written: &[(f64, Option<f64>)]) -> Vec<(f64, f64)> {
    let mut inputs: Vec<Option<f64>> = written.iter().map(|(_, input)| *input).collect();
    if let Some(first) = inputs.first_mut() {
        first.get_or_insert(0.0);
    }
    if let Some(last) = inputs.last_mut() {
        last.get_or_insert(1.0);
    }
    let mut largest = f64::NEG_INFINITY;
    for input in inputs.iter_mut().flatten() {
        largest = largest.max(*input);
        *input = largest;
    }
    let mut i = 0;
    while i < inputs.len() {
        if inputs[i].is_some() {
            i += 1;
            continue;
        }
        // A run from `i` to `end`, with inputs on both sides: the first
        // and the last points have theirs.
        let end = (i..inputs.len())
            .find(|&j| inputs[j].is_some())
            .expect("the last point has an input");
        let from = inputs[i - 1].expect("the point before a run has an input");
        let to = inputs[end].expect("found with one");
        let spans = (end - i + 1) as f64;
        for (k, input) in inputs[i..end].iter_mut().enumerate() {
            *input = Some(from + (to - from) * (k + 1) as f64 / spans);
        }
        i = end;
    }
    written
        .iter()
        .zip(inputs)
        .map(|((output, _), input)| (input.expect("every point has its input"), *output))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn the_keywords_are_their_curves() {
        assert_eq!(Easing::default(), Easing::EASE);
        for easing in [
            Easing::EASE,
            Easing::EASE_IN,
            Easing::EASE_OUT,
            Easing::EASE_IN_OUT,
        ] {
            assert_eq!(easing.output(0.0, false), 0.0);
            assert!(close(easing.output(1.0, false), 1.0));
        }
        assert_eq!(Easing::Linear.output(0.3, false), 0.3);
        assert_eq!(Easing::Linear.output(-2.0, true), -2.0);
        // Each against the curve sampled at a million points of t, the
        // y of the sample whose x is nearest the input.
        for easing in [Easing::EASE, Easing::EASE_IN, Easing::EASE_OUT] {
            let Easing::CubicBezier { x1, y1, x2, y2 } = easing else {
                unreachable!()
            };
            let at = |t: f64, a: f64, b: f64| {
                3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t
            };
            for x in [0.1, 0.3, 0.7, 0.9] {
                let t = (0..=1_000_000)
                    .map(|i| f64::from(i) / 1e6)
                    .min_by(|a, b| {
                        (at(*a, x1, x2) - x)
                            .abs()
                            .total_cmp(&(at(*b, x1, x2) - x).abs())
                    })
                    .expect("samples");
                let y = easing.output(x, false);
                assert!((y - at(t, y1, y2)).abs() < 1e-5, "{easing:?} {x} {y}");
            }
        }
        // A symmetric curve is a half at a half.
        assert!(close(Easing::EASE_IN_OUT.output(0.5, false), 0.5));
    }

    #[test]
    fn a_cubic_bezier_is_solved_as_the_reference_browser_solves_it() {
        // `cubic-bezier(0.1, 0.7, 1.0, 0.1)` takes a margin from 0 to
        // 100px to 36.9156px three tenths of the way through in the
        // reference browser.
        let curve = Easing::CubicBezier {
            x1: 0.1,
            y1: 0.7,
            x2: 1.0,
            y2: 0.1,
        };
        assert!((curve.output(0.3, false) * 100.0 - 36.9156).abs() < 1e-3);
        // Its overshoot: `cubic-bezier(0.5, 2, 0.5, 2)` is 1.625 half way.
        let over = Easing::CubicBezier {
            x1: 0.5,
            y1: 2.0,
            x2: 0.5,
            y2: 2.0,
        };
        assert!(close(over.output(0.5, false), 1.625));
        // A flat start: x(t) has no slope at 0, where Newton's method
        // stalls and bisection finds it.
        let flat = Easing::CubicBezier {
            x1: 0.0,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        };
        for x in [0.0, 1e-9, 0.01, 0.5, 0.99, 1.0] {
            let y = flat.output(x, false);
            assert!((0.0..=1.0).contains(&y), "{x} {y}");
        }
        assert!(close(flat.output(0.5, false), 0.5));
    }

    #[test]
    fn a_cubic_bezier_goes_on_along_its_tangents() {
        // Easing 1 §2.2.1: through P0 and P1 below 0, P2 and P3 above 1.
        let curve = Easing::CubicBezier {
            x1: 0.5,
            y1: 1.0,
            x2: 0.5,
            y2: 0.0,
        };
        assert!(close(curve.output(-0.5, false), -1.0));
        assert!(close(curve.output(1.5, false), 2.0));
        // P1 on the y axis: through P2 instead; both there, flat.
        let steep = Easing::CubicBezier {
            x1: 0.0,
            y1: 3.0,
            x2: 0.5,
            y2: 0.5,
        };
        assert!(close(steep.output(-1.0, false), -1.0));
        let flat = Easing::CubicBezier {
            x1: 0.0,
            y1: 3.0,
            x2: 0.0,
            y2: 0.5,
        };
        assert_eq!(flat.output(-1.0, false), 0.0);
        let level = Easing::CubicBezier {
            x1: 1.0,
            y1: 0.0,
            x2: 1.0,
            y2: 0.0,
        };
        assert_eq!(level.output(2.0, false), 1.0);
    }

    #[test]
    fn steps_rise_where_their_position_says() {
        let steps = |count, position| Easing::Steps { count, position };
        let end = steps(4, StepPosition::JumpEnd);
        // `steps(4)` takes a margin to 50px half way in the reference
        // browser.
        assert_eq!(end.output(0.5, false), 0.5);
        assert_eq!(end.output(0.0, false), 0.0);
        assert_eq!(end.output(0.24, false), 0.0);
        assert_eq!(end.output(0.25, false), 0.25);
        assert_eq!(end.output(1.0, false), 1.0);
        let start = steps(4, StepPosition::JumpStart);
        assert_eq!(start.output(0.0, false), 0.25);
        assert_eq!(start.output(1.0, false), 1.0);
        // In a backwards-filled delay the before flag holds the bottom
        // step, as the reference browser shows `steps(4, jump-start)`
        // there at 0px.
        assert_eq!(start.output(0.0, true), 0.0);
        let none = steps(3, StepPosition::JumpNone);
        assert_eq!(none.output(0.0, false), 0.0);
        assert_eq!(none.output(0.4, false), 0.5);
        assert_eq!(none.output(1.0, false), 1.0);
        let both = steps(1, StepPosition::JumpBoth);
        assert_eq!(both.output(0.0, false), 0.5);
        assert_eq!(both.output(0.99, false), 0.5);
        assert_eq!(both.output(1.0, false), 1.0);
        // Past [0, 1], where a keyframe's easing can be asked: below
        // zero the steps go on down, above one up.
        assert_eq!(end.output(-0.3, false), -0.5);
        assert_eq!(end.output(1.3, false), 1.25);
    }

    #[test]
    fn linear_points_canonicalize_as_the_tests_read_them_back() {
        let written = |points: &[(f64, Option<f64>)]| canonical_points(points);
        // `linear(-10, -5, 0, 5, 10)`.
        assert_eq!(
            written(&[
                (-10.0, None),
                (-5.0, None),
                (0.0, None),
                (5.0, None),
                (10.0, None)
            ]),
            vec![
                (0.0, -10.0),
                (0.25, -5.0),
                (0.5, 0.0),
                (0.75, 5.0),
                (1.0, 10.0)
            ]
        );
        // `linear(-10 -10%, -5 -5%, 0, 5, 10)`: 30% and 65% between.
        let run = written(&[
            (-10.0, Some(-0.1)),
            (-5.0, Some(-0.05)),
            (0.0, None),
            (5.0, None),
            (10.0, None),
        ]);
        assert!(close(run[2].0, 0.3) && close(run[3].0, 0.65), "{run:?}");
        // `linear(0 20%, 0.5 10%, 1)`: an input below an earlier one.
        assert_eq!(
            written(&[(0.0, Some(0.2)), (0.5, Some(0.1)), (1.0, None)]),
            vec![(0.2, 0.0), (0.2, 0.5), (1.0, 1.0)]
        );
        // `linear(0, 0 40%, 1, 0.5, 1)`.
        let spaced = written(&[
            (0.0, None),
            (0.0, Some(0.4)),
            (1.0, None),
            (0.5, None),
            (1.0, None),
        ]);
        let inputs: Vec<f64> = spaced.iter().map(|p| p.0).collect();
        for (got, want) in inputs.iter().zip([0.0, 0.4, 0.6, 0.8, 1.0]) {
            assert!(close(*got, want), "{inputs:?}");
        }
    }

    #[test]
    fn linear_points_interpolate_and_extrapolate() {
        let points = |list: &[(f64, f64)]| Easing::Points(Arc::from(list));
        // `linear(0, 0.25, 1)`.
        let bend = points(&[(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)]);
        assert_eq!(bend.output(0.25, false), 0.125);
        assert_eq!(bend.output(0.75, false), 0.625);
        // Outside the points, along the first or last two.
        assert_eq!(bend.output(-0.5, false), -0.25);
        assert_eq!(bend.output(2.0, false), 2.5);
        // A jump: two points at one input, the last of them taken there,
        // and the first with the before flag.
        let jump = points(&[(0.0, 0.0), (0.0, 0.5), (1.0, 0.5), (1.0, 1.0)]);
        assert_eq!(jump.output(0.0, false), 0.5);
        assert_eq!(jump.output(0.0, true), 0.0);
        assert_eq!(jump.output(1.0, false), 1.0);
        assert_eq!(jump.output(-1.0, false), 0.0);
        assert_eq!(jump.output(2.0, false), 1.0);
    }

    #[test]
    fn the_output_is_finite_whatever_the_numbers() {
        let wild = [
            Easing::CubicBezier {
                x1: 1e-300,
                y1: 1e300,
                x2: 1.0,
                y2: -1e300,
            },
            Easing::Points(Arc::from(
                &[(-1e300, -1e300), (1e300, 1e300), (f64::MAX, 0.0)][..],
            )),
            Easing::Steps {
                count: u32::MAX,
                position: StepPosition::JumpBoth,
            },
        ];
        for easing in &wild {
            for input in [
                f64::NEG_INFINITY,
                -1e300,
                -1.0,
                0.0,
                0.5,
                1.0,
                1e300,
                f64::INFINITY,
                f64::NAN,
            ] {
                for before in [false, true] {
                    let output = easing.output(input, before);
                    assert!(
                        output.is_finite() && output.abs() <= MAX_PROGRESS,
                        "{easing:?} {input} {output}"
                    );
                }
            }
        }
    }
}
