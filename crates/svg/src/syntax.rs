//! SVG's microsyntaxes (SVG 2 sections 4.2, 8.5, 8.6 and 10.5): numbers,
//! lengths, number and point lists, `viewBox` and
//! `preserveAspectRatio`, and the matrix a `viewBox` gives. The
//! transform list is `style`'s (`style::svg_transform`), since the
//! cascade reads the `transform` attribute too.

/// White space as SVG's grammars have it.
pub(crate) fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

/// A cursor over an attribute's bytes.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(text: &'a str) -> Cursor<'a> {
        Cursor {
            bytes: text.as_bytes(),
            at: 0,
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    pub(crate) fn skip_space(&mut self) {
        while self.peek().is_some_and(is_space) {
            self.at += 1;
        }
    }

    /// `comma-wsp`, optional: white space, a comma at most, white space.
    /// Whether a comma was there.
    pub(crate) fn comma_space(&mut self) -> bool {
        self.skip_space();
        let comma = self.peek() == Some(b',');
        if comma {
            self.at += 1;
            self.skip_space();
        }
        comma
    }

    /// A `number`: a sign, digits with a fraction or a fraction, an
    /// exponent; finite, or none and the cursor where it was.
    pub(crate) fn number(&mut self) -> Option<f64> {
        let start = self.at;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let mut seen = self.digits();
        if self.peek() == Some(b'.') {
            self.at += 1;
            seen += self.digits();
        }
        if seen == 0 {
            self.at = start;
            return None;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.at;
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                self.at = mark;
            }
        }
        let value = std::str::from_utf8(&self.bytes[start..self.at])
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|v| v.is_finite());
        if value.is_none() {
            self.at = start;
        }
        value
    }

    fn digits(&mut self) -> usize {
        let from = self.at;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
        }
        self.at - from
    }

    /// The letters (and `%`) that follow, as a unit.
    fn unit(&mut self) -> &'a str {
        let from = self.at;
        if self.peek() == Some(b'%') {
            self.at += 1;
        } else {
            while self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
                self.at += 1;
            }
        }
        std::str::from_utf8(&self.bytes[from..self.at]).unwrap_or("")
    }
}

/// A `number` alone, white space around it.
pub fn parse_number(text: &str) -> Option<f64> {
    let mut c = Cursor::new(text);
    c.skip_space();
    let value = c.number()?;
    c.skip_space();
    c.done().then_some(value)
}

/// A length's unit (CSS Values 4's, which SVG 2's lengths are).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// None written: user units, which are pixels.
    None,
    /// `px`.
    Px,
    /// `em`: of the element's font size.
    Em,
    /// `ex`: half of it.
    Ex,
    /// `rem`: of the root element's.
    Rem,
    /// `%`: of the nearest viewport's width, height or normalized
    /// diagonal.
    Percent,
    /// `in`.
    In,
    /// `cm`.
    Cm,
    /// `mm`.
    Mm,
    /// `q`.
    Q,
    /// `pt`.
    Pt,
    /// `pc`.
    Pc,
    /// `vw`, of the viewport the drawing is in.
    Vw,
    /// `vh`.
    Vh,
    /// `vmin`.
    Vmin,
    /// `vmax`.
    Vmax,
}

/// A length as written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    /// The number.
    pub value: f64,
    /// Its unit.
    pub unit: Unit,
}

impl Length {
    /// Zero user units.
    pub const ZERO: Length = Length {
        value: 0.0,
        unit: Unit::None,
    };

    /// In user units, along `axis` of `viewport`.
    pub fn resolve(self, axis: Axis, viewport: &Viewport) -> f64 {
        let v = self.value;
        match self.unit {
            Unit::None | Unit::Px => v,
            Unit::Em => v * viewport.font_size,
            Unit::Ex => v * viewport.font_size / 2.0,
            Unit::Rem => v * viewport.root_font_size,
            Unit::Percent => v / 100.0 * viewport.along(axis),
            Unit::In => v * 96.0,
            Unit::Cm => v * 96.0 / 2.54,
            Unit::Mm => v * 96.0 / 25.4,
            Unit::Q => v * 96.0 / 101.6,
            Unit::Pt => v * 96.0 / 72.0,
            Unit::Pc => v * 16.0,
            Unit::Vw => v / 100.0 * viewport.width,
            Unit::Vh => v / 100.0 * viewport.height,
            Unit::Vmin => v / 100.0 * viewport.width.min(viewport.height),
            Unit::Vmax => v / 100.0 * viewport.width.max(viewport.height),
        }
    }
}

fn unit_of(name: &str) -> Option<Unit> {
    Some(match name.to_ascii_lowercase().as_str() {
        "" => Unit::None,
        "px" => Unit::Px,
        "em" => Unit::Em,
        "ex" => Unit::Ex,
        "rem" => Unit::Rem,
        "%" => Unit::Percent,
        "in" => Unit::In,
        "cm" => Unit::Cm,
        "mm" => Unit::Mm,
        "q" => Unit::Q,
        "pt" => Unit::Pt,
        "pc" => Unit::Pc,
        "vw" => Unit::Vw,
        "vh" => Unit::Vh,
        "vmin" => Unit::Vmin,
        "vmax" => Unit::Vmax,
        _ => return None,
    })
}

impl Cursor<'_> {
    /// A number and its unit.
    pub(crate) fn length(&mut self) -> Option<Length> {
        let start = self.at;
        let value = self.number()?;
        match unit_of(self.unit()) {
            Some(unit) => Some(Length { value, unit }),
            None => {
                self.at = start;
                None
            }
        }
    }
}

/// A length alone, white space around it.
pub fn parse_length(text: &str) -> Option<Length> {
    let mut c = Cursor::new(text);
    c.skip_space();
    let length = c.length()?;
    c.skip_space();
    c.done().then_some(length)
}

/// Which way a length goes, for its percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Of the viewport's width.
    X,
    /// Of its height.
    Y,
    /// Of its normalized diagonal, `sqrt((w^2 + h^2) / 2)`.
    Diagonal,
}

/// What a length is resolved against: the nearest viewport's size in
/// user units (its `viewBox`'s, when it has one), and the fonts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// The width.
    pub width: f64,
    /// The height.
    pub height: f64,
    /// The element's font size, in user units.
    pub font_size: f64,
    /// The root element's.
    pub root_font_size: f64,
}

impl Viewport {
    /// The basis of a percentage along `axis`.
    pub fn along(&self, axis: Axis) -> f64 {
        match axis {
            Axis::X => self.width,
            Axis::Y => self.height,
            Axis::Diagonal => ((self.width * self.width + self.height * self.height) / 2.0).sqrt(),
        }
    }
}

/// A list of numbers separated by commas or white space, up to its
/// first error (SVG 2 says a list in error is used as far as it is
/// right, as a path's data is).
pub fn parse_numbers(text: &str) -> Vec<f64> {
    let mut c = Cursor::new(text);
    let mut out = Vec::new();
    c.skip_space();
    while !c.done() {
        if !out.is_empty() {
            c.comma_space();
        }
        match c.number() {
            Some(v) => out.push(v),
            None => break,
        }
        c.skip_space();
    }
    out
}

/// A list of lengths separated by commas or white space (a text's `x`
/// and the rest), up to its first error.
pub fn parse_lengths(text: &str) -> Vec<Length> {
    let mut c = Cursor::new(text);
    let mut out = Vec::new();
    c.skip_space();
    while !c.done() {
        if !out.is_empty() {
            c.comma_space();
        }
        match c.length() {
            Some(length) => out.push(length),
            None => break,
        }
        c.skip_space();
    }
    out
}

/// A `points` attribute: its numbers in pairs, an odd one left over
/// dropped.
pub fn parse_points(text: &str) -> Vec<(f64, f64)> {
    parse_numbers(text)
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[x, y]| (x, y))
        .collect()
}

/// A `viewBox`: its rectangle in user units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewBox {
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
    /// The width, not negative.
    pub width: f64,
    /// The height, not negative.
    pub height: f64,
}

impl ViewBox {
    /// Whether it has an area; one of none disables rendering.
    pub fn has_area(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

/// `viewBox`: four numbers; `None` for anything else, or a negative
/// width or height, which are errors that leave the attribute unset.
pub fn parse_view_box(text: &str) -> Option<ViewBox> {
    let mut c = Cursor::new(text);
    c.skip_space();
    let mut n = [0.0; 4];
    for (i, slot) in n.iter_mut().enumerate() {
        if i > 0 {
            c.comma_space();
        }
        *slot = c.number()?;
    }
    c.skip_space();
    if !c.done() || n[2] < 0.0 || n[3] < 0.0 {
        return None;
    }
    Some(ViewBox {
        x: n[0],
        y: n[1],
        width: n[2],
        height: n[3],
    })
}

/// Where a `preserveAspectRatio` aligns along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// `Min`.
    Min,
    /// `Mid`.
    Mid,
    /// `Max`.
    Max,
}

/// `preserveAspectRatio`: the alignment along x and y, `None` for
/// `none` (stretched), and whether it slices rather than meets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AspectRatio {
    /// The alignment, or `None` for `none`.
    pub align: Option<(Align, Align)>,
    /// `slice`.
    pub slice: bool,
}

impl Default for AspectRatio {
    /// `xMidYMid meet`.
    fn default() -> Self {
        AspectRatio {
            align: Some((Align::Mid, Align::Mid)),
            slice: false,
        }
    }
}

/// `preserveAspectRatio`: `defer`, ignored, an alignment and `meet` or
/// `slice`, each case-sensitive; the default for anything else.
pub fn parse_preserve_aspect_ratio(text: &str) -> AspectRatio {
    let mut words = text
        .split(|c: char| c.is_ascii() && is_space(c as u8))
        .filter(|w| !w.is_empty());
    let mut word = words.next();
    if word == Some("defer") {
        word = words.next();
    }
    let align = match word {
        Some("none") => None,
        Some(w) if w.is_ascii() && w.len() == 8 && w.starts_with('x') && &w[4..5] == "Y" => {
            let axis = |part: &str| match part {
                "Min" => Some(Align::Min),
                "Mid" => Some(Align::Mid),
                "Max" => Some(Align::Max),
                _ => None,
            };
            match (axis(&w[1..4]), axis(&w[5..8])) {
                (Some(x), Some(y)) => Some((x, y)),
                _ => return AspectRatio::default(),
            }
        }
        _ => return AspectRatio::default(),
    };
    let slice = match words.next() {
        None | Some("meet") => false,
        Some("slice") => true,
        Some(_) => return AspectRatio::default(),
    };
    if words.next().is_some() {
        return AspectRatio::default();
    }
    AspectRatio { align, slice }
}

/// The matrix from a `viewBox`'s user units to a viewport `width` by
/// `height` at the origin (SVG 2 section 8.2): scaled to fit or fill by
/// `ratio`, or stretched for `none`, and aligned.
pub fn view_box_matrix(
    view_box: &ViewBox,
    ratio: &AspectRatio,
    width: f64,
    height: f64,
) -> [f64; 6] {
    let (mut sx, mut sy) = (width / view_box.width, height / view_box.height);
    let Some((ax, ay)) = ratio.align else {
        return [sx, 0.0, 0.0, sy, -view_box.x * sx, -view_box.y * sy];
    };
    let s = if ratio.slice { sx.max(sy) } else { sx.min(sy) };
    (sx, sy) = (s, s);
    let mut tx = -view_box.x * sx;
    let mut ty = -view_box.y * sy;
    let (extra_x, extra_y) = (width - view_box.width * sx, height - view_box.height * sy);
    match ax {
        Align::Min => {}
        Align::Mid => tx += extra_x / 2.0,
        Align::Max => tx += extra_x,
    }
    match ay {
        Align::Min => {}
        Align::Mid => ty += extra_y / 2.0,
        Align::Max => ty += extra_y,
    }
    [sx, 0.0, 0.0, sy, tx, ty]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_lengths_are_svgs() {
        assert_eq!(parse_number(" 1e1 "), Some(10.0));
        assert_eq!(parse_number("-.5"), Some(-0.5));
        assert_eq!(parse_number("1."), Some(1.0));
        assert_eq!(parse_number("1e"), None);
        assert_eq!(parse_number("."), None);
        assert_eq!(parse_number("1 2"), None);
        assert_eq!(parse_number("1e400"), None);
        assert_eq!(
            parse_length("2em"),
            Some(Length {
                value: 2.0,
                unit: Unit::Em
            })
        );
        assert_eq!(
            parse_length(" 50% "),
            Some(Length {
                value: 50.0,
                unit: Unit::Percent
            })
        );
        assert_eq!(parse_length("3PX").map(|l| l.unit), Some(Unit::Px));
        assert_eq!(parse_length("3 px"), None);
        assert_eq!(parse_length("3furlongs"), None);
        let viewport = Viewport {
            width: 300.0,
            height: 150.0,
            font_size: 16.0,
            root_font_size: 10.0,
        };
        let at = |text: &str, axis| parse_length(text).unwrap().resolve(axis, &viewport);
        assert_eq!(at("10%", Axis::X), 30.0);
        assert_eq!(at("10%", Axis::Y), 15.0);
        assert!((at("10%", Axis::Diagonal) - 23.717).abs() < 1e-3);
        assert_eq!(at("2em", Axis::X), 32.0);
        assert_eq!(at("1in", Axis::X), 96.0);
        assert_eq!(at("2rem", Axis::X), 20.0);
        assert_eq!(at("72pt", Axis::X), 96.0);
    }

    #[test]
    fn lists_are_used_up_to_their_first_error() {
        assert_eq!(parse_numbers("1,2 3  4"), [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(parse_numbers("1-2.5.5"), [1.0, -2.5, 0.5]);
        assert_eq!(parse_numbers("1,,2"), [1.0]);
        assert_eq!(parse_numbers("1 x 2"), [1.0]);
        assert_eq!(parse_points("0,0 10,0 10"), [(0.0, 0.0), (10.0, 0.0)]);
        assert!(parse_points("").is_empty());
    }

    #[test]
    fn a_view_box_maps_to_its_viewport_by_its_ratio() {
        assert_eq!(
            parse_view_box("0 0 100 50"),
            Some(ViewBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0
            })
        );
        assert_eq!(parse_view_box("0,0,1,1").map(|v| v.width), Some(1.0));
        assert_eq!(parse_view_box("0 0 -1 1"), None);
        assert_eq!(parse_view_box("0 0 1"), None);
        assert_eq!(parse_view_box("0 0 1 1 1"), None);
        assert!(!parse_view_box("0 0 0 0").unwrap().has_area());
        let vb = parse_view_box("0 0 100 50").unwrap();
        // Met in the middle: 200 by 200 shows it 200 by 100, 50 down.
        let meet = view_box_matrix(&vb, &AspectRatio::default(), 200.0, 200.0);
        assert_eq!(meet, [2.0, 0.0, 0.0, 2.0, 0.0, 50.0]);
        let slice = parse_preserve_aspect_ratio("xMinYMax slice");
        assert_eq!(
            view_box_matrix(&vb, &slice, 200.0, 200.0),
            [4.0, 0.0, 0.0, 4.0, 0.0, 0.0]
        );
        let none = parse_preserve_aspect_ratio("none");
        assert_eq!(
            view_box_matrix(&vb, &none, 200.0, 200.0),
            [2.0, 0.0, 0.0, 4.0, 0.0, 0.0]
        );
        // Wider than its ratio: centred across.
        assert_eq!(
            view_box_matrix(&vb, &AspectRatio::default(), 400.0, 100.0),
            [2.0, 0.0, 0.0, 2.0, 100.0, 0.0]
        );
        let max = parse_preserve_aspect_ratio("defer xMaxYMax");
        assert_eq!(
            view_box_matrix(&vb, &max, 200.0, 200.0),
            [2.0, 0.0, 0.0, 2.0, 0.0, 100.0]
        );
        let moved = parse_view_box("10 20 100 50").unwrap();
        assert_eq!(
            view_box_matrix(&moved, &AspectRatio::default(), 100.0, 50.0),
            [1.0, 0.0, 0.0, 1.0, -10.0, -20.0]
        );
        for bad in [
            "xmidymid",
            "xMidYMid bogus",
            "xMidYMid meet x",
            "",
            "xMedYMid",
        ] {
            assert_eq!(
                parse_preserve_aspect_ratio(bad),
                AspectRatio::default(),
                "{bad}"
            );
        }
    }
}
