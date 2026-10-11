//! SVG path data (SVG 2 section 9.3), as `new Path2D(d)` reads it: the
//! commands up to the first error, the rest left out, as SVG's error
//! handling says; an arc by its end points turned into an ellipse's arc
//! by SVG's implementation notes (B.2.4).

use std::f64::consts::TAU;

use crate::path::Path;
use crate::state::Matrix;

type Point = (f64, f64);

/// The reader of the data: where it is, and what the last command left.
struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn skip_space(&mut self) {
        while self
            .text
            .get(self.at)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c'))
        {
            self.at += 1;
        }
    }

    /// Space, then a comma and space after it if there is one.
    fn skip_separator(&mut self) {
        self.skip_space();
        if self.text.get(self.at) == Some(&b',') {
            self.at += 1;
            self.skip_space();
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    /// Whether a number starts here.
    fn number_next(&self) -> bool {
        matches!(self.peek(), Some(b'0'..=b'9' | b'-' | b'+' | b'.'))
    }

    /// A number, as SVG writes one, and the separator after it.
    fn number(&mut self) -> Option<f64> {
        let start = self.at;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let digits = |reader: &mut Reader<'_>| {
            let from = reader.at;
            while reader.peek().is_some_and(|c| c.is_ascii_digit()) {
                reader.at += 1;
            }
            reader.at - from
        };
        let mut count = digits(self);
        if self.peek() == Some(b'.') {
            self.at += 1;
            count += digits(self);
        }
        if count == 0 {
            self.at = start;
            return None;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.at;
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                self.at = mark;
            }
        }
        let value = std::str::from_utf8(&self.text[start..self.at])
            .ok()?
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())?;
        self.skip_separator();
        Some(value)
    }

    /// An arc's flag: a lone `0` or `1`, which needs nothing after it.
    fn flag(&mut self) -> Option<bool> {
        let flag = match self.peek()? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.at += 1;
        self.skip_separator();
        Some(flag)
    }

    fn numbers<const N: usize>(&mut self) -> Option<[f64; N]> {
        let mut out = [0.0; N];
        for value in &mut out {
            *value = self.number()?;
        }
        Some(out)
    }
}

/// `d` added to `path`, up to its first error.
pub(crate) fn parse(d: &str, path: &mut Path) {
    let m = Matrix::IDENTITY;
    let mut reader = Reader {
        text: d.as_bytes(),
        at: 0,
    };
    reader.skip_space();
    // The current point, the subpath's first, and the control point a
    // smooth curve reflects (with whether the last command was cubic).
    let mut current: Point = (0.0, 0.0);
    let mut first: Point = (0.0, 0.0);
    let mut control: Option<(Point, bool)> = None;
    let mut command: Option<u8> = None;
    loop {
        let letter = match reader.peek() {
            None => return,
            Some(c) if c.is_ascii_alphabetic() => {
                reader.at += 1;
                reader.skip_space();
                c
            }
            // Numbers after a command repeat it; after a move, as lines.
            Some(_) if reader.number_next() => match command {
                Some(b'M') => b'L',
                Some(b'm') => b'l',
                Some(c) if !matches!(c, b'Z' | b'z') => c,
                _ => return,
            },
            Some(_) => return,
        };
        if command.is_none() && !matches!(letter, b'M' | b'm') {
            return;
        }
        let relative = letter.is_ascii_lowercase();
        let base = if relative { current } else { (0.0, 0.0) };
        let at = |x: f64, y: f64| (base.0 + x, base.1 + y);
        let done = match letter.to_ascii_uppercase() {
            b'M' => reader.numbers::<2>().map(|[x, y]| {
                current = at(x, y);
                first = current;
                path.move_to(current.0, current.1, &m);
                control = None;
            }),
            b'L' => reader.numbers::<2>().map(|[x, y]| {
                current = at(x, y);
                path.line_to(current.0, current.1, &m);
                control = None;
            }),
            b'H' => reader.number().map(|x| {
                current = (base.0 + x, current.1);
                path.line_to(current.0, current.1, &m);
                control = None;
            }),
            b'V' => reader.number().map(|y| {
                current = (current.0, if relative { current.1 + y } else { y });
                path.line_to(current.0, current.1, &m);
                control = None;
            }),
            b'C' => reader.numbers::<6>().map(|[x1, y1, x2, y2, x, y]| {
                let (c1, c2, end) = (at(x1, y1), at(x2, y2), at(x, y));
                path.cubic_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1, &m);
                control = Some((c2, true));
                current = end;
            }),
            b'S' => reader.numbers::<4>().map(|[x2, y2, x, y]| {
                let c1 = match control {
                    Some((c, true)) => (2.0 * current.0 - c.0, 2.0 * current.1 - c.1),
                    _ => current,
                };
                let (c2, end) = (at(x2, y2), at(x, y));
                path.cubic_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1, &m);
                control = Some((c2, true));
                current = end;
            }),
            b'Q' => reader.numbers::<4>().map(|[x1, y1, x, y]| {
                let (c, end) = (at(x1, y1), at(x, y));
                path.quad_to(c.0, c.1, end.0, end.1, &m);
                control = Some((c, false));
                current = end;
            }),
            b'T' => reader.numbers::<2>().map(|[x, y]| {
                let c = match control {
                    Some((c, false)) => (2.0 * current.0 - c.0, 2.0 * current.1 - c.1),
                    _ => current,
                };
                let end = at(x, y);
                path.quad_to(c.0, c.1, end.0, end.1, &m);
                control = Some((c, false));
                current = end;
            }),
            b'A' => (|| {
                let [rx, ry, rotation] = reader.numbers::<3>()?;
                let large = reader.flag()?;
                let sweep = reader.flag()?;
                let [x, y] = reader.numbers::<2>()?;
                let end = at(x, y);
                arc(path, current, end, (rx, ry), rotation, large, sweep);
                current = end;
                control = None;
                Some(())
            })(),
            b'Z' => {
                path.close();
                current = first;
                control = None;
                reader.skip_space();
                Some(())
            }
            _ => None,
        };
        if done.is_none() {
            return;
        }
        command = Some(letter);
    }
}

/// An elliptical arc from `from` to `to` (SVG 2 section 9.5.1 and its
/// implementation notes): a line when a radius is 0, nothing when the
/// ends are one point, the radii scaled up when too small to reach.
fn arc(
    path: &mut Path,
    from: Point,
    to: Point,
    (rx, ry): (f64, f64),
    rotation: f64,
    large: bool,
    sweep: bool,
) {
    let m = Matrix::IDENTITY;
    if from == to {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 {
        path.line_to(to.0, to.1, &m);
        return;
    }
    let phi = rotation.to_radians();
    let (sin, cos) = phi.sin_cos();
    let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let x1 = cos * dx + sin * dy;
    let y1 = -sin * dx + cos * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = (numerator / denominator).max(0.0).sqrt();
    if large == sweep {
        coef = -coef;
    }
    let cx1 = coef * rx * y1 / ry;
    let cy1 = -coef * ry * x1 / rx;
    let cx = cos * cx1 - sin * cy1 + (from.0 + to.0) / 2.0;
    let cy = sin * cx1 + cos * cy1 + (from.1 + to.1) / 2.0;
    let angle = |ux: f64, uy: f64| uy.atan2(ux);
    let start = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut delta = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry) - start;
    if sweep && delta < 0.0 {
        delta += TAU;
    } else if !sweep && delta > 0.0 {
        delta -= TAU;
    }
    path.add_ellipse(cx, cy, rx, ry, phi, start, start + delta, !sweep, &m);
}
