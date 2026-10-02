//! A stylesheet's bytes decoded into text (css-syntax-3 §3.2): a byte
//! order mark, else the fallback encoding the transport layer, an
//! `@charset` declaration or the referring document gives, else UTF-8.

use encoding::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

/// How far into a stylesheet an `@charset` declaration may end.
pub const CHARSET_SCAN_LENGTH: usize = 1024;

/// *Determine the fallback encoding* of a stylesheet:
///
/// 1. `transport`, the encoding the `Content-Type` charset names;
/// 2. the one an `@charset "…";` at the very start names, written
///    exactly so and ending within the first 1024 bytes, UTF-16 read as
///    UTF-8;
/// 3. `environment`, the referring document's: for a `<link>`, its
///    `charset` attribute's or the document's encoding; for an
///    `@import`, the importing sheet's;
/// 4. UTF-8.
pub fn determine_fallback_encoding(
    bytes: &[u8],
    transport: Option<Encoding>,
    environment: Option<Encoding>,
) -> Encoding {
    if let Some(encoding) = transport {
        return encoding;
    }
    if let Some(encoding) = charset_declaration(bytes) {
        return if encoding == UTF_16BE || encoding == UTF_16LE {
            UTF_8
        } else {
            encoding
        };
    }
    environment.unwrap_or(UTF_8)
}

/// The encoding an `@charset "…";` at the start of `bytes` names: the
/// bytes `@charset "`, then any bytes up to 0x7F but `"`, then `";`,
/// all within the first [`CHARSET_SCAN_LENGTH`] bytes. No other spelling
/// counts: not single quotes, more spaces, or a comment.
fn charset_declaration(bytes: &[u8]) -> Option<Encoding> {
    let bytes = &bytes[..bytes.len().min(CHARSET_SCAN_LENGTH)];
    let rest = bytes.strip_prefix(b"@charset \"")?;
    let length = rest.iter().position(|&b| b == b'"' || b > 0x7F)?;
    if rest[length] != b'"' || rest.get(length + 1) != Some(&b';') {
        return None;
    }
    Encoding::for_label_bytes(&rest[..length])
}

/// *Decode* a stylesheet's bytes: with the encoding a byte order mark
/// names, or else with the fallback encoding. Returns the text and the
/// encoding it was decoded with, which an `@import` inside passes on as
/// its environment encoding.
///
/// An encoding the `encoding` crate cannot decode yet (a legacy CJK
/// one) decodes as UTF-8, and UTF-8 is returned.
pub fn decode_stylesheet(
    bytes: &[u8],
    transport: Option<Encoding>,
    environment: Option<Encoding>,
) -> (String, Encoding) {
    encoding::decode(
        bytes,
        determine_fallback_encoding(bytes, transport, environment),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding::{GBK, ISO_8859_2, KOI8_R, WINDOWS_1252};

    fn fallback(bytes: &[u8]) -> &'static str {
        determine_fallback_encoding(bytes, None, None).name()
    }

    #[test]
    fn the_order_of_the_hints() {
        let sheet = b"@charset \"koi8-r\"; a {}";
        assert_eq!(
            determine_fallback_encoding(sheet, Some(ISO_8859_2), Some(GBK)),
            ISO_8859_2
        );
        assert_eq!(determine_fallback_encoding(sheet, None, Some(GBK)), KOI8_R);
        assert_eq!(determine_fallback_encoding(b"a {}", None, Some(GBK)), GBK);
        assert_eq!(determine_fallback_encoding(b"a {}", None, None), UTF_8);
    }

    #[test]
    fn charset_must_be_written_exactly() {
        assert_eq!(fallback(b"@charset \"koi8-r\";"), "KOI8-R");
        assert_eq!(fallback(b"@charset \"KOI8-R\";"), "KOI8-R");
        // The label is trimmed as any label is.
        assert_eq!(fallback(b"@charset \" latin1 \";"), "windows-1252");
        assert_eq!(fallback(b"@charset \"\";"), "UTF-8");
        for wrong in [
            &b"@charset 'koi8-r';"[..],
            b"@charset  \"koi8-r\";",
            b"@CHARSET \"koi8-r\";",
            b" @charset \"koi8-r\";",
            b"@charset \"koi8-r\" ;",
            b"@charset \"koi8-r\"",
            b"@charset \"koi8-r",
            b"/**/@charset \"koi8-r\";",
            b"@charset \"koi8\xC3\xA9\";",
            b"@charset \"bogus\";",
        ] {
            assert_eq!(
                fallback(wrong),
                "UTF-8",
                "{}",
                String::from_utf8_lossy(wrong)
            );
        }
    }

    #[test]
    fn utf16_declared_in_ascii_means_utf8() {
        assert_eq!(fallback(b"@charset \"utf-16\";"), "UTF-8");
        assert_eq!(fallback(b"@charset \"utf-16be\";"), "UTF-8");
        // The transport layer's UTF-16 stands: it is not read from the
        // bytes it describes.
        let sheet = b"a\x00{\x00}\x00";
        assert_eq!(
            determine_fallback_encoding(sheet, Some(UTF_16LE), None),
            UTF_16LE
        );
    }

    #[test]
    fn the_declaration_ends_within_1024_bytes() {
        let declaration = |label_length: usize| {
            let mut bytes = b"@charset \"".to_vec();
            bytes.extend(std::iter::repeat_n(b' ', label_length - 6));
            bytes.extend_from_slice(b"koi8-r\";");
            bytes
        };
        let fits = CHARSET_SCAN_LENGTH - b"@charset \"\";".len();
        assert_eq!(fallback(&declaration(fits)), "KOI8-R");
        assert_eq!(fallback(&declaration(fits + 1)), "UTF-8");
    }

    #[test]
    fn decoding() {
        let (text, encoding) = decode_stylesheet(
            b"@charset \"latin1\"; p::after { content: '\xE9' }",
            None,
            None,
        );
        assert_eq!(encoding, WINDOWS_1252);
        assert!(text.ends_with("content: 'é' }"));
        // A byte order mark wins over every hint.
        let (text, encoding) = decode_stylesheet(
            b"\xEF\xBB\xBF@charset \"latin1\"; '\xC3\xA9'",
            Some(KOI8_R),
            None,
        );
        assert_eq!(encoding, UTF_8);
        assert_eq!(text, "@charset \"latin1\"; 'é'");
        // An encoding not decoded yet reads as UTF-8.
        let (text, encoding) = decode_stylesheet(b"a\xC3\xA9", Some(GBK), None);
        assert_eq!((text.as_str(), encoding), ("aé", UTF_8));
    }
}
