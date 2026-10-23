//! Sound and pictures (M40, D36, `docs/design/audio-video.md`): the
//! containers a media element or `decodeAudioData()` reads, demuxed into
//! tracks and packets, and the packets decoded into interleaved `f32`
//! samples and RGBA pictures, under bounds a hostile file cannot exceed.
//! A leaf crate: it is given bytes and opens nothing.
//!
//! The codecs are those of Chromium's open-source build: Opus, Vorbis,
//! FLAC, MP3 and PCM sound, AV1 pictures, in WebM and Matroska, MP4, Ogg
//! and WAV, and FLAC's and MP3's own files. `symphonia` demuxes and
//! decodes all but Opus and AV1; `opus-decoder` decodes Opus; `rav1d`
//! decodes AV1 through its C interface, the one place here that writes
//! `unsafe` (the `av1` module, each block argued).

#![deny(unsafe_code)]

mod av1;
mod demux;
mod opus;
mod resample;
mod sound;
mod source;
mod types;
mod video;
mod yuv;

#[cfg(test)]
mod tests;

pub use demux::{Demuxer, Hint, Packet, PictureCodec, SoundCodec, Track, TrackKind};
pub use resample::{Resampler, resample};
pub use sound::{Sound, SoundDecoder, decode_all};
pub use source::{ByteSource, Bytes};
pub use types::{CanPlay, can_play_type};
pub use video::{Picture, PictureDecoder};

/// The largest side of a picture, in pixels.
pub const MAX_VIDEO_SIZE: u32 = 8_192;
/// The most channels a sound track may have.
pub const MAX_CHANNELS: u16 = 32;
/// The most bytes a packet may hold.
pub const MAX_PACKET_BYTES: usize = 32 << 20;
/// The most tracks a container may have; past them, the rest are not
/// read.
pub const MAX_TRACKS: usize = 64;
/// The lowest and highest sample rates taken, the Web Audio API's range
/// of an `AudioBuffer`'s.
pub const MIN_SAMPLE_RATE: u32 = 3_000;
pub const MAX_SAMPLE_RATE: u32 = 768_000;

/// Why media could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a container or a codec this crate reads.
    Unsupported(String),
    /// A container or a stream that does not read as its format says.
    Malformed(String),
    /// Past one of this crate's bounds.
    TooLarge(String),
    /// The bytes ended, or the source could not give them.
    Ended,
    /// A decoder panicked: caught, and the decoder is not used again.
    Panicked,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported(what) => write!(f, "unsupported: {what}"),
            Error::Malformed(what) => write!(f, "malformed: {what}"),
            Error::TooLarge(what) => write!(f, "too large: {what}"),
            Error::Ended => f.write_str("the media ended"),
            Error::Panicked => f.write_str("a decoder panicked"),
        }
    }
}

impl std::error::Error for Error {}

impl From<symphonia::core::errors::Error> for Error {
    fn from(error: symphonia::core::errors::Error) -> Self {
        use symphonia::core::errors::Error as E;
        match error {
            E::IoError(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Error::Ended,
            E::IoError(_) => Error::Ended,
            E::Unsupported(what) => Error::Unsupported(what.to_owned()),
            other => Error::Malformed(other.to_string()),
        }
    }
}

/// Makes a panic inside `rav1d` end the process rather than unwind: its
/// C-shaped state is not sound to unwind through (M40's fuzzing found a
/// guard released on freed memory while unwinding from its panic), so
/// such a panic is never caught by [`guarded`]. Other panics go to the
/// hook there was. Installed by the first AV1 decoder a process makes;
/// later calls do nothing.
pub fn abort_on_decoder_panic() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.location().is_some_and(|l| l.file().contains("rav1d")) {
                eprintln!(
                    "media: the AV1 decoder panicked: {info}
{}",
                    std::backtrace::Backtrace::capture()
                );
                std::process::abort();
            }
            previous(info);
        }));
    });
}

/// `f()`, a panic in it caught as [`Error::Panicked`]: the crates read
/// bytes a page chose.
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(Err(Error::Panicked))
}
