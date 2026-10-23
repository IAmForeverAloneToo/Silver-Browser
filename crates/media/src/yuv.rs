//! Pictures' YUV planes to RGBA: the stream's matrix (ITU-T H.273's
//! matrix coefficients) and range, chroma taken from its nearest sample.

/// How chroma is subsampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    /// Luma alone.
    I400,
    I420,
    I422,
    I444,
}

impl Layout {
    /// The chroma planes' size for a picture of `width` by `height`.
    pub(crate) fn chroma_size(self, width: u32, height: u32) -> (u32, u32) {
        match self {
            Layout::I400 => (0, 0),
            Layout::I420 => (width.div_ceil(2), height.div_ceil(2)),
            Layout::I422 => (width.div_ceil(2), height),
            Layout::I444 => (width, height),
        }
    }

    fn shifts(self) -> (u32, u32) {
        match self {
            Layout::I420 => (1, 1),
            Layout::I422 => (1, 0),
            Layout::I400 | Layout::I444 => (0, 0),
        }
    }
}

/// The samples' range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Range {
    /// Luma from 16 to 235, chroma 16 to 240 (scaled for more bits).
    Limited,
    Full,
}

/// A matrix's luma weights of red and blue; `Identity` for planes that
/// are G, B and R themselves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Matrix {
    Weights { kr: f32, kb: f32 },
    Identity,
}

impl Matrix {
    /// H.273's matrix coefficients `code`; an unspecified or unknown one
    /// is BT.709's, as the reference reads AV1's.
    pub(crate) fn of(code: u32) -> Matrix {
        let (kr, kb) = match code {
            0 => return Matrix::Identity,
            4 => (0.30, 0.11),
            5 | 6 => (0.299, 0.114),
            7 => (0.212, 0.087),
            9 | 10 => (0.2627, 0.0593),
            _ => (0.2126, 0.0722),
        };
        Matrix::Weights { kr, kb }
    }
}

/// A picture's planes: 8-bit samples a byte each, more bits two bytes
/// each, little-endian.
pub(crate) struct Planes<'a> {
    pub width: u32,
    pub height: u32,
    pub layout: Layout,
    pub bits: u32,
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: usize,
    pub c_stride: usize,
}

impl Planes<'_> {
    fn sample(&self, plane: &[u8], stride: usize, x: usize, y: usize) -> f32 {
        if self.bits > 8 {
            let at = y * stride + x * 2;
            f32::from(u16::from_le_bytes([plane[at], plane[at + 1]]))
        } else {
            f32::from(plane[y * stride + x])
        }
    }
}

/// `planes` as opaque RGBA, 8 bits a channel.
pub(crate) fn to_rgba(planes: &Planes<'_>, matrix: Matrix, range: Range) -> Vec<u8> {
    let (w, h) = (planes.width as usize, planes.height as usize);
    let mut out = vec![255u8; w * h * 4];
    let bits = planes.bits.clamp(8, 16);
    let max = ((1u32 << bits) - 1) as f32;
    let scale = (1u32 << (bits - 8)) as f32;
    let (y_off, y_span, c_span) = match range {
        Range::Limited => (16.0 * scale, 219.0 * scale, 224.0 * scale),
        Range::Full => (0.0, max, max),
    };
    let mid = 128.0 * scale;
    let (sx, sy) = planes.layout.shifts();
    let gray = planes.layout == Layout::I400;
    for row in 0..h {
        for col in 0..w {
            let luma = (planes.sample(planes.y, planes.y_stride, col, row) - y_off) / y_span;
            let (cb, cr) = if gray {
                (0.0, 0.0)
            } else {
                let (cx, cy) = (col >> sx, row >> sy);
                (
                    (planes.sample(planes.u, planes.c_stride, cx, cy) - mid) / c_span,
                    (planes.sample(planes.v, planes.c_stride, cx, cy) - mid) / c_span,
                )
            };
            let (r, g, b) = match matrix {
                Matrix::Identity => {
                    // The planes are G, B and R; their "chroma" kept with
                    // its offset taken back.
                    (cr + 0.5, luma, cb + 0.5)
                }
                Matrix::Weights { kr, kb } => {
                    let kg = 1.0 - kr - kb;
                    let r = luma + 2.0 * (1.0 - kr) * cr;
                    let b = luma + 2.0 * (1.0 - kb) * cb;
                    let g = luma
                        - (2.0 * kb * (1.0 - kb) / kg) * cb
                        - (2.0 * kr * (1.0 - kr) / kg) * cr;
                    (r, g, b)
                }
            };
            let at = (row * w + col) * 4;
            out[at] = channel(r);
            out[at + 1] = channel(g);
            out[at + 2] = channel(b);
        }
    }
    out
}

fn channel(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_bt709_white_black_and_red_come_out_as_they_went_in() {
        // BT.709 limited: red is Y 63, Cb 102, Cr 240.
        let planes = |y: u8, u: u8, v: u8| -> Vec<u8> {
            let ys = [y; 4];
            let us = [u; 1];
            let vs = [v; 1];
            let p = Planes {
                width: 2,
                height: 2,
                layout: Layout::I420,
                bits: 8,
                y: &ys,
                u: &us,
                v: &vs,
                y_stride: 2,
                c_stride: 1,
            };
            to_rgba(&p, Matrix::of(1), Range::Limited)[..4].to_vec()
        };
        assert_eq!(planes(235, 128, 128), [255, 255, 255, 255]);
        assert_eq!(planes(16, 128, 128), [0, 0, 0, 255]);
        let red = planes(63, 102, 240);
        assert!(red[0] >= 253 && red[1] <= 2 && red[2] <= 2, "{red:?}");
    }

    #[test]
    fn ten_bits_full_range_reads_two_bytes_a_sample() {
        let ys = [0xFFu8, 0x03];
        let us = [0x00u8, 0x02];
        let vs = [0x00u8, 0x02];
        let p = Planes {
            width: 1,
            height: 1,
            layout: Layout::I444,
            bits: 10,
            y: &ys,
            u: &us,
            v: &vs,
            y_stride: 2,
            c_stride: 2,
        };
        assert_eq!(
            to_rgba(&p, Matrix::of(1), Range::Full),
            [255, 255, 255, 255]
        );
    }
}
