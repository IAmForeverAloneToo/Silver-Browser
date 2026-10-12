//! SVG's transform list (css-transforms-1 section 7.1, M32): the grammar
//! of the `transform` attribute, and of `gradientTransform` and
//! `patternTransform`, which is not CSS's: numbers without units, angles
//! in degrees, commas or white space between the arguments, and
//! `rotate()` about a point. The cascade turns a `transform` attribute
//! into its property's `matrix()`; the `svg` crate reads the others.

/// A two-dimensional affine matrix, `[a, b, c, d, e, f]` as `matrix()`
/// writes it: a point `(x, y)` goes to `(a x + c y + e, b x + d y + f)`.
pub type Matrix2d = [f64; 6];

/// The identity.
pub const IDENTITY: Matrix2d = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m * n`: `n` applied first, then `m`, as a list's functions compose.
pub fn multiply(m: &Matrix2d, n: &Matrix2d) -> Matrix2d {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

/// The matrix of a transform list, its functions multiplied left to
/// right; the identity for a list of none. `None` when `text` does not
/// follow the grammar at any point, or the product is not finite: the
/// reference browser keeps nothing of a list with an error in it (a
/// unit, an empty argument, a comma after the last).
pub fn parse_transform_list(text: &str) -> Option<Matrix2d> {
    let mut c = Cursor {
        bytes: text.as_bytes(),
        at: 0,
    };
    let mut matrix = IDENTITY;
    c.skip_space();
    let mut first = true;
    while !c.done() {
        if !first {
            c.comma_space();
        }
        first = false;
        let next = c.function()?;
        matrix = multiply(&matrix, &next);
        c.skip_space();
    }
    matrix.iter().all(|v| v.is_finite()).then_some(matrix)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')) {
            self.at += 1;
        }
    }

    /// SVG's `comma-wsp`, optional: white space, a comma at most, white
    /// space.
    fn comma_space(&mut self) {
        self.skip_space();
        if self.peek() == Some(b',') {
            self.at += 1;
            self.skip_space();
        }
    }

    /// One transform function, as its matrix.
    fn function(&mut self) -> Option<Matrix2d> {
        let start = self.at;
        while self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
            self.at += 1;
        }
        let name = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
        self.skip_space();
        if self.peek() != Some(b'(') {
            return None;
        }
        self.at += 1;
        self.skip_space();
        let mut args = Vec::new();
        while self.peek() != Some(b')') {
            if !args.is_empty() {
                self.comma_space();
            }
            args.push(self.number()?);
            self.skip_space();
            if args.len() > 6 {
                return None;
            }
        }
        self.at += 1;
        let degrees = |a: f64| {
            // Quarter turns exact, so that `rotate(90)` is no 6e-17.
            let turn = a.rem_euclid(360.0);
            if turn == 0.0 {
                (1.0, 0.0)
            } else if turn == 90.0 {
                (0.0, 1.0)
            } else if turn == 180.0 {
                (-1.0, 0.0)
            } else if turn == 270.0 {
                (0.0, -1.0)
            } else {
                let r = a.to_radians();
                (r.cos(), r.sin())
            }
        };
        Some(match (name, args.as_slice()) {
            ("matrix", &[a, b, c, d, e, f]) => [a, b, c, d, e, f],
            ("translate", &[x]) => [1.0, 0.0, 0.0, 1.0, x, 0.0],
            ("translate", &[x, y]) => [1.0, 0.0, 0.0, 1.0, x, y],
            ("scale", &[s]) => [s, 0.0, 0.0, s, 0.0, 0.0],
            ("scale", &[x, y]) => [x, 0.0, 0.0, y, 0.0, 0.0],
            ("rotate", &[a]) => {
                let (cos, sin) = degrees(a);
                [cos, sin, -sin, cos, 0.0, 0.0]
            }
            ("rotate", &[a, x, y]) => {
                let (cos, sin) = degrees(a);
                let rotate = [cos, sin, -sin, cos, 0.0, 0.0];
                let to = [1.0, 0.0, 0.0, 1.0, x, y];
                let back = [1.0, 0.0, 0.0, 1.0, -x, -y];
                multiply(&multiply(&to, &rotate), &back)
            }
            ("skewX", &[a]) => [1.0, 0.0, a.to_radians().tan(), 1.0, 0.0, 0.0],
            ("skewY", &[a]) => [1.0, a.to_radians().tan(), 0.0, 1.0, 0.0, 0.0],
            _ => return None,
        })
    }

    /// SVG's `number`: a sign, digits with a fraction or a fraction, and
    /// an exponent; finite, or no number.
    fn number(&mut self) -> Option<f64> {
        let start = self.at;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let digits = |c: &mut Self| {
            let from = c.at;
            while c.peek().is_some_and(|b| b.is_ascii_digit()) {
                c.at += 1;
            }
            c.at - from
        };
        let mut seen = digits(self);
        if self.peek() == Some(b'.') {
            self.at += 1;
            seen += digits(self);
        }
        if seen == 0 {
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
        let text = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
        text.parse::<f64>().ok().filter(|v| v.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_is_its_functions_multiplied_left_to_right() {
        assert_eq!(
            parse_transform_list("translate(3 4)"),
            Some([1.0, 0.0, 0.0, 1.0, 3.0, 4.0])
        );
        assert_eq!(
            parse_transform_list("translate(10,20) scale(2)"),
            Some([2.0, 0.0, 0.0, 2.0, 10.0, 20.0])
        );
        // White space around and between, none before a function.
        assert_eq!(
            parse_transform_list("  translate(10 , 20)scale(2)  "),
            Some([2.0, 0.0, 0.0, 2.0, 10.0, 20.0])
        );
        assert_eq!(
            parse_transform_list("rotate(90)"),
            Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0])
        );
        assert_eq!(
            parse_transform_list("rotate(90 10 10)"),
            Some([0.0, 1.0, -1.0, 0.0, 20.0, 0.0])
        );
        assert_eq!(
            parse_transform_list("scale(2, 3)"),
            Some([2.0, 0.0, 0.0, 3.0, 0.0, 0.0])
        );
        assert_eq!(
            parse_transform_list("matrix(1 2 3 4 5 6)"),
            Some([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
        );
        let skew = parse_transform_list("skewX(45)").unwrap();
        assert!((skew[2] - 1.0).abs() < 1e-12 && skew[1] == 0.0);
        let skew = parse_transform_list("skewY(45)").unwrap();
        assert!((skew[1] - 1.0).abs() < 1e-12 && skew[2] == 0.0);
        assert_eq!(parse_transform_list(""), Some(IDENTITY));
        assert_eq!(
            parse_transform_list("translate(1e1 .5)"),
            Some([1.0, 0.0, 0.0, 1.0, 10.0, 0.5])
        );
        assert_eq!(
            parse_transform_list("translate(1,2),scale(2)"),
            Some([2.0, 0.0, 0.0, 2.0, 1.0, 2.0])
        );
    }

    #[test]
    fn an_error_anywhere_keeps_nothing() {
        for text in [
            "translate(3px 4px)",
            "scale(2) foo",
            "translate(1,,2)",
            "translate(1 2,)",
            "translate()",
            "rotate(1 2)",
            "matrix(1 2 3 4 5)",
            "translate(1 2",
            "Translate(1)",
            "translate(1)x",
            ",translate(1)",
            "translate(1e400)",
            "scale(1 2 3 4 5 6 7)",
        ] {
            assert_eq!(parse_transform_list(text), None, "{text}");
        }
    }
}
