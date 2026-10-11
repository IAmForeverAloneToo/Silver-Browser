//! The context's state (HTML 4.12.5.1.3), what `save()` keeps and
//! `restore()` gives back, and the names its enumerations go by.

use std::sync::{Arc, Mutex};

use tiny_skia::{BlendMode, Mask};

use crate::color::Rgba;
use crate::{CanvasError, MAX_DASHES};

/// An affine transform, as the 2D context and `DOMMatrix` write one:
/// a point (x, y) goes to (a x + c y + e, b x + d y + f).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Matrix {
        Matrix { a, b, c, d, e, f }
    }

    /// `self` after `other`: `other` applied to a point first, as the
    /// context's `transform()` multiplies.
    pub fn then(&self, other: &Matrix) -> Matrix {
        Matrix {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    pub fn is_finite(&self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f]
            .iter()
            .all(|v| v.is_finite())
    }

    pub fn invert(&self) -> Option<Matrix> {
        let det = self.a * self.d - self.b * self.c;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inverse = Matrix {
            a: self.d / det,
            b: -self.b / det,
            c: -self.c / det,
            d: self.a / det,
            e: (self.c * self.f - self.d * self.e) / det,
            f: (self.b * self.e - self.a * self.f) / det,
        };
        inverse.is_finite().then_some(inverse)
    }

    /// How much the transform scales a length at most, for the
    /// precision a curve is flattened to.
    pub fn scale(&self) -> f64 {
        let x = self.a.hypot(self.b);
        let y = self.c.hypot(self.d);
        x.max(y)
    }

    pub(crate) fn to_skia(self) -> tiny_skia::Transform {
        tiny_skia::Transform::from_row(
            self.a as f32,
            self.b as f32,
            self.c as f32,
            self.d as f32,
            self.e as f32,
            self.f as f32,
        )
    }
}

/// What a fill or a stroke paints with: `fillStyle` and `strokeStyle`.
#[derive(Debug, Clone)]
pub enum Paint {
    Color(Rgba),
    /// A `CanvasGradient`, shared with the script that holds it, whose
    /// stops it may still add to.
    Gradient(Arc<Mutex<Gradient>>),
    /// A `CanvasPattern`, whose transform the script may still set.
    Pattern(Arc<Mutex<Pattern>>),
}

/// A gradient's geometry, as the 2D context's three constructors give
/// it, in the coordinates the gradient is used in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientKind {
    Linear {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    },
    Radial {
        x0: f64,
        y0: f64,
        r0: f64,
        x1: f64,
        y1: f64,
        r1: f64,
    },
    Conic {
        angle: f64,
        x: f64,
        y: f64,
    },
}

/// A `CanvasGradient`: its geometry and its colour stops, in offset
/// order, those of one offset in the order they were added.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<(f64, Rgba)>,
}

impl Gradient {
    pub fn new(kind: GradientKind) -> Gradient {
        Gradient {
            kind,
            stops: Vec::new(),
        }
    }

    /// `addColorStop()`, the colour already parsed: an offset outside 0
    /// to 1 an `IndexSizeError`.
    pub fn add_stop(&mut self, offset: f64, color: Rgba) -> Result<(), CanvasError> {
        if !(0.0..=1.0).contains(&offset) {
            return Err(CanvasError::new(
                "IndexSizeError",
                format!(
                    "The provided value ({}) is outside the range (0.0, 1.0).",
                    number(offset)
                ),
            ));
        }
        let at = self.stops.partition_point(|&(o, _)| o <= offset);
        self.stops.insert(at, (offset, color));
        Ok(())
    }
}

/// How a pattern repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repetition {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

impl Repetition {
    /// The repetition `createPattern` names; an empty string or null is
    /// `repeat`.
    pub fn from_name(name: &str) -> Option<Repetition> {
        Some(match name {
            "" | "repeat" => Repetition::Repeat,
            "repeat-x" => Repetition::RepeatX,
            "repeat-y" => Repetition::RepeatY,
            "no-repeat" => Repetition::NoRepeat,
            _ => return None,
        })
    }
}

/// A `CanvasPattern`: an image's pixels, premultiplied RGBA, how they
/// repeat, the pattern's own transform, and whether the image was
/// origin-clean.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    pub repetition: Repetition,
    pub transform: Matrix,
    pub origin_clean: bool,
}

/// `lineCap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

impl LineCap {
    pub fn from_name(name: &str) -> Option<LineCap> {
        Some(match name {
            "butt" => LineCap::Butt,
            "round" => LineCap::Round,
            "square" => LineCap::Square,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            LineCap::Butt => "butt",
            LineCap::Round => "round",
            LineCap::Square => "square",
        }
    }
}

/// `lineJoin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

impl LineJoin {
    pub fn from_name(name: &str) -> Option<LineJoin> {
        Some(match name {
            "miter" => LineJoin::Miter,
            "round" => LineJoin::Round,
            "bevel" => LineJoin::Bevel,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            LineJoin::Miter => "miter",
            LineJoin::Round => "round",
            LineJoin::Bevel => "bevel",
        }
    }
}

/// `imageSmoothingQuality`: read back as it was set, every one drawn
/// with bilinear interpolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Low,
    Medium,
    High,
}

impl Quality {
    pub fn from_name(name: &str) -> Option<Quality> {
        Some(match name {
            "low" => Quality::Low,
            "medium" => Quality::Medium,
            "high" => Quality::High,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Quality::Low => "low",
            Quality::Medium => "medium",
            Quality::High => "high",
        }
    }
}

/// `globalCompositeOperation`: Compositing and Blending's operators and
/// blend modes, by their names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Composite {
    SourceOver,
    SourceIn,
    SourceOut,
    SourceAtop,
    DestinationOver,
    DestinationIn,
    DestinationOut,
    DestinationAtop,
    Lighter,
    Copy,
    Xor,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

const COMPOSITES: [(&str, Composite); 26] = [
    ("source-over", Composite::SourceOver),
    ("source-in", Composite::SourceIn),
    ("source-out", Composite::SourceOut),
    ("source-atop", Composite::SourceAtop),
    ("destination-over", Composite::DestinationOver),
    ("destination-in", Composite::DestinationIn),
    ("destination-out", Composite::DestinationOut),
    ("destination-atop", Composite::DestinationAtop),
    ("lighter", Composite::Lighter),
    ("copy", Composite::Copy),
    ("xor", Composite::Xor),
    ("multiply", Composite::Multiply),
    ("screen", Composite::Screen),
    ("overlay", Composite::Overlay),
    ("darken", Composite::Darken),
    ("lighten", Composite::Lighten),
    ("color-dodge", Composite::ColorDodge),
    ("color-burn", Composite::ColorBurn),
    ("hard-light", Composite::HardLight),
    ("soft-light", Composite::SoftLight),
    ("difference", Composite::Difference),
    ("exclusion", Composite::Exclusion),
    ("hue", Composite::Hue),
    ("saturation", Composite::Saturation),
    ("color", Composite::Color),
    ("luminosity", Composite::Luminosity),
];

impl Composite {
    pub fn from_name(name: &str) -> Option<Composite> {
        COMPOSITES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, composite)| composite)
    }

    pub fn name(self) -> &'static str {
        COMPOSITES
            .iter()
            .find(|(_, c)| *c == self)
            .map_or("source-over", |&(n, _)| n)
    }

    pub(crate) fn blend_mode(self) -> BlendMode {
        match self {
            Composite::SourceOver => BlendMode::SourceOver,
            Composite::SourceIn => BlendMode::SourceIn,
            Composite::SourceOut => BlendMode::SourceOut,
            Composite::SourceAtop => BlendMode::SourceAtop,
            Composite::DestinationOver => BlendMode::DestinationOver,
            Composite::DestinationIn => BlendMode::DestinationIn,
            Composite::DestinationOut => BlendMode::DestinationOut,
            Composite::DestinationAtop => BlendMode::DestinationAtop,
            Composite::Lighter => BlendMode::Plus,
            Composite::Copy => BlendMode::Source,
            Composite::Xor => BlendMode::Xor,
            Composite::Multiply => BlendMode::Multiply,
            Composite::Screen => BlendMode::Screen,
            Composite::Overlay => BlendMode::Overlay,
            Composite::Darken => BlendMode::Darken,
            Composite::Lighten => BlendMode::Lighten,
            Composite::ColorDodge => BlendMode::ColorDodge,
            Composite::ColorBurn => BlendMode::ColorBurn,
            Composite::HardLight => BlendMode::HardLight,
            Composite::SoftLight => BlendMode::SoftLight,
            Composite::Difference => BlendMode::Difference,
            Composite::Exclusion => BlendMode::Exclusion,
            Composite::Hue => BlendMode::Hue,
            Composite::Saturation => BlendMode::Saturation,
            Composite::Color => BlendMode::Color,
            Composite::Luminosity => BlendMode::Luminosity,
        }
    }

    /// Whether the operator changes the bitmap where the shape is not:
    /// drawn through a layer over the whole clip, as the standard says.
    pub(crate) fn unbounded(self) -> bool {
        matches!(
            self,
            Composite::Copy
                | Composite::SourceIn
                | Composite::SourceOut
                | Composite::DestinationIn
                | Composite::DestinationAtop
        )
    }

    /// Whether drawing with it can leave a pixel less than opaque on an
    /// opaque canvas.
    pub(crate) fn may_clear(self) -> bool {
        self.unbounded() || matches!(self, Composite::DestinationOut | Composite::Xor)
    }
}

/// The dash list and its offset.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dash {
    pub list: Vec<f64>,
    pub offset: f64,
}

impl Dash {
    /// `setLineDash()`: a value negative or not finite, or a list past
    /// [`MAX_DASHES`], changes nothing; an odd list is doubled.
    pub fn set(&mut self, list: &[f64]) {
        if list.len() > MAX_DASHES || list.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return;
        }
        self.list = list.to_vec();
        if list.len() % 2 == 1 {
            self.list.extend_from_slice(list);
        }
    }
}

/// The drawing state.
#[derive(Debug, Clone)]
pub struct State {
    pub transform: Matrix,
    pub fill: Paint,
    pub stroke: Paint,
    pub global_alpha: f64,
    pub composite: Composite,
    pub line_width: f64,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub miter_limit: f64,
    pub dash: Dash,
    pub shadow_offset_x: f64,
    pub shadow_offset_y: f64,
    pub shadow_blur: f64,
    pub shadow_color: Rgba,
    pub image_smoothing: bool,
    pub image_smoothing_quality: Quality,
    /// `filter`, as it was set: read back, not drawn (G81).
    pub filter: String,
    /// The clipping region, shared until a `clip()` changes it.
    pub(crate) clip: Option<Arc<Mask>>,
}

impl Default for State {
    fn default() -> State {
        State {
            transform: Matrix::IDENTITY,
            fill: Paint::Color(Rgba::BLACK),
            stroke: Paint::Color(Rgba::BLACK),
            global_alpha: 1.0,
            composite: Composite::SourceOver,
            line_width: 1.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: 10.0,
            dash: Dash::default(),
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            shadow_blur: 0.0,
            shadow_color: Rgba::TRANSPARENT,
            image_smoothing: true,
            image_smoothing_quality: Quality::Low,
            filter: "none".to_owned(),
            clip: None,
        }
    }
}

impl State {
    /// `lineWidth`: a value not finite or not positive changes nothing.
    pub fn set_line_width(&mut self, value: f64) {
        if value.is_finite() && value > 0.0 {
            self.line_width = value;
        }
    }

    /// `miterLimit`, likewise.
    pub fn set_miter_limit(&mut self, value: f64) {
        if value.is_finite() && value > 0.0 {
            self.miter_limit = value;
        }
    }

    /// `globalAlpha`: a value not finite or outside 0 to 1 changes
    /// nothing.
    pub fn set_global_alpha(&mut self, value: f64) {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            self.global_alpha = value;
        }
    }

    /// `shadowBlur`: a value not finite or negative changes nothing.
    pub fn set_shadow_blur(&mut self, value: f64) {
        if value.is_finite() && value >= 0.0 {
            self.shadow_blur = value;
        }
    }

    /// `shadowOffsetX` and `shadowOffsetY`: a value not finite changes
    /// nothing.
    pub fn set_shadow_offset(&mut self, x: Option<f64>, y: Option<f64>) {
        if let Some(x) = x.filter(|v| v.is_finite()) {
            self.shadow_offset_x = x;
        }
        if let Some(y) = y.filter(|v| v.is_finite()) {
            self.shadow_offset_y = y;
        }
    }

    /// `lineDashOffset`: a value not finite changes nothing.
    pub fn set_dash_offset(&mut self, value: f64) {
        if value.is_finite() {
            self.dash.offset = value;
        }
    }

    /// Whether a shadow is drawn: a colour not transparent, and a blur
    /// or an offset.
    pub(crate) fn casts_shadow(&self) -> bool {
        self.shadow_color.a != 0
            && (self.shadow_blur > 0.0
                || self.shadow_offset_x != 0.0
                || self.shadow_offset_y != 0.0)
    }
}

/// A number as JavaScript writes it, for the words of an error.
pub(crate) fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}
