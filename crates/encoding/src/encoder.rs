//! Encoders (Encoding §8.1.2, §9.2, §14.5): UTF-8's, the single-byte
//! encodings' and x-user-defined's. None of them keeps state, but an
//! [`Encoder`] is taken by `&mut` because the legacy CJK encoders to
//! come do (ISO-2022-JP's).

use crate::{Encoding, UTF_8, X_USER_DEFINED};

/// What [`Encoder::encode`] does with a character the encoding cannot
/// represent (Encoding §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderMode {
    /// Stop, and return the character.
    Fatal,
    /// Write it as the decimal character reference `&#N;`, as HTML forms
    /// do, and go on.
    Html,
}

/// A character the encoding cannot represent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmappable(pub char);

impl std::fmt::Display for Unmappable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "U+{:04X} has no byte sequence in the encoding",
            u32::from(self.0)
        )
    }
}

impl std::error::Error for Unmappable {}

/// An encoder for one encoding: [`Encoding::new_encoder`] makes one.
#[derive(Debug, Clone)]
pub struct Encoder {
    kind: Kind,
}

#[derive(Debug, Clone)]
enum Kind {
    Utf8,
    SingleByte(&'static [u16; 128]),
    XUserDefined,
}

impl Encoder {
    pub(crate) fn new(encoding: Encoding) -> Option<Encoder> {
        let kind = if encoding == UTF_8 {
            Kind::Utf8
        } else if let Some(table) = encoding.single_byte_table() {
            Kind::SingleByte(table)
        } else if encoding == X_USER_DEFINED {
            Kind::XUserDefined
        } else {
            return None;
        };
        Some(Encoder { kind })
    }

    /// Appends the bytes for `c` to `out`, or returns [`Unmappable`]
    /// and appends nothing. The standard's *encode or fail* is this, a
    /// character at a time: URL percent-encoding writes its own
    /// replacement for a character that fails.
    pub fn encode_char(&mut self, c: char, out: &mut Vec<u8>) -> Result<(), Unmappable> {
        if c.is_ascii() {
            out.push(c as u8);
            return Ok(());
        }
        match &self.kind {
            Kind::Utf8 => {
                let mut buffer = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                Ok(())
            }
            Kind::SingleByte(table) => {
                // The index pointer for the code point: its first
                // position in the index.
                let code_point = u32::from(c);
                let pointer = table
                    .iter()
                    .position(|&entry| entry != 0 && u32::from(entry) == code_point)
                    .ok_or(Unmappable(c))?;
                out.push(0x80 + pointer as u8);
                Ok(())
            }
            Kind::XUserDefined => match u32::from(c) {
                code_point @ 0xF780..=0xF7FF => {
                    out.push((code_point - 0xF780 + 0x80) as u8);
                    Ok(())
                }
                _ => Err(Unmappable(c)),
            },
        }
    }

    /// Encodes `text`, appending to `out`. In [`EncoderMode::Fatal`] the
    /// first character the encoding cannot represent stops it, the bytes
    /// before it already appended; in [`EncoderMode::Html`] that
    /// character is written as `&#N;` and this never fails.
    pub fn encode(
        &mut self,
        text: &str,
        mode: EncoderMode,
        out: &mut Vec<u8>,
    ) -> Result<(), Unmappable> {
        for c in text.chars() {
            if let Err(unmappable) = self.encode_char(c, out) {
                match mode {
                    EncoderMode::Fatal => return Err(unmappable),
                    EncoderMode::Html => {
                        out.extend_from_slice(format!("&#{};", u32::from(c)).as_bytes());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KOI8_R, WINDOWS_1252};

    fn bytes(encoding: Encoding, text: &str, mode: EncoderMode) -> Result<Vec<u8>, Unmappable> {
        let mut out = Vec::new();
        encoding
            .new_encoder()
            .unwrap()
            .encode(text, mode, &mut out)
            .map(|()| out)
    }

    #[test]
    fn single_byte_through_the_index() {
        assert_eq!(
            bytes(WINDOWS_1252, "a\u{20AC}\u{E9}", EncoderMode::Fatal),
            Ok(b"a\x80\xE9".to_vec())
        );
        assert_eq!(bytes(KOI8_R, "\u{430}", EncoderMode::Fatal), Ok(vec![0xC1]));
    }

    #[test]
    fn fatal_stops_and_html_writes_a_reference() {
        let mut out = Vec::new();
        let mut encoder = WINDOWS_1252.new_encoder().unwrap();
        assert_eq!(
            encoder.encode("ab\u{2603}c", EncoderMode::Fatal, &mut out),
            Err(Unmappable('\u{2603}'))
        );
        assert_eq!(out, b"ab");
        assert_eq!(
            bytes(WINDOWS_1252, "\u{1F600}!", EncoderMode::Html),
            Ok(b"&#128512;!".to_vec())
        );
    }

    #[test]
    fn utf8_and_x_user_defined() {
        assert_eq!(
            bytes(UTF_8, "\u{1F600}", EncoderMode::Fatal),
            Ok("\u{1F600}".as_bytes().to_vec())
        );
        assert_eq!(
            bytes(X_USER_DEFINED, "a\u{F780}\u{F7FF}", EncoderMode::Fatal),
            Ok(b"a\x80\xFF".to_vec())
        );
        assert_eq!(
            bytes(X_USER_DEFINED, "\u{E9}", EncoderMode::Fatal),
            Err(Unmappable('\u{E9}'))
        );
    }

    #[test]
    fn encodings_without_an_encoder() {
        for encoding in [
            crate::UTF_16BE,
            crate::UTF_16LE,
            crate::REPLACEMENT,
            crate::SHIFT_JIS,
        ] {
            assert!(encoding.new_encoder().is_none(), "{encoding:?}");
        }
    }
}
