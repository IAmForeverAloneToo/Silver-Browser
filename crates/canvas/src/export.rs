//! Exporting (HTML 4.12.5, `toDataURL()`, `toBlob()` and
//! `convertToBlob()`; M30.2): the bitmap as a PNG file, its pixels
//! unpremultiplied, through `png`'s encoder. PNG answers for any type
//! asked for, as the standard allows.

use crate::Canvas;
use crate::pixels::unpremultiply;

/// Straight RGBA pixels, `width` by `height`, as a PNG file; `None`
/// when the encoder refuses them.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}

impl Canvas {
    /// The bitmap as a PNG file; `None` for one with no bitmap.
    pub fn to_png(&self) -> Option<Vec<u8>> {
        let bytes = self.bytes()?;
        let mut straight = Vec::with_capacity(bytes.len());
        for pixel in bytes.as_chunks::<4>().0 {
            straight.extend_from_slice(&unpremultiply(pixel));
        }
        encode_png(&straight, self.width(), self.height())
    }
}
