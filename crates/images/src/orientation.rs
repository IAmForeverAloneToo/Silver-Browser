//! EXIF orientation (CSS Images 3 section 5.1, `image-orientation:
//! from-image`): the eight ways a camera's pixels are stored.

/// Tag 0x0112's value; `Normal` when absent or out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Normal,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl Orientation {
    pub fn from_exif(value: u16) -> Orientation {
        match value {
            2 => Orientation::FlipHorizontal,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipVertical,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => Orientation::Normal,
        }
    }

    /// Whether width and height trade places.
    pub fn swaps_sides(self) -> bool {
        matches!(
            self,
            Orientation::Transpose
                | Orientation::Rotate90
                | Orientation::Transverse
                | Orientation::Rotate270
        )
    }

    /// The size once oriented.
    pub fn size(self, width: u32, height: u32) -> (u32, u32) {
        if self.swaps_sides() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// `pixels` (RGBA, `width` by `height`) oriented.
    pub fn apply(self, pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
        if self == Orientation::Normal {
            return pixels.to_vec();
        }
        let (w, h) = (width as usize, height as usize);
        let (ow, oh) = if self.swaps_sides() { (h, w) } else { (w, h) };
        let mut out = vec![0; pixels.len()];
        for oy in 0..oh {
            for ox in 0..ow {
                // The source of each output pixel (EXIF 2.3, tag 0x0112).
                let (sx, sy) = match self {
                    Orientation::Normal => (ox, oy),
                    Orientation::FlipHorizontal => (w - 1 - ox, oy),
                    Orientation::Rotate180 => (w - 1 - ox, h - 1 - oy),
                    Orientation::FlipVertical => (ox, h - 1 - oy),
                    Orientation::Transpose => (oy, ox),
                    Orientation::Rotate90 => (oy, h - 1 - ox),
                    Orientation::Transverse => (w - 1 - oy, h - 1 - ox),
                    Orientation::Rotate270 => (w - 1 - oy, ox),
                };
                let s = (sy * w + sx) * 4;
                let o = (oy * ow + ox) * 4;
                out[o..o + 4].copy_from_slice(&pixels[s..s + 4]);
            }
        }
        out
    }
}
