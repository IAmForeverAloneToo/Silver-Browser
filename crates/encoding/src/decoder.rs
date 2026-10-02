//! Decoders, as Encoding §8 to §14 write them: state machines fed one
//! byte at a time, here in pieces of any size, with the end of the input
//! said explicitly. Each keeps exactly the state the standard's does, so
//! a sequence split between two pieces decodes as it would whole.

use crate::{Encoding, REPLACEMENT, UTF_8, UTF_16BE, UTF_16LE, X_USER_DEFINED};

/// What a decoder does with bytes that do not decode (Encoding §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderMode {
    /// Each error becomes one U+FFFD, and decoding goes on.
    Replacement,
    /// The first error ends decoding: [`Decoder::decode`] returns
    /// [`DecodeError`], then and on every later call.
    Fatal,
}

/// A fatal decoder met bytes that do not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError;

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the bytes do not decode in their encoding")
    }
}

impl std::error::Error for DecodeError {}

/// A decoder for one encoding: [`Encoding::new_decoder`] makes one.
#[derive(Debug, Clone)]
pub struct Decoder {
    encoding: Encoding,
    mode: DecoderMode,
    state: State,
    /// Set at the end of the input, by a fatal error, and by the
    /// replacement decoder's one error: nothing more is decoded.
    finished: bool,
    /// Set by a fatal error: every later call fails too.
    failed: bool,
}

#[derive(Debug, Clone)]
enum State {
    Utf8(Utf8),
    Utf16 {
        big_endian: bool,
        lead_byte: Option<u8>,
        lead_surrogate: Option<u16>,
    },
    SingleByte(&'static [u16; 128]),
    XUserDefined,
    Replacement,
}

/// UTF-8's decoder state (Encoding §8.1.1).
#[derive(Debug, Clone)]
struct Utf8 {
    code_point: u32,
    bytes_seen: u8,
    bytes_needed: u8,
    lower_boundary: u8,
    upper_boundary: u8,
}

impl Utf8 {
    const fn new() -> Self {
        Utf8 {
            code_point: 0,
            bytes_seen: 0,
            bytes_needed: 0,
            lower_boundary: 0x80,
            upper_boundary: 0xBF,
        }
    }
}

/// One step's result, the standard's handler results short of
/// "finished".
enum Step {
    /// The byte was taken into the state.
    Continue,
    /// A code point is complete.
    Char(char),
    /// The bytes so far do not decode.
    Error,
    /// The bytes so far do not decode, and the current byte is to be
    /// read again ("restore byte to ioQueue").
    ErrorAgain,
}

impl Decoder {
    pub(crate) fn new(encoding: Encoding, mode: DecoderMode) -> Option<Decoder> {
        let state = if encoding == UTF_8 {
            State::Utf8(Utf8::new())
        } else if encoding == UTF_16BE || encoding == UTF_16LE {
            State::Utf16 {
                big_endian: encoding == UTF_16BE,
                lead_byte: None,
                lead_surrogate: None,
            }
        } else if let Some(table) = encoding.single_byte_table() {
            State::SingleByte(table)
        } else if encoding == X_USER_DEFINED {
            State::XUserDefined
        } else if encoding == REPLACEMENT {
            State::Replacement
        } else {
            return None;
        };
        Some(Decoder {
            encoding,
            mode,
            state,
            finished: false,
            failed: false,
        })
    }

    /// The encoding this decoder decodes.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Decodes `input`, appending the text to `out`. `last` says the
    /// input ends with these bytes: a sequence still incomplete then is
    /// an error. A decoder given `last` decodes nothing more.
    ///
    /// In [`DecoderMode::Fatal`] the first error returns
    /// [`DecodeError`], the text before it already appended; every later
    /// call returns it too. In [`DecoderMode::Replacement`] this never
    /// fails.
    pub fn decode(
        &mut self,
        input: &[u8],
        last: bool,
        out: &mut String,
    ) -> Result<(), DecodeError> {
        if self.failed {
            return Err(DecodeError);
        }
        if self.finished {
            return Ok(());
        }
        let result = match &mut self.state {
            State::Utf8(state) => decode_utf8(state, self.mode, input, out),
            State::Utf16 {
                big_endian,
                lead_byte,
                lead_surrogate,
            } => decode_utf16(
                *big_endian,
                lead_byte,
                lead_surrogate,
                self.mode,
                input,
                out,
            ),
            State::SingleByte(table) => decode_single_byte(table, self.mode, input, out),
            State::XUserDefined => {
                out.extend(input.iter().map(|&b| x_user_defined(b)));
                Ok(())
            }
            State::Replacement => {
                if input.is_empty() {
                    Ok(())
                } else {
                    // One error for the whole input, then nothing more.
                    self.finished = true;
                    on_error(self.mode, out)
                }
            }
        };
        if result.is_err() {
            self.finished = true;
            self.failed = true;
            return result;
        }
        if last {
            let pending = match &mut self.state {
                State::Utf8(state) => std::mem::replace(state, Utf8::new()).bytes_needed != 0,
                State::Utf16 {
                    lead_byte,
                    lead_surrogate,
                    ..
                } => lead_byte.take().is_some() | lead_surrogate.take().is_some(),
                _ => false,
            };
            self.finished = true;
            if pending {
                let result = on_error(self.mode, out);
                self.failed = result.is_err();
                return result;
            }
        }
        Ok(())
    }
}

/// x-user-defined's decoder (Encoding §14.5): ASCII as itself, every
/// other byte to U+F780 + (byte - 0x80).
fn x_user_defined(byte: u8) -> char {
    if byte < 0x80 {
        byte as char
    } else {
        char::from_u32(0xF780 + u32::from(byte) - 0x80).expect("U+F780 to U+F7FF are scalar values")
    }
}

/// Pushes U+FFFD in replacement mode, or fails in fatal mode.
fn on_error(mode: DecoderMode, out: &mut String) -> Result<(), DecodeError> {
    match mode {
        DecoderMode::Replacement => {
            out.push('\u{FFFD}');
            Ok(())
        }
        DecoderMode::Fatal => Err(DecodeError),
    }
}

/// UTF-8's decoder over `input`. Between sequences, the longest run that
/// is valid UTF-8 is copied at once; the standard's handler takes the
/// bytes from the first that is not.
fn decode_utf8(
    state: &mut Utf8,
    mode: DecoderMode,
    input: &[u8],
    out: &mut String,
) -> Result<(), DecodeError> {
    let mut at = 0;
    while at < input.len() {
        if state.bytes_needed == 0 {
            let rest = &input[at..];
            let valid = match std::str::from_utf8(rest) {
                Ok(text) => {
                    out.push_str(text);
                    return Ok(());
                }
                Err(e) => e.valid_up_to(),
            };
            out.push_str(std::str::from_utf8(&rest[..valid]).expect("valid up to here"));
            at += valid;
        }
        // At least one byte is left: the start of a sequence that is
        // invalid or incomplete, or the rest of one begun earlier.
        match utf8_step(state, input[at]) {
            Step::Continue => at += 1,
            Step::Char(c) => {
                out.push(c);
                at += 1;
            }
            Step::Error => {
                on_error(mode, out)?;
                at += 1;
            }
            Step::ErrorAgain => on_error(mode, out)?,
        }
    }
    Ok(())
}

/// UTF-8's handler for one byte (Encoding §8.1.1, steps 3 to 9).
fn utf8_step(state: &mut Utf8, byte: u8) -> Step {
    if state.bytes_needed == 0 {
        match byte {
            0x00..=0x7F => return Step::Char(byte as char),
            0xC2..=0xDF => {
                state.bytes_needed = 1;
                state.code_point = u32::from(byte & 0x1F);
            }
            0xE0..=0xEF => {
                if byte == 0xE0 {
                    state.lower_boundary = 0xA0;
                }
                if byte == 0xED {
                    state.upper_boundary = 0x9F;
                }
                state.bytes_needed = 2;
                state.code_point = u32::from(byte & 0xF);
            }
            0xF0..=0xF4 => {
                if byte == 0xF0 {
                    state.lower_boundary = 0x90;
                }
                if byte == 0xF4 {
                    state.upper_boundary = 0x8F;
                }
                state.bytes_needed = 3;
                state.code_point = u32::from(byte & 0x7);
            }
            _ => return Step::Error,
        }
        return Step::Continue;
    }
    if !(state.lower_boundary..=state.upper_boundary).contains(&byte) {
        *state = Utf8::new();
        return Step::ErrorAgain;
    }
    state.lower_boundary = 0x80;
    state.upper_boundary = 0xBF;
    state.code_point = (state.code_point << 6) | u32::from(byte & 0x3F);
    state.bytes_seen += 1;
    if state.bytes_seen != state.bytes_needed {
        return Step::Continue;
    }
    let code_point = state.code_point;
    *state = Utf8::new();
    // The boundaries above rule out surrogates, overlong forms and
    // anything past U+10FFFF.
    Step::Char(char::from_u32(code_point).expect("the boundaries keep code points scalar"))
}

/// The shared UTF-16 decoder (Encoding §14.2). A lead surrogate followed
/// by anything but a trail surrogate is an error, and the code unit that
/// followed it is read again; reading its two bytes again yields the
/// same code unit, so it is decoded at once with no lead surrogate.
fn decode_utf16(
    big_endian: bool,
    lead_byte: &mut Option<u8>,
    lead_surrogate: &mut Option<u16>,
    mode: DecoderMode,
    input: &[u8],
    out: &mut String,
) -> Result<(), DecodeError> {
    for &byte in input {
        let Some(lead) = lead_byte.take() else {
            *lead_byte = Some(byte);
            continue;
        };
        let unit = if big_endian {
            u16::from(lead) << 8 | u16::from(byte)
        } else {
            u16::from(byte) << 8 | u16::from(lead)
        };
        if let Some(high) = lead_surrogate.take() {
            if (0xDC00..=0xDFFF).contains(&unit) {
                let code_point =
                    0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(unit) - 0xDC00);
                out.push(char::from_u32(code_point).expect("a surrogate pair is a scalar value"));
                continue;
            }
            on_error(mode, out)?;
        }
        match unit {
            0xD800..=0xDBFF => *lead_surrogate = Some(unit),
            0xDC00..=0xDFFF => on_error(mode, out)?,
            _ => out.push(char::from_u32(u32::from(unit)).expect("not a surrogate")),
        }
    }
    Ok(())
}

/// A single-byte decoder (Encoding §9.1): ASCII as itself, every other
/// byte through the index, an error where the index has no entry.
fn decode_single_byte(
    table: &[u16; 128],
    mode: DecoderMode,
    input: &[u8],
    out: &mut String,
) -> Result<(), DecodeError> {
    for &byte in input {
        if byte < 0x80 {
            out.push(byte as char);
            continue;
        }
        match table[usize::from(byte - 0x80)] {
            0 => on_error(mode, out)?,
            code_point => {
                out.push(char::from_u32(u32::from(code_point)).expect("indexes hold scalar values"))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WINDOWS_874, WINDOWS_1252};

    fn whole(encoding: Encoding, mode: DecoderMode, bytes: &[u8]) -> Result<String, DecodeError> {
        let mut out = String::new();
        let mut decoder = Decoder::new(encoding, mode).unwrap();
        decoder.decode(bytes, true, &mut out).map(|()| out)
    }

    fn lossy(encoding: Encoding, bytes: &[u8]) -> String {
        whole(encoding, DecoderMode::Replacement, bytes).unwrap()
    }

    #[test]
    fn utf8_replaces_maximal_subparts() {
        // Unicode's best practice for U+FFFD, which the standard requires.
        assert_eq!(lossy(UTF_8, b"a\x80b"), "a\u{FFFD}b");
        assert_eq!(lossy(UTF_8, b"\xF0\x80\x80"), "\u{FFFD}\u{FFFD}\u{FFFD}");
        assert_eq!(lossy(UTF_8, b"\xF0\x90\x80"), "\u{FFFD}");
        assert_eq!(lossy(UTF_8, b"\xF0\x90\x80a"), "\u{FFFD}a");
        assert_eq!(lossy(UTF_8, b"\xED\xA0\x80"), "\u{FFFD}\u{FFFD}\u{FFFD}");
        assert_eq!(lossy(UTF_8, b"\xC0\xAF"), "\u{FFFD}\u{FFFD}");
        assert_eq!(
            lossy(UTF_8, b"\xF4\x90\x80\x80"),
            "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}"
        );
        assert_eq!(lossy(UTF_8, b"\xE2\x82"), "\u{FFFD}");
        assert_eq!(
            lossy(UTF_8, "\u{10FFFF}\u{20AC}".as_bytes()),
            "\u{10FFFF}\u{20AC}"
        );
    }

    #[test]
    fn a_sequence_split_between_pieces_decodes_as_it_would_whole() {
        let mut decoder = Decoder::new(UTF_8, DecoderMode::Fatal).unwrap();
        let mut out = String::new();
        for &byte in "a\u{20AC}\u{1F600}".as_bytes() {
            decoder.decode(&[byte], false, &mut out).unwrap();
        }
        decoder.decode(&[], true, &mut out).unwrap();
        assert_eq!(out, "a\u{20AC}\u{1F600}");
    }

    #[test]
    fn an_incomplete_sequence_at_the_end_is_one_error() {
        let mut decoder = Decoder::new(UTF_8, DecoderMode::Replacement).unwrap();
        let mut out = String::new();
        decoder.decode(b"a\xF0\x9F", false, &mut out).unwrap();
        assert_eq!(out, "a");
        decoder.decode(b"", true, &mut out).unwrap();
        assert_eq!(out, "a\u{FFFD}");
        // Finished: more input decodes to nothing.
        decoder.decode(b"b", true, &mut out).unwrap();
        assert_eq!(out, "a\u{FFFD}");
    }

    #[test]
    fn fatal_stops_at_the_first_error_and_stays_stopped() {
        let mut decoder = Decoder::new(UTF_8, DecoderMode::Fatal).unwrap();
        let mut out = String::new();
        assert_eq!(
            decoder.decode(b"ab\xFFc", false, &mut out),
            Err(DecodeError)
        );
        assert_eq!(out, "ab");
        assert_eq!(decoder.decode(b"d", true, &mut out), Err(DecodeError));
        assert_eq!(
            whole(UTF_8, DecoderMode::Fatal, b"\xE2\x82"),
            Err(DecodeError)
        );
        // One that ended cleanly decodes nothing more, without failing.
        let mut decoder = Decoder::new(UTF_8, DecoderMode::Fatal).unwrap();
        let mut out = String::new();
        decoder.decode(b"a", true, &mut out).unwrap();
        assert_eq!(decoder.decode(b"b", true, &mut out), Ok(()));
        assert_eq!(out, "a");
    }

    #[test]
    fn utf16_pairs_and_lone_surrogates() {
        assert_eq!(lossy(UTF_16LE, b"a\x00\x3D\xD8\x00\xDE"), "a\u{1F600}");
        assert_eq!(lossy(UTF_16BE, b"\x00a\xD8\x3D\xDE\x00"), "a\u{1F600}");
        // A lead surrogate before a non-surrogate: an error, then the
        // unit after it as itself.
        assert_eq!(lossy(UTF_16LE, b"\x3D\xD8a\x00"), "\u{FFFD}a");
        // Before another lead surrogate, which then pairs.
        assert_eq!(
            lossy(UTF_16LE, b"\x3D\xD8\x3D\xD8\x00\xDE"),
            "\u{FFFD}\u{1F600}"
        );
        // A trail surrogate alone.
        assert_eq!(lossy(UTF_16LE, b"\x00\xDEa\x00"), "\u{FFFD}a");
        // An odd byte at the end, and a lead surrogate with an odd byte
        // after it: one error each time.
        assert_eq!(lossy(UTF_16LE, b"a\x00b"), "a\u{FFFD}");
        assert_eq!(lossy(UTF_16LE, b"\x3D\xD8b"), "\u{FFFD}");
    }

    #[test]
    fn single_byte_through_the_index() {
        assert_eq!(lossy(WINDOWS_1252, b"a\x80\xE9"), "a\u{20AC}\u{E9}");
        // windows-1252's 0x81 is U+0081 in the standard's index.
        assert_eq!(lossy(WINDOWS_1252, b"\x81"), "\u{81}");
        // windows-874 has no 0xDB.
        assert_eq!(lossy(WINDOWS_874, b"\xDB"), "\u{FFFD}");
        assert_eq!(
            whole(WINDOWS_874, DecoderMode::Fatal, b"\xDB"),
            Err(DecodeError)
        );
    }

    #[test]
    fn x_user_defined_and_replacement() {
        assert_eq!(lossy(X_USER_DEFINED, b"a\x80\xFF"), "a\u{F780}\u{F7FF}");
        assert_eq!(lossy(REPLACEMENT, b""), "");
        assert_eq!(lossy(REPLACEMENT, b"anything"), "\u{FFFD}");
        let mut decoder = Decoder::new(REPLACEMENT, DecoderMode::Replacement).unwrap();
        let mut out = String::new();
        decoder.decode(b"a", false, &mut out).unwrap();
        decoder.decode(b"b", true, &mut out).unwrap();
        assert_eq!(out, "\u{FFFD}");
        assert_eq!(
            whole(REPLACEMENT, DecoderMode::Fatal, b"a"),
            Err(DecodeError)
        );
        assert_eq!(
            whole(REPLACEMENT, DecoderMode::Fatal, b""),
            Ok(String::new())
        );
    }
}
