//! Images (M23.1, `docs/design/images.md`): PNG with APNG, JPEG, GIF and
//! WebP decoded to premultiplied 8-bit sRGB, under bounds a hostile image
//! cannot exceed. A leaf crate: it is given bytes and opens nothing.

mod compose;
mod exif;
mod formats;
mod orientation;

#[cfg(test)]
mod tests;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

pub use orientation::Orientation;

/// The largest encoded image taken, in bytes.
pub const MAX_IMAGE_BYTES: usize = 32 << 20;
/// The largest side of an image, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 16_384;
/// The most pixels an image may have: width times height.
pub const MAX_IMAGE_PIXELS: u64 = 1 << 25;
/// The most frames an animation may have.
pub const MAX_FRAMES: usize = 1_000;
/// The most bytes an animation's composed frames may take; past it the
/// image keeps its first frame alone.
pub const MAX_ANIMATION_BYTES: usize = 256 << 20;

/// A format this crate decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    WebP,
}

impl Format {
    /// The format whose signature `bytes` starts with.
    pub fn of(bytes: &[u8]) -> Option<Format> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Format::Png)
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Format::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Format::Gif)
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(Format::WebP)
        } else {
            None
        }
    }
}

/// Why an image could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Not a format this crate decodes.
    Unsupported,
    /// Corrupt or truncated.
    Malformed,
    /// Over one of the bounds above.
    TooLarge,
    /// The decoder panicked; caught.
    Panicked,
}

/// What an image's header says: enough to lay it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub format: Format,
    /// The width once oriented.
    pub width: u32,
    /// The height once oriented.
    pub height: u32,
    pub orientation: Orientation,
}

/// How many times an animation plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plays {
    Forever,
    Times(u32),
}

/// One frame, whole: premultiplied RGBA, `width * height * 4` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub pixels: Arc<[u8]>,
    /// How long it is shown, in milliseconds, after [`frame_delay`].
    pub delay: u32,
}

/// A decoded image, oriented: one frame, or an animation's frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<Frame>,
    pub plays: Plays,
}

impl Image {
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// The bytes its frames take.
    pub fn bytes(&self) -> usize {
        self.frames.iter().map(|f| f.pixels.len()).sum()
    }
}

/// A frame's delay as the engines show it: 10ms or less is 100ms.
pub fn frame_delay(ms: u32) -> u32 {
    if ms <= 10 { 100 } else { ms }
}

/// The frame of `image` shown `elapsed` milliseconds into its timeline
/// (M23.1), and when on it, in milliseconds, the next frame comes: each
/// frame for its delay, the frames again as many times as the image
/// plays, and then the last frame for good. `None` for a still image and
/// for one that has played out.
pub fn frame_at(image: &Image, elapsed: f64) -> (usize, Option<f64>) {
    let last = image.frames.len().saturating_sub(1);
    if last == 0 {
        return (0, None);
    }
    let elapsed = elapsed.max(0.0);
    let period: f64 = image.frames.iter().map(|f| f64::from(f.delay)).sum();
    if period <= 0.0 {
        return (last, None);
    }
    let round = (elapsed / period).floor();
    if let Plays::Times(n) = image.plays
        && round >= f64::from(n.max(1))
    {
        return (last, None);
    }
    let mut end = round * period;
    for (i, frame) in image.frames.iter().enumerate() {
        end += f64::from(frame.delay);
        if elapsed < end {
            return (i, Some(end));
        }
    }
    // Only rounding puts it past the round's last frame.
    (last, Some(end))
}

/// `image` made `width` by `height`, neither more than its own (M23.5):
/// each new pixel the average of the old pixels its area covers, a part
/// pixel weighted by how much of it is covered; a box filter, in
/// premultiplied colour. Every frame is, its delay kept.
pub fn downscale(image: &Image, width: u32, height: u32) -> Image {
    let width = width.clamp(1, image.width.max(1));
    let height = height.clamp(1, image.height.max(1));
    let frames = image
        .frames
        .iter()
        .map(|frame| Frame {
            pixels: Arc::from(box_filter(
                &frame.pixels,
                (image.width, image.height),
                (width, height),
            )),
            delay: frame.delay,
        })
        .collect();
    Image {
        width,
        height,
        frames,
        plays: image.plays,
    }
}

/// For each of `to` new pixels along an axis `from` old ones long, the
/// first old pixel it covers and the weight of each it covers, which
/// add up to one.
fn spans(from: u32, to: u32) -> Vec<(usize, Vec<f32>)> {
    let scale = f64::from(from) / f64::from(to);
    (0..to)
        .map(|i| {
            let start = f64::from(i) * scale;
            let end = (f64::from(i + 1) * scale).min(f64::from(from));
            let first = start.floor() as usize;
            let last = (end.ceil() as usize).clamp(first + 1, from as usize);
            let weights = (first..last)
                .map(|j| {
                    let covered = end.min(j as f64 + 1.0) - start.max(j as f64);
                    (covered.max(0.0) / scale) as f32
                })
                .collect();
            (first, weights)
        })
        .collect()
}

/// [`downscale`]'s filter of one frame's `pixels`, along x and then y.
fn box_filter(pixels: &[u8], (w, h): (u32, u32), (to_w, to_h): (u32, u32)) -> Vec<u8> {
    let (w, to_w) = (w as usize, to_w as usize);
    let xs = spans(w as u32, to_w as u32);
    let ys = spans(h, to_h);
    let mut rows = vec![0f32; to_w * h as usize * 4];
    for (y, row) in rows.chunks_exact_mut(to_w * 4).enumerate() {
        let source = &pixels[y * w * 4..(y + 1) * w * 4];
        for (out, (first, weights)) in row.as_chunks_mut::<4>().0.iter_mut().zip(&xs) {
            for (k, weight) in weights.iter().enumerate() {
                let texel = &source[(first + k) * 4..(first + k + 1) * 4];
                for (o, &v) in out.iter_mut().zip(texel) {
                    *o += f32::from(v) * weight;
                }
            }
        }
    }
    let mut out = vec![0u8; to_w * to_h as usize * 4];
    for (row, (first, weights)) in out.chunks_exact_mut(to_w * 4).zip(&ys) {
        for (x, texel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let mut sum = [0f32; 4];
            for (k, weight) in weights.iter().enumerate() {
                let at = ((first + k) * to_w + x) * 4;
                for (s, &v) in sum.iter_mut().zip(&rows[at..at + 4]) {
                    *s += v * weight;
                }
            }
            // No channel passes the alpha: each old one did not, and
            // all are weighed alike.
            for (t, s) in texel.iter_mut().zip(sum) {
                *t = s.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Reads the header of `bytes`, panics caught.
pub fn read_header(bytes: &[u8]) -> Result<Header, Error> {
    isolated(|| formats::header(bytes))
}

/// Decodes `bytes` whole, panics caught.
pub fn decode(bytes: &[u8]) -> Result<Image, Error> {
    isolated(|| formats::decode(bytes))
}

fn isolated<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(Err(Error::Panicked))
}

/// Refuses sizes over the bounds before anything is allocated.
fn check_size(width: u32, height: u32) -> Result<(), Error> {
    if width == 0 || height == 0 {
        return Err(Error::Malformed);
    }
    if width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err(Error::TooLarge);
    }
    Ok(())
}
