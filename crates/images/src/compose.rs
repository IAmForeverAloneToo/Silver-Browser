//! Composing an animation's frames (PNG 3 section 4.9, GIF89a section
//! 23): each frame drawn over the canvas, then disposed of.

/// How a frame's pixels meet the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    /// Replace the region.
    Source,
    /// Draw over it.
    Over,
}

/// What becomes of a frame's region once it has been shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispose {
    /// Left as drawn.
    Keep,
    /// Cleared to transparent black.
    Clear,
    /// Put back as it was before the frame.
    Restore,
}

/// The canvas frames are drawn on: premultiplied RGBA.
pub struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Canvas {
        let (width, height) = (width as usize, height as usize);
        Canvas {
            width,
            height,
            pixels: vec![0; width * height * 4],
        }
    }

    /// Draws `region` (premultiplied RGBA, `rw` by `rh`) at `x`, `y`,
    /// clipped to the canvas; returns the frame shown, then disposes.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        region: &[u8],
        x: u32,
        y: u32,
        rw: u32,
        rh: u32,
        blend: Blend,
        dispose: Dispose,
    ) -> Vec<u8> {
        let (x, y, rw, rh) = (x as usize, y as usize, rw as usize, rh as usize);
        let x1 = x.saturating_add(rw).min(self.width);
        let y1 = y.saturating_add(rh).min(self.height);
        let (x0, y0) = (x.min(x1), y.min(y1));
        let saved = (dispose == Dispose::Restore).then(|| self.pixels.clone());
        for cy in y0..y1 {
            for cx in x0..x1 {
                let s = ((cy - y) * rw + (cx - x)) * 4;
                let Some(src) = region.get(s..s + 4) else {
                    continue;
                };
                let d = (cy * self.width + cx) * 4;
                let dst = &mut self.pixels[d..d + 4];
                match blend {
                    Blend::Source => dst.copy_from_slice(src),
                    Blend::Over => {
                        let keep = 255 - u32::from(src[3]);
                        for i in 0..4 {
                            let under = (u32::from(dst[i]) * keep + 127) / 255;
                            dst[i] = (u32::from(src[i]) + under).min(255) as u8;
                        }
                    }
                }
            }
        }
        let shown = self.pixels.clone();
        match dispose {
            Dispose::Keep => {}
            Dispose::Clear => {
                for cy in y0..y1 {
                    let row = (cy * self.width + x0) * 4..(cy * self.width + x1) * 4;
                    self.pixels[row].fill(0);
                }
            }
            Dispose::Restore => {
                if let Some(saved) = saved {
                    self.pixels = saved;
                }
            }
        }
        shown
    }
}

/// Multiplies each pixel's colour by its alpha.
pub fn premultiply(rgba: &mut [u8]) {
    for p in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a != 255 {
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * a + 127) / 255) as u8;
            }
        }
    }
}
