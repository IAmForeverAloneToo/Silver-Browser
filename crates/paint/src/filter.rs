//! `filter` (Filter Effects 1 section 13, gap batch E, D52,
//! `docs/design/boxes-colour.md`): a box's filter functions as the
//! operations a backend runs on its group's pixels, in order. Each
//! colour function is a matrix on unpremultiplied colour, applied alone
//! and clamped, as each primitive is (section 15); the blur and the drop
//! shadow are Gaussians of standard deviation half their radius.

use style::computed::{Color, Filter};

use crate::Rgba8;

/// One operation of a group's filter, in the group's coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterOp {
    /// A colour matrix on unpremultiplied sRGB colour, four rows of
    /// five, the last column added: `r' = m[0] r + m[1] g + m[2] b +
    /// m[3] a + m[4]`, then `g'`, `b'` and `a'`; each result clamped to
    /// `[0, 1]`.
    Matrix([f32; 20]),
    /// A Gaussian blur of standard deviation `sigma` pixels.
    Blur(f32),
    /// The group's alpha in `color`, moved by `(x, y)` and blurred by
    /// `sigma`, under the group.
    DropShadow {
        x: f32,
        y: f32,
        sigma: f32,
        color: Rgba8,
    },
}

/// A matrix of the three colour rows `rgb`, alpha kept.
fn colour_matrix(rgb: [[f32; 3]; 3], offset: f32) -> FilterOp {
    let mut m = [0.0f32; 20];
    for (row, c) in rgb.iter().enumerate() {
        m[row * 5] = c[0];
        m[row * 5 + 1] = c[1];
        m[row * 5 + 2] = c[2];
        m[row * 5 + 4] = offset;
    }
    m[18] = 1.0;
    FilterOp::Matrix(m)
}

/// The operations of `filters`, in order (section 13's equivalents).
pub fn filter_ops(filters: &[Filter], to_rgba8: impl Fn(Color) -> Rgba8) -> Vec<FilterOp> {
    filters
        .iter()
        .map(|f| match *f {
            Filter::Blur(radius) => FilterOp::Blur(radius / 2.0),
            Filter::Brightness(a) => {
                colour_matrix([[a, 0.0, 0.0], [0.0, a, 0.0], [0.0, 0.0, a]], 0.0)
            }
            Filter::Contrast(a) => {
                colour_matrix([[a, 0.0, 0.0], [0.0, a, 0.0], [0.0, 0.0, a]], 0.5 - 0.5 * a)
            }
            Filter::Grayscale(a) => {
                let k = 1.0 - a;
                colour_matrix(
                    [
                        [
                            0.2126 + 0.7874 * k,
                            0.7152 - 0.7152 * k,
                            0.0722 - 0.0722 * k,
                        ],
                        [
                            0.2126 - 0.2126 * k,
                            0.7152 + 0.2848 * k,
                            0.0722 - 0.0722 * k,
                        ],
                        [
                            0.2126 - 0.2126 * k,
                            0.7152 - 0.7152 * k,
                            0.0722 + 0.9278 * k,
                        ],
                    ],
                    0.0,
                )
            }
            Filter::Sepia(a) => {
                let k = 1.0 - a;
                colour_matrix(
                    [
                        [0.393 + 0.607 * k, 0.769 - 0.769 * k, 0.189 - 0.189 * k],
                        [0.349 - 0.349 * k, 0.686 + 0.314 * k, 0.168 - 0.168 * k],
                        [0.272 - 0.272 * k, 0.534 - 0.534 * k, 0.131 + 0.869 * k],
                    ],
                    0.0,
                )
            }
            Filter::Saturate(s) => colour_matrix(
                [
                    [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
                    [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
                    [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
                ],
                0.0,
            ),
            Filter::HueRotate(degrees) => {
                let (sin, cos) = degrees.to_radians().sin_cos();
                colour_matrix(
                    [
                        [
                            0.213 + cos * 0.787 - sin * 0.213,
                            0.715 - cos * 0.715 - sin * 0.715,
                            0.072 - cos * 0.072 + sin * 0.928,
                        ],
                        [
                            0.213 - cos * 0.213 + sin * 0.143,
                            0.715 + cos * 0.285 + sin * 0.140,
                            0.072 - cos * 0.072 - sin * 0.283,
                        ],
                        [
                            0.213 - cos * 0.213 - sin * 0.787,
                            0.715 - cos * 0.715 + sin * 0.715,
                            0.072 + cos * 0.928 + sin * 0.072,
                        ],
                    ],
                    0.0,
                )
            }
            Filter::Invert(a) => {
                let k = 1.0 - 2.0 * a;
                colour_matrix([[k, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, k]], a)
            }
            Filter::Opacity(a) => {
                let mut m = [0.0f32; 20];
                m[0] = 1.0;
                m[6] = 1.0;
                m[12] = 1.0;
                m[18] = a;
                FilterOp::Matrix(m)
            }
            Filter::DropShadow { color, x, y, blur } => FilterOp::DropShadow {
                x,
                y,
                sigma: blur / 2.0,
                color: to_rgba8(color),
            },
        })
        .collect()
}

/// How far past its content a group's `ops` reach, left, top, right
/// and bottom: three standard deviations of each blur, and each drop
/// shadow's offset and blur besides, what came before it reaching
/// further.
pub fn filter_reach(ops: &[FilterOp]) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for op in ops {
        let e = |sigma: f32| (3.0 * sigma.max(0.0)).ceil();
        match *op {
            FilterOp::Matrix(_) => {}
            FilterOp::Blur(sigma) => {
                for v in &mut out {
                    *v += e(sigma);
                }
            }
            FilterOp::DropShadow { x, y, sigma, .. } => {
                let s = e(sigma);
                out[0] += (s - x).max(0.0);
                out[1] += (s - y).max(0.0);
                out[2] += (s + x).max(0.0);
                out[3] += (s + y).max(0.0);
            }
        }
    }
    out.map(|v| v.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(op: &FilterOp, c: [f32; 4]) -> [f32; 4] {
        let FilterOp::Matrix(m) = op else {
            panic!("a matrix");
        };
        [0, 1, 2, 3].map(|r| {
            (m[r * 5] * c[0]
                + m[r * 5 + 1] * c[1]
                + m[r * 5 + 2] * c[2]
                + m[r * 5 + 3] * c[3]
                + m[r * 5 + 4])
                .clamp(0.0, 1.0)
        })
    }

    #[test]
    fn the_colour_functions_are_their_matrices() {
        let ops = filter_ops(
            &[
                Filter::Brightness(0.5),
                Filter::Contrast(2.0),
                Filter::Grayscale(1.0),
                Filter::Invert(1.0),
                Filter::Opacity(0.5),
                Filter::HueRotate(0.0),
                Filter::Saturate(1.0),
                Filter::Sepia(0.0),
            ],
            |_| Rgba8::rgb(0, 0, 0),
        );
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        assert!(near(
            apply(&ops[0], [1.0, 0.5, 0.0, 1.0]),
            [0.5, 0.25, 0.0, 1.0]
        ));
        assert!(near(
            apply(&ops[1], [0.75, 0.5, 0.25, 1.0]),
            [1.0, 0.5, 0.0, 1.0]
        ));
        let gray = apply(&ops[2], [1.0, 0.0, 0.0, 1.0]);
        assert!(near(gray, [0.2126, 0.2126, 0.2126, 1.0]));
        assert!(near(
            apply(&ops[3], [1.0, 0.25, 0.0, 1.0]),
            [0.0, 0.75, 1.0, 1.0]
        ));
        assert!(near(
            apply(&ops[4], [1.0, 1.0, 1.0, 0.8]),
            [1.0, 1.0, 1.0, 0.4]
        ));
        // A rotation of no degrees, a saturation of 1, a sepia of 0 keep
        // the colour.
        for op in &ops[5..] {
            assert!(near(apply(op, [0.3, 0.6, 0.9, 1.0]), [0.3, 0.6, 0.9, 1.0]));
        }
    }

    #[test]
    fn a_blurs_radius_is_two_standard_deviations() {
        let ops = filter_ops(&[Filter::Blur(8.0)], |_| Rgba8::rgb(0, 0, 0));
        assert_eq!(ops, [FilterOp::Blur(4.0)]);
    }

    #[test]
    fn blurs_and_shadows_reach_past_the_content() {
        let reach = filter_reach(&[
            FilterOp::Blur(2.0),
            FilterOp::DropShadow {
                x: 10.0,
                y: -4.0,
                sigma: 1.0,
                color: Rgba8::rgb(0, 0, 0),
            },
        ]);
        assert_eq!(reach, [6.0, 13.0, 19.0, 6.0]);
    }
}
