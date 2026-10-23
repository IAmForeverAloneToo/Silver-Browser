//! Picture tracks decoded to RGBA.

use crate::av1::Av1;
use crate::demux::{Packet, PictureCodec, Track, TrackKind};
use crate::{Error, guarded};

/// A decoded picture: its time in seconds, its size, and its pixels as
/// opaque RGBA, 8 bits a channel, rows top to bottom.
#[derive(Clone, PartialEq)]
pub struct Picture {
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picture")
            .field("time", &self.time)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Picture {
    /// The pixel at `x`, `y`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[at],
            self.rgba[at + 1],
            self.rgba[at + 2],
            self.rgba[at + 3],
        ]
    }
}

/// Whether an AV1 temporal unit starts with a keyframe: its first frame
/// header (in an `OBU_FRAME_HEADER` or an `OBU_FRAME`) shows a new frame
/// (`show_existing_frame` 0) of `frame_type` `KEY_FRAME`, or its sequence
/// header says every frame is one (`reduced_still_picture_header`).
pub(crate) fn is_key(data: &[u8]) -> bool {
    let mut at = 0;
    while at < data.len() {
        let header = data[at];
        let kind = (header >> 3) & 0xF;
        let extension = usize::from((header >> 2) & 1);
        let sized = (header >> 1) & 1 == 1;
        at += 1 + extension;
        let size = if sized {
            let mut size = 0usize;
            let mut shift = 0;
            loop {
                let Some(&b) = data.get(at) else {
                    return false;
                };
                at += 1;
                size |= usize::from(b & 0x7F) << shift;
                shift += 7;
                if b & 0x80 == 0 || shift > 56 {
                    break;
                }
            }
            size
        } else {
            data.len().saturating_sub(at)
        };
        let Some(payload) = data.get(at..at.saturating_add(size).min(data.len())) else {
            return false;
        };
        match kind {
            // A sequence header: its third bit is
            // reduced_still_picture_header.
            1 if payload.first().is_some_and(|b| b & 0x08 != 0) => return true,
            3 | 6 => {
                return payload
                    .first()
                    .is_some_and(|b| b & 0x80 == 0 && (b >> 5) & 0x3 == 0);
            }
            _ => {}
        }
        at = at.saturating_add(size);
    }
    false
}

/// A decoder of one picture track.
pub struct PictureDecoder {
    av1: Option<Av1>,
    setup: Vec<u8>,
    /// Seconds a timestamp tick is.
    tick: f64,
}

impl PictureDecoder {
    pub fn new(track: &Track) -> Result<PictureDecoder, Error> {
        let TrackKind::Picture { codec, setup, .. } = &track.kind else {
            return Err(Error::Unsupported("not a picture track".to_owned()));
        };
        if *codec != PictureCodec::Av1 {
            return Err(Error::Unsupported("a picture codec".to_owned()));
        }
        let tick = track.time_base.map_or(1.0, |tb| {
            f64::from(tb.numer.get()) / f64::from(tb.denom.get())
        });
        let mut decoder = PictureDecoder {
            av1: Some(guarded(Av1::new)?),
            setup: setup.clone(),
            tick,
        };
        decoder.send_setup();
        Ok(decoder)
    }

    /// The configuration record's sequence header, sent first: a stream
    /// whose first temporal unit lacks one decodes all the same.
    fn send_setup(&mut self) {
        if self.setup.len() > 4 && self.setup[0] & 0x80 != 0 {
            let obus = self.setup[4..].to_vec();
            if let Some(av1) = self.av1.as_mut() {
                let _ = guarded(|| av1.decode(&obus, i64::MIN));
            }
        }
    }

    fn pictures(&self, frames: Vec<crate::av1::Frame>) -> Vec<Picture> {
        frames
            .into_iter()
            .filter(|f| f.timestamp != i64::MIN)
            .map(|f| Picture {
                time: f.timestamp as f64 * self.tick,
                width: f.width,
                height: f.height,
                rgba: f.rgba,
            })
            .collect()
    }

    /// A decoder of AV1 temporal units given alone, their timestamps in
    /// seconds' millionths (the fuzz target's and the tests').
    pub fn av1() -> Result<PictureDecoder, Error> {
        Ok(PictureDecoder {
            av1: Some(guarded(Av1::new)?),
            setup: Vec::new(),
            tick: 1e-6,
        })
    }

    /// The pictures one packet gave.
    pub fn decode(&mut self, packet: &Packet) -> Result<Vec<Picture>, Error> {
        self.decode_bytes(&packet.data, packet.timestamp)
    }

    /// The pictures one temporal unit, `data`, stamped `timestamp` in the
    /// track's ticks, gave.
    pub fn decode_bytes(&mut self, data: &[u8], timestamp: i64) -> Result<Vec<Picture>, Error> {
        let Some(av1) = self.av1.as_mut() else {
            return Err(Error::Panicked);
        };
        match guarded(|| av1.decode(data, timestamp)) {
            Ok(frames) => Ok(self.pictures(frames)),
            Err(Error::Panicked) => {
                // Its state is not to be trusted: never used again, nor
                // closed, which might panic again.
                std::mem::forget(self.av1.take());
                Err(Error::Panicked)
            }
            Err(e) => Err(e),
        }
    }

    /// The pictures still held, at the end of the stream.
    pub fn drain(&mut self) -> Result<Vec<Picture>, Error> {
        let Some(av1) = self.av1.as_mut() else {
            return Ok(Vec::new());
        };
        let frames = guarded(|| av1.drain())?;
        Ok(self.pictures(frames))
    }

    /// Forgets what came before: after a seek, a new decoder.
    pub fn reset(&mut self) -> Result<(), Error> {
        self.av1 = Some(guarded(Av1::new)?);
        self.send_setup();
        Ok(())
    }
}
