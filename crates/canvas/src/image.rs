//! Images (HTML 4.12.5.1.14): `drawImage()` in its three forms, the
//! source rectangle of an image's pixels drawn into the destination
//! rectangle through the context's transform, alpha, shadow, operator
//! and clip, filtered as `imageSmoothingEnabled` says, its edges clamped.
//! An image of another origin drawn without its consent leaves the
//! bitmap no longer origin-clean.

use std::sync::Arc;

use crate::draw::Which;
use crate::path::Path;
use crate::state::Matrix;
use crate::{Canvas, CanvasError};

/// An image to draw: premultiplied RGBA, shared, not copied, and
/// whether the canvas stays origin-clean for drawing it.
#[derive(Debug, Clone)]
pub struct Source {
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub origin_clean: bool,
}

/// The image a draw paints with, in the user's coordinates.
#[derive(Debug, Clone)]
pub(crate) struct ImagePaint {
    pub(crate) pixels: Arc<[u8]>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// From the image's pixels to the user's coordinates.
    pub(crate) transform: Matrix,
}

/// A rectangle given with a negative width or height made the same
/// rectangle with positive ones.
fn normalized([x, y, w, h]: [f64; 4]) -> [f64; 4] {
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    [x, y, w, h]
}

impl Canvas {
    /// `drawImage()`: `src` of `image` (all of it when `None`) into
    /// `dst`, as the standard says: nothing for a rectangle of no area
    /// or a value not finite; the source cut to the image, the
    /// destination cut with it.
    pub fn draw_image(&mut self, image: &Source, src: Option<[f64; 4]>, dst: [f64; 4]) {
        let (w, h) = (f64::from(image.width), f64::from(image.height));
        let src = src.unwrap_or([0.0, 0.0, w, h]);
        if !src.iter().chain(dst.iter()).all(|v| v.is_finite()) {
            return;
        }
        let [sx, sy, sw, sh] = normalized(src);
        let [dx, dy, dw, dh] = normalized(dst);
        if sw == 0.0 || sh == 0.0 || dw == 0.0 || dh == 0.0 || w == 0.0 || h == 0.0 {
            return;
        }
        if !image.origin_clean {
            self.taint();
        }
        // From the source's coordinates to the destination's.
        let (kx, ky) = (dw / sw, dh / sh);
        let to_dst = Matrix::new(kx, 0.0, 0.0, ky, dx - sx * kx, dy - sy * ky);
        // The source cut to the image.
        let (x0, y0) = (sx.max(0.0), sy.max(0.0));
        let (x1, y1) = ((sx + sw).min(w), (sy + sh).min(h));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (ax, ay) = to_dst.apply(x0, y0);
        let (bx, by) = to_dst.apply(x1, y1);
        self.charge(u64::from(image.width) * u64::from(image.height) / 64);
        self.drawing = Some(ImagePaint {
            pixels: Arc::clone(&image.pixels),
            width: image.width,
            height: image.height,
            transform: to_dst,
        });
        let mut rect = Path::new();
        rect.rect(ax, ay, bx - ax, by - ay, &self.state.transform);
        if let Some(skia) = rect.to_skia() {
            self.paint_path(&skia, tiny_skia::FillRule::Winding, Which::Image);
        }
        self.drawing = None;
    }
}

/// The refusal of an image that cannot be drawn: a canvas of no area.
pub fn empty_canvas() -> CanvasError {
    CanvasError::new(
        "InvalidStateError",
        "The image argument is a canvas element with a width or height of 0.",
    )
}
