//! Shapes reaching far past what they are drawn on, cut down first. The
//! rasterizer's clipping of edges loses its precision far out (a
//! rectangle a billion pixels long under a skew broke its walk of the
//! edges), so a shape reaching more than [`REACH`] past its target is
//! cut to the target grown by that much: each contour clipped as a
//! polygon, its curves kept where they lie within the cut, made lines
//! where they lie wholly outside what shows, and halved between.

use std::borrow::Cow;

use tiny_skia::{PathBuilder, PathSegment};

use crate::draw::Area;

/// How far past its target a shape may reach and be drawn as it is.
const REACH: f64 = 8192.0;

/// The margin round the target within which a curve is kept a curve:
/// more than the pixel the smoothing of an edge touches.
const MARGIN: f64 = 2.0;

/// The most times a curve is halved.
const DEPTH: u32 = 24;

type Point = (f64, f64);

/// A rectangle as its left, top, right and bottom.
type Bounds = [f64; 4];

/// How a contour reaches a vertex: along a line, or a cubic curve with
/// these control points.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Edge {
    Line,
    Cubic(Point, Point),
}

/// `path`, in the coordinates of `area`'s pixels, as it may be drawn
/// on `area`: itself when it reaches no more than [`REACH`] past it,
/// else cut down; `None` when nothing of it is left.
pub(crate) fn within(path: &tiny_skia::Path, area: Area) -> Option<Cow<'_, tiny_skia::Path>> {
    let (l, t) = (f64::from(area.x), f64::from(area.y));
    let (r, b) = (l + f64::from(area.w), t + f64::from(area.h));
    let cut = [l - REACH, t - REACH, r + REACH, b + REACH];
    let bounds = path.bounds();
    if f64::from(bounds.left()) >= cut[0]
        && f64::from(bounds.top()) >= cut[1]
        && f64::from(bounds.right()) <= cut[2]
        && f64::from(bounds.bottom()) <= cut[3]
    {
        return Some(Cow::Borrowed(path));
    }
    let shows = [l - MARGIN, t - MARGIN, r + MARGIN, b + MARGIN];
    let mut builder = PathBuilder::new();
    for contour in contours(path, &cut, &shows) {
        let contour = clip(contour, &cut);
        let Some(&(_, last)) = contour.last() else {
            continue;
        };
        if contour.len() < 2 {
            continue;
        }
        builder.move_to(last.0 as f32, last.1 as f32);
        for &(edge, (x, y)) in &contour {
            match edge {
                Edge::Line => builder.line_to(x as f32, y as f32),
                Edge::Cubic(a, b) => builder.cubic_to(
                    a.0 as f32, a.1 as f32, b.0 as f32, b.1 as f32, x as f32, y as f32,
                ),
            }
        }
        builder.close();
    }
    builder.finish().map(Cow::Owned)
}

/// The path's contours, each closed by a line back to its start: its
/// vertices, each with the edge that reaches it from the one before
/// (the first's from the last), and no curve reaching past `cut`.
fn contours(path: &tiny_skia::Path, cut: &Bounds, shows: &Bounds) -> Vec<Vec<(Edge, Point)>> {
    let mut all = Vec::new();
    let mut contour = Vec::new();
    let mut last = (0.0, 0.0);
    let point = |p: tiny_skia::Point| (f64::from(p.x), f64::from(p.y));
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                all.push(std::mem::take(&mut contour));
                last = point(p);
                contour.push((Edge::Line, last));
            }
            PathSegment::LineTo(p) => {
                last = point(p);
                contour.push((Edge::Line, last));
            }
            PathSegment::QuadTo(c, p) => {
                let (c, p) = (point(c), point(p));
                let a = (
                    last.0 + (c.0 - last.0) * 2.0 / 3.0,
                    last.1 + (c.1 - last.1) * 2.0 / 3.0,
                );
                let b = (p.0 + (c.0 - p.0) * 2.0 / 3.0, p.1 + (c.1 - p.1) * 2.0 / 3.0);
                curve(&mut contour, [last, a, b, p], cut, shows, 0);
                last = p;
            }
            PathSegment::CubicTo(a, b, p) => {
                let (a, b, p) = (point(a), point(b), point(p));
                curve(&mut contour, [last, a, b, p], cut, shows, 0);
                last = p;
            }
            PathSegment::Close => {}
        }
    }
    all.push(contour);
    all.retain(|contour| contour.len() > 1);
    all
}

/// The cubic `c` added to the contour: kept when it lies within
/// `cut`, a line when it lies wholly outside `shows` (the area
/// between it and the line is then outside too), else halved.
fn curve(
    contour: &mut Vec<(Edge, Point)>,
    c: [Point; 4],
    cut: &Bounds,
    shows: &Bounds,
    depth: u32,
) {
    let hull = hull(&c);
    let inside = hull[0] >= cut[0] && hull[1] >= cut[1] && hull[2] <= cut[2] && hull[3] <= cut[3];
    if inside {
        contour.push((Edge::Cubic(c[1], c[2]), c[3]));
        return;
    }
    let apart =
        hull[2] < shows[0] || hull[3] < shows[1] || hull[0] > shows[2] || hull[1] > shows[3];
    if apart || depth >= DEPTH {
        contour.push((Edge::Line, c[3]));
        return;
    }
    let mid = |a: Point, b: Point| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (ab, bc, cd) = (mid(c[0], c[1]), mid(c[1], c[2]), mid(c[2], c[3]));
    let (abc, bcd) = (mid(ab, bc), mid(bc, cd));
    let half = mid(abc, bcd);
    curve(contour, [c[0], ab, abc, half], cut, shows, depth + 1);
    curve(contour, [half, bcd, cd, c[3]], cut, shows, depth + 1);
}

/// The bounds of the points, which hold the curve they control.
fn hull(points: &[Point; 4]) -> Bounds {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)],
    )
}

/// The closed contour clipped to `bounds` a side at a time (Sutherland
/// and Hodgman): what it leaves outside a side replaced by a line along
/// it, which keeps every point inside wound as often. Only lines cross
/// a side, the curves lying within `bounds`.
fn clip(mut contour: Vec<(Edge, Point)>, bounds: &Bounds) -> Vec<(Edge, Point)> {
    for side in 0..4 {
        let inside = |p: Point| match side {
            0 => p.0 >= bounds[0],
            1 => p.1 >= bounds[1],
            2 => p.0 <= bounds[2],
            _ => p.1 <= bounds[3],
        };
        let cross = |a: Point, b: Point| {
            if side % 2 == 0 {
                let x = bounds[side];
                (x, a.1 + (x - a.0) / (b.0 - a.0) * (b.1 - a.1))
            } else {
                let y = bounds[side];
                (a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0), y)
            }
        };
        let Some(&(_, mut before)) = contour.last() else {
            break;
        };
        let mut kept = Vec::with_capacity(contour.len());
        for &(edge, p) in &contour {
            match (inside(before), inside(p)) {
                (true, true) => kept.push((edge, p)),
                (true, false) => kept.push((Edge::Line, cross(before, p))),
                (false, true) => {
                    kept.push((Edge::Line, cross(before, p)));
                    kept.push((edge, p));
                }
                (false, false) => {}
            }
            before = p;
        }
        contour = kept;
    }
    contour
}
