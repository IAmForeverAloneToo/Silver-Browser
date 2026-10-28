//! `TextDecoder`'s decoding for scripts (gap batch A): the encoding
//! crate's decoders, a stream's kept between its calls by the number the
//! bindings gave it.

use std::collections::BTreeMap;

use encoding::{Decoder, DecoderMode, Encoding};
use js::Answer;

/// The most streams a realm decodes at once; past it the oldest's state
/// is dropped, its next piece decoded as the start of a stream.
pub(crate) const MAX_TEXT_STREAMS: usize = 1_000;

/// The name, lower case, of the encoding `label` names, which
/// `TextDecoder` reports as its `encoding`; `null` for a label of none,
/// of replacement, or of one the crate cannot decode yet.
pub(super) fn name(label: &str) -> Answer {
    let encoding = Encoding::for_label(label)
        .filter(|&encoding| encoding != encoding::REPLACEMENT && encoding.can_decode());
    Answer::MaybeString(encoding.map(|encoding| encoding.name().to_ascii_lowercase()))
}

/// The encoding a MIME type's `charset` names, as [`name`] answers;
/// `null` for a type without one.
pub(super) fn mime_charset(kind: &str) -> Answer {
    let charset =
        net::MimeType::parse(kind).and_then(|kind| kind.parameter("charset").map(str::to_owned));
    match charset {
        Some(label) => name(&label),
        None => Answer::MaybeString(None),
    }
}

/// A realm's streams being decoded, by their numbers.
#[derive(Default)]
pub(crate) struct TextDecoders {
    streams: BTreeMap<u32, Decoder>,
}

impl TextDecoders {
    /// `bytes` decoded in the encoding `name` names, as stream `id`'s
    /// next piece, which `stream` says is not its last; `null` for bytes
    /// a `fatal` decoder fails on, which ends the stream.
    pub(super) fn decode(
        &mut self,
        id: u32,
        name: &str,
        bytes: &[u8],
        fatal: bool,
        stream: bool,
    ) -> Answer {
        let mode = match fatal {
            true => DecoderMode::Fatal,
            false => DecoderMode::Replacement,
        };
        let decoder = self
            .streams
            .remove(&id)
            .or_else(|| Encoding::for_label(name)?.new_decoder(mode));
        let Some(mut decoder) = decoder else {
            return Answer::MaybeString(None);
        };
        let mut out = String::new();
        if decoder.decode(bytes, !stream, &mut out).is_err() {
            return Answer::MaybeString(None);
        }
        if stream {
            if self.streams.len() >= MAX_TEXT_STREAMS {
                self.streams.pop_first();
            }
            self.streams.insert(id, decoder);
        }
        Answer::MaybeString(Some(out))
    }
}
