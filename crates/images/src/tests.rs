use std::borrow::Cow;

use crate::*;

/// 16 by 8, red on the left half and blue on the right, as the reference
/// browser's canvas encodes them.
const JPEG: &[u8] = include_bytes!("../fixtures/red-blue-16x8.jpg");
const LOSSY_WEBP: &[u8] = include_bytes!("../fixtures/red-blue-16x8-lossy.webp");

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn pixel(image: &Image, frame: usize, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * image.width + x) * 4) as usize;
    image.frames[frame].pixels[at..at + 4].try_into().unwrap()
}

fn near(a: [u8; 4], b: [u8; 4], within: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= within)
}

fn png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(pixels).unwrap();
    writer.finish().unwrap();
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A PNG whose header claims `width` by `height`, with no pixels.
fn png_claiming(width: u32, height: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let body = [kind, data].concat();
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let ihdr = [
        &width.to_be_bytes()[..],
        &height.to_be_bytes(),
        &[8, 6, 0, 0, 0],
    ]
    .concat();
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
    chunk(b"IEND", &[]);
    out
}

#[test]
fn formats_are_known_by_their_signatures() {
    assert_eq!(Format::of(&png(1, 1, &RED)), Some(Format::Png));
    assert_eq!(Format::of(JPEG), Some(Format::Jpeg));
    assert_eq!(Format::of(b"GIF89a...."), Some(Format::Gif));
    assert_eq!(Format::of(LOSSY_WEBP), Some(Format::WebP));
    assert_eq!(Format::of(b"<svg/>"), None);
    assert_eq!(decode(b"hello"), Err(Error::Unsupported));
}

#[test]
fn a_png_decodes_premultiplied() {
    let image = decode(&png(2, 1, &[255, 0, 0, 255, 255, 255, 255, 128])).unwrap();
    assert_eq!(
        (image.width, image.height, image.plays),
        (2, 1, Plays::Times(1))
    );
    assert_eq!(pixel(&image, 0, 0, 0), RED);
    assert_eq!(pixel(&image, 0, 1, 0), [128, 128, 128, 128]);
    assert!(!image.is_animated());
}

fn apng(plays: u32, separate_default: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 3, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_animated(2, plays).unwrap();
    encoder.set_sep_def_img(separate_default).unwrap();
    encoder.set_frame_delay(1, 10).unwrap();
    let mut writer = encoder.write_header().unwrap();
    if separate_default {
        // Shown where APNG is not; no frame of the animation.
        writer
            .write_image_data(&[BLUE, BLUE, BLUE].concat())
            .unwrap();
    }
    writer.write_image_data(&[RED, RED, RED].concat()).unwrap();
    writer.set_frame_dimension(1, 1).unwrap();
    writer.set_frame_position(1, 0).unwrap();
    writer.set_frame_delay(0, 0).unwrap();
    writer.set_blend_op(png::BlendOp::Over).unwrap();
    writer.set_dispose_op(png::DisposeOp::Background).unwrap();
    writer.write_image_data(&BLUE).unwrap();
    writer.finish().unwrap();
    out
}

#[test]
fn a_downscaled_copy_averages_the_area_each_pixel_covers() {
    let image = |width, height, pixels: &[[u8; 4]]| Image {
        width,
        height,
        frames: vec![
            Frame {
                pixels: Arc::from(pixels.concat()),
                delay: 70,
            },
            Frame {
                pixels: Arc::from(vec![0; pixels.len() * 4]),
                delay: 80,
            },
        ],
        plays: Plays::Forever,
    };
    let (white, clear) = ([255; 4], [0; 4]);
    // Two by two blocks to one pixel each: half white is grey.
    let four = image(4, 2, &[white, white, RED, BLUE, clear, white, RED, BLUE]);
    let two = downscale(&four, 2, 1);
    assert_eq!((two.width, two.height, two.frames.len()), (2, 1, 2));
    assert_eq!(pixel(&two, 0, 0, 0), [191; 4]);
    assert_eq!(pixel(&two, 0, 1, 0), [128, 0, 128, 255]);
    assert_eq!(
        (two.frames[1].delay, two.plays),
        (80, Plays::Forever),
        "every frame, as timed"
    );
    // Three to two: the middle pixel half in each.
    let three = image(3, 1, &[RED, [0, 90, 0, 255], BLUE]);
    let thirds = downscale(&three, 2, 1);
    assert_eq!(pixel(&thirds, 0, 0, 0), [170, 30, 0, 255]);
    assert_eq!(pixel(&thirds, 0, 1, 0), [0, 30, 170, 255]);
    // Never larger, never empty.
    let same = downscale(&three, 9, 0);
    assert_eq!((same.width, same.height), (3, 1));
    assert_eq!(pixel(&same, 0, 2, 0), BLUE);
}

#[test]
fn an_animation_shows_each_frame_for_its_delay_as_many_times_as_it_plays() {
    let frame = |delay| Frame {
        pixels: Arc::from(vec![0; 4]),
        delay,
    };
    let image = |plays| Image {
        width: 1,
        height: 1,
        frames: vec![frame(100), frame(200), frame(50)],
        plays,
    };
    let twice = image(Plays::Times(2));
    assert_eq!(frame_at(&twice, 0.0), (0, Some(100.0)));
    assert_eq!(frame_at(&twice, 99.9), (0, Some(100.0)));
    assert_eq!(frame_at(&twice, 100.0), (1, Some(300.0)));
    assert_eq!(frame_at(&twice, 320.0), (2, Some(350.0)));
    assert_eq!(frame_at(&twice, 350.0), (0, Some(450.0)), "again");
    assert_eq!(frame_at(&twice, 690.0), (2, Some(700.0)));
    assert_eq!(frame_at(&twice, 700.0), (2, None), "played out");
    assert_eq!(frame_at(&twice, 1e9), (2, None));
    assert_eq!(frame_at(&twice, -5.0), (0, Some(100.0)));
    let forever = image(Plays::Forever);
    assert_eq!(frame_at(&forever, 3500.0 + 120.0), (1, Some(3800.0)));
    assert_eq!(
        frame_at(&image(Plays::Times(0)), 50.0),
        (0, Some(100.0)),
        "once at least"
    );
    let still = Image {
        frames: vec![frame(100)],
        ..image(Plays::Forever)
    };
    assert_eq!(frame_at(&still, 50.0), (0, None));
}

#[test]
fn an_apng_composes_its_frames() {
    let image = decode(&apng(2, false)).unwrap();
    assert_eq!(image.frames.len(), 2);
    assert_eq!(image.plays, Plays::Times(2));
    assert_eq!(
        [pixel(&image, 0, 1, 0), pixel(&image, 1, 1, 0)],
        [RED, BLUE]
    );
    assert_eq!(pixel(&image, 1, 0, 0), RED);
    // 1/10s, then 0/0: as fast as possible, shown for 100ms.
    assert_eq!(
        image.frames.iter().map(|f| f.delay).collect::<Vec<_>>(),
        [100, 100]
    );
    assert_eq!(decode(&apng(0, false)).unwrap().plays, Plays::Forever);
}

#[test]
fn an_apngs_separate_default_image_is_no_frame() {
    let image = decode(&apng(1, true)).unwrap();
    assert_eq!(image.frames.len(), 2);
    assert_eq!(
        [pixel(&image, 0, 0, 0), pixel(&image, 1, 1, 0)],
        [RED, BLUE]
    );
}

#[test]
fn grey_pixels_become_rgba() {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 2, 1);
    encoder.set_color(png::ColorType::GrayscaleAlpha);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[200, 255, 100, 0]).unwrap();
    writer.finish().unwrap();
    let image = decode(&out).unwrap();
    assert_eq!(
        [pixel(&image, 0, 0, 0), pixel(&image, 0, 1, 0)],
        [[200, 200, 200, 255], [0, 0, 0, 0]]
    );
}

#[test]
fn a_frame_of_no_area_is_no_pixels() {
    // Found by the image_decode fuzz target (M23): a frame 0 wide or 0
    // tall, its rows cut zero bytes apart, panicked; it is now no pixels,
    // drawing nothing.
    assert_eq!(formats::rgba(&[], 4, 0, 3, 0), Ok(Vec::new()));
    assert_eq!(formats::rgba(&[1, 2, 3, 4], 4, 1, 0, 4), Ok(Vec::new()));
    assert_eq!(
        formats::rgba(&[9, 8, 7], 3, 1, 1, 3),
        Ok(vec![9, 8, 7, 255])
    );
}

fn gif(frames: &[(&[u8], Option<u8>, u16, gif::DisposalMethod)], repeat: gif::Repeat) -> Vec<u8> {
    let palette = [255, 0, 0, 0, 0, 255, 0, 0, 0];
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, 2, 1, &palette).unwrap();
        encoder.set_repeat(repeat).unwrap();
        for &(indices, transparent, delay, dispose) in frames {
            let frame = gif::Frame {
                width: 2,
                height: 1,
                buffer: Cow::Borrowed(indices),
                transparent,
                delay,
                dispose,
                ..gif::Frame::default()
            };
            encoder.write_frame(&frame).unwrap();
        }
    }
    out
}

#[test]
fn a_gif_draws_each_frame_over_the_last() {
    let bytes = gif(
        &[
            (&[0, 0], None, 0, gif::DisposalMethod::Keep),
            (&[2, 1], Some(2), 2, gif::DisposalMethod::Background),
        ],
        gif::Repeat::Finite(1),
    );
    let image = decode(&bytes).unwrap();
    assert_eq!([pixel(&image, 0, 0, 0), pixel(&image, 0, 1, 0)], [RED, RED]);
    // The transparent pixel leaves the red beneath it.
    assert_eq!(
        [pixel(&image, 1, 0, 0), pixel(&image, 1, 1, 0)],
        [RED, BLUE]
    );
    assert_eq!(
        image.frames.iter().map(|f| f.delay).collect::<Vec<_>>(),
        [100, 20]
    );
    // A loop count of 1 repeats once after the first play.
    assert_eq!(image.plays, Plays::Times(2));
    let forever = gif(
        &[(&[0, 1], None, 5, gif::DisposalMethod::Keep)],
        gif::Repeat::Infinite,
    );
    assert_eq!(decode(&forever).unwrap().plays, Plays::Forever);
}

#[test]
fn an_animation_over_its_bounds_keeps_its_first_frame() {
    let frame: (&[u8], Option<u8>, u16, gif::DisposalMethod) =
        (&[0, 1], None, 5, gif::DisposalMethod::Keep);
    let image = decode(&gif(&vec![frame; MAX_FRAMES + 1], gif::Repeat::Infinite)).unwrap();
    assert_eq!(image.frames.len(), 1);
}

#[test]
fn a_jpeg_decodes_with_its_colours() {
    let header = read_header(JPEG).unwrap();
    assert_eq!(
        (header.format, header.width, header.height),
        (Format::Jpeg, 16, 8)
    );
    let image = decode(JPEG).unwrap();
    assert!(
        near(pixel(&image, 0, 2, 4), RED, 40),
        "{:?}",
        pixel(&image, 0, 2, 4)
    );
    assert!(
        near(pixel(&image, 0, 13, 4), BLUE, 40),
        "{:?}",
        pixel(&image, 0, 13, 4)
    );
}

/// `JPEG` with an EXIF segment recording `orientation`.
fn jpeg_oriented(orientation: u16) -> Vec<u8> {
    let tiff = [
        &b"MM\0*"[..],
        &8u32.to_be_bytes(),
        &1u16.to_be_bytes(),
        &0x0112u16.to_be_bytes(),
        &3u16.to_be_bytes(),
        &1u32.to_be_bytes(),
        &orientation.to_be_bytes(),
        &[0, 0],
        &0u32.to_be_bytes(),
    ]
    .concat();
    let payload = [&b"Exif\0\0"[..], &tiff].concat();
    let segment = [
        &[0xff, 0xe1][..],
        &((payload.len() + 2) as u16).to_be_bytes(),
        &payload,
    ]
    .concat();
    [&JPEG[..2], &segment, &JPEG[2..]].concat()
}

#[test]
fn an_exif_orientation_turns_the_image() {
    let bytes = jpeg_oriented(6);
    let header = read_header(&bytes).unwrap();
    assert_eq!(
        (header.orientation, header.width, header.height),
        (Orientation::Rotate90, 8, 16)
    );
    let image = decode(&bytes).unwrap();
    assert_eq!((image.width, image.height), (8, 16));
    // Turned a quarter clockwise, the left half is on top.
    assert!(near(pixel(&image, 0, 4, 2), RED, 40));
    assert!(near(pixel(&image, 0, 4, 13), BLUE, 40));
}

#[test]
fn exif_is_read_in_either_byte_order_and_otherwise_ignored() {
    let little = [
        &b"II*\0"[..],
        &8u32.to_le_bytes(),
        &1u16.to_le_bytes(),
        &0x0112u16.to_le_bytes(),
        &3u16.to_le_bytes(),
        &1u32.to_le_bytes(),
        &3u16.to_le_bytes(),
        &[0, 0],
    ]
    .concat();
    assert_eq!(exif::orientation(&little), Orientation::Rotate180);
    assert_eq!(
        exif::orientation(b"II*\0\xff\xff\xff\xff"),
        Orientation::Normal
    );
    assert_eq!(exif::orientation(b"Exif\0\0junk"), Orientation::Normal);
    // Cut before its value.
    assert_eq!(exif::orientation(&little[..17]), Orientation::Normal);
}

#[test]
fn the_eight_orientations_place_each_pixel() {
    // 2 by 3, pixel n holding n in its red byte.
    let source: Vec<u8> = (0..6u8).flat_map(|n| [n, 0, 0, 255]).collect();
    let corners = |o: Orientation| {
        let (w, h) = o.size(2, 3);
        let out = o.apply(&source, 2, 3);
        let at = |x: u32, y: u32| out[((y * w + x) * 4) as usize];
        (
            w,
            h,
            [at(0, 0), at(w - 1, 0), at(0, h - 1), at(w - 1, h - 1)],
        )
    };
    assert_eq!(corners(Orientation::Normal), (2, 3, [0, 1, 4, 5]));
    assert_eq!(corners(Orientation::FlipHorizontal), (2, 3, [1, 0, 5, 4]));
    assert_eq!(corners(Orientation::Rotate180), (2, 3, [5, 4, 1, 0]));
    assert_eq!(corners(Orientation::FlipVertical), (2, 3, [4, 5, 0, 1]));
    assert_eq!(corners(Orientation::Transpose), (3, 2, [0, 4, 1, 5]));
    assert_eq!(corners(Orientation::Rotate90), (3, 2, [4, 0, 5, 1]));
    assert_eq!(corners(Orientation::Transverse), (3, 2, [5, 1, 4, 0]));
    assert_eq!(corners(Orientation::Rotate270), (3, 2, [1, 5, 0, 4]));
}

#[test]
fn a_lossy_webp_decodes() {
    let image = decode(LOSSY_WEBP).unwrap();
    assert_eq!((image.width, image.height), (16, 8));
    assert!(near(pixel(&image, 0, 2, 4), RED, 40));
    assert!(near(pixel(&image, 0, 13, 4), BLUE, 40));
}

fn lossless_webp(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    image_webp::WebPEncoder::new(&mut out)
        .encode(pixels, width, height, image_webp::ColorType::Rgba8)
        .unwrap();
    out
}

fn riff_chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = [kind, &(data.len() as u32).to_le_bytes(), data].concat();
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn u24(n: u32) -> [u8; 3] {
    let b = n.to_le_bytes();
    [b[0], b[1], b[2]]
}

/// An animated WebP of 2 by 1 frames, each `(pixels, ms)`, played
/// `loops` times (0 for ever).
fn animated_webp(frames: &[(&[u8], u32)], loops: u16) -> Vec<u8> {
    let vp8x = [&[0x12u8, 0, 0, 0][..], &u24(1), &u24(0)].concat();
    let mut body = [b"WEBP".to_vec(), riff_chunk(b"VP8X", &vp8x)].concat();
    body.extend(riff_chunk(
        b"ANIM",
        &[&[0u8; 4][..], &loops.to_le_bytes()].concat(),
    ));
    for &(pixels, ms) in frames {
        let still = lossless_webp(2, 1, pixels);
        let anmf = [
            &u24(0)[..],
            &u24(0),
            &u24(1),
            &u24(0),
            &u24(ms),
            &[0],
            &still[12..],
        ]
        .concat();
        body.extend(riff_chunk(b"ANMF", &anmf));
    }
    riff_chunk(b"RIFF", &body)
}

#[test]
fn a_lossless_webp_decodes_exactly() {
    let image = decode(&lossless_webp(2, 1, &[RED, [255, 255, 255, 128]].concat())).unwrap();
    assert_eq!(
        [pixel(&image, 0, 0, 0), pixel(&image, 0, 1, 0)],
        [RED, [128, 128, 128, 128]]
    );
}

#[test]
fn an_animated_webp_keeps_its_frames_and_loops() {
    let bytes = animated_webp(
        &[(&[RED, RED].concat(), 50), (&[BLUE, BLUE].concat(), 5)],
        3,
    );
    let image = decode(&bytes).unwrap();
    assert_eq!(image.frames.len(), 2);
    assert_eq!(image.plays, Plays::Times(3));
    // image-webp's blending rounds 255 to 254.
    assert!(near(pixel(&image, 0, 0, 0), RED, 1) && near(pixel(&image, 1, 1, 0), BLUE, 1));
    assert_eq!(
        image.frames.iter().map(|f| f.delay).collect::<Vec<_>>(),
        [50, 100]
    );
    let forever = animated_webp(
        &[(&[RED, RED].concat(), 50), (&[BLUE, BLUE].concat(), 50)],
        0,
    );
    assert_eq!(decode(&forever).unwrap().plays, Plays::Forever);
}

#[test]
fn images_over_the_bounds_are_refused() {
    assert!(read_header(&png_claiming(4, 4)).is_ok());
    assert_eq!(
        read_header(&png_claiming(MAX_IMAGE_SIDE + 1, 1)),
        Err(Error::TooLarge)
    );
    // Each side within bounds, their product not.
    assert_eq!(read_header(&png_claiming(8192, 8192)), Err(Error::TooLarge));
    let mut big = png(1, 1, &RED);
    big.resize(MAX_IMAGE_BYTES + 1, 0);
    assert_eq!(read_header(&big), Err(Error::TooLarge));
}

#[test]
fn a_truncated_image_is_malformed() {
    let bytes = png(4, 4, &[0; 64]);
    assert_eq!(decode(&bytes[..40]), Err(Error::Malformed));
    assert_eq!(decode(&JPEG[..20]), Err(Error::Malformed));
}

#[test]
fn a_panic_in_a_decoder_is_caught() {
    assert_eq!(
        isolated(|| -> Result<(), Error> { panic!("a decoder's panic") }),
        Err(Error::Panicked)
    );
}

#[test]
fn short_delays_are_shown_for_a_tenth_of_a_second() {
    assert_eq!(
        [0, 10, 11, 20, 500].map(frame_delay),
        [100, 100, 11, 20, 500]
    );
}

#[test]
fn frames_drawn_over_and_disposed_of() {
    use crate::compose::{Blend, Canvas, Dispose};
    let clear = [0u8; 8];
    let mut canvas = Canvas::new(2, 1);
    let shown = canvas.draw(
        &[RED, RED].concat(),
        0,
        0,
        2,
        1,
        Blend::Source,
        Dispose::Keep,
    );
    assert_eq!(shown, [RED, RED].concat());
    // Half-transparent blue over red, then put back as it was.
    let shown = canvas.draw(&[0, 0, 128, 128], 1, 0, 1, 1, Blend::Over, Dispose::Restore);
    assert_eq!(&shown[4..], &[127, 0, 128, 255]);
    let shown = canvas.draw(&clear, 1, 0, 1, 1, Blend::Over, Dispose::Keep);
    assert_eq!(shown, [RED, RED].concat());
    // Blue, then cleared; a region past the canvas is clipped.
    let shown = canvas.draw(&BLUE, 1, 0, 1, 1, Blend::Source, Dispose::Clear);
    assert_eq!(&shown[4..], &BLUE);
    let shown = canvas.draw(&clear, 1, 0, 2, 1, Blend::Over, Dispose::Keep);
    assert_eq!(shown, [&RED[..], &[0; 4]].concat());
}
