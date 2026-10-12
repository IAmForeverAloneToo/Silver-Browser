//! The four decoders, each read into premultiplied RGBA frames.

use std::io::Cursor;
use std::num::NonZeroU64;
use std::sync::Arc;

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use crate::compose::{Blend, Canvas, Dispose, premultiply};
use crate::{
    Error, Format, Frame, Header, Image, MAX_ANIMATION_BYTES, MAX_FRAMES, MAX_IMAGE_BYTES,
    MAX_IMAGE_SIDE, Orientation, Plays, check_size, exif, frame_delay,
};

/// An image as decoded, before it is oriented.
struct Decoded {
    width: u32,
    height: u32,
    orientation: Orientation,
    /// Premultiplied RGBA frames, with their delays in milliseconds.
    frames: Vec<(Vec<u8>, u32)>,
    plays: Plays,
}

pub fn header(bytes: &[u8]) -> Result<Header, Error> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(Error::TooLarge);
    }
    let format = Format::of(bytes).ok_or(Error::Unsupported)?;
    let (width, height, orientation) = match format {
        // Never a signature's: the engine reads SVG.
        Format::Svg => return Err(Error::Unsupported),
        Format::Png => {
            let reader = png_reader(bytes)?;
            let info = reader.info();
            let orientation = info
                .exif_metadata
                .as_deref()
                .map_or_else(Orientation::default, exif::orientation);
            (info.width, info.height, orientation)
        }
        Format::Jpeg => {
            let mut decoder = jpeg_decoder(bytes);
            decoder.decode_headers().map_err(|_| Error::Malformed)?;
            let info = decoder.info().ok_or(Error::Malformed)?;
            let orientation = decoder
                .exif()
                .map_or_else(Orientation::default, |e| exif::orientation(e));
            (u32::from(info.width), u32::from(info.height), orientation)
        }
        Format::Gif => {
            let decoder = gif_decoder(bytes)?;
            (
                u32::from(decoder.width()),
                u32::from(decoder.height()),
                Orientation::Normal,
            )
        }
        Format::WebP => {
            let mut decoder = webp_decoder(bytes)?;
            let (width, height) = decoder.dimensions();
            let orientation = webp_orientation(&mut decoder);
            (width, height, orientation)
        }
    };
    check_size(width, height)?;
    let (width, height) = orientation.size(width, height);
    Ok(Header {
        format,
        width,
        height,
        orientation,
        vector: None,
    })
}

pub fn decode(bytes: &[u8]) -> Result<Image, Error> {
    let header = header(bytes)?;
    let decoded = match header.format {
        Format::Png => decode_png(bytes)?,
        Format::Jpeg => decode_jpeg(bytes)?,
        Format::Gif => decode_gif(bytes)?,
        Format::WebP => decode_webp(bytes)?,
        Format::Svg => return Err(Error::Unsupported),
    };
    let (width, height) = decoded.orientation.size(decoded.width, decoded.height);
    let frames = decoded
        .frames
        .into_iter()
        .map(|(pixels, delay)| Frame {
            pixels: Arc::from(
                decoded
                    .orientation
                    .apply(&pixels, decoded.width, decoded.height),
            ),
            delay: frame_delay(delay),
        })
        .collect();
    Ok(Image {
        width,
        height,
        frames,
        plays: decoded.plays,
    })
}

/// Collects an animation's frames within the bounds: past either, the
/// first frame alone is kept.
struct Frames {
    frames: Vec<(Vec<u8>, u32)>,
    bytes: usize,
    over: bool,
}

impl Frames {
    fn new() -> Frames {
        Frames {
            frames: Vec::new(),
            bytes: 0,
            over: false,
        }
    }

    /// Adds a frame; false once a bound is passed.
    fn push(&mut self, pixels: Vec<u8>, delay: u32) -> bool {
        self.bytes += pixels.len();
        if self.frames.len() >= MAX_FRAMES || self.bytes > MAX_ANIMATION_BYTES {
            self.over = true;
            return false;
        }
        self.frames.push((pixels, delay));
        true
    }

    fn finish(mut self) -> Result<Vec<(Vec<u8>, u32)>, Error> {
        if self.over {
            self.frames.truncate(1);
        }
        if self.frames.is_empty() {
            Err(Error::Malformed)
        } else {
            Ok(self.frames)
        }
    }
}

/// Premultiplied RGBA from `channels` (1 grey, 2 grey and alpha, 3 RGB,
/// 4 RGBA) samples a pixel, rows `stride` bytes apart.
pub(crate) fn rgba(
    buf: &[u8],
    channels: usize,
    width: u32,
    height: u32,
    stride: usize,
) -> Result<Vec<u8>, Error> {
    let (w, h) = (width as usize, height as usize);
    // A frame of no area, which GIF and APNG allow, draws nothing.
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }
    if stride < w * channels || buf.len() < stride * h {
        return Err(Error::Malformed);
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for row in buf.chunks(stride).take(h) {
        for p in row[..w * channels].chunks_exact(channels) {
            out.extend_from_slice(&match channels {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            });
        }
    }
    premultiply(&mut out);
    Ok(out)
}

fn png_reader(bytes: &[u8]) -> Result<png::Reader<Cursor<&[u8]>>, Error> {
    let limits = png::Limits {
        bytes: MAX_ANIMATION_BYTES,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    decoder.read_info().map_err(png_error)
}

fn png_error(error: png::DecodingError) -> Error {
    match error {
        png::DecodingError::LimitsExceeded => Error::TooLarge,
        _ => Error::Malformed,
    }
}

fn png_channels(color: png::ColorType) -> usize {
    match color {
        png::ColorType::Grayscale | png::ColorType::Indexed => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
    }
}

fn decode_png(bytes: &[u8]) -> Result<Decoded, Error> {
    let mut reader = png_reader(bytes)?;
    let (width, height) = (reader.info().width, reader.info().height);
    let orientation = reader
        .info()
        .exif_metadata
        .as_deref()
        .map_or_else(Orientation::default, exif::orientation);
    let mut buf = vec![0; reader.output_buffer_size().ok_or(Error::TooLarge)?];
    let Some(control) = reader.info().animation_control else {
        let out = reader.next_frame(&mut buf).map_err(png_error)?;
        let pixels = rgba(
            &buf,
            png_channels(out.color_type),
            width,
            height,
            out.line_size,
        )?;
        return Ok(Decoded {
            width,
            height,
            orientation,
            frames: vec![(pixels, 0)],
            plays: Plays::Times(1),
        });
    };
    // An fcTL before IDAT makes the default image frame 0 (section 4.9.1).
    let default_is_frame = reader.info().frame_control.is_some();
    let total = control.num_frames as usize + usize::from(!default_is_frame);
    let mut canvas = Canvas::new(width, height);
    let mut frames = Frames::new();
    for n in 0..total {
        let out = match reader.next_frame(&mut buf) {
            Ok(out) => out,
            Err(_) if !frames.frames.is_empty() => break,
            Err(error) => return Err(png_error(error)),
        };
        if n == 0 && !default_is_frame {
            continue;
        }
        let control = reader.info().frame_control.ok_or(Error::Malformed)?;
        let region = rgba(
            &buf,
            png_channels(out.color_type),
            out.width,
            out.height,
            out.line_size,
        )?;
        let dispose = match control.dispose_op {
            png::DisposeOp::None => Dispose::Keep,
            png::DisposeOp::Background => Dispose::Clear,
            // On the first frame this restores the transparent canvas,
            // which is what section 11.3.6.2 asks.
            png::DisposeOp::Previous => Dispose::Restore,
        };
        let blend = match control.blend_op {
            png::BlendOp::Source => Blend::Source,
            png::BlendOp::Over => Blend::Over,
        };
        let shown = canvas.draw(
            &region,
            control.x_offset,
            control.y_offset,
            out.width,
            out.height,
            blend,
            dispose,
        );
        // num/den seconds, a den of 0 meaning 100.
        let den = if control.delay_den == 0 {
            100
        } else {
            u32::from(control.delay_den)
        };
        if !frames.push(shown, u32::from(control.delay_num) * 1000 / den) {
            break;
        }
    }
    let plays = match control.num_plays {
        0 => Plays::Forever,
        n => Plays::Times(n),
    };
    Ok(Decoded {
        width,
        height,
        orientation,
        frames: frames.finish()?,
        plays,
    })
}

fn jpeg_decoder(bytes: &[u8]) -> JpegDecoder<ZCursor<&[u8]>> {
    let side = MAX_IMAGE_SIDE as usize;
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(side)
        .set_max_height(side);
    JpegDecoder::new_with_options(ZCursor::new(bytes), options)
}

fn decode_jpeg(bytes: &[u8]) -> Result<Decoded, Error> {
    let mut decoder = jpeg_decoder(bytes);
    decoder.decode_headers().map_err(|_| Error::Malformed)?;
    let info = decoder.info().ok_or(Error::Malformed)?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    let orientation = decoder
        .exif()
        .map_or_else(Orientation::default, |e| exif::orientation(e));
    let pixels = decoder.decode().map_err(|_| Error::Malformed)?;
    if pixels.len() != width as usize * height as usize * 4 {
        return Err(Error::Malformed);
    }
    Ok(Decoded {
        width,
        height,
        orientation,
        frames: vec![(pixels, 0)],
        plays: Plays::Times(1),
    })
}

fn gif_decoder(bytes: &[u8]) -> Result<gif::Decoder<Cursor<&[u8]>>, Error> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let limit = NonZeroU64::new(MAX_ANIMATION_BYTES as u64).expect("not zero");
    options.set_memory_limit(gif::MemoryLimit::Bytes(limit));
    options
        .read_info(Cursor::new(bytes))
        .map_err(|_| Error::Malformed)
}

fn decode_gif(bytes: &[u8]) -> Result<Decoded, Error> {
    let mut decoder = gif_decoder(bytes)?;
    let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    let mut canvas = Canvas::new(width, height);
    let mut frames = Frames::new();
    loop {
        let frame = match decoder.read_next_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(_) if !frames.frames.is_empty() => break,
            Err(_) => return Err(Error::Malformed),
        };
        let (fw, fh) = (u32::from(frame.width), u32::from(frame.height));
        // Transparent pixels come out with alpha 0: drawn over, they leave
        // the canvas as it was.
        let region = rgba(&frame.buffer, 4, fw, fh, fw as usize * 4)?;
        let dispose = match frame.dispose {
            gif::DisposalMethod::Any | gif::DisposalMethod::Keep => Dispose::Keep,
            gif::DisposalMethod::Background => Dispose::Clear,
            gif::DisposalMethod::Previous => Dispose::Restore,
        };
        let (left, top) = (u32::from(frame.left), u32::from(frame.top));
        let delay = u32::from(frame.delay) * 10;
        let shown = canvas.draw(&region, left, top, fw, fh, Blend::Over, dispose);
        if !frames.push(shown, delay) {
            break;
        }
    }
    // The loop count says how many times the animation repeats after its
    // first play; 0 is for ever.
    let plays = match decoder.repeat() {
        gif::Repeat::Infinite => Plays::Forever,
        gif::Repeat::Finite(n) => Plays::Times(u32::from(n) + 1),
    };
    Ok(Decoded {
        width,
        height,
        orientation: Orientation::Normal,
        frames: frames.finish()?,
        plays,
    })
}

fn webp_decoder(bytes: &[u8]) -> Result<image_webp::WebPDecoder<Cursor<&[u8]>>, Error> {
    image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|_| Error::Malformed)
}

fn webp_orientation(decoder: &mut image_webp::WebPDecoder<Cursor<&[u8]>>) -> Orientation {
    match decoder.exif_metadata() {
        Ok(Some(exif)) => exif::orientation(&exif),
        _ => Orientation::Normal,
    }
}

fn decode_webp(bytes: &[u8]) -> Result<Decoded, Error> {
    let mut decoder = webp_decoder(bytes)?;
    let (width, height) = decoder.dimensions();
    decoder.set_memory_limit(MAX_ANIMATION_BYTES);
    let orientation = webp_orientation(&mut decoder);
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let mut buf = vec![0; decoder.output_buffer_size().ok_or(Error::TooLarge)?];
    let stride = width as usize * channels;
    if !decoder.is_animated() {
        decoder.read_image(&mut buf).map_err(|_| Error::Malformed)?;
        let pixels = rgba(&buf, channels, width, height, stride)?;
        return Ok(Decoded {
            width,
            height,
            orientation,
            frames: vec![(pixels, 0)],
            plays: Plays::Times(1),
        });
    }
    let plays = match decoder.loop_count() {
        image_webp::LoopCount::Forever => Plays::Forever,
        image_webp::LoopCount::Times(n) => Plays::Times(u32::from(n.get())),
    };
    let mut frames = Frames::new();
    for _ in 0..decoder.num_frames() {
        // image-webp composes the frames itself.
        let delay = match decoder.read_frame(&mut buf) {
            Ok(delay) => delay,
            Err(_) if !frames.frames.is_empty() => break,
            Err(_) => return Err(Error::Malformed),
        };
        if !frames.push(rgba(&buf, channels, width, height, stride)?, delay) {
            break;
        }
    }
    Ok(Decoded {
        width,
        height,
        orientation,
        frames: frames.finish()?,
        plays,
    })
}
