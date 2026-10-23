//! A container read into its tracks and packets (symphonia's readers).

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::well_known as sound_ids;
use symphonia::core::codecs::video::well_known as picture_ids;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase};

use std::collections::VecDeque;

use crate::source::{Adapter, ByteSource, Tap};
use crate::{Error, MAX_CHANNELS, MAX_PACKET_BYTES, MAX_TRACKS, MAX_VIDEO_SIZE, guarded};

/// What helps tell a container: its type as the server said, and the
/// URL's extension.
#[derive(Debug, Clone, Default)]
pub struct Hint {
    pub mime: Option<String>,
    pub extension: Option<String>,
}

/// A sound codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCodec {
    Opus,
    Vorbis,
    Flac,
    Mp3,
    Pcm,
    /// One this crate does not decode.
    Other,
}

/// A picture codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureCodec {
    Av1,
    /// One this crate does not decode.
    Other,
}

/// What a track carries.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackKind {
    Sound {
        codec: SoundCodec,
        rate: u32,
        channels: u16,
        /// The codec's own setup (an Opus header, Vorbis's headers).
        setup: Vec<u8>,
    },
    Picture {
        codec: PictureCodec,
        width: u32,
        height: u32,
        /// The codec's own setup (AV1's configuration record).
        setup: Vec<u8>,
    },
}

/// A track of a container.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: u32,
    pub kind: TrackKind,
    /// Its length in seconds, when the container says.
    pub duration: Option<f64>,
    /// The frames at its start a decoder makes but no one hears (Opus's
    /// pre-skip, an encoder's delay).
    pub delay: u32,
    pub(crate) params: CodecParameters,
    pub(crate) time_base: Option<TimeBase>,
}

impl Track {
    pub fn is_sound(&self) -> bool {
        matches!(self.kind, TrackKind::Sound { .. })
    }

    pub fn is_picture(&self) -> bool {
        matches!(self.kind, TrackKind::Picture { .. })
    }

    /// Whether this crate decodes it.
    pub fn decodable(&self) -> bool {
        match self.kind {
            TrackKind::Sound { codec, .. } => codec != SoundCodec::Other,
            TrackKind::Picture { codec, .. } => codec != PictureCodec::Other,
        }
    }
}

/// A packet of a track: its time in seconds, how long it lasts, and the
/// frames of its decoded sound to trim at its start and its end.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub track: u32,
    pub time: f64,
    pub duration: f64,
    pub trim_start: u32,
    pub trim_end: u32,
    pub data: Vec<u8>,
    /// The time and length as the decoder keeps them: the track's
    /// timestamp and ticks.
    pub(crate) timestamp: i64,
    pub(crate) ticks: u64,
}

/// A container being read.
pub struct Demuxer {
    /// None only while it is being opened again.
    reader: Option<Box<dyn FormatReader>>,
    /// What it was probed with, to probe it again from the start.
    hint: symphonia::core::formats::probe::Hint,
    tracks: Vec<Track>,
    duration: Option<f64>,
    /// Packets read ahead (by a seek, finding a keyframe), given first.
    pending: VecDeque<Packet>,
}

impl std::fmt::Debug for Demuxer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Demuxer")
            .field("tracks", &self.tracks)
            .field("duration", &self.duration)
            .finish()
    }
}

fn seconds(time_base: Option<TimeBase>, ts: i64) -> f64 {
    match time_base {
        Some(tb) => f64::from(tb.numer.get()) * ts as f64 / f64::from(tb.denom.get()),
        None => 0.0,
    }
}

fn track_of(track: &symphonia::core::formats::Track) -> Option<Track> {
    let params = track.codec_params.clone()?;
    let kind = match &params {
        CodecParameters::Audio(audio) => {
            let codec = match audio.codec {
                sound_ids::CODEC_ID_OPUS => SoundCodec::Opus,
                sound_ids::CODEC_ID_VORBIS => SoundCodec::Vorbis,
                sound_ids::CODEC_ID_FLAC => SoundCodec::Flac,
                sound_ids::CODEC_ID_MP3 => SoundCodec::Mp3,
                id if is_pcm(id) => SoundCodec::Pcm,
                _ => SoundCodec::Other,
            };
            let channels = audio.channels.as_ref().map_or(0, |c| c.count());
            let channels = u16::try_from(channels).unwrap_or(u16::MAX);
            TrackKind::Sound {
                codec: if channels > MAX_CHANNELS {
                    SoundCodec::Other
                } else {
                    codec
                },
                rate: audio.sample_rate.unwrap_or(0),
                channels,
                setup: audio.extra_data.as_deref().unwrap_or_default().to_vec(),
            }
        }
        CodecParameters::Video(video) => {
            let width = u32::from(video.width.unwrap_or(0));
            let height = u32::from(video.height.unwrap_or(0));
            let codec = match video.codec {
                picture_ids::CODEC_ID_AV1
                    if width <= MAX_VIDEO_SIZE && height <= MAX_VIDEO_SIZE =>
                {
                    PictureCodec::Av1
                }
                _ => PictureCodec::Other,
            };
            TrackKind::Picture {
                codec,
                width,
                height,
                setup: video
                    .extra_data
                    .first()
                    .map(|extra| extra.data.to_vec())
                    .unwrap_or_default(),
            }
        }
        _ => return None,
    };
    let time_base = track.time_base;
    Some(Track {
        id: track.id,
        kind,
        duration: track
            .duration
            .map(|d| seconds(time_base, i64::try_from(d.get()).unwrap_or(i64::MAX))),
        delay: track.delay.unwrap_or(0),
        params,
        time_base,
    })
}

/// The `av1C` record of an MP4's `av01` sample entry, found in the bytes
/// its reader read: the entry is the box `av01` whose 78 bytes of
/// visual sample entry are followed by its boxes, `av1C` among them.
fn mp4_av1_config(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut from = 0;
    while let Some(i) = find(&bytes[from..], b"av01") {
        let at = from + i;
        from = at + 4;
        if at < 4 {
            continue;
        }
        let size = u32::from_be_bytes(bytes[at - 4..at].try_into().ok()?) as usize;
        let end = (at - 4).saturating_add(size).min(bytes.len());
        let Some(entry) = bytes.get(at + 4 + 78..end) else {
            continue;
        };
        let mut i = 0;
        while i + 8 <= entry.len() {
            let len = u32::from_be_bytes(entry[i..i + 4].try_into().ok()?) as usize;
            if len < 8 || i + len > entry.len() {
                break;
            }
            if &entry[i + 4..i + 8] == b"av1C" {
                return Some(entry[i + 8..i + len].to_vec());
            }
            i += len;
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn is_pcm(id: symphonia::core::codecs::audio::AudioCodecId) -> bool {
    use sound_ids::*;
    [
        CODEC_ID_PCM_S8,
        CODEC_ID_PCM_U8,
        CODEC_ID_PCM_S16LE,
        CODEC_ID_PCM_S16BE,
        CODEC_ID_PCM_S24LE,
        CODEC_ID_PCM_S24BE,
        CODEC_ID_PCM_S32LE,
        CODEC_ID_PCM_S32BE,
        CODEC_ID_PCM_F32LE,
        CODEC_ID_PCM_F32BE,
        CODEC_ID_PCM_F64LE,
        CODEC_ID_PCM_F64BE,
        CODEC_ID_PCM_ALAW,
        CODEC_ID_PCM_MULAW,
    ]
    .contains(&id)
}

impl Demuxer {
    /// Reads `source`'s container: its format told by its bytes, `hint`
    /// helping; its tracks.
    pub fn open(source: Box<dyn ByteSource>, hint: &Hint) -> Result<Demuxer, Error> {
        guarded(|| {
            let mut probe_hint = symphonia::core::formats::probe::Hint::new();
            if let Some(mime) = &hint.mime {
                probe_hint.mime_type(mime);
            }
            if let Some(extension) = &hint.extension {
                probe_hint.with_extension(extension);
            }
            let tap: Tap = std::sync::Arc::new(std::sync::Mutex::new(Some(Vec::new())));
            let stream = MediaSourceStream::new(
                Box::new(Adapter {
                    source,
                    tap: tap.clone(),
                }),
                MediaSourceStreamOptions::default(),
            );
            let reader = symphonia::default::get_probe().probe(
                &probe_hint.clone(),
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )?;
            let tapped = tap
                .lock()
                .ok()
                .and_then(|mut t| t.take())
                .unwrap_or_default();
            let mut tracks: Vec<Track> = reader
                .tracks()
                .iter()
                .take(MAX_TRACKS)
                .filter_map(track_of)
                .collect();
            // symphonia reads an MP4's `av01` sample entry but not its
            // configuration: a picture track it names no codec for is AV1
            // when the file's sample description is `av01`.
            if let Some(config) = mp4_av1_config(&tapped) {
                for track in &mut tracks {
                    if let TrackKind::Picture {
                        codec,
                        width,
                        height,
                        setup,
                    } = &mut track.kind
                        && let CodecParameters::Video(params) = &track.params
                        && params.codec == symphonia::core::codecs::video::CODEC_ID_NULL_VIDEO
                        && *width <= MAX_VIDEO_SIZE
                        && *height <= MAX_VIDEO_SIZE
                    {
                        *codec = PictureCodec::Av1;
                        setup.clone_from(&config);
                    }
                }
            }
            if tracks.is_empty() {
                return Err(Error::Unsupported("no sound or picture track".to_owned()));
            }
            let info = reader.media_info();
            let duration = info
                .duration
                .map(|d| seconds(info.time_base, i64::try_from(d.get()).unwrap_or(i64::MAX)))
                .or_else(|| {
                    tracks
                        .iter()
                        .filter_map(|t| t.duration)
                        .fold(None, |most: Option<f64>, d| {
                            Some(most.map_or(d, |m| m.max(d)))
                        })
                })
                .filter(|d| d.is_finite() && *d >= 0.0);
            Ok(Demuxer {
                reader: Some(reader),
                hint: probe_hint,
                tracks,
                duration,
                pending: VecDeque::new(),
            })
        })
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The first track of sound or of pictures this crate decodes.
    pub fn first_sound(&self) -> Option<&Track> {
        self.tracks.iter().find(|t| t.is_sound() && t.decodable())
    }

    pub fn first_picture(&self) -> Option<&Track> {
        self.tracks.iter().find(|t| t.is_picture() && t.decodable())
    }

    /// The media's length in seconds, when the container says.
    pub fn duration(&self) -> Option<f64> {
        self.duration
    }

    /// The next packet of any track, or `None` at the end.
    pub fn next_packet(&mut self) -> Result<Option<Packet>, Error> {
        if let Some(packet) = self.pending.pop_front() {
            return Ok(Some(packet));
        }
        self.read_packet()
    }

    fn read_packet(&mut self) -> Result<Option<Packet>, Error> {
        let Demuxer { reader, tracks, .. } = self;
        let reader = reader.as_mut().ok_or(Error::Panicked)?;
        guarded(|| {
            loop {
                let Some(packet) = reader.next_packet()? else {
                    return Ok(None);
                };
                let Some(track) = tracks.iter().find(|t| t.id == packet.track_id) else {
                    continue;
                };
                if packet.data.len() > MAX_PACKET_BYTES {
                    return Err(Error::TooLarge("a packet".to_owned()));
                }
                let time = seconds(track.time_base, packet.pts.get());
                return Ok(Some(Packet {
                    track: packet.track_id,
                    time,
                    duration: seconds(
                        track.time_base,
                        i64::try_from(packet.dur.get()).unwrap_or(i64::MAX),
                    ),
                    trim_start: u32::try_from(packet.trim_start.get()).unwrap_or(u32::MAX),
                    trim_end: u32::try_from(packet.trim_end.get()).unwrap_or(u32::MAX),
                    timestamp: packet.pts.get(),
                    ticks: packet.dur.get(),
                    data: packet.data.into_vec(),
                }));
            }
        })
    }

    /// Seeks to `time` seconds by the track `track`'s packets: to the
    /// packet a decoder must start from to reach `time`, at or before it,
    /// for a picture track its keyframe. The time that packet is at.
    pub fn seek(&mut self, time: f64, track: u32) -> Result<f64, Error> {
        let picture = self
            .tracks
            .iter()
            .find(|t| t.id == track)
            .is_some_and(Track::is_picture);
        self.pending.clear();
        if !picture {
            return self.seek_once(time, track);
        }
        // Back off a second at a time, then further, until the track's
        // first packet is a keyframe; the start at the latest.
        let mut at = time.max(0.0);
        let mut back = 1.0;
        loop {
            self.pending.clear();
            self.seek_once(at, track)?;
            let mut ahead = VecDeque::new();
            let first = loop {
                match self.read_packet()? {
                    Some(packet) => {
                        let mine = packet.track == track;
                        let key = mine && crate::video::is_key(&packet.data);
                        let time = packet.time;
                        ahead.push_back(packet);
                        if mine {
                            break Some((key, time));
                        }
                    }
                    None => break None,
                }
            };
            match first {
                Some((true, start)) => {
                    self.pending = ahead;
                    return Ok(start);
                }
                _ if at <= 0.0 => {
                    // No keyframe at the start: decode from it anyway.
                    self.pending = ahead;
                    return Ok(0.0);
                }
                _ => {
                    at = (at - back).max(0.0);
                    back *= 2.0;
                }
            }
        }
    }

    /// A seek; one that could not go back (a WebM whose cues its reader
    /// did not find seeks forward alone) is made again from the start.
    fn seek_once(&mut self, time: f64, track: u32) -> Result<f64, Error> {
        let at = self.seek_reader(time, track)?;
        if at > time + 1e-3 {
            self.restart()?;
            return self.seek_reader(time, track);
        }
        Ok(at)
    }

    /// The container read again from its first byte.
    fn restart(&mut self) -> Result<(), Error> {
        use std::io::Seek;
        let reader = self.reader.take().ok_or(Error::Panicked)?;
        let hint = self.hint.clone();
        let reader = guarded(move || {
            let mut stream = reader.into_inner();
            stream
                .seek(std::io::SeekFrom::Start(0))
                .map_err(|_| Error::Ended)?;
            Ok(symphonia::default::get_probe().probe(
                &hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )?)
        })?;
        self.reader = Some(reader);
        Ok(())
    }

    fn seek_reader(&mut self, time: f64, track: u32) -> Result<f64, Error> {
        let Demuxer { reader, tracks, .. } = self;
        let reader = reader.as_mut().ok_or(Error::Panicked)?;
        guarded(|| {
            let time = Time::try_from_secs_f64(time.max(0.0))
                .ok_or_else(|| Error::Malformed("a seek's time".to_owned()))?;
            let seeked = reader.seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(track),
                },
            )?;
            let base = tracks
                .iter()
                .find(|t| t.id == seeked.track_id)
                .and_then(|t| t.time_base);
            Ok(seconds(base, seeked.actual_ts.get()))
        })
    }
}
