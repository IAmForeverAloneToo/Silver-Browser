//! AV1 decoded by `rav1d` (D36), the Rust port of `dav1d`, through the C
//! interface it keeps: the one module of this crate that writes
//! `unsafe`. Each block says why it holds. The decoder runs on one
//! thread with one frame in flight (`max_frame_delay` 1), so that each
//! temporal unit sent comes back as its picture before the next is sent;
//! its frames are bounded at [`MAX_VIDEO_SIZE`] squared pixels. rav1d is
//! the vendored 1.1.0 with the upstream fix its release lacks
//! (`vendor/README.md`).

#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::ptr::NonNull;

use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422,
};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture,
    dav1d_open, dav1d_picture_unref, dav1d_send_data,
};

use crate::yuv::{Layout, Planes, Range};
use crate::{Error, MAX_VIDEO_SIZE};

/// The rounds of sending a unit and taking pictures before a decoder
/// that takes none of it is given up on.
const MAX_ROUNDS: u32 = 64;

/// `dav1d`'s "try again": its result for a picture not ready, or data
/// not taken until a picture is.
fn again(result: i32) -> bool {
    result == -libc::EAGAIN
}

/// Whether `unit` holds a sequence header OBU.
fn has_sequence_header(unit: &[u8]) -> bool {
    let mut at = 0;
    while at < unit.len() {
        let header = unit[at];
        if (header >> 3) & 0xF == 1 {
            return true;
        }
        at += 1 + usize::from((header >> 2) & 1);
        let mut size = 0usize;
        let mut shift = 0;
        while let Some(&b) = unit.get(at) {
            at += 1;
            size |= usize::from(b & 0x7F) << shift;
            shift += 7;
            if b & 0x80 == 0 || shift >= 56 {
                break;
            }
        }
        at = at.saturating_add(size);
    }
    false
}

/// A decoded frame, converted: its time (the timestamp it was sent
/// with), its size and its RGBA pixels.
pub(crate) struct Frame {
    pub timestamp: i64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One `dav1d` context.
pub(crate) struct Av1 {
    context: Option<Dav1dContext>,
    /// Whether it has taken a sequence header: until then it is given
    /// no unit without one (rav1d 1.1.0's error cleanup panics on frames
    /// it has no sequence for).
    sequenced: bool,
}

impl Av1 {
    pub(crate) fn new() -> Result<Av1, Error> {
        // Whatever process decodes AV1 never unwinds through rav1d.
        crate::abort_on_decoder_panic();
        let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
        // SAFETY: `dav1d_default_settings` writes a whole `Dav1dSettings`
        // through the pointer, which points at memory of that type and
        // size; nothing reads it before.
        unsafe { dav1d_default_settings(NonNull::from(&mut settings).cast()) };
        // SAFETY: written whole just above.
        let mut settings = unsafe { settings.assume_init() };
        // One thread, one frame in flight: rav1d 1.1.0's threaded path
        // deadlocked now and then on a malformed stream (M40's fuzzing).
        settings.n_threads = 1;
        settings.max_frame_delay = 1;
        settings.frame_size_limit = MAX_VIDEO_SIZE * MAX_VIDEO_SIZE;
        let mut context: Option<Dav1dContext> = None;
        // SAFETY: both pointers point at live values of the types
        // `dav1d_open` takes: it writes the context it opens into
        // `context` and reads `settings` alone.
        let result = unsafe {
            dav1d_open(
                Some(NonNull::from(&mut context)),
                Some(NonNull::from(&mut settings)),
            )
        };
        if result.0 != 0 || context.is_none() {
            return Err(Error::Malformed(format!("dav1d_open: {}", result.0)));
        }
        Ok(Av1 {
            context,
            sequenced: false,
        })
    }

    /// Sends one temporal unit, `bytes`, stamped `timestamp`, and returns
    /// the frames it gave.
    pub(crate) fn decode(&mut self, bytes: &[u8], timestamp: i64) -> Result<Vec<Frame>, Error> {
        let Some(context) = self.context else {
            return Err(Error::Panicked);
        };
        let mut frames = Vec::new();
        if bytes.is_empty() {
            return Ok(frames);
        }
        let sequence = has_sequence_header(bytes);
        if !self.sequenced && !sequence {
            // Nothing to decode it against: skipped, until a keyframe
            // brings its sequence header.
            return Ok(frames);
        }
        let mut data = Dav1dData::default();
        // SAFETY: `data` is a live `Dav1dData`, which `dav1d_data_create`
        // overwrites with a new buffer of `bytes.len()` bytes and returns
        // that buffer's start, or null when it could not make it.
        let buffer = unsafe { dav1d_data_create(Some(NonNull::from(&mut data)), bytes.len()) };
        if buffer.is_null() {
            return Err(Error::TooLarge("an AV1 packet".to_owned()));
        }
        // SAFETY: `buffer` is the start of `bytes.len()` writable bytes
        // the data owns, made just above and not yet given to the
        // decoder, and `bytes` cannot overlap a buffer made now.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len()) };
        data.m.timestamp = timestamp;
        let mut rounds = 0;
        loop {
            // SAFETY: `context` is the open context; `data` is a live
            // `Dav1dData` made by `dav1d_data_create`, whose bytes the
            // decoder takes (in part or whole, its size then less).
            let sent = unsafe { dav1d_send_data(Some(context), Some(NonNull::from(&mut data))) };
            if sent.0 != 0 && !again(sent.0) {
                self.drop_data(&mut data);
                return Err(Error::Malformed(format!("dav1d_send_data: {}", sent.0)));
            }
            if let Err(e) = self.take(&context, &mut frames) {
                self.drop_data(&mut data);
                return Err(e);
            }
            if data.sz == 0 {
                self.sequenced |= sequence;
                break;
            }
            // Not all taken: its threads are busy, and the next take
            // waits for a picture (dav1d's own loop). A decoder that
            // takes nothing for long is given up on.
            rounds += 1;
            if rounds > MAX_ROUNDS {
                self.drop_data(&mut data);
                return Err(Error::Malformed("dav1d took none of a unit".to_owned()));
            }
        }
        Ok(frames)
    }

    /// The frames the decoder still holds.
    pub(crate) fn drain(&mut self) -> Result<Vec<Frame>, Error> {
        let Some(context) = self.context else {
            return Ok(Vec::new());
        };
        let mut frames = Vec::new();
        self.take(&context, &mut frames)?;
        Ok(frames)
    }

    fn drop_data(&self, data: &mut Dav1dData) {
        // SAFETY: `data` is a `Dav1dData` made by `dav1d_data_create` and
        // not yet wholly taken; `dav1d_data_unref` frees what is left and
        // zeroes it.
        unsafe { dav1d_data_unref(Some(NonNull::from(data))) };
    }

    /// Takes every picture ready, converted.
    fn take(&self, context: &Dav1dContext, frames: &mut Vec<Frame>) -> Result<(), Error> {
        loop {
            let mut picture = Dav1dPicture::default();
            // SAFETY: `context` is open; `picture` is a live, empty
            // `Dav1dPicture` the call fills when it gives one.
            let got =
                unsafe { dav1d_get_picture(Some(*context), Some(NonNull::from(&mut picture))) };
            if again(got.0) {
                return Ok(());
            }
            if got.0 != 0 {
                return Err(Error::Malformed(format!("dav1d_get_picture: {}", got.0)));
            }
            let frame = convert(&picture);
            // SAFETY: `picture` was filled by `dav1d_get_picture` above
            // and is released once, here, after `convert` is done with
            // its planes.
            unsafe { dav1d_picture_unref(Some(NonNull::from(&mut picture))) };
            frames.push(frame?);
        }
    }
}

/// The picture's planes read and converted to RGBA.
fn convert(picture: &Dav1dPicture) -> Result<Frame, Error> {
    let width = u32::try_from(picture.p.w).unwrap_or(0);
    let height = u32::try_from(picture.p.h).unwrap_or(0);
    if width == 0 || height == 0 || width > MAX_VIDEO_SIZE || height > MAX_VIDEO_SIZE {
        return Err(Error::TooLarge("an AV1 picture".to_owned()));
    }
    let layout = match picture.p.layout {
        DAV1D_PIXEL_LAYOUT_I400 => Layout::I400,
        DAV1D_PIXEL_LAYOUT_I420 => Layout::I420,
        DAV1D_PIXEL_LAYOUT_I422 => Layout::I422,
        _ => Layout::I444,
    };
    let high = picture.p.bpc > 8;
    let bytes_per_sample = if high { 2 } else { 1 };
    let (chroma_w, chroma_h) = layout.chroma_size(width, height);
    let (mtrx, full) = match picture.seq_hdr {
        // SAFETY: a picture `dav1d_get_picture` gave holds a reference to
        // its sequence header, alive until `dav1d_picture_unref`, which is
        // not called before this function returns.
        Some(header) => {
            let header = unsafe { header.as_ref() };
            (header.mtrx, header.color_range != 0)
        }
        None => (2, false),
    };
    let luma_stride = usize::try_from(picture.stride[0]).unwrap_or(0);
    let chroma_stride = usize::try_from(picture.stride[1]).unwrap_or(0);
    let plane = |index: usize, stride: usize, rows: u32, cols: u32| -> Result<&[u8], Error> {
        let length = stride
            .checked_mul(rows as usize - 1)
            .and_then(|n| n.checked_add(cols as usize * bytes_per_sample))
            .ok_or_else(|| Error::TooLarge("an AV1 plane".to_owned()))?;
        if stride < cols as usize * bytes_per_sample {
            return Err(Error::Malformed("an AV1 plane's stride".to_owned()));
        }
        match picture.data[index] {
            // SAFETY: `dav1d` gives each plane as `rows` rows of at least
            // `cols` samples, each row `stride` bytes after the one
            // before; the slice covers exactly those bytes, the last row
            // to its last sample, inside the picture's buffer, which
            // stays alive and unwritten until `dav1d_picture_unref`.
            Some(data) => {
                Ok(unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), length) })
            }
            None => Err(Error::Malformed("an AV1 plane missing".to_owned())),
        }
    };
    let y = plane(0, luma_stride, height, width)?;
    let (u, v) = if layout == Layout::I400 {
        (&[][..], &[][..])
    } else {
        (
            plane(1, chroma_stride, chroma_h, chroma_w)?,
            plane(2, chroma_stride, chroma_h, chroma_w)?,
        )
    };
    let planes = Planes {
        width,
        height,
        layout,
        bits: u32::try_from(picture.p.bpc).unwrap_or(8),
        y,
        u,
        v,
        y_stride: luma_stride,
        c_stride: chroma_stride,
    };
    let rgba = crate::yuv::to_rgba(
        &planes,
        crate::yuv::Matrix::of(mtrx),
        if full { Range::Full } else { Range::Limited },
    );
    Ok(Frame {
        timestamp: picture.m.timestamp,
        width,
        height,
        rgba,
    })
}

impl Drop for Av1 {
    fn drop(&mut self) {
        let mut context = self.context.take();
        if context.is_some() {
            // SAFETY: `context` holds the context `dav1d_open` gave, closed
            // once, here; `dav1d_close` takes it out and frees it.
            unsafe { dav1d_close(Some(NonNull::from(&mut context))) };
        }
    }
}
