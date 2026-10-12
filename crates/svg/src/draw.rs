//! Drawing a render tree through a 2D canvas (SVG 2 section 13): each
//! group under its transform, one with an opacity below 1 drawn into a
//! layer of its own and composited at it, as the reference browser
//! does; each shape's fill and stroke in `paint-order`.

use std::sync::Arc;

use canvas::{Canvas, Matrix, Source};

use crate::tree::{Group, Item, Paint, Shape, Tree};
use crate::{MAX_SVG_LAYER_PIXELS, MAX_SVG_LAYERS};

/// Draws `tree` on `canvas`, its viewport's CSS pixels mapped to the
/// canvas's by `base` (a scale for the pixels it lands on, a
/// translation for where it lands).
pub fn draw(tree: &Tree, canvas: &mut Canvas, base: &Matrix) {
    draw_within(tree, canvas, base, MAX_SVG_LAYER_PIXELS);
}

/// [`draw`], its layers taking at most `budget` pixels.
pub(crate) fn draw_within(tree: &Tree, canvas: &mut Canvas, base: &Matrix, budget: u64) {
    let mut drawer = Drawer {
        base: *base,
        layer_pixels: 0,
        budget,
    };
    drawer.group(&tree.root, canvas, base, 1.0, 0);
}

struct Drawer {
    base: Matrix,
    /// The pixels layers took so far, against `budget`.
    layer_pixels: u64,
    /// The most pixels layers take ([`MAX_SVG_LAYER_PIXELS`]).
    budget: u64,
}

impl Drawer {
    fn group(
        &mut self,
        group: &Group,
        canvas: &mut Canvas,
        ctm: &Matrix,
        alpha: f64,
        depth: usize,
    ) {
        let ctm = ctm.then(&group.transform);
        if group.opacity <= 0.0 || alpha <= 0.0 || !ctm.is_finite() {
            return;
        }
        let area = u64::from(canvas.width()) * u64::from(canvas.height());
        let layered = group.opacity < 1.0
            && depth < MAX_SVG_LAYERS
            && self.layer_pixels + area <= self.budget;
        if !layered {
            // Past the bounds, the opacity goes to each child, recorded.
            for item in &group.children {
                self.item(item, canvas, &ctm, alpha * group.opacity, depth);
            }
            return;
        }
        self.layer_pixels += area;
        let mut layer = Canvas::new(canvas.width(), canvas.height(), false);
        for item in &group.children {
            self.item(item, &mut layer, &ctm, 1.0, depth + 1);
        }
        composite(canvas, &layer, alpha * group.opacity);
    }

    fn item(&mut self, item: &Item, canvas: &mut Canvas, ctm: &Matrix, alpha: f64, depth: usize) {
        match item {
            Item::Group(group) => self.group(group, canvas, ctm, alpha, depth),
            Item::Shape(shape) => self.shape(shape, canvas, ctm, alpha * shape.opacity),
        }
    }

    fn shape(&self, shape: &Shape, canvas: &mut Canvas, ctm: &Matrix, alpha: f64) {
        if alpha <= 0.0 {
            return;
        }
        canvas.save();
        canvas.state_mut().transform = *ctm;
        canvas.state_mut().anti_alias = shape.anti_alias;
        let fill = |canvas: &mut Canvas| {
            if let Some(fill) = &shape.fill {
                let state = canvas.state_mut();
                set_paint(&mut state.fill, &fill.paint);
                state.set_global_alpha(alpha * fill.opacity);
                if state.global_alpha > 0.0 {
                    canvas.fill(Some(&shape.path), fill.rule);
                }
            }
        };
        let stroke = |canvas: &mut Canvas| {
            let Some(stroke) = &shape.stroke else {
                return;
            };
            let state = canvas.state_mut();
            set_paint(&mut state.stroke, &stroke.paint);
            state.set_global_alpha(alpha * stroke.opacity);
            state.set_line_width(stroke.width);
            state.line_cap = stroke.cap;
            state.line_join = stroke.join;
            state.set_miter_limit(stroke.miter_limit);
            state.dash.set(&stroke.dashes);
            state.dash.offset = stroke.offset;
            if state.global_alpha <= 0.0 {
                return;
            }
            if stroke.non_scaling {
                // Traced where the drawing is placed: the path taken
                // there, the transforms within it left out.
                let Some(inverse) = self.base.invert() else {
                    return;
                };
                let host = inverse.then(ctm);
                canvas.state_mut().transform = self.base;
                canvas.stroke(Some(&shape.path.transformed(&host)));
            } else {
                canvas.stroke(Some(&shape.path));
            }
        };
        if shape.stroke_first {
            stroke(canvas);
            fill(canvas);
        } else {
            fill(canvas);
            stroke(canvas);
        }
        canvas.restore();
    }
}

fn set_paint(slot: &mut canvas::Paint, paint: &Paint) {
    match paint {
        Paint::Color(color) => *slot = canvas::Paint::Color(*color),
    }
}

/// `layer`, a canvas of `canvas`'s size, composited on it at `alpha`.
fn composite(canvas: &mut Canvas, layer: &Canvas, alpha: f64) {
    let Some(bytes) = layer.bytes() else {
        return;
    };
    let source = Source {
        pixels: Arc::from(bytes),
        width: layer.width(),
        height: layer.height(),
        origin_clean: true,
    };
    canvas.save();
    let state = canvas.state_mut();
    state.transform = Matrix::IDENTITY;
    state.set_global_alpha(alpha);
    state.image_smoothing = false;
    let (w, h) = (f64::from(layer.width()), f64::from(layer.height()));
    canvas.draw_image(&source, None, [0.0, 0.0, w, h]);
    canvas.restore();
}
