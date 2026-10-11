//! Drawing (HTML 4.12.5.1.13 to .15 and .20): a shape filled with the
//! paint, its shadow first when there is one, composited by the current
//! operator within the clipping region. A shape is drawn straight onto
//! the bitmap when the operator leaves alone what the shape does not
//! cover; through a layer the size of the shape when it does not, the
//! rest of the clipping region then cleared, as compositing transparent
//! black by such an operator does.

use std::sync::Arc;

use tiny_skia::{
    BlendMode, FillRule, FilterQuality, GradientStop, LinearGradient, Mask, PathBuilder, Pixmap,
    PixmapPaint, PixmapRef, Point, RadialGradient, Rect, Shader, SpreadMode, StrokeDash,
    SweepGradient, Transform,
};

use crate::blur::blur;
use crate::path::Path;
use crate::reach::within;
use crate::state::{Gradient, GradientKind, LineCap, LineJoin, Matrix, Paint, Repetition};
use crate::{Canvas, MAX_SHADOW_BLUR};

/// What a shape is painted with: either style, or the image a
/// `drawImage()` draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Which {
    Fill,
    Stroke,
    Image,
}

const TRANSPARENT: Shader<'static> = Shader::SolidColor(tiny_skia::Color::TRANSPARENT);

/// The farthest from the bitmap a layer is made, in pixels.
const FAR: f64 = 1e7;

/// The finest a stroke's outline and dashes are traced, as a scale of
/// the user's space: past it a curve under a vast scale is traced as at
/// this one, as the bitmap's single-precision coordinates hold less
/// there anyway. Traced to a billionth of a unit, a circle of radius 7
/// made millions of points and took a second a stroke (found by
/// fuzzing).
const MAX_STROKE_RESOLUTION: f64 = 65_536.0;

/// A rectangle of whole pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Area {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Area {
    /// The pixels the rectangle from (l, t) to (r, b) touches; `None`
    /// when it is empty or too far out.
    pub(crate) fn new(l: f64, t: f64, r: f64, b: f64) -> Option<Area> {
        let (l, t, r, b) = (l.floor(), t.floor(), r.ceil(), b.ceil());
        let near = |v: f64| v.abs() <= FAR;
        if !(near(l) && near(t) && near(r) && near(b)) || r <= l || b <= t {
            return None;
        }
        Some(Area {
            x: l as i32,
            y: t as i32,
            w: (r - l) as u32,
            h: (b - t) as u32,
        })
    }
}

/// A gradient as a shader, its colours faded by `alpha`, in the
/// coordinates `m` gives: nothing painted for a degenerate geometry or
/// no stops, one stop's colour everywhere, as the standard says.
fn gradient_shader(gradient: &Gradient, alpha: f64, m: &Matrix) -> Shader<'static> {
    let degenerate = match gradient.kind {
        GradientKind::Linear { x0, y0, x1, y1 } => x0 == x1 && y0 == y1,
        GradientKind::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
        } => x0 == x1 && y0 == y1 && r0 == r1,
        GradientKind::Conic { .. } => false,
    };
    if degenerate || gradient.stops.is_empty() {
        return TRANSPARENT;
    }
    let stops: Vec<GradientStop> = gradient
        .stops
        .iter()
        .map(|&(offset, color)| GradientStop::new(offset as f32, color.faded(alpha).to_skia()))
        .collect();
    let point = |x: f64, y: f64| Point::from_xy(x as f32, y as f32);
    let shader = match gradient.kind {
        GradientKind::Linear { x0, y0, x1, y1 } => LinearGradient::new(
            point(x0, y0),
            point(x1, y1),
            stops,
            SpreadMode::Pad,
            m.to_skia(),
        ),
        GradientKind::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
        } => RadialGradient::new(
            point(x0, y0),
            r0 as f32,
            point(x1, y1),
            r1 as f32,
            stops,
            SpreadMode::Pad,
            m.to_skia(),
        ),
        GradientKind::Conic { angle, x, y } => {
            let (sin, cos) = angle.sin_cos();
            let at = m.then(&Matrix::new(cos, sin, -sin, cos, x, y));
            SweepGradient::new(
                point(0.0, 0.0),
                0.0,
                360.0,
                stops,
                SpreadMode::Pad,
                at.to_skia(),
            )
        }
    };
    shader.unwrap_or(TRANSPARENT)
}

impl Canvas {
    /// `fill()`: the current default path, or a `Path2D` transformed by
    /// the current transform, by `rule`.
    pub fn fill(&mut self, path: Option<&Path>, rule: FillRule) {
        let device = path.map(|path| path.transformed(&self.state.transform));
        let skia = device.as_ref().unwrap_or(&self.path).to_skia();
        if let Some(skia) = skia {
            self.paint_path(&skia, rule, Which::Fill);
        }
    }

    /// `stroke()`: the path traced with the line styles in the current
    /// transform's space.
    pub fn stroke(&mut self, path: Option<&Path>) {
        let user = match path {
            Some(path) => path.clone(),
            None => match self.state.transform.invert() {
                Some(inverse) => self.path.transformed(&inverse),
                None => return,
            },
        };
        if let Some(outline) = self.stroke_outline(&user) {
            self.paint_path(&outline, FillRule::Winding, Which::Stroke);
        }
    }

    /// The outline of `user`'s stroke, in the bitmap's coordinates: of
    /// the path with its pieces of no length pruned.
    pub(crate) fn stroke_outline(&self, user: &Path) -> Option<tiny_skia::Path> {
        let state = &self.state;
        state.transform.invert()?;
        let mut path = user.pruned().to_skia()?;
        // Traced to a device pixel's precision in the user's space, but no
        // finer than [`MAX_STROKE_RESOLUTION`].
        let resolution = state.transform.scale().clamp(1.0, MAX_STROKE_RESOLUTION) as f32;
        if !state.dash.list.is_empty() && state.dash.list.iter().sum::<f64>() > 0.0 {
            let list: Vec<f32> = state.dash.list.iter().map(|&v| v as f32).collect();
            let dash = StrokeDash::new(list, state.dash.offset as f32)?;
            path = path.dash(&dash, resolution)?;
        }
        let stroke = tiny_skia::Stroke {
            width: state.line_width as f32,
            miter_limit: state.miter_limit as f32,
            line_cap: match state.line_cap {
                LineCap::Butt => tiny_skia::LineCap::Butt,
                LineCap::Round => tiny_skia::LineCap::Round,
                LineCap::Square => tiny_skia::LineCap::Square,
            },
            line_join: match state.line_join {
                LineJoin::Miter => tiny_skia::LineJoin::Miter,
                LineJoin::Round => tiny_skia::LineJoin::Round,
                LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
            },
            dash: None,
        };
        path.stroke(&stroke, resolution)?
            .transform(state.transform.to_skia())
    }

    /// `fillRect()`.
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut rect = Path::new();
        rect.rect(x, y, w, h, &self.state.transform);
        if let Some(skia) = rect.to_skia() {
            self.paint_path(&skia, FillRule::Winding, Which::Fill);
        }
    }

    /// `strokeRect()`: nothing for an empty rectangle; for one of no
    /// width or no height, a line closed back on itself, which joins at
    /// both ends and has no caps, as the standard's subpath of two points
    /// is closed.
    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if ![x, y, w, h].iter().all(|v| v.is_finite()) || (w == 0.0 && h == 0.0) {
            return;
        }
        let mut user = Path::new();
        if w == 0.0 || h == 0.0 {
            user.move_to(x, y, &Matrix::IDENTITY);
            user.line_to(x + w, y + h, &Matrix::IDENTITY);
            user.close();
        } else {
            user.rect(x, y, w, h, &Matrix::IDENTITY);
        }
        if let Some(outline) = self.stroke_outline(&user) {
            self.paint_path(&outline, FillRule::Winding, Which::Stroke);
        }
    }

    /// `clearRect()`: the pixels within the clip made transparent black,
    /// untouched by the styles, the alpha, the operator and shadows.
    pub fn clear_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut rect = Path::new();
        rect.rect(x, y, w, h, &self.state.transform);
        // A rectangle of a value not finite clears nothing: the bitmap
        // is taken only once there is one.
        let Some(skia) = rect.to_skia() else {
            return;
        };
        let Some(mut pixmap) = self.pixmap.take() else {
            return;
        };
        self.charge_bounds(&skia);
        if let Some(skia) = within(&skia, self.bitmap_area()) {
            clear(
                &mut pixmap,
                &skia,
                FillRule::Winding,
                true,
                self.state.clip.as_deref(),
            );
        }
        self.pixmap = Some(pixmap);
        if self.opaque {
            self.keep_opaque();
        }
        self.changed();
    }

    /// `clip()`: the clipping region intersected with the path.
    pub fn clip(&mut self, path: Option<&Path>, rule: FillRule) {
        let device = path.map(|path| path.transformed(&self.state.transform));
        let skia = device.as_ref().unwrap_or(&self.path).to_skia();
        let skia = skia
            .as_ref()
            .and_then(|skia| within(skia, self.bitmap_area()));
        let (width, height) = (self.width, self.height);
        if self.pixmap.is_none() {
            return;
        }
        match (&mut self.state.clip, skia) {
            (Some(mask), Some(skia)) => {
                Arc::make_mut(mask).intersect_path(&skia, rule, true, Transform::identity());
            }
            (Some(mask), None) => Arc::make_mut(mask).clear(),
            (clip @ None, skia) => {
                let Some(mut mask) = Mask::new(width, height) else {
                    return;
                };
                if let Some(skia) = skia {
                    mask.fill_path(&skia, rule, true, Transform::identity());
                }
                *clip = Some(Arc::new(mask));
            }
        }
    }

    /// `isPointInPath()`: (x, y) in the bitmap's coordinates, the
    /// current transform not applied to it.
    pub fn is_point_in_path(&self, path: Option<&Path>, x: f64, y: f64, rule: FillRule) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        match path {
            Some(path) => path.transformed(&self.state.transform).contains(x, y, rule),
            None => self.path.contains(x, y, rule),
        }
    }

    /// `isPointInStroke()`.
    pub fn is_point_in_stroke(&self, path: Option<&Path>, x: f64, y: f64) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        let user = match path {
            Some(path) => path.clone(),
            None => match self.state.transform.invert() {
                Some(inverse) => self.path.transformed(&inverse),
                None => return false,
            },
        };
        self.stroke_outline(&user)
            .is_some_and(|outline| Path::from_skia(&outline).contains(x, y, FillRule::Winding))
    }

    /// The work of drawing a shape of these bounds, charged: a unit for
    /// sixteen pixels within the bitmap, and one for each point.
    fn charge_bounds(&mut self, path: &tiny_skia::Path) {
        let bounds = path.bounds();
        let area = Area::new(
            f64::from(bounds.left()).max(0.0),
            f64::from(bounds.top()).max(0.0),
            f64::from(bounds.right()).min(f64::from(self.width)),
            f64::from(bounds.bottom()).min(f64::from(self.height)),
        );
        let pixels = area.map_or(0, |a| u64::from(a.w) * u64::from(a.h));
        self.charge(pixels / 16 + path.len() as u64);
    }

    /// `path`, in the bitmap's coordinates, painted with the style of
    /// `which` as the state says: its shadow, then itself.
    pub(crate) fn paint_path(&mut self, path: &tiny_skia::Path, rule: FillRule, which: Which) {
        let Some(mut pixmap) = self.pixmap.take() else {
            return;
        };
        // A pattern of an image another origin did not let it read
        // leaves the bitmap no longer clean (HTML 4.12.5.1.8).
        if self.pattern_taints(which) {
            self.taint();
        }
        self.charge_bounds(path);
        let bounds = path.bounds();
        // One pixel round the bounds for the smoothing of the edges.
        let shape = (
            f64::from(bounds.left()) - 1.0,
            f64::from(bounds.top()) - 1.0,
            f64::from(bounds.right()) + 1.0,
            f64::from(bounds.bottom()) + 1.0,
        );
        if self.state.casts_shadow() {
            self.cast_shadow(&mut pixmap, path, rule, which, shape);
        }
        let composite = self.state.composite;
        if composite.unbounded() {
            let (w, h) = (f64::from(self.width), f64::from(self.height));
            let area = Area::new(
                shape.0.max(0.0),
                shape.1.max(0.0),
                shape.2.min(w),
                shape.3.min(h),
            );
            let layer = area.and_then(|area| Some((self.render(path, rule, which, area)?, area)));
            match &layer {
                Some((layer, area)) => {
                    self.composite(&mut pixmap, layer, f64::from(area.x), f64::from(area.y))
                }
                None => self.clear_outside(&mut pixmap, None),
            }
        } else {
            let band = self.pattern_band(which, self.bitmap_area());
            let clip = match (band, self.state.clip.as_deref()) {
                (Some(mut band), Some(clip)) => {
                    intersect(&mut band, clip);
                    Some(band)
                }
                (Some(band), None) => Some(band),
                (None, Some(clip)) => Some(clip.clone()),
                (None, None) => None,
            };
            if let Some(path) = within(path, self.bitmap_area()) {
                self.with_shader(which, |shader| {
                    let paint = tiny_skia::Paint {
                        shader,
                        blend_mode: composite.blend_mode(),
                        anti_alias: true,
                        ..tiny_skia::Paint::default()
                    };
                    pixmap.fill_path(&path, &paint, rule, Transform::identity(), clip.as_ref());
                });
            }
        }
        self.pixmap = Some(pixmap);
        if self.opaque && composite.may_clear() {
            self.keep_opaque();
        }
        self.changed();
    }

    fn bitmap_area(&self) -> Area {
        Area {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
        }
    }

    /// The shape's shadow (HTML 4.12.5.1.18) composited onto the bitmap:
    /// the shape's coverage blurred, coloured with the shadow's colour,
    /// moved by the offset. Only the part of the shape whose shadow can
    /// land on the bitmap is drawn for it, so the layer is never much
    /// larger than the bitmap however far the offset reaches.
    fn cast_shadow(
        &self,
        pixmap: &mut Pixmap,
        path: &tiny_skia::Path,
        rule: FillRule,
        which: Which,
        shape: (f64, f64, f64, f64),
    ) {
        let state = &self.state;
        let sigma = state.shadow_blur.min(MAX_SHADOW_BLUR) / 2.0;
        let spread = (3.0 * sigma).ceil() + 1.0;
        let (ox, oy) = (state.shadow_offset_x, state.shadow_offset_y);
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let Some(area) = Area::new(
            (shape.0 - spread).max(-ox - spread),
            (shape.1 - spread).max(-oy - spread),
            (shape.2 + spread).min(w - ox + spread),
            (shape.3 + spread).min(h - oy + spread),
        ) else {
            return;
        };
        let Some(layer) = self.render(path, rule, which, area) else {
            return;
        };
        let alpha: Vec<u8> = layer
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3])
            .collect();
        let alpha = blur(&alpha, area.w as usize, area.h as usize, sigma);
        let Some(mut cast) = Pixmap::new(area.w, area.h) else {
            return;
        };
        let color = state.shadow_color;
        for (out, &a) in cast
            .data_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&alpha)
        {
            let sa = (u32::from(color.a) * u32::from(a) + 127) / 255;
            let channel = |c: u8| ((u32::from(c) * sa + 127) / 255) as u8;
            *out = [
                channel(color.r),
                channel(color.g),
                channel(color.b),
                sa as u8,
            ];
        }
        self.composite(
            pixmap,
            &cast,
            f64::from(area.x) + ox,
            f64::from(area.y) + oy,
        );
    }

    /// The shape drawn alone into a layer covering `area`.
    fn render(
        &self,
        path: &tiny_skia::Path,
        rule: FillRule,
        which: Which,
        area: Area,
    ) -> Option<Pixmap> {
        let mut layer = Pixmap::new(area.w, area.h)?;
        let path = within(path, area)?;
        let shift = Transform::from_translate(-area.x as f32, -area.y as f32);
        let band = self.pattern_band(which, area);
        self.with_shader(which, |shader| {
            let paint = tiny_skia::Paint {
                shader,
                anti_alias: true,
                ..tiny_skia::Paint::default()
            };
            layer.fill_path(&path, &paint, rule, shift, band.as_ref());
        });
        Some(layer)
    }

    /// `layer`, its corner at (x, y), composited onto the bitmap by the
    /// current operator within the clip; the rest of the clip cleared
    /// when the operator clears where there is nothing.
    fn composite(&self, pixmap: &mut Pixmap, layer: &Pixmap, x: f64, y: f64) {
        let clip = self.state.clip.as_deref();
        let whole = x.fract() == 0.0 && y.fract() == 0.0;
        let paint = PixmapPaint {
            opacity: 1.0,
            blend_mode: self.state.composite.blend_mode(),
            quality: if whole {
                FilterQuality::Nearest
            } else {
                FilterQuality::Bilinear
            },
        };
        if whole {
            let identity = Transform::identity();
            pixmap.draw_pixmap(x as i32, y as i32, layer.as_ref(), &paint, identity, clip);
        } else {
            let at = Transform::from_translate(x as f32, y as f32);
            pixmap.draw_pixmap(0, 0, layer.as_ref(), &paint, at, clip);
        }
        if self.state.composite.unbounded() {
            let (w, h) = (f64::from(layer.width()), f64::from(layer.height()));
            let keep = Rect::from_ltrb(x as f32, y as f32, (x + w) as f32, (y + h) as f32);
            self.clear_outside(pixmap, keep);
        }
    }

    /// The clipping region cleared but for `keep`.
    fn clear_outside(&self, pixmap: &mut Pixmap, keep: Option<Rect>) {
        let mut builder = PathBuilder::new();
        if let Some(all) = Rect::from_xywh(0.0, 0.0, self.width as f32, self.height as f32) {
            builder.push_rect(all);
        }
        if let Some(keep) = keep.and_then(|keep| keep.round_out()).and_then(|keep| {
            Rect::from_xywh(
                keep.x() as f32,
                keep.y() as f32,
                keep.width() as f32,
                keep.height() as f32,
            )
        }) {
            builder.push_rect(keep);
        }
        if let Some(path) = builder.finish() {
            let clip = self.state.clip.as_deref();
            clear(pixmap, &path, FillRule::EvenOdd, false, clip);
        }
    }

    /// Whether the style `which` paints with is a pattern of an image
    /// that is not origin-clean.
    fn pattern_taints(&self, which: Which) -> bool {
        let paint = match which {
            Which::Fill => &self.state.fill,
            Which::Stroke => &self.state.stroke,
            Which::Image => return false,
        };
        match paint {
            Paint::Pattern(pattern) => {
                !pattern
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .origin_clean
            }
            _ => false,
        }
    }

    /// `f` given the shader the style of `which` paints with, faded by
    /// the global alpha.
    pub(crate) fn with_shader(&self, which: Which, f: impl FnOnce(Shader<'_>)) {
        let state = &self.state;
        let quality = if state.image_smoothing {
            FilterQuality::Bilinear
        } else {
            FilterQuality::Nearest
        };
        let paint = match which {
            Which::Fill => &state.fill,
            Which::Stroke => &state.stroke,
            // An image drawn: its edges clamped, not repeated.
            Which::Image => {
                let Some(image) = &self.drawing else {
                    return f(TRANSPARENT);
                };
                let Some(pixels) = PixmapRef::from_bytes(&image.pixels, image.width, image.height)
                else {
                    return f(TRANSPARENT);
                };
                let at = state.transform.then(&image.transform);
                return f(tiny_skia::Pattern::new(
                    pixels,
                    SpreadMode::Pad,
                    quality,
                    state.global_alpha as f32,
                    at.to_skia(),
                ));
            }
        };
        let alpha = state.global_alpha;
        match paint {
            Paint::Color(color) => f(Shader::SolidColor(color.faded(alpha).to_skia())),
            Paint::Gradient(gradient) => {
                let gradient = gradient.lock().unwrap_or_else(|e| e.into_inner()).clone();
                f(gradient_shader(&gradient, alpha, &state.transform));
            }
            Paint::Pattern(pattern) => {
                let pattern = pattern.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let Some(image) =
                    PixmapRef::from_bytes(&pattern.pixels, pattern.width, pattern.height)
                else {
                    return f(TRANSPARENT);
                };
                let at = state.transform.then(&pattern.transform);
                f(tiny_skia::Pattern::new(
                    image,
                    SpreadMode::Repeat,
                    quality,
                    alpha as f32,
                    at.to_skia(),
                ))
            }
        }
    }

    /// For a pattern that does not repeat both ways, the band its tiles
    /// cover within `area`, as a mask of `area`'s size.
    fn pattern_band(&self, which: Which, area: Area) -> Option<Mask> {
        let paint = match which {
            Which::Fill => &self.state.fill,
            Which::Stroke => &self.state.stroke,
            Which::Image => return None,
        };
        let Paint::Pattern(pattern) = paint else {
            return None;
        };
        let pattern = pattern.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (w, h) = (f64::from(pattern.width), f64::from(pattern.height));
        let (x0, x1, y0, y1) = match pattern.repetition {
            Repetition::Repeat => return None,
            Repetition::RepeatX => (-FAR, FAR, 0.0, h),
            Repetition::RepeatY => (0.0, w, -FAR, FAR),
            Repetition::NoRepeat => (0.0, w, 0.0, h),
        };
        let at = Matrix::new(1.0, 0.0, 0.0, 1.0, -f64::from(area.x), -f64::from(area.y))
            .then(&self.state.transform)
            .then(&pattern.transform);
        let mut band = Path::new();
        band.rect(x0, y0, x1 - x0, y1 - y0, &at);
        let mut mask = Mask::new(area.w, area.h)?;
        let whole = Area { x: 0, y: 0, ..area };
        if let Some(skia) = band.to_skia().as_ref().and_then(|skia| within(skia, whole)) {
            mask.fill_path(&skia, FillRule::Winding, true, Transform::identity());
        }
        Some(mask)
    }

    /// Every pixel's alpha made opaque again, on a canvas made with
    /// `alpha: false`: what was cleared shows black.
    pub(crate) fn keep_opaque(&mut self) {
        if let Some(pixmap) = &mut self.pixmap {
            for pixel in pixmap.data_mut().as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
        }
    }
}

/// `path` made transparent black within `clip`.
fn clear(
    pixmap: &mut Pixmap,
    path: &tiny_skia::Path,
    rule: FillRule,
    smooth: bool,
    clip: Option<&Mask>,
) {
    let paint = tiny_skia::Paint {
        shader: TRANSPARENT,
        blend_mode: BlendMode::Clear,
        anti_alias: smooth,
        ..tiny_skia::Paint::default()
    };
    pixmap.fill_path(path, &paint, rule, Transform::identity(), clip);
}

/// `mask` narrowed to where `other` covers too.
fn intersect(mask: &mut Mask, other: &Mask) {
    for (a, &b) in mask.data_mut().iter_mut().zip(other.data()) {
        *a = ((u32::from(*a) * u32::from(b) + 127) / 255) as u8;
    }
}
