//! Opus (RFC 6716, RFC 8251) by `opus-decoder`, its stream described by
//! its identification header (RFC 7845 section 5.1), which every
//! container here carries as the track's setup.

use opus_decoder::OpusMultistreamDecoder;

use crate::Error;

/// The most frames an Opus packet decodes to: 120 ms at 48 kHz.
const MAX_PACKET_FRAMES: usize = 5_760;

/// An identification header's fields.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Head {
    pub channels: u16,
    pub pre_skip: u32,
    /// The output gain, as a factor.
    pub gain: f32,
    pub streams: usize,
    pub coupled: usize,
    pub mapping: Vec<u8>,
}

/// Reads an `OpusHead` (version 1, its fields little-endian); also the
/// `dOps` box's payload of MP4, which is the same fields big-endian after
/// a version byte of 0, with or without that magic before it.
pub(crate) fn head(setup: &[u8], channels_hint: u16) -> Result<Head, Error> {
    let bad = || Error::Malformed("an Opus header".to_owned());
    let (channels, pre_skip, gain, family, rest) = if let Some(rest) =
        setup.strip_prefix(b"OpusHead")
        && rest.first() != Some(&0)
    {
        if rest.len() < 11 {
            return Err(bad());
        }
        (
            u16::from(rest[1]),
            u32::from(u16::from_le_bytes([rest[2], rest[3]])),
            i16::from_le_bytes([rest[8], rest[9]]),
            rest[10],
            &rest[11..],
        )
    } else if let Some(setup) = setup.strip_prefix(b"OpusHead").or(Some(setup))
        && setup.len() >= 11
        && setup[0] == 0
    {
        // A `dOps` (version 0): symphonia gives an MP4's after the
        // "OpusHead" magic, its fields still big-endian.
        (
            u16::from(setup[1]),
            u32::from(u16::from_be_bytes([setup[2], setup[3]])),
            i16::from_be_bytes([setup[8], setup[9]]),
            setup[10],
            &setup[11..],
        )
    } else if setup.is_empty() && (1..=2).contains(&channels_hint) {
        (channels_hint, 0, 0, 0, &setup[..0])
    } else {
        return Err(bad());
    };
    if channels == 0 {
        return Err(bad());
    }
    let (streams, coupled, mapping) = if family == 0 {
        if channels > 2 {
            return Err(bad());
        }
        (1, usize::from(channels == 2), (0..channels as u8).collect())
    } else {
        if rest.len() < 2 + usize::from(channels) {
            return Err(bad());
        }
        let streams = usize::from(rest[0]);
        let coupled = usize::from(rest[1]);
        if streams == 0 || coupled > streams {
            return Err(bad());
        }
        (
            streams,
            coupled,
            rest[2..2 + usize::from(channels)].to_vec(),
        )
    };
    Ok(Head {
        channels,
        pre_skip,
        gain: 10f32.powf(f32::from(gain) / (20.0 * 256.0)),
        streams,
        coupled,
        mapping,
    })
}

/// A decoder of one Opus stream, at 48 kHz.
pub(crate) struct Opus {
    pub head: Head,
    decoder: OpusMultistreamDecoder,
    scratch: Vec<f32>,
}

impl Opus {
    pub(crate) fn new(setup: &[u8], channels_hint: u16) -> Result<Opus, Error> {
        let head = head(setup, channels_hint)?;
        let decoder = OpusMultistreamDecoder::new(
            48_000,
            usize::from(head.channels),
            head.streams,
            head.coupled,
            &head.mapping,
        )
        .map_err(|e| Error::Malformed(format!("Opus: {e:?}")))?;
        let scratch = vec![0.0; MAX_PACKET_FRAMES * usize::from(head.channels)];
        Ok(Opus {
            head,
            decoder,
            scratch,
        })
    }

    /// One packet's samples, interleaved.
    pub(crate) fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>, Error> {
        let frames = self
            .decoder
            .decode_float(packet, &mut self.scratch, false)
            .map_err(|e| Error::Malformed(format!("Opus: {e:?}")))?;
        let channels = usize::from(self.head.channels);
        let mut out = self.scratch[..frames * channels].to_vec();
        if self.head.gain != 1.0 {
            for s in &mut out {
                *s *= self.head.gain;
            }
        }
        Ok(out)
    }

    pub(crate) fn reset(&mut self) {
        if let Ok(decoder) = OpusMultistreamDecoder::new(
            48_000,
            usize::from(self.head.channels),
            self.head.streams,
            self.head.coupled,
            &self.head.mapping,
        ) {
            self.decoder = decoder;
        }
    }
}
