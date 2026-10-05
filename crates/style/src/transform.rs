//! The transformation matrix (css-transforms-1 §2, css-transforms-2 §13,
//! M19): a four-by-four matrix of `f32` and the arithmetic the engine
//! needs over it. It lives here because the matrix is the transformation
//! matrix of a computed value, and because layout and paint, above
//! `style`, both read it: layout for the bounds a transformed box adds
//! to the scrollable overflow, paint for the groups of its display list,
//! hit testing for the inverse.
//!
//! The matrix is column-major, as `matrix3d()` is written (css-transforms-2
//! §13): the sixteen numbers of `matrix3d(a1, b1, c1, d1, a2, ..., d4)`
//! are `Matrix([a1, b1, c1, d1, a2, ...])`, so that column `c` holds the
//! image of the `c`th basis vector and `Matrix::at(row, col)` reads
//! `self.0[col * 4 + row]`, both zero-based. The specs count from one:
//! their "row 3, column 3", the component `backface-visibility` reads,
//! is `at(2, 2)` here. A point is a column vector on the right, so
//! `a.then(&b)`, the product `a × b`, applies `b` first and `a` second,
//! which is how the functions of a `transform` list compose: the list
//! `translate(10px) rotate(45deg)` is `translate × rotate`, and rotates
//! a point before it moves it.
//!
//! Every function here keeps the arithmetic finite where its inputs are:
//! a matrix that is not finite, or not invertible, renders its box and
//! its content not at all (css-transforms-1 §2), and [`Matrix::inverse`]
//! says `None` for both. The parsers clamp the numbers they take to the
//! magnitude of [`crate::cascade::MAX_LENGTH_PX`], so that the product
//! of a few functions stays finite; past that the check is here.

use crate::computed::{Effects, TransformFunction};

/// A four-by-four transformation matrix, column-major.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix(pub [f32; 16]);

impl Matrix {
    /// The identity.
    pub const IDENTITY: Matrix = Matrix([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    /// The component at `row`, `col`, both zero-based.
    pub fn at(&self, row: usize, col: usize) -> f32 {
        self.0[col * 4 + row]
    }

    /// `matrix(a, b, c, d, e, f)` (css-transforms-1 §7): the 2D matrix
    /// whose columns are `(a, b)`, `(c, d)` and `(e, f)`, as the 3D one
    /// `matrix3d(a, b, 0, 0, c, d, 0, 0, 0, 0, 1, 0, e, f, 0, 1)`.
    pub fn from_2d([a, b, c, d, e, f]: [f32; 6]) -> Matrix {
        Matrix([
            a, b, 0.0, 0.0, //
            c, d, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            e, f, 0.0, 1.0,
        ])
    }

    /// `translate3d(x, y, z)`.
    pub fn translate(x: f32, y: f32, z: f32) -> Matrix {
        let mut m = Matrix::IDENTITY;
        m.0[12] = x;
        m.0[13] = y;
        m.0[14] = z;
        m
    }

    /// `scale3d(x, y, z)`.
    pub fn scale(x: f32, y: f32, z: f32) -> Matrix {
        let mut m = Matrix::IDENTITY;
        m.0[0] = x;
        m.0[5] = y;
        m.0[10] = z;
        m
    }

    /// `rotate3d(x, y, z, angle)`, the angle in degrees (css-transforms-2
    /// §13): a rotation about the unit vector along `(x, y, z)`; an axis
    /// that cannot be normalized, `(0, 0, 0)`, applies no rotation. The
    /// matrix is the spec's, written with the whole angle's sine and
    /// cosine rather than the half angle's, which are the same numbers
    /// and exact at the right angles: `rotate(90deg)` is
    /// `matrix(0, 1, -1, 0, 0, 0)`, not that within a rounding.
    pub fn rotate(x: f32, y: f32, z: f32, degrees: f32) -> Matrix {
        let length = (x * x + y * y + z * z).sqrt();
        if !length.is_finite() || length <= 0.0 {
            return Matrix::IDENTITY;
        }
        let (x, y, z) = (x / length, y / length, z / length);
        let (s, c) = sin_cos_degrees(degrees);
        let t = 1.0 - c;
        Matrix([
            c + x * x * t,
            y * x * t + z * s,
            z * x * t - y * s,
            0.0,
            x * y * t - z * s,
            c + y * y * t,
            z * y * t + x * s,
            0.0,
            x * z * t + y * s,
            y * z * t - x * s,
            c + z * z * t,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    /// `rotate(angle)`: about the z axis.
    pub fn rotate_z(degrees: f32) -> Matrix {
        Matrix::rotate(0.0, 0.0, 1.0, degrees)
    }

    /// `skew(ax, ay)`, the angles in degrees (css-transforms-1 §7):
    /// `matrix(1, tan(ay), tan(ax), 1, 0, 0)`. An angle at a right angle
    /// has no tangent; its skew is the largest finite one, which draws
    /// the box as a line, as the reference browser does.
    pub fn skew(ax_degrees: f32, ay_degrees: f32) -> Matrix {
        Matrix::from_2d([
            1.0,
            tan_degrees(ay_degrees),
            tan_degrees(ax_degrees),
            1.0,
            0.0,
            0.0,
        ])
    }

    /// `perspective(d)` (css-transforms-2 §13): the identity with
    /// `-1/d` at row 3, column 2, so that a point `z` nearer the viewer
    /// is scaled up about the origin by `1 / (1 - z/d)`. A depth below
    /// one pixel is one pixel (§10, §13); `None`, the function's `none`,
    /// is the identity.
    pub fn perspective(depth: Option<f32>) -> Matrix {
        let mut m = Matrix::IDENTITY;
        if let Some(d) = depth {
            m.0[11] = -1.0 / d.max(1.0);
        }
        m
    }

    /// The perspective of a parent box (css-transforms-2 §10), applied
    /// to its children's matrices: [`Matrix::perspective`] translated to
    /// the `perspective-origin` `(ox, oy)` and back.
    pub fn perspective_at(depth: f32, ox: f32, oy: f32) -> Matrix {
        Matrix::translate(ox, oy, 0.0)
            .then(&Matrix::perspective(Some(depth)))
            .then(&Matrix::translate(-ox, -oy, 0.0))
    }

    /// The product `self × other`: the matrix that applies `other` to a
    /// point first and `self` after.
    pub fn then(&self, other: &Matrix) -> Matrix {
        let mut out = [0.0; 16];
        for col in 0..4 {
            for row in 0..4 {
                out[col * 4 + row] = (0..4)
                    .map(|k| self.0[k * 4 + row] * other.0[col * 4 + k])
                    .sum();
            }
        }
        Matrix(out)
    }

    /// Whether every component is finite.
    pub fn is_finite(&self) -> bool {
        self.0.iter().all(|v| v.is_finite())
    }

    /// Whether this is the identity.
    pub fn is_identity(&self) -> bool {
        *self == Matrix::IDENTITY
    }

    /// Whether the matrix is 2D (css-transforms-2 §13): its action on
    /// the plane `z = 0` is an affine map of the plane, and it leaves
    /// `z` alone, so that `matrix(a, b, c, d, e, f)` could write it.
    pub fn is_2d(&self) -> bool {
        let m = &self.0;
        m[2] == 0.0
            && m[3] == 0.0
            && m[6] == 0.0
            && m[7] == 0.0
            && m[8] == 0.0
            && m[9] == 0.0
            && m[10] == 1.0
            && m[11] == 0.0
            && m[14] == 0.0
            && m[15] == 1.0
    }

    /// The six numbers of `matrix()` when the matrix is 2D.
    pub fn to_2d(&self) -> Option<[f32; 6]> {
        let m = &self.0;
        self.is_2d()
            .then_some([m[0], m[1], m[4], m[5], m[12], m[13]])
    }

    /// Whether the matrix is affine: its last row is `(0, 0, 0, 1)`, so
    /// that no point's `w` changes and straight lines stay parallel.
    pub fn is_affine(&self) -> bool {
        let m = &self.0;
        m[3] == 0.0 && m[7] == 0.0 && m[11] == 0.0 && m[15] == 1.0
    }

    /// Whether the matrix only translates and scales the plane, by
    /// positive factors: `matrix(a, 0, 0, d, e, f)` with `a` and `d`
    /// above zero. Such a group is drawn through its parent's raster
    /// (the compositing note): an axis-aligned rectangle stays one, and
    /// text is rasterized at the size the scale makes.
    pub fn translates_and_scales_only(&self) -> bool {
        match self.to_2d() {
            Some([a, b, c, d, e, f]) => {
                b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0 && e.is_finite() && f.is_finite()
            }
            None => false,
        }
    }

    /// The scale factors and translation of a matrix that
    /// [`Matrix::translates_and_scales_only`]: `(sx, sy, tx, ty)`.
    pub fn scale_and_translation(&self) -> Option<(f32, f32, f32, f32)> {
        let [a, _, _, d, e, f] = self.to_2d()?;
        self.translates_and_scales_only().then_some((a, d, e, f))
    }

    /// The matrix flattened onto the plane (css-transforms-2 §5): what
    /// it does to a point of the plane `z = 0`, its `z` then dropped,
    /// which is all a raster of the plane needs; a flattened matrix
    /// whose `w` row is `(0, 0, 0, k)` is divided through by `k`, so
    /// that a `translateZ()` under a perspective, which only scales the
    /// plane, reads as the scale it is. A box's matrix is flattened
    /// when its parent's used `transform-style` is `flat`, and a 3D
    /// rendering context's accumulated matrices are flattened last.
    pub fn flattened(&self) -> Matrix {
        let mut m = self.0;
        // The z input column and the z output row become the identity's.
        m[8] = 0.0;
        m[9] = 0.0;
        m[11] = 0.0;
        m[2] = 0.0;
        m[6] = 0.0;
        m[14] = 0.0;
        m[10] = 1.0;
        let k = m[15];
        if m[3] == 0.0 && m[7] == 0.0 && k != 1.0 && k.is_finite() && k != 0.0 {
            for (i, v) in m.iter_mut().enumerate() {
                if i != 10 {
                    *v /= k;
                }
            }
        }
        Matrix(m)
    }

    /// The depth of the point `(x, y)` of the plane `z = 0` under the
    /// matrix: its `z` divided by its `w`, which orders the members of
    /// a 3D rendering context (css-transforms-2 §9), further off the
    /// lesser; a point at or behind the eye is furthest of all.
    pub fn depth(&self, x: f32, y: f32) -> f32 {
        let [_, _, z, w] = self.apply(x, y, 0.0);
        if w > 0.0 && (z / w).is_finite() {
            z / w
        } else {
            f32::NEG_INFINITY
        }
    }

    /// The determinant.
    pub fn determinant(&self) -> f32 {
        let m = &self.0;
        let s0 = m[0] * m[5] - m[4] * m[1];
        let s1 = m[0] * m[6] - m[4] * m[2];
        let s2 = m[0] * m[7] - m[4] * m[3];
        let s3 = m[1] * m[6] - m[5] * m[2];
        let s4 = m[1] * m[7] - m[5] * m[3];
        let s5 = m[2] * m[7] - m[6] * m[3];
        let c5 = m[10] * m[15] - m[14] * m[11];
        let c4 = m[9] * m[15] - m[13] * m[11];
        let c3 = m[9] * m[14] - m[13] * m[10];
        let c2 = m[8] * m[15] - m[12] * m[11];
        let c1 = m[8] * m[14] - m[12] * m[10];
        let c0 = m[8] * m[13] - m[12] * m[9];
        s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0
    }

    /// The inverse, or `None` when the matrix is singular or not finite:
    /// a box with such a matrix is not displayed and not hit
    /// (css-transforms-1 §2).
    pub fn inverse(&self) -> Option<Matrix> {
        if !self.is_finite() {
            return None;
        }
        let m = &self.0;
        let s0 = m[0] * m[5] - m[4] * m[1];
        let s1 = m[0] * m[6] - m[4] * m[2];
        let s2 = m[0] * m[7] - m[4] * m[3];
        let s3 = m[1] * m[6] - m[5] * m[2];
        let s4 = m[1] * m[7] - m[5] * m[3];
        let s5 = m[2] * m[7] - m[6] * m[3];
        let c5 = m[10] * m[15] - m[14] * m[11];
        let c4 = m[9] * m[15] - m[13] * m[11];
        let c3 = m[9] * m[14] - m[13] * m[10];
        let c2 = m[8] * m[15] - m[12] * m[11];
        let c1 = m[8] * m[14] - m[12] * m[10];
        let c0 = m[8] * m[13] - m[12] * m[9];
        let det = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inv_det = 1.0 / det;
        let out = Matrix([
            (m[5] * c5 - m[6] * c4 + m[7] * c3) * inv_det,
            (-m[1] * c5 + m[2] * c4 - m[3] * c3) * inv_det,
            (m[13] * s5 - m[14] * s4 + m[15] * s3) * inv_det,
            (-m[9] * s5 + m[10] * s4 - m[11] * s3) * inv_det,
            (-m[4] * c5 + m[6] * c2 - m[7] * c1) * inv_det,
            (m[0] * c5 - m[2] * c2 + m[3] * c1) * inv_det,
            (-m[12] * s5 + m[14] * s2 - m[15] * s1) * inv_det,
            (m[8] * s5 - m[10] * s2 + m[11] * s1) * inv_det,
            (m[4] * c4 - m[5] * c2 + m[7] * c0) * inv_det,
            (-m[0] * c4 + m[1] * c2 - m[3] * c0) * inv_det,
            (m[12] * s4 - m[13] * s2 + m[15] * s0) * inv_det,
            (-m[8] * s4 + m[9] * s2 - m[11] * s0) * inv_det,
            (-m[4] * c3 + m[5] * c1 - m[6] * c0) * inv_det,
            (m[0] * c3 - m[1] * c1 + m[2] * c0) * inv_det,
            (-m[12] * s3 + m[13] * s1 - m[14] * s0) * inv_det,
            (m[8] * s3 - m[9] * s1 + m[10] * s0) * inv_det,
        ]);
        out.is_finite().then_some(out)
    }

    /// The image of the point `(x, y, z, 1)`, as `(x, y, z, w)`, not yet
    /// divided by `w`.
    pub fn apply(&self, x: f32, y: f32, z: f32) -> [f32; 4] {
        let m = &self.0;
        [
            m[0] * x + m[4] * y + m[8] * z + m[12],
            m[1] * x + m[5] * y + m[9] * z + m[13],
            m[2] * x + m[6] * y + m[10] * z + m[14],
            m[3] * x + m[7] * y + m[11] * z + m[15],
        ]
    }

    /// Where the point `(x, y)` of the plane `z = 0` lands on the plane:
    /// its image divided by `w`. `None` when `w` is not positive, the
    /// point being at or behind the eye of a perspective, or when the
    /// result is not finite.
    pub fn project(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let [px, py, _, w] = self.apply(x, y, 0.0);
        if !w.is_finite() || w <= 0.0 {
            return None;
        }
        let (px, py) = (px / w, py / w);
        (px.is_finite() && py.is_finite()).then_some((px, py))
    }

    /// The axis-aligned bounds of the image of the rectangle at `(x, y)`
    /// of size `w` by `h`, as `(left, top, right, bottom)`: the bounds
    /// of its four projected corners. `None` when a corner does not
    /// project.
    pub fn bounds(&self, x: f32, y: f32, w: f32, h: f32) -> Option<(f32, f32, f32, f32)> {
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for (cx, cy) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
            let (px, py) = self.project(cx, cy)?;
            bounds = Some(match bounds {
                None => (px, py, px, py),
                Some((l, t, r, b)) => (l.min(px), t.min(py), r.max(px), b.max(py)),
            });
        }
        bounds
    }
}

/// The sine and cosine of an angle in degrees, exact at the multiples of
/// ninety where `f32`'s `sin` and `cos` are not, as the reference
/// browser has them: `rotate(90deg)` is `matrix(0, 1, -1, 0, 0, 0)` and
/// nothing else.
fn sin_cos_degrees(degrees: f32) -> (f32, f32) {
    let degrees = degrees.rem_euclid(360.0);
    if degrees == 0.0 {
        return (0.0, 1.0);
    }
    if degrees == 90.0 {
        return (1.0, 0.0);
    }
    if degrees == 180.0 {
        return (0.0, -1.0);
    }
    if degrees == 270.0 {
        return (-1.0, 0.0);
    }
    let radians = (degrees as f64).to_radians();
    (radians.sin() as f32, radians.cos() as f32)
}

/// The tangent of an angle in degrees, exact at zero and at the half
/// turns, and finite at the right angles, where it is the largest finite
/// value with the sign the angle approaches it from.
fn tan_degrees(degrees: f32) -> f32 {
    let degrees = degrees.rem_euclid(360.0);
    if degrees == 0.0 || degrees == 180.0 {
        return 0.0;
    }
    if degrees == 90.0 || degrees == 270.0 {
        return f32::MAX;
    }
    (degrees as f64).to_radians().tan() as f32
}

impl TransformFunction {
    /// The matrix of this function (css-transforms-2 §13), its
    /// percentages, which only a translation has, resolved against the
    /// `width` and `height` of the reference box.
    pub fn matrix(&self, width: f32, height: f32) -> Matrix {
        match *self {
            TransformFunction::Matrix(m) => Matrix::from_2d(m),
            TransformFunction::Matrix3d(m) => Matrix(m),
            TransformFunction::Translate(x, y, z) => {
                Matrix::translate(x.resolve(width), y.resolve(height), z)
            }
            TransformFunction::Scale(x, y, z) => Matrix::scale(x, y, z),
            TransformFunction::Rotate { x, y, z, angle } => Matrix::rotate(x, y, z, angle),
            TransformFunction::Skew(ax, ay) => Matrix::skew(ax, ay),
            TransformFunction::Perspective(depth) => Matrix::perspective(depth),
        }
    }

    /// Whether the function is 3D: a `translateZ`, a rotation about
    /// another axis than z, a scale along z, a `matrix3d()` or a
    /// `perspective()`, which make the box's matrix one that
    /// [`Matrix::is_2d`] refuses. A 3D function on a box whose parent's
    /// used `transform-style` is `flat` is flattened in the end; this
    /// says what was written.
    pub fn is_3d(&self) -> bool {
        match *self {
            TransformFunction::Matrix(_) | TransformFunction::Skew(..) => false,
            TransformFunction::Matrix3d(_) | TransformFunction::Perspective(_) => true,
            TransformFunction::Translate(_, _, z) => z != 0.0,
            TransformFunction::Scale(_, _, z) => z != 1.0,
            TransformFunction::Rotate { x, y, .. } => x != 0.0 || y != 0.0,
        }
    }
}

/// The reference box of a transform (css-transforms-1 §9): the box its
/// origin and its percentages are of, placed relative to the border
/// box's top left corner, which is where the matrix applies. The border
/// box itself is at `(0, 0)`; the content box is inset by the border
/// and padding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceBox {
    /// The box's left edge, from the border box's.
    pub x: f32,
    /// The box's top edge, from the border box's.
    pub y: f32,
    /// The box's width.
    pub width: f32,
    /// The box's height.
    pub height: f32,
}

impl ReferenceBox {
    /// The border box of a box `width` by `height`.
    pub fn border(width: f32, height: f32) -> ReferenceBox {
        ReferenceBox {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }
}

impl Effects {
    /// Whether the box has a transform: a `transform` other than `none`,
    /// or a `translate`, `rotate` or `scale` other than `none`. Any of
    /// them, an identity included, makes the box a stacking context and
    /// a containing block for all its descendants (css-transforms-1 §2,
    /// css-transforms-2 §2).
    pub fn has_transform(&self) -> bool {
        !self.transform.is_empty()
            || self.translate.is_some()
            || self.rotate.is_some()
            || self.scale.is_some()
    }

    /// The transformation matrix of a box, in its border box's
    /// coordinates, from its `reference` box (css-transforms-1 §2,
    /// css-transforms-2 §2.4): the identity translated by the
    /// `transform-origin`, multiplied by the `translate`, the `rotate`,
    /// the `scale` and then each function of `transform` in order, and
    /// translated back by the negated origin. `None` when the box has no
    /// transform at all, which is not the same as the identity: a box
    /// with `translate: 0px` is still a stacking context and a
    /// containing block.
    pub fn matrix(&self, reference: &ReferenceBox) -> Option<Matrix> {
        if !self.has_transform() {
            return None;
        }
        let (width, height) = (reference.width, reference.height);
        let origin = &self.transform_origin;
        let (ox, oy, oz) = (
            reference.x + origin.x.resolve(width),
            reference.y + origin.y.resolve(height),
            origin.z,
        );
        let mut m = Matrix::translate(ox, oy, oz);
        if let Some(t) = &self.translate {
            m = m.then(&Matrix::translate(
                t.x.resolve(width),
                t.y.resolve(height),
                t.z,
            ));
        }
        if let Some(r) = &self.rotate {
            m = m.then(&Matrix::rotate(r.x, r.y, r.z, r.angle));
        }
        if let Some([sx, sy, sz]) = self.scale {
            m = m.then(&Matrix::scale(sx, sy, sz));
        }
        for function in &self.transform {
            m = m.then(&function.matrix(width, height));
        }
        Some(m.then(&Matrix::translate(-ox, -oy, -oz)))
    }

    /// Whether the transform, as written, has a 3D part.
    pub fn has_3d_transform(&self) -> bool {
        self.transform.iter().any(TransformFunction::is_3d)
            || self.translate.as_ref().is_some_and(|t| t.z != 0.0)
            || self
                .rotate
                .as_ref()
                .is_some_and(|r| r.x != 0.0 || r.y != 0.0)
            || self.scale.is_some_and(|[_, _, z]| z != 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::{LengthPercentage, Offset, Rotate, TransformOrigin, Translate};

    /// The origin at the top left corner.
    const TOP_LEFT: TransformOrigin = TransformOrigin {
        x: Offset {
            percent: 0.0,
            px: 0.0,
        },
        y: Offset {
            percent: 0.0,
            px: 0.0,
        },
        z: 0.0,
    };

    fn close(a: &Matrix, b: &Matrix) -> bool {
        a.0.iter()
            .zip(b.0.iter())
            .all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn the_functions_give_the_matrices_of_the_spec() {
        // css-transforms-2 §13: each function's matrix.
        assert_eq!(
            Matrix::translate(3.0, 4.0, 5.0).apply(1.0, 1.0, 1.0),
            [4.0, 5.0, 6.0, 1.0]
        );
        assert_eq!(
            Matrix::scale(2.0, 3.0, 4.0).apply(1.0, 1.0, 1.0),
            [2.0, 3.0, 4.0, 1.0]
        );
        // rotate(90deg) is matrix(0, 1, -1, 0, 0, 0), exactly.
        assert_eq!(
            Matrix::rotate_z(90.0).to_2d(),
            Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0])
        );
        assert_eq!(Matrix::rotate_z(90.0).project(1.0, 0.0), Some((0.0, 1.0)));
        assert_eq!(
            Matrix::rotate_z(-90.0).to_2d(),
            Matrix::rotate_z(270.0).to_2d()
        );
        assert_eq!(Matrix::rotate_z(360.0), Matrix::IDENTITY);
        assert!(close(
            &Matrix::rotate_z(45.0),
            &Matrix::from_2d([0.70710677, 0.70710677, -0.70710677, 0.70710677, 0.0, 0.0])
        ));
        // rotateX(90deg) turns y into z.
        assert_eq!(
            Matrix::rotate(1.0, 0.0, 0.0, 90.0).apply(0.0, 1.0, 0.0),
            [0.0, 0.0, 1.0, 1.0]
        );
        // rotateY(90deg) turns z into x.
        assert_eq!(
            Matrix::rotate(0.0, 1.0, 0.0, 90.0).apply(0.0, 0.0, 1.0),
            [1.0, 0.0, 0.0, 1.0]
        );
        // The axis is normalized, and a zero axis rotates nothing.
        assert_eq!(Matrix::rotate(0.0, 0.0, 7.0, 90.0), Matrix::rotate_z(90.0));
        assert_eq!(Matrix::rotate(0.0, 0.0, 0.0, 90.0), Matrix::IDENTITY);
        // skew(ax, ay) is matrix(1, tan(ay), tan(ax), 1, 0, 0).
        assert!(close(
            &Matrix::skew(45.0, 0.0),
            &Matrix::from_2d([1.0, 0.0, 1.0, 1.0, 0.0, 0.0])
        ));
        assert!(close(
            &Matrix::skew(0.0, 45.0),
            &Matrix::from_2d([1.0, 1.0, 0.0, 1.0, 0.0, 0.0])
        ));
        assert!(Matrix::skew(90.0, 0.0).is_finite());
        // perspective(d) puts -1/d at row 3, column 2.
        let p = Matrix::perspective(Some(500.0));
        assert_eq!(p.at(3, 2), -1.0 / 500.0);
        assert_eq!(p.apply(100.0, 0.0, 250.0), [100.0, 0.0, 250.0, 0.5]);
        assert_eq!(
            Matrix::perspective(Some(0.5)),
            Matrix::perspective(Some(1.0))
        );
        assert_eq!(Matrix::perspective(None), Matrix::IDENTITY);
        assert_eq!(
            Matrix::from_2d([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).to_2d(),
            Some([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
        );
    }

    #[test]
    fn products_apply_the_right_operand_first() {
        // `translate(10px) rotate(90deg)`: a point is rotated, then moved.
        let m = Matrix::translate(10.0, 0.0, 0.0).then(&Matrix::rotate_z(90.0));
        assert_eq!(m.project(1.0, 0.0), Some((10.0, 1.0)));
        let m = Matrix::rotate_z(90.0).then(&Matrix::translate(10.0, 0.0, 0.0));
        assert_eq!(m.project(1.0, 0.0), Some((0.0, 11.0)));
        assert_eq!(Matrix::IDENTITY.then(&m), m);
        assert_eq!(m.then(&Matrix::IDENTITY), m);
    }

    #[test]
    fn inverses_undo_and_singular_matrices_have_none() {
        let m = Matrix::translate(10.0, 20.0, 0.0)
            .then(&Matrix::rotate_z(30.0))
            .then(&Matrix::scale(2.0, 3.0, 1.0))
            .then(&Matrix::skew(10.0, 0.0));
        let inverse = m.inverse().expect("invertible");
        assert!(close(&m.then(&inverse), &Matrix::IDENTITY));
        assert!(close(&inverse.then(&m), &Matrix::IDENTITY));
        let p =
            Matrix::perspective_at(400.0, 50.0, 50.0).then(&Matrix::rotate(0.0, 1.0, 0.0, 40.0));
        let inverse = p.inverse().expect("invertible");
        assert!(close(&p.then(&inverse), &Matrix::IDENTITY));
        assert!(close(&inverse.then(&p), &Matrix::IDENTITY));
        assert_eq!(Matrix::scale(0.0, 1.0, 1.0).inverse(), None);
        assert_eq!(Matrix::from_2d([0.0; 6]).inverse(), None);
        assert_eq!(Matrix::scale(f32::INFINITY, 1.0, 1.0).inverse(), None);
        assert_eq!(Matrix::scale(f32::NAN, 1.0, 1.0).inverse(), None);
        assert_eq!(Matrix::IDENTITY.determinant(), 1.0);
        assert_eq!(Matrix::scale(2.0, 3.0, 4.0).determinant(), 24.0);
    }

    #[test]
    fn projection_divides_by_w_and_refuses_points_behind_the_eye() {
        // A point 250px nearer a viewer 500px away looks twice as big.
        let m = Matrix::perspective(Some(500.0)).then(&Matrix::translate(0.0, 0.0, 250.0));
        assert_eq!(m.project(100.0, 50.0), Some((200.0, 100.0)));
        // At the eye, or behind it, there is no image.
        let m = Matrix::perspective(Some(500.0)).then(&Matrix::translate(0.0, 0.0, 500.0));
        assert_eq!(m.project(100.0, 50.0), None);
        let m = Matrix::perspective(Some(500.0)).then(&Matrix::translate(0.0, 0.0, 900.0));
        assert_eq!(m.project(100.0, 50.0), None);
        assert_eq!(m.bounds(0.0, 0.0, 10.0, 10.0), None);
        // The bounds of a rotated square.
        let (l, t, r, b) = Matrix::rotate_z(45.0)
            .bounds(0.0, 0.0, 10.0, 10.0)
            .expect("projects");
        let half = 10.0 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((l + half).abs() < 1e-4 && (r - half).abs() < 1e-4);
        assert!(t.abs() < 1e-4 && (b - 2.0 * half).abs() < 1e-4);
    }

    #[test]
    fn the_shape_of_a_matrix_is_told_apart() {
        assert!(Matrix::IDENTITY.is_identity() && Matrix::IDENTITY.is_2d());
        let t = Matrix::translate(5.0, 6.0, 0.0).then(&Matrix::scale(2.0, 0.5, 1.0));
        assert!(t.is_2d() && t.is_affine() && t.translates_and_scales_only());
        assert_eq!(t.scale_and_translation(), Some((2.0, 0.5, 5.0, 6.0)));
        assert!(!Matrix::rotate_z(10.0).translates_and_scales_only());
        assert!(!Matrix::skew(10.0, 0.0).translates_and_scales_only());
        assert!(!Matrix::scale(-1.0, 1.0, 1.0).translates_and_scales_only());
        assert!(!Matrix::scale(0.0, 1.0, 1.0).translates_and_scales_only());
        assert!(!Matrix::translate(0.0, 0.0, 1.0).is_2d());
        assert!(Matrix::translate(0.0, 0.0, 1.0).is_affine());
        assert!(!Matrix::rotate(1.0, 0.0, 0.0, 30.0).is_2d());
        assert!(!Matrix::perspective(Some(100.0)).is_affine());
        assert!(!Matrix::perspective(Some(100.0)).is_2d());
        assert!(Matrix::rotate_z(10.0).to_2d().is_some());
        assert_eq!(Matrix::perspective(Some(100.0)).to_2d(), None);
    }

    #[test]
    fn flattening_keeps_what_the_plane_sees() {
        let m = Matrix::perspective(Some(500.0)).then(&Matrix::rotate(0.0, 1.0, 0.0, 30.0));
        let flat = m.flattened();
        for (x, y) in [(0.0, 0.0), (10.0, 20.0), (-5.0, 7.0)] {
            let a = m.project(x, y).unwrap();
            let b = flat.project(x, y).unwrap();
            assert!((a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4);
        }
        // The flattened matrix leaves z alone.
        assert_eq!(flat.apply(0.0, 0.0, 1.0)[2], 1.0);
        assert_eq!(Matrix::IDENTITY.flattened(), Matrix::IDENTITY);
        assert!(Matrix::translate(0.0, 0.0, 10.0).flattened().is_identity());
    }

    #[test]
    fn a_box_resolves_its_matrix_about_its_origin() {
        // Transforms 1 §2's four steps, about the default origin, the
        // centre of a 100 by 50 box: a rotation by 180deg maps the top
        // left corner to the bottom right.
        let border = ReferenceBox::border(100.0, 50.0);
        assert_eq!(
            Effects::default().matrix(&border),
            None,
            "no transform at all"
        );
        let mut effects = Effects {
            transform: vec![TransformFunction::Rotate {
                x: 0.0,
                y: 0.0,
                z: 1.0,
                angle: 180.0,
            }],
            ..Default::default()
        };
        let m = effects.matrix(&border).unwrap();
        assert_eq!(m.project(0.0, 0.0), Some((100.0, 50.0)));
        // About the content box, inset by 10px on each side, the centre
        // is the same and the percentages are of the smaller box.
        let content = ReferenceBox {
            x: 10.0,
            y: 10.0,
            width: 80.0,
            height: 30.0,
        };
        assert_eq!(effects.matrix(&content), Some(m));
        effects.transform = vec![TransformFunction::Translate(
            LengthPercentage::Percent(100.0),
            LengthPercentage::Percent(100.0),
            0.0,
        )];
        assert_eq!(
            effects.matrix(&content).unwrap().project(0.0, 0.0),
            Some((80.0, 30.0))
        );
        effects.transform = vec![TransformFunction::Rotate {
            x: 0.0,
            y: 0.0,
            z: 1.0,
            angle: 180.0,
        }];
        // An origin at the top left leaves the corner where it is.
        effects.transform_origin = TOP_LEFT;
        let m = effects.matrix(&border).unwrap();
        assert_eq!(m.project(0.0, 0.0), Some((0.0, 0.0)));
        assert_eq!(m.project(100.0, 50.0), Some((-100.0, -50.0)));
        // The top left of the content box is where it is.
        let m = effects.matrix(&content).unwrap();
        assert_eq!(m.project(10.0, 10.0), Some((10.0, 10.0)));
        assert_eq!(m.project(0.0, 0.0), Some((20.0, 20.0)));
        // A percentage translation is of the box's own size, and the
        // individual properties go before the list, in their order.
        let mut effects = Effects {
            translate: Some(Translate {
                x: LengthPercentage::Percent(50.0),
                y: LengthPercentage::Px(0.0),
                z: 0.0,
            }),
            rotate: Some(Rotate {
                x: 0.0,
                y: 0.0,
                z: 1.0,
                angle: 90.0,
            }),
            scale: Some([2.0, 2.0, 1.0]),
            transform: vec![TransformFunction::Translate(
                LengthPercentage::Px(1.0),
                LengthPercentage::Px(0.0),
                0.0,
            )],
            transform_origin: TOP_LEFT,
            ..Default::default()
        };
        // (1, 0) translated by the list to (2, 0), scaled to (4, 0),
        // rotated to (0, 4), moved by half the width: (50, 4).
        let m = effects.matrix(&border).unwrap();
        assert_eq!(m.project(1.0, 0.0), Some((50.0, 4.0)));
        assert!(!effects.has_3d_transform());
        effects.translate.as_mut().unwrap().z = 1.0;
        assert!(effects.has_3d_transform());
        assert!(TransformFunction::Perspective(None).is_3d());
        assert!(!TransformFunction::Scale(1.0, 2.0, 1.0).is_3d());
        assert!(TransformFunction::Scale(1.0, 1.0, 2.0).is_3d());
    }

    #[test]
    fn a_huge_scale_stays_finite_and_an_infinite_matrix_is_singular() {
        // The parsers clamp numbers to the largest length; the product of
        // a few of them is still finite, and a matrix that is not is
        // not drawn.
        let big = crate::cascade::MAX_LENGTH_PX;
        let m = Matrix::scale(big, big, 1.0).then(&Matrix::translate(big, big, 0.0));
        assert!(m.is_finite());
        assert!(m.inverse().is_some());
        let inf = Matrix([f32::INFINITY; 16]);
        assert!(!inf.is_finite());
        assert_eq!(inf.inverse(), None);
        assert_eq!(inf.project(0.0, 0.0), None);
        assert_eq!(inf.bounds(0.0, 0.0, 1.0, 1.0), None);
    }

    #[test]
    fn a_perspective_parent_and_a_turned_child_project_as_the_reference_browser_does() {
        // A 200px box with `perspective: 300px` (origin at its centre,
        // (100, 100)) holding a 100px box at (20, 20) turned by
        // `rotateY(60deg)` about its centre: the reference browser's
        // bounding box is 59.91 by 116.87 at (35.72, 6.51). With the
        // origin at (0, 0) and the child at `translateZ(100px)`, the
        // child scales by 300 / (300 - 100) about the origin: (30, 30),
        // 150 square, which flattening reads as that scale.
        let own = Matrix::translate(70.0, 70.0, 0.0)
            .then(&Matrix::rotate(0.0, 1.0, 0.0, 60.0))
            .then(&Matrix::translate(-70.0, -70.0, 0.0));
        let full = Matrix::perspective_at(300.0, 100.0, 100.0).then(&own);
        let (l, t, r, b) = full.bounds(20.0, 20.0, 100.0, 100.0).unwrap();
        for (got, want) in [(l, 35.72), (t, 6.51), (r - l, 59.91), (b - t, 116.87)] {
            assert!((got - want).abs() < 0.05, "{got} for {want}");
        }
        // The flattened matrix does the same to the plane.
        let flat = full.flattened();
        assert_eq!(
            flat.bounds(20.0, 20.0, 100.0, 100.0),
            full.bounds(20.0, 20.0, 100.0, 100.0)
        );
        assert!(flat.inverse().is_some());
        let (x, y) = flat.project(20.0, 20.0).unwrap();
        let (bx, by) = flat.inverse().unwrap().project(x, y).unwrap();
        assert!((bx - 20.0).abs() < 1e-3 && (by - 20.0).abs() < 1e-3);
        let own = Matrix::translate(70.0, 70.0, 0.0)
            .then(&Matrix::translate(0.0, 0.0, 100.0))
            .then(&Matrix::translate(-70.0, -70.0, 0.0));
        let full = Matrix::perspective_at(300.0, 0.0, 0.0).then(&own);
        let flat = full.flattened();
        let [a, b2, c, d, e, f] = flat.to_2d().expect("a scale");
        for (got, want) in [(a, 1.5), (b2, 0.0), (c, 0.0), (d, 1.5), (e, 0.0), (f, 0.0)] {
            assert!((got - want).abs() < 1e-5, "{got} for {want}");
        }
        assert!(flat.translates_and_scales_only());
        // Depth: nearer the viewer is greater, and a point behind the
        // eye is furthest of all.
        assert_eq!(Matrix::translate(0.0, 0.0, 10.0).depth(5.0, 5.0), 10.0);
        assert!(full.depth(70.0, 70.0) > 0.0);
        let behind = Matrix::perspective(Some(100.0)).then(&Matrix::translate(0.0, 0.0, 200.0));
        assert_eq!(behind.depth(0.0, 0.0), f32::NEG_INFINITY);
    }
}
