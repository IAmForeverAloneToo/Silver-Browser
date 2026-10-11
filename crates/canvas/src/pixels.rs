//! Pixel manipulation (HTML 4.12.5.1.16): the bitmap read out as
//! `ImageData`'s straight 8-bit RGBA and written back from it, the
//! premultiplied bitmap converted each way with rounding, untouched by
//! the transform, the alpha, the operator, shadows and the clip.

use crate::{Canvas, CanvasError, MAX_CANVAS_PIXELS};

/// A rectangle given with a negative width or height made the same
/// rectangle with positive ones.
fn normalize(x: i64, w: i64) -> (i64, i64) {
    if w < 0 { (x + w, -w) } else { (x, w) }
}

/// The error for an `ImageData` too large to make.
pub fn too_large() -> CanvasError {
    CanvasError::new("RangeError", "Out of memory at ImageData creation")
}

/// The bytes of an `ImageData` of `width` by `height`, transparent; a
/// `RangeError` past [`MAX_CANVAS_PIXELS`].
pub fn blank_image_data(width: u64, height: u64) -> Result<Vec<u8>, CanvasError> {
    let pixels = width.checked_mul(height).ok_or_else(too_large)?;
    if pixels > MAX_CANVAS_PIXELS {
        return Err(too_large());
    }
    Ok(vec![0; pixels as usize * 4])
}

impl Canvas {
    /// `getImageData()`: the rectangle's pixels, transparent black where
    /// it is outside the bitmap. A width or height of 0 is an
    /// `IndexSizeError`, a bitmap that is not origin-clean a
    /// `SecurityError`.
    pub fn get_image_data(
        &self,
        sx: i64,
        sy: i64,
        sw: i64,
        sh: i64,
    ) -> Result<Vec<u8>, CanvasError> {
        if sw == 0 {
            return Err(CanvasError::new("IndexSizeError", "The source width is 0."));
        }
        if sh == 0 {
            return Err(CanvasError::new(
                "IndexSizeError",
                "The source height is 0.",
            ));
        }
        if !self.origin_clean() {
            return Err(CanvasError::new(
                "SecurityError",
                "The canvas has been tainted by cross-origin data.",
            ));
        }
        let (sx, sw) = normalize(sx, sw);
        let (sy, sh) = normalize(sy, sh);
        let mut out = blank_image_data(sw as u64, sh as u64)?;
        let Some(bytes) = self.bytes() else {
            return Ok(out);
        };
        let (width, height) = (i64::from(self.width()), i64::from(self.height()));
        // The rows and columns of the rectangle within the bitmap.
        let (x0, x1) = (sx.max(0), (sx + sw).min(width));
        let (y0, y1) = (sy.max(0), (sy + sh).min(height));
        if x0 >= x1 || y0 >= y1 {
            return Ok(out);
        }
        for y in y0..y1 {
            let from = ((y * width + x0) * 4) as usize;
            let to = (((y - sy) * sw + (x0 - sx)) * 4) as usize;
            let count = ((x1 - x0) * 4) as usize;
            let source = &bytes[from..from + count];
            for (pixel, out) in source
                .as_chunks::<4>()
                .0
                .iter()
                .zip(out[to..to + count].as_chunks_mut::<4>().0)
            {
                *out = unpremultiply(pixel);
            }
        }
        Ok(out)
    }

    /// `putImageData()`: `data`, `width` by `height` straight RGBA, its
    /// `dirty` rectangle (all of it when none) written with its corner at
    /// (dx, dy).
    #[allow(clippy::too_many_arguments)]
    pub fn put_image_data(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        dx: i64,
        dy: i64,
        dirty: Option<(i64, i64, i64, i64)>,
    ) {
        let (w, h) = (i64::from(width), i64::from(height));
        if data.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return;
        }
        let (dirty_x, dirty_y, dirty_w, dirty_h) = dirty.unwrap_or((0, 0, w, h));
        let (mut dirty_x, mut dirty_w) = normalize(dirty_x, dirty_w);
        let (mut dirty_y, mut dirty_h) = normalize(dirty_y, dirty_h);
        if dirty_x < 0 {
            dirty_w += dirty_x;
            dirty_x = 0;
        }
        if dirty_y < 0 {
            dirty_h += dirty_y;
            dirty_y = 0;
        }
        dirty_w = dirty_w.min(w - dirty_x);
        dirty_h = dirty_h.min(h - dirty_y);
        if dirty_w <= 0 || dirty_h <= 0 {
            return;
        }
        let (bw, bh) = (i64::from(self.width()), i64::from(self.height()));
        // The part of the dirty rectangle that lands on the bitmap.
        let x0 = dirty_x.max(-dx);
        let x1 = (dirty_x + dirty_w).min(bw - dx);
        let y0 = dirty_y.max(-dy);
        let y1 = (dirty_y + dirty_h).min(bh - dy);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let opaque = self.opaque();
        let Some(pixmap) = self.pixmap.as_mut() else {
            return;
        };
        let bytes = pixmap.data_mut();
        for y in y0..y1 {
            let from = ((y * w + x0) * 4) as usize;
            let to = (((y + dy) * bw + x0 + dx) * 4) as usize;
            let count = ((x1 - x0) * 4) as usize;
            for (pixel, out) in data[from..from + count]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(bytes[to..to + count].as_chunks_mut::<4>().0)
            {
                *out = if opaque {
                    [pixel[0], pixel[1], pixel[2], 255]
                } else {
                    premultiply(pixel)
                };
            }
        }
        self.charge(((x1 - x0) * (y1 - y0)) as u64 / 16);
        self.changed();
    }
}

/// A premultiplied pixel as straight RGBA, each channel rounded.
pub(crate) fn unpremultiply(pixel: &[u8; 4]) -> [u8; 4] {
    let a = u32::from(pixel[3]);
    if a == 0 {
        return [0, 0, 0, 0];
    }
    let channel = |c: u8| ((u32::from(c) * 255 + a / 2) / a).min(255) as u8;
    [
        channel(pixel[0]),
        channel(pixel[1]),
        channel(pixel[2]),
        pixel[3],
    ]
}

/// A straight pixel premultiplied, each channel rounded.
pub(crate) fn premultiply(pixel: &[u8; 4]) -> [u8; 4] {
    let a = u32::from(pixel[3]);
    let channel = |c: u8| ((u32::from(c) * a + 127) / 255) as u8;
    [
        channel(pixel[0]),
        channel(pixel[1]),
        channel(pixel[2]),
        pixel[3],
    ]
}
