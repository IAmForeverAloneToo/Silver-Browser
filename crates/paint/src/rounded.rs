//! Rounded boxes (CSS Backgrounds 3 section 5, gap batch E, D50,
//! `docs/design/boxes-colour.md`): a box's corner radii resolved against
//! its border box, the rounded rectangles inside it inset by its border
//! and padding, and their outlines flattened into the contours a
//! [`crate::DisplayItem::Path`] fills.
//!
//! Each corner is a quarter of an ellipse, flattened into chords few
//! enough that a chord is within about a quarter of a pixel of the curve
//! at four times the size, so that a page zoomed or drawn at a high
//! resolution keeps its curves; at most [`MAX_ARC_SEGMENTS`] a quarter.

use std::f32::consts::{FRAC_PI_2, PI};

use style::computed::{BOTTOM, BorderCollapse, ComputedStyle, Display, LEFT, RIGHT, TOP};

use crate::Rect;

/// The most chords a quarter ellipse is flattened into: a contour has
/// at most four times as many points, whatever the radii.
pub const MAX_ARC_SEGMENTS: usize = 64;

/// The corners, clockwise from the top-left, as [`style::computed::Radii`]
/// orders them.
pub const TOP_LEFT: usize = 0;
pub const TOP_RIGHT: usize = 1;
pub const BOTTOM_RIGHT: usize = 2;
pub const BOTTOM_LEFT: usize = 3;

/// A rectangle with elliptical corners: each corner's horizontal and
/// vertical radius, at most half the side they are on together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundedRect {
    pub rect: Rect,
    pub radii: [(f32, f32); 4],
}

impl RoundedRect {
    /// The border box `rect` of a box of `style` with its corners'
    /// radii, percentages of the box's width and height, reduced by
    /// section 5.5's factor where adjacent radii add up past their side;
    /// `None` when every radius is zero, so that a square box keeps the
    /// items it has, and for a table or a part of one whose borders
    /// collapse, which section 5 has ignore its radii.
    pub fn of(style: &ComputedStyle, rect: Rect) -> Option<RoundedRect> {
        let radii = style.radii.as_deref()?;
        if style.border_collapse == BorderCollapse::Collapse
            && matches!(
                style.display,
                Display::Table
                    | Display::InlineTable
                    | Display::TableRowGroup
                    | Display::TableHeaderGroup
                    | Display::TableFooterGroup
                    | Display::TableRow
                    | Display::TableColumnGroup
                    | Display::TableColumn
                    | Display::TableCell
            )
        {
            return None;
        }
        let resolved = radii
            .0
            .each_ref()
            .map(|[h, v]| (h.resolve(rect.w).max(0.0), v.resolve(rect.h).max(0.0)));
        if resolved.iter().all(|(h, v)| *h <= 0.0 || *v <= 0.0) {
            return None;
        }
        // A corner with one radius zero is square (section 5.2).
        let resolved = resolved.map(|(h, v)| {
            if h > 0.0 && v > 0.0 {
                (h, v)
            } else {
                (0.0, 0.0)
            }
        });
        let ratio = |side: f32, a: f32, b: f32| {
            if a + b > 0.0 {
                side / (a + b)
            } else {
                f32::INFINITY
            }
        };
        let f = [
            ratio(rect.w, resolved[TOP_LEFT].0, resolved[TOP_RIGHT].0),
            ratio(rect.h, resolved[TOP_RIGHT].1, resolved[BOTTOM_RIGHT].1),
            ratio(rect.w, resolved[BOTTOM_RIGHT].0, resolved[BOTTOM_LEFT].0),
            ratio(rect.h, resolved[BOTTOM_LEFT].1, resolved[TOP_LEFT].1),
        ]
        .into_iter()
        .fold(1.0f32, f32::min);
        let radii = resolved.map(|(h, v)| (h * f, v * f));
        Some(RoundedRect { rect, radii })
    }

    /// Whether every corner is square.
    pub fn is_square(&self) -> bool {
        self.radii.iter().all(|(h, v)| *h <= 0.0 || *v <= 0.0)
    }

    /// The rounded rectangle inside this one by `widths`, top, right,
    /// bottom and left: each radius less the width beside it, never
    /// below zero (section 5.2's inner border edge).
    pub fn inset(&self, widths: [f32; 4]) -> RoundedRect {
        let Rect { x, y, w, h } = self.rect;
        let rect = Rect {
            x: x + widths[LEFT],
            y: y + widths[TOP],
            w: (w - widths[LEFT] - widths[RIGHT]).max(0.0),
            h: (h - widths[TOP] - widths[BOTTOM]).max(0.0),
        };
        let less = |(rh, rv): (f32, f32), across: f32, down: f32| {
            let (rh, rv) = ((rh - across).max(0.0), (rv - down).max(0.0));
            if rh > 0.0 && rv > 0.0 {
                (rh, rv)
            } else {
                (0.0, 0.0)
            }
        };
        RoundedRect {
            rect,
            radii: [
                less(self.radii[TOP_LEFT], widths[LEFT], widths[TOP]),
                less(self.radii[TOP_RIGHT], widths[RIGHT], widths[TOP]),
                less(self.radii[BOTTOM_RIGHT], widths[RIGHT], widths[BOTTOM]),
                less(self.radii[BOTTOM_LEFT], widths[LEFT], widths[BOTTOM]),
            ],
        }
    }

    /// The centre of `corner`'s ellipse.
    fn centre(&self, corner: usize) -> (f32, f32) {
        let Rect { x, y, w, h } = self.rect;
        let (rh, rv) = self.radii[corner];
        match corner {
            TOP_LEFT => (x + rh, y + rv),
            TOP_RIGHT => (x + w - rh, y + rv),
            BOTTOM_RIGHT => (x + w - rh, y + h - rv),
            _ => (x + rh, y + h - rv),
        }
    }

    /// The angles `corner`'s quarter spans, clockwise on the screen
    /// (y down): the top-left's from pointing left to pointing up.
    pub fn span(corner: usize) -> (f32, f32) {
        match corner {
            TOP_LEFT => (PI, 1.5 * PI),
            TOP_RIGHT => (1.5 * PI, 2.0 * PI),
            BOTTOM_RIGHT => (0.0, FRAC_PI_2),
            _ => (FRAC_PI_2, PI),
        }
    }

    /// The points of `corner`'s arc from angle `from` to `to`, both ends
    /// among them: the corner's point alone where it is square.
    pub fn arc(&self, corner: usize, from: f32, to: f32) -> Vec<(f32, f32)> {
        let (cx, cy) = self.centre(corner);
        let (rh, rv) = self.radii[corner];
        if rh <= 0.0 || rv <= 0.0 {
            return vec![(cx, cy)];
        }
        let quarter = segments(rh.max(rv));
        let n = ((quarter as f32 * (to - from).abs() / FRAC_PI_2).ceil() as usize).max(1);
        (0..=n)
            .map(|i| {
                let t = from + (to - from) * i as f32 / n as f32;
                (cx + rh * t.cos(), cy + rv * t.sin())
            })
            .collect()
    }

    /// The outline, clockwise from the top-left corner's left end.
    pub fn contour(&self) -> Vec<(f32, f32)> {
        let mut points = Vec::new();
        for corner in [TOP_LEFT, TOP_RIGHT, BOTTOM_RIGHT, BOTTOM_LEFT] {
            let (from, to) = Self::span(corner);
            points.extend(self.arc(corner, from, to));
        }
        points
    }
}

/// How many chords a quarter ellipse of the larger radius `r` takes: a
/// chord of angle `a` is `r * a * a / 8` from the curve, a quarter of a
/// pixel at four times the size for `a = sqrt(0.5 / r)`.
fn segments(r: f32) -> usize {
    let angle = (0.5 / r.max(1e-3)).sqrt();
    ((FRAC_PI_2 / angle).ceil() as usize).clamp(1, MAX_ARC_SEGMENTS)
}

/// The lines each corner divides along between its two sides (section
/// 5.4's transition between the sides' colours): from the border box's
/// corner to the padding box's, a side of no width giving the whole
/// corner to the other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mitres {
    pub border: Rect,
    pub widths: [f32; 4],
}

impl Mitres {
    /// Where `corner`'s arc of `r` meets its mitre, as an angle of the
    /// arc; the arc's own share of the widths where the line misses it.
    pub fn split(&self, r: &RoundedRect, corner: usize) -> f32 {
        let Rect { x, y, w, h } = self.border;
        let [top, right, bottom, left] = self.widths;
        let (from, to) = RoundedRect::span(corner);
        // The border box's corner and the way in to the padding box's.
        let ((px, py), (dx, dy)) = match corner {
            TOP_LEFT => ((x, y), (left, top)),
            TOP_RIGHT => ((x + w, y), (-right, top)),
            BOTTOM_RIGHT => ((x + w, y + h), (-right, -bottom)),
            _ => ((x, y + h), (left, -bottom)),
        };
        // The side the corner starts on, clockwise, and the one it ends
        // on: a side of no width gives the corner to the other.
        let (first, second) = match corner {
            TOP_LEFT => (left, top),
            TOP_RIGHT => (top, right),
            BOTTOM_RIGHT => (right, bottom),
            _ => (bottom, left),
        };
        if first <= 0.0 && second <= 0.0 {
            return (from + to) / 2.0;
        }
        if first <= 0.0 {
            return from;
        }
        if second <= 0.0 {
            return to;
        }
        let (cx, cy) = r.centre(corner);
        let (rh, rv) = r.radii[corner];
        if rh <= 0.0 || rv <= 0.0 {
            return (from + to) / 2.0;
        }
        let side = |t: f32| {
            let (qx, qy) = (cx + rh * t.cos() - px, cy + rv * t.sin() - py);
            dx * qy - dy * qx
        };
        let (mut lo, mut hi) = (from, to);
        let (f_lo, f_hi) = (side(lo), side(hi));
        if f_lo.signum() == f_hi.signum() {
            // The line misses the arc: by the widths' share instead.
            return from + (to - from) * first / (first + second);
        }
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if side(mid).signum() == f_lo.signum() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    }
}

/// The corners a side runs between, clockwise: its start and its end.
pub fn corners_of(side: usize) -> (usize, usize) {
    match side {
        TOP => (TOP_LEFT, TOP_RIGHT),
        RIGHT => (TOP_RIGHT, BOTTOM_RIGHT),
        BOTTOM => (BOTTOM_RIGHT, BOTTOM_LEFT),
        _ => (BOTTOM_LEFT, TOP_LEFT),
    }
}

/// The edge of `side`'s part of `r`, from where its start corner meets
/// its mitre round to its end corner's; or, at an end `whole` says,
/// round the whole corner, for the side drawn after it to cover the
/// rest without a seam between them.
pub fn side_edge(
    r: &RoundedRect,
    side: usize,
    mitres: &Mitres,
    whole: (bool, bool),
) -> Vec<(f32, f32)> {
    let (start, end) = corners_of(side);
    let from = if whole.0 {
        RoundedRect::span(start).0
    } else {
        mitres.split(r, start)
    };
    let to = if whole.1 {
        RoundedRect::span(end).1
    } else {
        mitres.split(r, end)
    };
    let mut points = r.arc(start, from, RoundedRect::span(start).1);
    points.extend(r.arc(end, RoundedRect::span(end).0, to));
    points
}

/// The band of `side` between the rounded rectangles `outer` and
/// `inner`: the outer edge of its part, then the inner edge back.
pub fn side_band(
    outer: &RoundedRect,
    inner: &RoundedRect,
    side: usize,
    mitres: &Mitres,
    whole: (bool, bool),
) -> Vec<(f32, f32)> {
    let mut points = side_edge(outer, side, mitres, whole);
    let mut back = side_edge(inner, side, mitres, whole);
    back.reverse();
    points.extend(back);
    points
}

/// The band of `corner`'s part on `side`, from its mitre to where the
/// side's straight run begins or ends: a dashed side's corner, drawn
/// solid.
pub fn corner_band(
    outer: &RoundedRect,
    inner: &RoundedRect,
    side: usize,
    corner: usize,
    mitres: &Mitres,
) -> Vec<(f32, f32)> {
    let (start, _) = corners_of(side);
    let along = |r: &RoundedRect| {
        let split = mitres.split(r, corner);
        if corner == start {
            r.arc(corner, split, RoundedRect::span(corner).1)
        } else {
            r.arc(corner, RoundedRect::span(corner).0, split)
        }
    };
    let mut points = along(outer);
    let mut back = along(inner);
    back.reverse();
    points.extend(back);
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn style(css: &str) -> ComputedStyle {
        let document =
            dom::Document::from_node(&html::parse_document("<div>x</div>").expect("never fails"));
        let sheet = css::parse_stylesheet(&format!("div {{ {css} }}")).expect("valid");
        let styles = style::cascade_document(&document, &[sheet], &Default::default());
        let div = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("div"))
            .expect("the div");
        styles.get(div).expect("styled").clone()
    }

    #[test]
    fn radii_resolve_and_shrink_together_where_they_overlap() {
        let r = RoundedRect::of(
            &style("border-radius: 10px 20% / 5px"),
            rect(0.0, 0.0, 100.0, 50.0),
        )
        .expect("rounded");
        assert_eq!(
            r.radii,
            [(10.0, 5.0), (20.0, 5.0), (10.0, 5.0), (20.0, 5.0)]
        );
        // 80 and 80 across 100: every radius by 100 / 160 (section 5.5).
        let r = RoundedRect::of(&style("border-radius: 80px"), rect(0.0, 0.0, 100.0, 400.0))
            .expect("rounded");
        assert_eq!(r.radii[TOP_LEFT], (50.0, 50.0));
        // A zero radius on either axis makes the corner square.
        let r = RoundedRect::of(
            &style("border-top-left-radius: 10px 0; border-top-right-radius: 4px"),
            rect(0.0, 0.0, 100.0, 50.0),
        )
        .expect("rounded");
        assert_eq!(r.radii[TOP_LEFT], (0.0, 0.0));
        assert!(RoundedRect::of(&style(""), rect(0.0, 0.0, 10.0, 10.0)).is_none());
        assert!(
            RoundedRect::of(&style("border-radius: 0 0"), rect(0.0, 0.0, 10.0, 10.0)).is_none()
        );
    }

    #[test]
    fn an_inner_edge_takes_the_widths_off_its_radii() {
        let r = RoundedRect {
            rect: rect(0.0, 0.0, 100.0, 100.0),
            radii: [(20.0, 20.0), (5.0, 5.0), (20.0, 10.0), (0.0, 0.0)],
        };
        let inner = r.inset([4.0, 8.0, 4.0, 2.0]);
        assert_eq!(inner.rect, rect(2.0, 4.0, 90.0, 92.0));
        assert_eq!(
            inner.radii,
            [(18.0, 16.0), (0.0, 0.0), (12.0, 6.0), (0.0, 0.0)]
        );
    }

    #[test]
    fn a_contour_runs_clockwise_on_its_curves() {
        let r = RoundedRect {
            rect: rect(10.0, 20.0, 100.0, 60.0),
            radii: [(10.0, 10.0); 4],
        };
        let points = r.contour();
        let near =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        assert!(near(points[0], (10.0, 30.0)), "{:?}", points[0]);
        assert!(points.iter().any(|p| near(*p, (20.0, 20.0))));
        assert!(points.iter().any(|p| near(*p, (110.0, 30.0))));
        // Every point is on its corner's curve.
        for &(x, y) in &points {
            let (cx, cy) = (x.clamp(20.0, 100.0), y.clamp(30.0, 70.0));
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            assert!(d <= 10.0 + 1e-3, "({x}, {y})");
        }
        // Signed area positive: clockwise with y down.
        let area: f32 = (0..points.len())
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum();
        let exact = 100.0 * 60.0 - (4.0 - PI) * 100.0;
        assert!((area / 2.0 - exact).abs() < 2.0, "{}", area / 2.0);
        // The chords grow with the radius, to a bound.
        assert!(segments(1.0) < segments(100.0));
        assert_eq!(segments(1e9), MAX_ARC_SEGMENTS);
    }

    #[test]
    fn corners_divide_on_the_line_from_corner_to_corner() {
        let border = rect(0.0, 0.0, 100.0, 100.0);
        let r = RoundedRect {
            rect: border,
            radii: [(20.0, 20.0), (20.0, 20.0), (40.0, 10.0), (20.0, 20.0)],
        };
        let mitres = Mitres {
            border,
            widths: [5.0, 0.0, 5.0, 5.0],
        };
        // Equal widths and a round corner: half way round, 45 degrees.
        assert!((mitres.split(&r, TOP_LEFT) - 1.25 * PI).abs() < 1e-4);
        // A side of no width gives the corner to the other.
        assert_eq!(mitres.split(&r, TOP_RIGHT), 2.0 * PI);
        let left_none = Mitres {
            border,
            widths: [5.0, 5.0, 5.0, 0.0],
        };
        assert_eq!(left_none.split(&r, TOP_LEFT), PI);
        // An elliptical corner meets the line where the line is.
        let mitres = Mitres {
            border,
            widths: [5.0; 4],
        };
        let t = mitres.split(&r, BOTTOM_RIGHT);
        let (cx, cy) = (60.0, 90.0);
        let (px, py) = (cx + 40.0 * t.cos(), cy + 10.0 * t.sin());
        assert!(
            ((100.0 - px) - (100.0 - py)).abs() < 0.2,
            "({px}, {py}) off the line"
        );
    }
}
