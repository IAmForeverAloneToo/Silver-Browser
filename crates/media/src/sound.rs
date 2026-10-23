//! Sound tracks decoded to interleaved `f32` samples.

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::units::{Duration, Timestamp};

use crate::demux::{Demuxer, Hint, Packet, SoundCodec, Track, TrackKind};
use crate::opus::Opus;
use crate::source::Bytes;
use crate::{Error, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE, guarded};

/// Decoded sound: its time in seconds, its rate and channels, and its
/// samples, interleaved, a frame being one sample of each channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub time: f64,
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Sound {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }
}

enum Inner {
    Symphonia(Box<dyn AudioDecoder>),
    Opus(Box<Opus>),
}

/// A decoder of one sound track.
pub struct SoundDecoder {
    inner: Option<Inner>,
    rate: u32,
    channels: u16,
    /// Frames still to drop from the start (Opus's pre-skip, where its
    /// container gave no trim).
    skip: u32,
}

impl SoundDecoder {
    pub fn new(track: &Track) -> Result<SoundDecoder, Error> {
        guarded(|| {
            let TrackKind::Sound {
                codec,
                rate,
                channels,
                setup,
            } = &track.kind
            else {
                return Err(Error::Unsupported("not a sound track".to_owned()));
            };
            match codec {
                SoundCodec::Other => Err(Error::Unsupported("a sound codec".to_owned())),
                SoundCodec::Opus => {
                    let opus = Opus::new(setup, *channels)?;
                    let channels = opus.head.channels;
                    // The pre-skip is trimmed here, or by the packets'
                    // own trims where the container gives them: the more
                    // of the two (`decode`).
                    let pre_skip = opus.head.pre_skip;
                    Ok(SoundDecoder {
                        inner: Some(Inner::Opus(Box::new(opus))),
                        rate: 48_000,
                        channels,
                        skip: pre_skip,
                    })
                }
                _ => {
                    let CodecParameters::Audio(params) = &track.params else {
                        return Err(Error::Unsupported("a sound codec".to_owned()));
                    };
                    let decoder = symphonia::default::get_codecs()
                        .make_audio_decoder(params, &AudioDecoderOptions::default())?;
                    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(rate) {
                        return Err(Error::Unsupported(format!("a sample rate of {rate}")));
                    }
                    Ok(SoundDecoder {
                        inner: Some(Inner::Symphonia(decoder)),
                        rate: *rate,
                        channels: *channels,
                        skip: 0,
                    })
                }
            }
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// One packet's sound, its trims applied.
    pub fn decode(&mut self, packet: &Packet) -> Result<Sound, Error> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(Error::Panicked);
        };
        let channels = usize::from(self.channels.max(1));
        let result = guarded(|| match inner {
            Inner::Opus(opus) => opus.decode(&packet.data),
            Inner::Symphonia(decoder) => {
                let decoded = decoder.decode(&symphonia::core::packet::Packet::new(
                    packet.track,
                    Timestamp::new(packet.timestamp),
                    Duration::new(packet.ticks),
                    packet.data.clone(),
                ))?;
                let mut samples: Vec<f32> = Vec::new();
                decoded.copy_to_vec_interleaved(&mut samples);
                Ok(samples)
            }
        });
        let mut samples = match result {
            Ok(samples) => samples,
            Err(Error::Panicked) => {
                self.inner = None;
                return Err(Error::Panicked);
            }
            Err(e) => return Err(e),
        };
        let frames = samples.len() / channels;
        // The pre-skip still due, unless the container trims it itself.
        let start = (packet.trim_start.max(self.skip) as usize).min(frames);
        self.skip = self
            .skip
            .saturating_sub(u32::try_from(frames).unwrap_or(u32::MAX));
        let end = frames.saturating_sub(packet.trim_end as usize).max(start);
        samples.truncate(end * channels);
        samples.drain(..start * channels);
        Ok(Sound {
            time: packet.time,
            rate: self.rate,
            channels: self.channels,
            samples,
        })
    }

    /// Forgets what came before: after a seek.
    pub fn reset(&mut self) {
        match &mut self.inner {
            Some(Inner::Opus(opus)) => opus.reset(),
            Some(Inner::Symphonia(decoder)) => decoder.reset(),
            None => {}
        }
        self.skip = 0;
    }
}

/// Every sample of the first sound track of `bytes`, at its own rate:
/// `decodeAudioData()`'s, within `max_samples` samples. Its rate, its
/// channels and its samples, interleaved.
pub fn decode_all(bytes: &[u8], max_samples: usize) -> Result<(u32, u16, Vec<f32>), Error> {
    let mut demuxer = Demuxer::open(Box::new(Bytes::new(bytes.to_vec())), &Hint::default())?;
    let track = demuxer
        .first_sound()
        .cloned()
        .ok_or_else(|| Error::Unsupported("no sound track".to_owned()))?;
    let mut decoder = SoundDecoder::new(&track)?;
    let mut samples = Vec::new();
    while let Some(packet) = demuxer.next_packet()? {
        if packet.track != track.id {
            continue;
        }
        let sound = match decoder.decode(&packet) {
            Ok(sound) => sound,
            // A damaged packet is skipped, as the reference skips it.
            Err(Error::Malformed(_)) => continue,
            Err(e) => return Err(e),
        };
        if samples.len() + sound.samples.len() > max_samples {
            return Err(Error::TooLarge("the decoded sound".to_owned()));
        }
        samples.extend_from_slice(&sound.samples);
    }
    if samples.is_empty() {
        return Err(Error::Malformed("no sound decoded".to_owned()));
    }
    Ok((decoder.rate(), decoder.channels(), samples))
}
