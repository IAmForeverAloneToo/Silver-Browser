//! Gradients rendered (CSS Images 3 section 3, M23.3,
//! `docs/design/images.md`): by the engine's own code into an image the
//! size of a tile, once, which both backends then draw as they draw any
//! image, so that they draw the same pixels. Each pixel takes the colour
//! of the gradient at its centre, the colours interpolated in
//! gamma-encoded sRGB with premultiplied alpha (CSS Color 4 section 13.2,
//! for the legacy colours the engine parses). A linear gradient straight
//! across or down is the same along its other axis, and is rendered one
//! pixel thick there, to be stretched.

use std::sync::Arc;

use style::computed::{Color, Gradient, GradientItem, GradientShape, LineDirection, RadialSize};

/// The most pixels a gradient is rendered with: a larger tile is
/// rendered smaller and stretched over it.
pub const MAX_GRADIENT_PIXELS: u64 = 1 << 22;

/// The period below which a repeating gradient is drawn as its average
/// colour, the pixels being too coarse to show it (section 3.3).
const MIN_PERIOD: f32 = 0.5;

/// A radius taken for one of zero (section 3.2.3).
const TINY: f32 = 1e-4;

/// The bytes from which a gradient is rendered on several threads, a
/// band of rows each, and the most threads.
const PARALLEL_BYTES: usize = 1 << 20;
const MAX_THREADS: usize = 8;

/// A colour, premultiplied, its channels from 0 to 1.
type Premultiplied = [f32; 4];

fn premultiplied(c: Color) -> Premultiplied {
    [c.r * c.a, c.g * c.a, c.b * c.a, c.a]
}

fn mix(a: Premultiplied, b: Premultiplied, t: f32) -> Premultiplied {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// Where a point's colour is read on the gradient's line.
enum Geometry {
    /// Along the line from `start` in the direction `dir`, a unit
    /// vector.
    Linear { start: (f32, f32), dir: (f32, f32) },
    /// Out from `center` through ellipses of the ending shape's ratio,
    /// `rx` and `ry` its radii; `flat` for one of zero height, whose
    /// points are all far along the line.
    Radial {
        center: (f32, f32),
        rx: f32,
        ry: f32,
        flat: bool,
    },
}

impl Geometry {
    /// The gradient's geometry in a box `w` by `h`, and the length of
    /// its line, against which its percentages resolve.
    fn of(gradient: &Gradient, w: f32, h: f32) -> (Geometry, f32) {
        match &gradient.shape {
            GradientShape::Linear(direction) => {
                let dir = match *direction {
                    LineDirection::Angle(degrees) => {
                        let a = degrees.to_radians();
                        (a.sin(), -a.cos())
                    }
                    // Perpendicular to the diagonal between the two
                    // corners beside it, into its quadrant (section
                    // 3.1.1).
                    LineDirection::Corner { right, bottom } => {
                        let (sx, sy) = (
                            if right { 1.0 } else { -1.0 },
                            if bottom { 1.0 } else { -1.0 },
                        );
                        let (dx, dy) = (sx * h, sy * w);
                        let len = (dx * dx + dy * dy).sqrt();
                        if len > 0.0 {
                            (dx / len, dy / len)
                        } else {
                            (0.0, sy)
                        }
                    }
                };
                let length = (w * dir.0).abs() + (h * dir.1).abs();
                let start = (
                    w / 2.0 - dir.0 * length / 2.0,
                    h / 2.0 - dir.1 * length / 2.0,
                );
                (Geometry::Linear { start, dir }, length)
            }
            GradientShape::Radial {
                ellipse,
                size,
                center,
            } => {
                let (cx, cy) = (center[0].resolve(w), center[1].resolve(h));
                let (left, right, top, bottom) =
                    (cx.abs(), (w - cx).abs(), cy.abs(), (h - cy).abs());
                let corners = [(cx, cy), (cx - w, cy), (cx, cy - h), (cx - w, cy - h)];
                let corner = |farthest: bool| {
                    let distance = |(dx, dy): (f32, f32)| (dx * dx + dy * dy).sqrt();
                    let pick = corners.iter().copied().reduce(|a, b| {
                        let further = distance(b) > distance(a);
                        if further == farthest { b } else { a }
                    });
                    pick.unwrap_or((0.0, 0.0))
                };
                // An ellipse through a corner keeps the ratio of the one
                // to the sides the same way (section 3.2.1).
                let through = |(dx, dy): (f32, f32), (sx, sy): (f32, f32)| {
                    if sx <= 0.0 || sy <= 0.0 {
                        return (sx, sy);
                    }
                    let k = ((dx / sx).powi(2) + (dy / sy).powi(2)).sqrt();
                    (sx * k, sy * k)
                };
                let (rx, ry) = match size {
                    RadialSize::ClosestSide if *ellipse => (left.min(right), top.min(bottom)),
                    RadialSize::ClosestSide => {
                        let r = left.min(right).min(top).min(bottom);
                        (r, r)
                    }
                    RadialSize::FarthestSide if *ellipse => (left.max(right), top.max(bottom)),
                    RadialSize::FarthestSide => {
                        let r = left.max(right).max(top).max(bottom);
                        (r, r)
                    }
                    RadialSize::ClosestCorner if *ellipse => {
                        through(corner(false), (left.min(right), top.min(bottom)))
                    }
                    RadialSize::FarthestCorner if *ellipse => {
                        through(corner(true), (left.max(right), top.max(bottom)))
                    }
                    RadialSize::ClosestCorner | RadialSize::FarthestCorner => {
                        let (dx, dy) = corner(*size == RadialSize::FarthestCorner);
                        let r = (dx * dx + dy * dy).sqrt();
                        (r, r)
                    }
                    RadialSize::Circle(r) => (*r, *r),
                    RadialSize::Ellipse(rx, ry) => (rx.resolve(w), ry.resolve(h)),
                };
                // The degenerate shapes (section 3.2.3): a circle of no
                // radius a tiny one; an ellipse of no width a tiny wide
                // one very tall, its percentages nothing; of no height,
                // a flat one whose every point is past the line's end.
                let circle = !*ellipse || matches!(size, RadialSize::Circle(_));
                let (rx, ry, flat) = if circle && rx <= 0.0 {
                    (TINY, TINY, false)
                } else if rx <= 0.0 {
                    (TINY, f32::INFINITY, false)
                } else if ry <= 0.0 {
                    (rx, TINY, true)
                } else {
                    (rx, ry, false)
                };
                let length = if rx == TINY && ry.is_infinite() {
                    0.0
                } else {
                    rx
                };
                (
                    Geometry::Radial {
                        center: (cx, cy),
                        rx,
                        ry,
                        flat,
                    },
                    length,
                )
            }
        }
    }

    /// How far along the line the point `(x, y)` is, in pixels.
    fn position(&self, x: f32, y: f32) -> f32 {
        match *self {
            Geometry::Linear { start, dir } => (x - start.0) * dir.0 + (y - start.1) * dir.1,
            Geometry::Radial { flat: true, .. } => f32::INFINITY,
            Geometry::Radial { center, rx, ry, .. } => {
                let (dx, dy) = (x - center.0, y - center.1);
                if ry.is_infinite() {
                    return dx.abs();
                }
                ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt() * rx
            }
        }
    }
}

/// A gradient's stops on its line, `length` long: each its position in
/// pixels and its colour; after section 3.4.3's fixup: the first stop at
/// 0% and the last at 100% where they have no position, no position
/// before an earlier one, and the rest spaced evenly between those
/// around them; and each hint made stops, as [`hint_stops`] makes them.
fn stops(items: &[GradientItem], length: f32) -> Vec<(f32, Premultiplied)> {
    let mut out: Vec<(Option<f32>, Premultiplied, Option<f32>)> = Vec::new();
    let mut floor = f32::NEG_INFINITY;
    let count = items
        .iter()
        .filter(|i| matches!(i, GradientItem::Stop(..)))
        .count();
    let mut index = 0;
    for item in items {
        match item {
            GradientItem::Stop(color, position) => {
                let mut at = position.as_ref().map(|p| p.resolve(length));
                if at.is_none() && index == 0 {
                    at = Some(0.0);
                }
                if at.is_none() && index == count - 1 {
                    at = Some(length);
                }
                let at = at.map(|at| {
                    floor = floor.max(at);
                    floor
                });
                out.push((at, premultiplied(*color), None));
                index += 1;
            }
            GradientItem::Hint(position) => {
                floor = floor.max(position.resolve(length));
                if let Some(last) = out.last_mut() {
                    last.2 = Some(floor);
                }
            }
        }
    }
    // The runs without a position, evenly between those with one.
    let mut placed: Vec<(f32, Premultiplied, Option<f32>)> = Vec::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        match out[i].0 {
            Some(at) => {
                placed.push((at, out[i].1, out[i].2));
                i += 1;
            }
            None => {
                let before = placed.last().map_or(0.0, |s| s.0);
                let end = (i..out.len())
                    .find(|&j| out[j].0.is_some())
                    .unwrap_or(out.len() - 1);
                let after = out[end].0.unwrap_or(before);
                let n = (end - i + 1) as f32;
                for (k, j) in (i..end).enumerate() {
                    let at = before + (after - before) * (k + 1) as f32 / n;
                    placed.push((at, out[j].1, out[j].2));
                }
                i = end;
            }
        }
    }
    let mut out = Vec::with_capacity(placed.len());
    for (i, &(at, color, hint)) in placed.iter().enumerate() {
        out.push((at, color));
        if let (Some(hint), Some(&(right, next, _))) = (hint, placed.get(i + 1)) {
            out.extend(hint_stops((at, color), hint, (right, next)));
        }
    }
    out
}

/// The stops the reference browser draws a hint at `hint` between the
/// stops `left` and `right` as (M23.3; Blink's
/// `ReplaceColorHintsWithColorStops`): none halfway, where the blend is
/// even; one of the far stop's colour at a hint on a stop; else nine,
/// seven on the longer side from its middle and two on the shorter,
/// each coloured by section 3.4.2's curve `p^(ln 0.5 / ln h)`. Between
/// them the colours blend evenly: a hint's curve is followed in pieces.
fn hint_stops(
    left: (f32, Premultiplied),
    hint: f32,
    right: (f32, Premultiplied),
) -> Vec<(f32, Premultiplied)> {
    let (left_dist, right_dist) = (hint - left.0, right.0 - hint);
    let total = right.0 - left.0;
    let near = |a: f32, b: f32| (a - b).abs() <= f32::EPSILON * total.abs().max(1.0);
    if near(left_dist, right_dist) {
        return Vec::new();
    }
    if near(left_dist, 0.0) {
        return vec![(hint, right.1)];
    }
    if near(right_dist, 0.0) {
        return vec![(hint, left.1)];
    }
    let mut at = [0.0f32; 9];
    if left_dist > right_dist {
        for (y, at) in at[..7].iter_mut().enumerate() {
            *at = left.0 + left_dist * ((7.0 + y as f32) / 13.0);
        }
        at[7] = hint + right_dist / 3.0;
        at[8] = hint + right_dist * 2.0 / 3.0;
    } else {
        at[0] = left.0 + left_dist / 3.0;
        at[1] = left.0 + left_dist * 2.0 / 3.0;
        for (y, at) in at[2..].iter_mut().enumerate() {
            *at = hint + right_dist * (y as f32 / 13.0);
        }
    }
    let exponent = 0.5f32.ln() / (left_dist / total).ln();
    at.iter()
        .map(|&at| {
            let weight = ((at - left.0) / total).powf(exponent);
            (at, mix(left.1, right.1, weight))
        })
        .collect()
}

/// The colour of the gradient line at `t` (section 3.4.2): the first
/// stop's before it, the last's after, and between two stops a blend of
/// theirs; of coincident stops, the last from their position on.
fn color_at(stops: &[(f32, Premultiplied)], t: f32) -> Premultiplied {
    let first = stops[0];
    if t < first.0 || stops.len() == 1 {
        return first.1;
    }
    let i = stops.partition_point(|s| s.0 <= t);
    if i >= stops.len() {
        return stops[stops.len() - 1].1;
    }
    let (a, b) = (stops[i - 1], stops[i]);
    let span = b.0 - a.0;
    if span <= 0.0 {
        return b.1;
    }
    mix(a.1, b.1, (t - a.0) / span)
}

/// The average colour of stops spread over their span (section 3.3):
/// each pair of neighbours weighted by half the distance between them.
fn average(stops: &[(f32, Premultiplied)]) -> Premultiplied {
    let total = stops[stops.len() - 1].0 - stops[0].0;
    // Stops all at one place are taken evenly spread.
    let spread: Vec<f32> = if total > 0.0 {
        stops.iter().map(|s| s.0).collect()
    } else {
        (0..stops.len()).map(|i| i as f32).collect()
    };
    let total = spread[spread.len() - 1] - spread[0];
    if total <= 0.0 {
        return stops[0].1;
    }
    let mut sum = [0.0; 4];
    for (pair, at) in stops.windows(2).zip(spread.windows(2)) {
        let weight = (at[1] - at[0]) / 2.0 / total;
        for (c, total) in sum.iter_mut().enumerate() {
            *total += (pair[0].1[c] + pair[1].1[c]) * weight;
        }
    }
    sum
}

/// The pixels `gradient` is rendered at for a tile `w` by `h`: one
/// across a vertical gradient's width or down a horizontal one's height,
/// and no more than [`MAX_GRADIENT_PIXELS`] in all.
fn resolution(gradient: &Gradient, w: f32, h: f32) -> (u32, u32) {
    let (mut pw, mut ph) = (w.ceil().max(1.0), h.ceil().max(1.0));
    if let GradientShape::Linear(LineDirection::Angle(degrees)) = gradient.shape {
        let turned = degrees.rem_euclid(180.0);
        if turned == 0.0 {
            pw = 1.0;
        } else if turned == 90.0 {
            ph = 1.0;
        }
    }
    let pixels = pw * ph;
    let most = MAX_GRADIENT_PIXELS as f32;
    if pixels > most {
        let scale = (most / pixels).sqrt();
        pw = (pw * scale).floor().max(1.0);
        ph = (ph * scale).floor().max(1.0);
    }
    (pw as u32, ph as u32)
}

/// `gradient` rendered for a tile `w` by `h` CSS pixels: premultiplied
/// RGBA, one frame, at [`resolution`]'s size, each pixel the colour at
/// the tile's point its centre stands for. A large one is rendered a
/// band of rows a thread.
pub fn render(gradient: &Gradient, w: f32, h: f32) -> images::Image {
    let (pw, ph) = resolution(gradient, w.max(0.0), h.max(0.0));
    let threads = match pw as usize * ph as usize * 4 >= PARALLEL_BYTES {
        true => std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_THREADS)),
        false => 1,
    };
    render_on(gradient, w, h, threads)
}

/// [`render`] on `threads` threads.
fn render_on(gradient: &Gradient, w: f32, h: f32, threads: usize) -> images::Image {
    let (w, h) = (w.max(0.0), h.max(0.0));
    let (pw, ph) = resolution(gradient, w, h);
    let (geometry, length) = Geometry::of(gradient, w, h);
    let stops = stops(&gradient.items, length);
    let (first, last) = (stops[0].0, stops[stops.len() - 1].0);
    let period = last - first;
    // A repeating gradient too fine to show, or flat, is its average.
    let flat = matches!(geometry, Geometry::Radial { flat: true, .. });
    let solid = (gradient.repeating && (period < MIN_PERIOD || flat)).then(|| average(&stops));
    // A channel to a byte, rounded; the cast saturates, below 0 and
    // above 255 alike.
    let byte = |v: f32| (v * 255.0 + 0.5) as u8;
    let xs: Vec<f32> = (0..pw)
        .map(|px| (px as f32 + 0.5) * w / pw as f32)
        .collect();
    let row = |py: usize, line: &mut [u8]| {
        let y = (py as f32 + 0.5) * h / ph as f32;
        for (&x, texel) in xs.iter().zip(line.as_chunks_mut::<4>().0) {
            let c = solid.unwrap_or_else(|| {
                let mut t = geometry.position(x, y);
                if gradient.repeating {
                    t = first + (t - first).rem_euclid(period);
                }
                color_at(&stops, t)
            });
            *texel = c.map(byte);
        }
    };
    let stride = pw as usize * 4;
    let mut pixels = vec![0u8; stride * ph as usize];
    let band = (ph as usize).div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (k, rows) in pixels.chunks_mut(band * stride).enumerate() {
            let row = &row;
            let mut draw = move || {
                for (i, line) in rows.chunks_mut(stride).enumerate() {
                    row(k * band + i, line);
                }
            };
            if threads > 1 {
                scope.spawn(draw);
            } else {
                draw();
            }
        }
    });
    images::Image {
        width: pw,
        height: ph,
        frames: vec![images::Frame {
            pixels: Arc::from(pixels),
            delay: 0,
        }],
        plays: images::Plays::Times(1),
    }
}

/// A number for `gradient` rendered for a tile `w` by `h`, the same for
/// the same, which display items compare by: its top bit set, apart from
/// the engine's images'.
pub fn number(gradient: &Gradient, w: f32, h: f32) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    format!("{gradient:?}").hash(&mut hasher);
    w.to_bits().hash(&mut hasher);
    h.to_bits().hash(&mut hasher);
    hasher.finish() | 1 << 63
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(css: &str) -> Gradient {
        let html = format!("<div style=\"background-image: {css}\"></div>");
        let document = dom::Document::from_node(&html::parse_document(&html).expect("never fails"));
        let styles = style::Stylesheets::new(Vec::new())
            .cascade(&document, &style::CascadeContext::default());
        let div = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("div"))
            .expect("the div");
        match &styles.get(div).expect("styled").background().image[0] {
            Some(style::computed::Image::Gradient(g)) => (**g).clone(),
            other => panic!("not a gradient: {other:?}"),
        }
    }

    /// The colour of `image` at `(x, y)` composited over white, as the
    /// reference browser's screen shows it.
    fn over_white(image: &images::Image, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * image.width + x) * 4) as usize;
        let p = &image.frames[0].pixels[i..i + 4];
        [0, 1, 2].map(|c| (p[c] as u16 + 255 - p[3] as u16) as u8)
    }

    /// Whether two colours are within one level of each other in each
    /// channel: the reference browser's ramps are quantized.
    fn near(a: [u8; 3], b: [u8; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1)
    }

    /// `css` rendered 100 by 40 holds the reference browser's colours
    /// at the points given, along the row at `y`.
    fn holds(css: &str, y: u32, expected: &[(u32, [u8; 3])]) {
        let image = render(&gradient(css), 100.0, 40.0);
        let (sx, sy) = (100 / image.width, 40 / image.height);
        for &(x, colour) in expected {
            let got = over_white(&image, x / sx, y / sy);
            assert!(near(got, colour), "{css} at {x}: {got:?}, not {colour:?}");
        }
    }

    #[test]
    fn a_gradient_is_the_same_rendered_on_one_thread_or_several() {
        for css in [
            "linear-gradient(45deg, red, blue 40%, lime)",
            "repeating-radial-gradient(circle at 30% 70%, red 0 7px, blue 7px 13px)",
        ] {
            let g = gradient(css);
            let one = render_on(&g, 301.0, 203.0, 1);
            for threads in [2, 5, 8] {
                assert!(
                    one == render_on(&g, 301.0, 203.0, threads),
                    "{css} on {threads}"
                );
            }
        }
    }

    #[test]
    fn gradients_take_the_reference_browsers_colours() {
        holds(
            "linear-gradient(to right, red, blue)",
            20,
            &[
                (0, [253, 0, 1]),
                (10, [228, 0, 26]),
                (25, [190, 0, 65]),
                (49, [129, 0, 126]),
                (75, [62, 0, 192]),
            ],
        );
        // Premultiplied: transparent blue takes nothing of its colour.
        holds(
            "linear-gradient(to right, red, #00f0)",
            20,
            &[
                (10, [255, 27, 27]),
                (50, [255, 129, 129]),
                (99, [255, 254, 254]),
            ],
        );
        // A hint at a quarter puts the halfway colour there. The
        // reference browser draws the curve as straight pieces, up to
        // two levels off it away from the hint.
        holds(
            "linear-gradient(to right, red, 25%, blue)",
            20,
            &[(24, [128, 0, 126]), (25, [126, 0, 128])],
        );
        holds(
            "linear-gradient(45deg, red, blue)",
            20,
            &[(0, [218, 0, 36]), (50, [127, 0, 127]), (99, [38, 0, 217])],
        );
        holds(
            "radial-gradient(circle 40px at 50px 20px, red, blue)",
            20,
            &[
                (0, [0, 0, 255]),
                (10, [3, 0, 251]),
                (24, [92, 0, 162]),
                (49, [250, 0, 4]),
            ],
        );
        holds(
            "repeating-linear-gradient(to right, red 0, blue 10px)",
            20,
            &[
                (0, [242, 0, 12]),
                (10, [242, 0, 12]),
                (24, [140, 0, 114]),
                (25, [115, 0, 140]),
            ],
        );
        holds(
            "linear-gradient(to right, red 20%, blue 20%)",
            20,
            &[(10, [255, 0, 0]), (19, [255, 0, 0]), (20, [0, 0, 255])],
        );
    }

    #[test]
    fn a_straight_gradient_is_rendered_a_pixel_thick() {
        let down = render(&gradient("linear-gradient(red, blue)"), 100.0, 40.0);
        assert_eq!((down.width, down.height), (1, 40));
        let across = render(&gradient("linear-gradient(-90deg, red, blue)"), 100.0, 40.0);
        assert_eq!((across.width, across.height), (100, 1));
        let corner = render(
            &gradient("linear-gradient(to top right, red, blue)"),
            100.0,
            40.0,
        );
        assert_eq!((corner.width, corner.height), (100, 40));
        // To a corner, the colour halfway is on the diagonal between the
        // corners beside it.
        assert!(near(over_white(&corner, 0, 0), over_white(&corner, 99, 39)));
        let huge = render(&gradient("radial-gradient(red, blue)"), 10_000.0, 10_000.0);
        assert!(u64::from(huge.width) * u64::from(huge.height) <= MAX_GRADIENT_PIXELS);
    }

    #[test]
    fn stops_are_fixed_up_as_images_3_says() {
        let at = |css: &str| {
            let GradientShape::Linear(_) = gradient(css).shape else {
                panic!("linear")
            };
            stops(&gradient(css).items, 100.0)
                .iter()
                .map(|s| s.0)
                .collect::<Vec<f32>>()
        };
        assert_eq!(at("linear-gradient(red, blue)"), [0.0, 100.0]);
        assert_eq!(
            at("linear-gradient(red, lime, blue, red)"),
            [0.0, 100.0 / 3.0, 200.0 / 3.0, 100.0]
        );
        assert_eq!(
            at("linear-gradient(red 40%, lime 20%, blue)"),
            [40.0, 40.0, 100.0]
        );
        assert_eq!(
            at("linear-gradient(red 10%, lime, blue 50%)"),
            [10.0, 30.0, 50.0]
        );
        // A hint is nine stops, as the reference browser makes them:
        // two on the shorter side, seven on the longer from the hint.
        let hinted = at("linear-gradient(red, 25%, blue)");
        let mut want = vec![0.0, 25.0 / 3.0, 50.0 / 3.0];
        want.extend((0..7).map(|y| 25.0 + 75.0 * y as f32 / 13.0));
        want.push(100.0);
        assert_eq!(hinted.len(), want.len());
        assert!(
            hinted.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-4),
            "{hinted:?}"
        );
        let long_left = at("linear-gradient(red, 75%, blue)");
        assert!(
            (long_left[1] - (75.0 * 7.0 / 13.0)).abs() < 1e-4,
            "{long_left:?}"
        );
        assert_eq!(
            at("linear-gradient(red, 50%, blue)"),
            [0.0, 100.0],
            "halfway: none"
        );
        // The hint on the stop after it: a stop of red there.
        assert_eq!(
            at("linear-gradient(red, 60%, lime 30%, blue)"),
            [0.0, 60.0, 60.0, 100.0]
        );
        assert_eq!(
            at("linear-gradient(red 10px 30px, blue)"),
            [10.0, 30.0, 100.0]
        );
    }

    #[test]
    fn degenerate_and_repeating_gradients_are_solid_where_images_3_says() {
        let solid = |css: &str| {
            let image = render(&gradient(css), 50.0, 20.0);
            let first = over_white(&image, 0, 0);
            let last = over_white(&image, image.width - 1, image.height - 1);
            (first, last)
        };
        // A period too fine to show, and none at all: the average.
        let (a, b) = solid("repeating-linear-gradient(to right, red 0, blue 0.2px)");
        assert_eq!(a, b);
        assert!(near(a, [128, 0, 128]), "{a:?}");
        let (a, _) = solid("repeating-linear-gradient(red 0px, white 0px, blue 0px)");
        assert!(near(a, [191, 128, 191]), "{a:?}");
        // An ellipse of no height: the last colour, or the average if it
        // repeats.
        let (a, b) = solid("radial-gradient(10px 0px, red, blue)");
        assert_eq!((a, b), ([0, 0, 255], [0, 0, 255]));
        let (a, _) = solid("repeating-radial-gradient(10px 0px, red, blue)");
        assert!(near(a, [128, 0, 128]), "{a:?}");
        // Of no width: mirrored across its centre, its percentages
        // nothing.
        let image = render(
            &gradient("radial-gradient(0px 10px at 25px 10px, red, blue 10px)"),
            50.0,
            20.0,
        );
        assert!(near(over_white(&image, 24, 3), over_white(&image, 25, 15)));
        assert_eq!(over_white(&image, 0, 10), [0, 0, 255]);
        // A circle of no radius: the last colour all round.
        let image = render(
            &gradient("radial-gradient(circle 0px, red, blue)"),
            50.0,
            20.0,
        );
        assert_eq!(over_white(&image, 0, 0), [0, 0, 255]);
    }

    #[test]
    fn radial_sizes_meet_the_sides_and_corners_they_name() {
        // A centre 20px from the left and the top of a 100 by 40 box:
        // 80px from the right, 20px from the bottom.
        let radii = |css: &str| match Geometry::of(&gradient(css), 100.0, 40.0).0 {
            Geometry::Radial { rx, ry, .. } => (rx, ry),
            Geometry::Linear { .. } => panic!("radial"),
        };
        let at = "at 20px 20px, red, blue)";
        assert_eq!(
            radii(&format!("radial-gradient(circle closest-side {at}")),
            (20.0, 20.0)
        );
        // The closest side down, nearer than either side across.
        assert_eq!(
            radii("radial-gradient(circle closest-side at 50px 10px, red, blue)"),
            (10.0, 10.0)
        );
        assert_eq!(
            radii(&format!("radial-gradient(circle farthest-side {at}")),
            (80.0, 80.0)
        );
        assert_eq!(
            radii(&format!("radial-gradient(closest-side {at}")),
            (20.0, 20.0)
        );
        assert_eq!(
            radii(&format!("radial-gradient(farthest-side {at}")),
            (80.0, 20.0)
        );
        let (r, _) = radii(&format!("radial-gradient(circle closest-corner {at}"));
        assert!((r - 800f32.sqrt()).abs() < 1e-3, "{r}");
        let (r, _) = radii(&format!("radial-gradient(circle farthest-corner {at}"));
        assert!((r - 6800f32.sqrt()).abs() < 1e-3, "{r}");
        // An ellipse through a corner keeps the sides' ratio: through
        // the far corner, (80, 20) away, at 80 by 20, scaled by the
        // square root of two.
        let (rx, ry) = radii(&format!("radial-gradient(farthest-corner {at}"));
        assert!((rx - 80.0 * 2f32.sqrt()).abs() < 1e-3 && (ry - 20.0 * 2f32.sqrt()).abs() < 1e-3);
        assert_eq!(
            radii(&format!("radial-gradient(10% 50% {at}")),
            (10.0, 20.0)
        );
    }

    #[test]
    fn a_linear_gradients_line_reaches_the_corners() {
        let length = |css: &str| Geometry::of(&gradient(css), 100.0, 40.0).1;
        assert_eq!(length("linear-gradient(to top, red, blue)"), 40.0);
        assert_eq!(length("linear-gradient(to right, red, blue)"), 100.0);
        let diagonal = length("linear-gradient(45deg, red, blue)");
        assert!(
            (diagonal - 140.0 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "{diagonal}"
        );
    }

    #[test]
    fn a_gradient_is_numbered_by_itself_and_its_size() {
        let g = gradient("linear-gradient(red, blue)");
        assert_eq!(number(&g, 10.0, 20.0), number(&g, 10.0, 20.0));
        assert_ne!(number(&g, 10.0, 20.0), number(&g, 10.0, 21.0));
        assert_ne!(
            number(&g, 10.0, 20.0),
            number(&gradient("linear-gradient(red, lime)"), 10.0, 20.0)
        );
        assert!(number(&g, 1.0, 1.0) >> 63 == 1);
    }
}
