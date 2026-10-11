//! Text (HTML 4.12.5.1.11): a string shaped by the `text` crate in the
//! engine's face, at the size the font says, each glyph's outline made
//! a path in the context's coordinates and filled or stroked as any
//! path is, so that text turns, scales, clips and takes a gradient as a
//! path does. Placed by `textAlign` and `textBaseline`, spaced by
//! `letterSpacing` and `wordSpacing`, squeezed to a `maxWidth`.

use text::{Curve, FontCollection};
use tiny_skia::FillRule;

use crate::Canvas;
use crate::path::Path;
use crate::state::{Matrix, TextAlign, TextBaseline};

/// `measureText()`'s answer: the advance, and the box the glyphs and
/// the font take, from the point the text is drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextMetrics {
    pub width: f64,
    pub actual_left: f64,
    pub actual_right: f64,
    pub actual_ascent: f64,
    pub actual_descent: f64,
    pub font_ascent: f64,
    pub font_descent: f64,
    pub alphabetic: f64,
    pub hanging: f64,
    pub ideographic: f64,
}

/// A glyph placed: its outline's curves, from the text's start on the
/// alphabetic baseline.
struct Placed {
    curves: Vec<Curve>,
    x: f64,
    y: f64,
}

/// A string laid out: its glyphs and its advance.
struct Laid {
    glyphs: Vec<Placed>,
    width: f64,
}

/// ASCII white space made spaces, as the standard's text preparation
/// does.
fn prepared(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\t' | '\n' | '\x0c' | '\r' => ' ',
            c => c,
        })
        .collect()
}

impl Canvas {
    /// `text` shaped at the font's size, spaced as the state says.
    fn lay_out(&self, fonts: &FontCollection, text: &str) -> Laid {
        let state = &self.state;
        let size = state.font.size as f32;
        let letter = state.letter_spacing.pixels(state.font.size);
        let word = state.word_spacing.pixels(state.font.size);
        let outline = |script, id| {
            let font = fonts.font_for(script).glyph_font();
            text::glyph_outline(font, id, size).unwrap_or_default()
        };
        let mut glyphs = Vec::new();
        if letter == 0.0 && word == 0.0 {
            for (script, glyph) in fonts.shape(text, size) {
                glyphs.push(Placed {
                    curves: outline(script, glyph.glyph_id),
                    x: f64::from(glyph.x),
                    y: f64::from(glyph.y),
                });
            }
            return Laid {
                glyphs,
                width: f64::from(fonts.measure(text, size)),
            };
        }
        // Spaced, each character shaped alone, as the reference browser
        // does without ligatures once letters are spaced.
        let mut pen = 0.0;
        let mut one = [0u8; 4];
        for c in text.chars() {
            let c_text = c.encode_utf8(&mut one);
            for (script, glyph) in fonts.shape(c_text, size) {
                glyphs.push(Placed {
                    curves: outline(script, glyph.glyph_id),
                    x: pen + f64::from(glyph.x),
                    y: f64::from(glyph.y),
                });
            }
            pen += f64::from(fonts.measure(c_text, size)) + letter;
            if c == ' ' {
                pen += word;
            }
        }
        Laid { glyphs, width: pen }
    }

    /// The font's ascent and descent at its size, and its em box's.
    fn font_box(&self, fonts: &FontCollection) -> (f64, f64, f64, f64) {
        let size = self.state.font.size;
        let metrics = fonts.primary().metrics(size as f32);
        let (ascent, descent) = (f64::from(metrics.ascent), f64::from(metrics.descent));
        let total = ascent + descent;
        let (em_ascent, em_descent) = if total > 0.0 {
            (size * ascent / total, size * descent / total)
        } else {
            (size, 0.0)
        };
        (ascent, descent, em_ascent, em_descent)
    }

    /// How far above the alphabetic baseline the baseline `textBaseline`
    /// names is.
    fn baseline_offset(&self, fonts: &FontCollection) -> f64 {
        let (ascent, descent, em_ascent, em_descent) = self.font_box(fonts);
        match self.state.text_baseline {
            TextBaseline::Alphabetic => 0.0,
            TextBaseline::Top => em_ascent,
            TextBaseline::Hanging => 0.8 * ascent,
            TextBaseline::Middle => (em_ascent - em_descent) / 2.0,
            TextBaseline::Ideographic => -descent,
            TextBaseline::Bottom => -em_descent,
        }
    }

    /// How far left of the point it is drawn at a text of `width` starts.
    fn align_offset(&self, width: f64, rtl: bool) -> f64 {
        let right = match self.state.text_align {
            TextAlign::Left => false,
            TextAlign::Right => true,
            TextAlign::Center => return width / 2.0,
            TextAlign::Start => rtl,
            TextAlign::End => !rtl,
        };
        if right { width } else { 0.0 }
    }

    /// `fillText()` or `strokeText()`: `rtl` the direction `inherit`
    /// takes from the element.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &mut self,
        fonts: &FontCollection,
        text: &str,
        x: f64,
        y: f64,
        max_width: Option<f64>,
        stroke: bool,
        rtl: bool,
    ) {
        if ![x, y].iter().all(|v| v.is_finite()) {
            return;
        }
        if let Some(max) = max_width
            && (max.is_nan() || max <= 0.0)
        {
            return;
        }
        let laid = self.lay_out(fonts, &prepared(text));
        let rtl = self.state.direction.is_rtl(rtl);
        // Squeezed horizontally to the maximum width, about its start.
        let squeeze = match max_width {
            Some(max) if laid.width > max && max.is_finite() => max / laid.width,
            _ => 1.0,
        };
        let start = x - self.align_offset(laid.width, rtl) * squeeze;
        let baseline = y + self.baseline_offset(fonts);
        let m = Matrix::new(squeeze, 0.0, 0.0, 1.0, start, baseline);
        let mut path = Path::new();
        for glyph in &laid.glyphs {
            let at = m.then(&Matrix::new(1.0, 0.0, 0.0, 1.0, glyph.x, glyph.y));
            add_outline(&mut path, &glyph.curves, &at);
        }
        self.charge(text.len() as u64);
        if stroke {
            self.stroke(Some(&path));
        } else {
            self.fill(Some(&path), FillRule::Winding);
        }
    }

    /// `measureText()`.
    pub fn measure_text(&self, fonts: &FontCollection, text: &str, rtl: bool) -> TextMetrics {
        let laid = self.lay_out(fonts, &prepared(text));
        let rtl = self.state.direction.is_rtl(rtl);
        let offset = self.baseline_offset(fonts);
        let (ascent, descent, _, _) = self.font_box(fonts);
        let start = -self.align_offset(laid.width, rtl);
        // The glyphs' bounds, from their outlines' points.
        let (mut left, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut top, mut bottom) = (f64::INFINITY, f64::NEG_INFINITY);
        for glyph in &laid.glyphs {
            for curve in &glyph.curves {
                for (px, py) in points(curve) {
                    let (gx, gy) = (start + glyph.x + f64::from(px), glyph.y + f64::from(py));
                    left = left.min(gx);
                    right = right.max(gx);
                    top = top.min(gy);
                    bottom = bottom.max(gy);
                }
            }
        }
        let inked = left.is_finite();
        TextMetrics {
            width: laid.width,
            actual_left: if inked { -left } else { 0.0 },
            actual_right: if inked { right } else { 0.0 },
            actual_ascent: if inked { -top - offset } else { -offset },
            actual_descent: if inked { bottom + offset } else { offset },
            font_ascent: ascent - offset,
            font_descent: descent + offset,
            alphabetic: -offset,
            hanging: 0.8 * ascent - offset,
            ideographic: -descent - offset,
        }
    }
}

/// The points of a curve, its control points among them.
fn points(curve: &Curve) -> Vec<(f32, f32)> {
    match *curve {
        Curve::Line(a, b) => vec![a, b],
        Curve::Quad(a, b, c) => vec![a, b, c],
        Curve::Cubic(a, b, c, d) => vec![a, b, c, d],
    }
}

/// A glyph's outline added to `path` through `m`: a curve that starts
/// away from where the last one ended begins a contour, each contour
/// closed.
fn add_outline(path: &mut Path, curves: &[Curve], m: &Matrix) {
    let mut at: Option<(f32, f32)> = None;
    for curve in curves {
        let start = points(curve)[0];
        if at != Some(start) {
            if at.is_some() {
                path.close();
            }
            path.move_to(f64::from(start.0), f64::from(start.1), m);
        }
        let f = |p: (f32, f32)| (f64::from(p.0), f64::from(p.1));
        match *curve {
            Curve::Line(_, b) => {
                let e = f(b);
                path.line_to(e.0, e.1, m);
                at = Some(b);
            }
            Curve::Quad(_, c, b) => {
                let (c, e) = (f(c), f(b));
                path.quad_to(c.0, c.1, e.0, e.1, m);
                at = Some(b);
            }
            Curve::Cubic(_, c1, c2, b) => {
                let (c1, c2, e) = (f(c1), f(c2), f(b));
                path.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1, m);
                at = Some(b);
            }
        }
    }
    if at.is_some() {
        path.close();
    }
}
