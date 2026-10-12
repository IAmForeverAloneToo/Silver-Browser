//! Paths as the 2D context builds them (HTML 4.12.5.1.5): subpaths of
//! lines and Bezier curves, an arc or an ellipse as cubic curves, each
//! point transformed as it is added. The context's default path is kept
//! in the bitmap's coordinates; a `Path2D` in its own, transformed when
//! it is drawn.

use std::f64::consts::{FRAC_PI_2, TAU};

use tiny_skia::{FillRule, PathBuilder, PathSegment};

use crate::state::{Matrix, number};
use crate::{CanvasError, MAX_PATH_POINTS};

type Point = (f64, f64);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Segment {
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
}

impl Segment {
    fn end(&self) -> Point {
        match *self {
            Segment::Line(p) | Segment::Quad(_, p) | Segment::Cubic(_, _, p) => p,
        }
    }

    fn points(&self) -> usize {
        match self {
            Segment::Line(_) => 1,
            Segment::Quad(..) => 2,
            Segment::Cubic(..) => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Subpath {
    start: Point,
    segments: Vec<Segment>,
    closed: bool,
}

impl Subpath {
    fn last(&self) -> Point {
        self.segments.last().map_or(self.start, Segment::end)
    }
}

/// A path: its subpaths, and how many points they hold, at most
/// [`MAX_PATH_POINTS`] or the lower limit its owner set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    subpaths: Vec<Subpath>,
    points: usize,
    limit: Option<usize>,
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

fn negative(what: &str, value: f64) -> CanvasError {
    CanvasError::new(
        "IndexSizeError",
        format!("The {what} provided ({}) is negative.", number(value)),
    )
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    /// `new Path2D(d)`: SVG path data, up to its first error.
    pub fn from_svg(d: &str) -> Path {
        let mut path = Path::new();
        crate::svg::parse(d, &mut path);
        path
    }

    pub fn is_empty(&self) -> bool {
        self.subpaths.is_empty()
    }

    /// The bounds of the path's geometry, its curves' extremes and not
    /// their control points: `[x, y, width, height]`; `None` for a path
    /// with nothing to bound (M32, an SVG shape's fill box).
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let r = self.to_skia()?.compute_tight_bounds()?;
        Some([
            f64::from(r.x()),
            f64::from(r.y()),
            f64::from(r.width()),
            f64::from(r.height()),
        ])
    }

    /// The points the path holds.
    pub fn len(&self) -> usize {
        self.points
    }

    fn room(&self, points: usize) -> bool {
        self.points + points <= self.limit.unwrap_or(MAX_PATH_POINTS).min(MAX_PATH_POINTS)
    }

    /// The most points the path may hold below [`MAX_PATH_POINTS`], as
    /// a document's budget for all its paths leaves it; `None` for that
    /// bound alone.
    pub fn set_limit(&mut self, limit: Option<usize>) {
        self.limit = limit;
    }

    fn push(&mut self, segment: Segment) {
        if !self.room(segment.points()) {
            return;
        }
        if let Some(subpath) = self.subpaths.last_mut() {
            self.points += segment.points();
            subpath.segments.push(segment);
        }
    }

    /// The last point, as stored.
    pub(crate) fn last_point(&self) -> Option<Point> {
        self.subpaths.last().map(Subpath::last)
    }

    /// `moveTo()`.
    pub fn move_to(&mut self, x: f64, y: f64, m: &Matrix) {
        if !finite(&[x, y]) || !self.room(1) {
            return;
        }
        self.points += 1;
        self.subpaths.push(Subpath {
            start: m.apply(x, y),
            segments: Vec::new(),
            closed: false,
        });
    }

    /// A subpath made with the point when there is none.
    fn ensure(&mut self, x: f64, y: f64, m: &Matrix) {
        if self.subpaths.is_empty() {
            self.move_to(x, y, m);
        }
    }

    /// `lineTo()`.
    pub fn line_to(&mut self, x: f64, y: f64, m: &Matrix) {
        if !finite(&[x, y]) {
            return;
        }
        if self.subpaths.is_empty() {
            self.move_to(x, y, m);
        } else {
            self.push(Segment::Line(m.apply(x, y)));
        }
    }

    /// `quadraticCurveTo()`.
    pub fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64, m: &Matrix) {
        if !finite(&[cx, cy, x, y]) {
            return;
        }
        self.ensure(cx, cy, m);
        self.push(Segment::Quad(m.apply(cx, cy), m.apply(x, y)));
    }

    /// `bezierCurveTo()`.
    #[allow(clippy::too_many_arguments)]
    pub fn cubic_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64, m: &Matrix) {
        if !finite(&[c1x, c1y, c2x, c2y, x, y]) {
            return;
        }
        self.ensure(c1x, c1y, m);
        self.push(Segment::Cubic(
            m.apply(c1x, c1y),
            m.apply(c2x, c2y),
            m.apply(x, y),
        ));
    }

    /// `closePath()`: the last subpath closed, and a new one begun at
    /// its first point.
    pub fn close(&mut self) {
        let Some(last) = self.subpaths.last_mut() else {
            return;
        };
        last.closed = true;
        let start = last.start;
        if self.room(1) {
            self.points += 1;
            self.subpaths.push(Subpath {
                start,
                segments: Vec::new(),
                closed: false,
            });
        }
    }

    /// `rect()`: a closed subpath of the four corners, then a new one at
    /// the first.
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, m: &Matrix) {
        if !finite(&[x, y, w, h]) {
            return;
        }
        self.move_to(x, y, m);
        self.line_to(x + w, y, m);
        self.line_to(x + w, y + h, m);
        self.line_to(x, y + h, m);
        self.close();
    }

    /// `arc()`: an ellipse of equal radii.
    #[allow(clippy::too_many_arguments)]
    pub fn arc(
        &mut self,
        x: f64,
        y: f64,
        radius: f64,
        start: f64,
        end: f64,
        anticlockwise: bool,
        m: &Matrix,
    ) -> Result<(), CanvasError> {
        if !finite(&[x, y, radius, start, end]) {
            return Ok(());
        }
        if radius < 0.0 {
            return Err(negative("radius", radius));
        }
        self.add_ellipse(x, y, radius, radius, 0.0, start, end, anticlockwise, m);
        Ok(())
    }

    /// `ellipse()`.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(
        &mut self,
        x: f64,
        y: f64,
        rx: f64,
        ry: f64,
        rotation: f64,
        start: f64,
        end: f64,
        anticlockwise: bool,
        m: &Matrix,
    ) -> Result<(), CanvasError> {
        if !finite(&[x, y, rx, ry, rotation, start, end]) {
            return Ok(());
        }
        if rx < 0.0 {
            return Err(negative("major-axis radius", rx));
        }
        if ry < 0.0 {
            return Err(negative("minor-axis radius", ry));
        }
        self.add_ellipse(x, y, rx, ry, rotation, start, end, anticlockwise, m);
        Ok(())
    }

    /// The arc of an ellipse from `start` to `end`, joined by a line to
    /// the path's last point when there is one: the whole circumference
    /// when the angles are a turn or more apart in its direction, else
    /// the way round in its direction, never more than a turn.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_ellipse(
        &mut self,
        x: f64,
        y: f64,
        rx: f64,
        ry: f64,
        rotation: f64,
        start: f64,
        end: f64,
        anticlockwise: bool,
        m: &Matrix,
    ) {
        // A turn the other way round by whole circles is a whole circle,
        // as the reference browser has it: `arc(x, y, r, 0, 2 * PI,
        // true)` draws one.
        let around = |turn: f64| {
            if turn >= TAU {
                return TAU;
            }
            let part = turn.rem_euclid(TAU);
            if part == 0.0 && turn < 0.0 { TAU } else { part }
        };
        let sweep = if anticlockwise {
            -around(start - end)
        } else {
            around(end - start)
        };
        let (sin_r, cos_r) = rotation.sin_cos();
        let on = |ux: f64, uy: f64| -> Point {
            (
                x + rx * ux * cos_r - ry * uy * sin_r,
                y + rx * ux * sin_r + ry * uy * cos_r,
            )
        };
        let (s0, c0) = start.sin_cos();
        let first = on(c0, s0);
        if self.subpaths.is_empty() {
            self.move_to(first.0, first.1, m);
        } else {
            self.line_to(first.0, first.1, m);
        }
        if sweep == 0.0 {
            return;
        }
        let pieces = (sweep.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
        let step = sweep / pieces as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let mut angle = start;
        for _ in 0..pieces {
            let next = angle + step;
            let (sa, ca) = angle.sin_cos();
            let (sb, cb) = next.sin_cos();
            let c1 = on(ca - k * sa, sa + k * ca);
            let c2 = on(cb + k * sb, sb - k * cb);
            let p = on(cb, sb);
            self.push(Segment::Cubic(
                m.apply(c1.0, c1.1),
                m.apply(c2.0, c2.1),
                m.apply(p.0, p.1),
            ));
            angle = next;
        }
    }

    /// `arcTo()`: a line to the first tangent point and the arc of
    /// `radius` tangent to both lines, or a line to (x1, y1) when the
    /// points do not make a corner. The last point is read back through
    /// the inverse of `m`, as the standard says.
    #[allow(clippy::too_many_arguments)]
    pub fn arc_to(
        &mut self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        radius: f64,
        m: &Matrix,
    ) -> Result<(), CanvasError> {
        if !finite(&[x1, y1, x2, y2, radius]) {
            return Ok(());
        }
        if radius < 0.0 {
            return Err(negative("radius", radius));
        }
        self.ensure(x1, y1, m);
        let Some(inverse) = m.invert() else {
            self.line_to(x1, y1, m);
            return Ok(());
        };
        let Some(last) = self.last_point() else {
            return Ok(());
        };
        let p0 = inverse.apply(last.0, last.1);
        let (p1, p2) = ((x1, y1), (x2, y2));
        let cross = (p1.0 - p0.0) * (p2.1 - p1.1) - (p1.1 - p0.1) * (p2.0 - p1.0);
        if p0 == p1 || p1 == p2 || radius == 0.0 || cross.abs() < 1e-12 {
            self.line_to(x1, y1, m);
            return Ok(());
        }
        let unit = |a: Point, b: Point| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let length = dx.hypot(dy);
            (dx / length, dy / length)
        };
        let v1 = unit(p1, p0);
        let v2 = unit(p1, p2);
        let cos = (v1.0 * v2.0 + v1.1 * v2.1).clamp(-1.0, 1.0);
        let angle = cos.acos();
        let along = radius / (angle / 2.0).tan();
        let t1 = (p1.0 + v1.0 * along, p1.1 + v1.1 * along);
        let t2 = (p1.0 + v2.0 * along, p1.1 + v2.1 * along);
        let bisector = unit((0.0, 0.0), (v1.0 + v2.0, v1.1 + v2.1));
        let away = radius / (angle / 2.0).sin();
        let center = (p1.0 + bisector.0 * away, p1.1 + bisector.1 * away);
        let a0 = (t1.1 - center.1).atan2(t1.0 - center.0);
        let a1 = (t2.1 - center.1).atan2(t2.0 - center.0);
        let anticlockwise = (a1 - a0).rem_euclid(TAU) > std::f64::consts::PI;
        self.line_to(t1.0, t1.1, m);
        self.add_ellipse(
            center.0,
            center.1,
            radius,
            radius,
            0.0,
            a0,
            a1,
            anticlockwise,
            m,
        );
        Ok(())
    }

    /// `roundRect()`, its radii already read as (x, y) pairs: one to
    /// four of them, none negative, else a `RangeError`.
    pub fn round_rect(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radii: &[(f64, f64)],
        m: &Matrix,
    ) -> Result<(), CanvasError> {
        if !finite(&[x, y, w, h]) {
            return Ok(());
        }
        if radii.is_empty() || radii.len() > 4 {
            return Err(CanvasError::new(
                "RangeError",
                format!(
                    "{} radii provided. Between one and four radii are necessary.",
                    radii.len()
                ),
            ));
        }
        if radii.iter().any(|&(rx, ry)| !finite(&[rx, ry])) {
            return Ok(());
        }
        if let Some(&(rx, ry)) = radii.iter().find(|&&(rx, ry)| rx < 0.0 || ry < 0.0) {
            let value = if rx < 0.0 { rx } else { ry };
            return Err(CanvasError::new(
                "RangeError",
                format!("Radius value {} is negative.", number(value)),
            ));
        }
        let [mut ul, mut ur, mut lr, mut ll] = match *radii {
            [a] => [a, a, a, a],
            [a, b] => [a, b, a, b],
            [a, b, c] => [a, b, c, b],
            [a, b, c, d] => [a, b, c, d],
            _ => unreachable!(),
        };
        // A side given negative mirrors the rectangle: each radius stays
        // with its corner as given, and a single mirror runs the path the
        // other way round, as in the reference browser.
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        let mut mirror = Matrix::IDENTITY;
        if w < 0.0 {
            x += w;
            w = -w;
            mirror = mirror.then(&Matrix::new(-1.0, 0.0, 0.0, 1.0, 2.0 * x + w, 0.0));
        }
        if h < 0.0 {
            y += h;
            h = -h;
            mirror = mirror.then(&Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, 2.0 * y + h));
        }
        let m = &m.then(&mirror);
        // The radii scaled down together where a side's two overlap.
        let mut scale: f64 = 1.0;
        for (side, sum) in [
            (w, ul.0 + ur.0),
            (h, ur.1 + lr.1),
            (w, lr.0 + ll.0),
            (h, ul.1 + ll.1),
        ] {
            if sum > 0.0 {
                scale = scale.min(side / sum);
            }
        }
        if scale < 1.0 {
            for r in [&mut ul, &mut ur, &mut lr, &mut ll] {
                r.0 *= scale;
                r.1 *= scale;
            }
        }
        const K: f64 = 0.552_284_749_830_793_6;
        self.move_to(x + ul.0, y, m);
        self.line_to(x + w - ur.0, y, m);
        self.corner(
            (x + w - ur.0, y),
            (x + w, y + ur.1),
            ur,
            (1.0, 0.0),
            (0.0, 1.0),
            K,
            m,
        );
        self.line_to(x + w, y + h - lr.1, m);
        self.corner(
            (x + w, y + h - lr.1),
            (x + w - lr.0, y + h),
            lr,
            (0.0, 1.0),
            (-1.0, 0.0),
            K,
            m,
        );
        self.line_to(x + ll.0, y + h, m);
        self.corner(
            (x + ll.0, y + h),
            (x, y + h - ll.1),
            ll,
            (-1.0, 0.0),
            (0.0, -1.0),
            K,
            m,
        );
        self.line_to(x, y + ul.1, m);
        self.corner(
            (x, y + ul.1),
            (x + ul.0, y),
            ul,
            (0.0, -1.0),
            (1.0, 0.0),
            K,
            m,
        );
        self.close();
        self.move_to(x, y, m);
        Ok(())
    }

    /// A rounded corner from `from` to `to`, leaving along `out` and
    /// arriving along `into`, as a quarter ellipse of radii `r`.
    #[allow(clippy::too_many_arguments)]
    fn corner(
        &mut self,
        from: Point,
        to: Point,
        r: (f64, f64),
        out: (f64, f64),
        into: (f64, f64),
        k: f64,
        m: &Matrix,
    ) {
        if r.0 == 0.0 || r.1 == 0.0 {
            self.line_to(to.0, to.1, m);
            return;
        }
        let c1 = (from.0 + out.0 * r.0 * k, from.1 + out.1 * r.1 * k);
        let c2 = (to.0 - into.0 * r.0 * k, to.1 - into.1 * r.1 * k);
        self.cubic_to(c1.0, c1.1, c2.0, c2.1, to.0, to.1, m);
    }

    /// `Path2D.addPath()`: `other`'s subpaths, transformed by `m`.
    pub fn add_path(&mut self, other: &Path, m: &Matrix) {
        let other = other.transformed(m);
        if !self.room(other.points) {
            return;
        }
        self.points += other.points;
        self.subpaths.extend(other.subpaths);
    }

    /// The path with every point transformed by `m`.
    pub fn transformed(&self, m: &Matrix) -> Path {
        let at = |p: Point| m.apply(p.0, p.1);
        Path {
            limit: self.limit,
            subpaths: self
                .subpaths
                .iter()
                .map(|subpath| Subpath {
                    start: at(subpath.start),
                    segments: subpath
                        .segments
                        .iter()
                        .map(|segment| match *segment {
                            Segment::Line(p) => Segment::Line(at(p)),
                            Segment::Quad(c, p) => Segment::Quad(at(c), at(p)),
                            Segment::Cubic(c1, c2, p) => Segment::Cubic(at(c1), at(c2), at(p)),
                        })
                        .collect(),
                    closed: subpath.closed,
                })
                .collect(),
            points: self.points,
        }
    }

    /// The path as a stroke traces it (HTML 4.12.5.1.15): its lines and
    /// curves of no length pruned, then the subpaths left with none.
    pub(crate) fn pruned(&self) -> Path {
        let subpaths = self
            .subpaths
            .iter()
            .filter_map(|subpath| {
                let mut at = subpath.start;
                let segments: Vec<Segment> = subpath
                    .segments
                    .iter()
                    .filter(|segment| {
                        let still = match **segment {
                            Segment::Line(p) => p == at,
                            Segment::Quad(c, p) => c == at && p == at,
                            Segment::Cubic(a, b, p) => a == at && b == at && p == at,
                        };
                        at = segment.end();
                        !still
                    })
                    .copied()
                    .collect();
                (!segments.is_empty()).then_some(Subpath {
                    start: subpath.start,
                    segments,
                    closed: subpath.closed,
                })
            })
            .collect();
        Path {
            subpaths,
            points: self.points,
            limit: self.limit,
        }
    }

    /// The path as `tiny-skia` draws it; `None` when it has nothing to
    /// draw. A subpath of no segment is left out.
    pub(crate) fn to_skia(&self) -> Option<tiny_skia::Path> {
        let mut builder = PathBuilder::new();
        let f = |p: Point| (p.0 as f32, p.1 as f32);
        for subpath in &self.subpaths {
            if subpath.segments.is_empty() {
                continue;
            }
            let (x, y) = f(subpath.start);
            builder.move_to(x, y);
            for segment in &subpath.segments {
                match *segment {
                    Segment::Line(p) => {
                        let (x, y) = f(p);
                        builder.line_to(x, y);
                    }
                    Segment::Quad(c, p) => {
                        let ((cx, cy), (x, y)) = (f(c), f(p));
                        builder.quad_to(cx, cy, x, y);
                    }
                    Segment::Cubic(c1, c2, p) => {
                        let ((ax, ay), (bx, by), (x, y)) = (f(c1), f(c2), f(p));
                        builder.cubic_to(ax, ay, bx, by, x, y);
                    }
                }
            }
            if subpath.closed {
                builder.close();
            }
        }
        builder.finish()
    }

    /// A `tiny-skia` path as one of these, for the test of a point in a
    /// stroke's outline.
    pub(crate) fn from_skia(path: &tiny_skia::Path) -> Path {
        let mut out = Path::new();
        let m = Matrix::IDENTITY;
        let f = |p: tiny_skia::Point| (f64::from(p.x), f64::from(p.y));
        for segment in path.segments() {
            match segment {
                PathSegment::MoveTo(p) => {
                    let (x, y) = f(p);
                    out.move_to(x, y, &m);
                }
                PathSegment::LineTo(p) => {
                    let (x, y) = f(p);
                    out.line_to(x, y, &m);
                }
                PathSegment::QuadTo(c, p) => {
                    let ((cx, cy), (x, y)) = (f(c), f(p));
                    out.quad_to(cx, cy, x, y, &m);
                }
                PathSegment::CubicTo(c1, c2, p) => {
                    let ((ax, ay), (bx, by), (x, y)) = (f(c1), f(c2), f(p));
                    out.cubic_to(ax, ay, bx, by, x, y, &m);
                }
                PathSegment::Close => out.close(),
            }
        }
        out
    }

    /// Each subpath flattened into a closed polygon, for the tests of a
    /// point in it: curves cut into lines within a tenth of a pixel.
    fn polygons(&self) -> Vec<Vec<Point>> {
        let mut out = Vec::new();
        for subpath in &self.subpaths {
            let mut polygon = vec![subpath.start];
            let mut at = subpath.start;
            for segment in &subpath.segments {
                match *segment {
                    Segment::Line(p) => polygon.push(p),
                    Segment::Quad(c, p) => {
                        let n = pieces(&[at, c, p]);
                        for i in 1..=n {
                            let t = i as f64 / n as f64;
                            let u = 1.0 - t;
                            polygon.push((
                                u * u * at.0 + 2.0 * u * t * c.0 + t * t * p.0,
                                u * u * at.1 + 2.0 * u * t * c.1 + t * t * p.1,
                            ));
                        }
                    }
                    Segment::Cubic(c1, c2, p) => {
                        let n = pieces(&[at, c1, c2, p]);
                        for i in 1..=n {
                            let t = i as f64 / n as f64;
                            let u = 1.0 - t;
                            let (a, b, c, d) =
                                (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                            polygon.push((
                                a * at.0 + b * c1.0 + c * c2.0 + d * p.0,
                                a * at.1 + b * c1.1 + c * c2.1 + d * p.1,
                            ));
                        }
                    }
                }
                at = segment.end();
            }
            if polygon.len() > 1 {
                out.push(polygon);
            }
        }
        out
    }

    /// Whether (x, y) is inside the path by `rule`, a point on an edge
    /// counting as inside, as `isPointInPath()` says.
    pub(crate) fn contains(&self, x: f64, y: f64, rule: FillRule) -> bool {
        let mut winding = 0i32;
        for polygon in self.polygons() {
            let n = polygon.len();
            for i in 0..n {
                let (a, b) = (polygon[i], polygon[(i + 1) % n]);
                if on_segment(a, b, (x, y)) {
                    return true;
                }
                if (a.1 <= y) != (b.1 <= y) {
                    let t = (y - a.1) / (b.1 - a.1);
                    if x < a.0 + t * (b.0 - a.0) {
                        winding += if b.1 > a.1 { 1 } else { -1 };
                    }
                }
            }
        }
        match rule {
            FillRule::Winding => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    }
}

/// How many lines a curve of `points` is cut into.
fn pieces(points: &[Point]) -> usize {
    let mut length = 0.0;
    for pair in points.windows(2) {
        length += (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
    }
    (length.sqrt() * 2.0).ceil().clamp(1.0, 256.0) as usize
}

fn on_segment(a: Point, b: Point, p: Point) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    (p.0 - cx).hypot(p.1 - cy) < 1e-9
}
