//! `data:` URLs: Fetch's data: URL processor (Fetch §6), and Infra's
//! forgiving-base64 decode (Infra §7) for the bodies marked `;base64`.

use crate::Url;
use crate::infra::isomorphic_decode;
use crate::mime::MimeType;
use crate::url::percent::percent_decode;

/// What a `data:` URL holds: a MIME type and a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataUrl {
    pub mime_type: MimeType,
    pub body: Vec<u8>,
}

/// *The data: URL processor*: the MIME type before the first comma and
/// the body after it, percent-decoded and, after `;base64`, decoded
/// from base64. `None` is failure: no comma, or a body that is not
/// base64 when it says it is.
///
/// # Panics
///
/// When `url`'s scheme is not `data`.
pub fn process(url: &Url) -> Option<DataUrl> {
    assert_eq!(url.scheme(), "data", "a data: URL");
    // The serialization is ASCII, so byte offsets are code point ones.
    let input = &url.without_fragment()["data:".len()..];
    let (mime_type, encoded_body) = input.split_once(',')?;
    let mut mime_type = mime_type.trim_matches(|c: char| c.is_ascii_whitespace());
    let mut body = percent_decode(encoded_body.as_bytes());
    if let Some(without) = without_base64(mime_type) {
        body = forgiving_base64_decode(&isomorphic_decode(&body))?;
        mime_type = without;
    }
    let mime_type = if mime_type.starts_with(';') {
        MimeType::parse(&format!("text/plain{mime_type}"))
    } else {
        MimeType::parse(mime_type)
    };
    Some(DataUrl {
        mime_type: mime_type.unwrap_or_else(|| MimeType::known("text/plain;charset=US-ASCII")),
        body,
    })
}

/// `mime_type` without its `;base64`, when it ends in `;`, any spaces,
/// and `base64` in any case.
fn without_base64(mime_type: &str) -> Option<&str> {
    let split = mime_type.len().checked_sub("base64".len())?;
    let (rest, base64) = mime_type.split_at_checked(split)?;
    if !base64.eq_ignore_ascii_case("base64") {
        return None;
    }
    rest.trim_end_matches(' ').strip_suffix(';')
}

/// *Forgiving-base64 decode*: ASCII whitespace ignored, the padding
/// optional, and leftover bits after the last whole byte dropped. `None`
/// is failure: a length that leaves one character over, padding out of
/// place, or a character outside the base64 alphabet.
pub fn forgiving_base64_decode(data: &str) -> Option<Vec<u8>> {
    let mut data: Vec<u8> = data
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        // Any other non-ASCII character is outside the alphabet, which
        // fails below; it fails here instead, with the same result.
        .map(|c| u8::try_from(c).ok().filter(u8::is_ascii))
        .collect::<Option<_>>()?;
    if data.len().is_multiple_of(4) {
        if data.ends_with(b"==") {
            data.truncate(data.len() - 2);
        } else if data.ends_with(b"=") {
            data.truncate(data.len() - 1);
        }
    }
    if data.len() % 4 == 1 {
        return None;
    }
    let mut output = Vec::with_capacity(data.len() / 4 * 3 + 2);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in data {
        let n = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = buffer << 6 | u32::from(n);
        bits += 6;
        if bits == 24 {
            output.extend_from_slice(&buffer.to_be_bytes()[1..]);
            buffer = 0;
            bits = 0;
        }
    }
    // 12 bits left are one byte and four dropped; 18 are two and two.
    match bits {
        12 => output.push((buffer >> 4) as u8),
        18 => output.extend_from_slice(&((buffer >> 2) as u16).to_be_bytes()),
        _ => debug_assert_eq!(bits, 0),
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(input: &str) -> Option<(String, Vec<u8>)> {
        let url = Url::parse(input).expect("parses");
        process(&url).map(|d| (d.mime_type.to_string(), d.body))
    }

    #[test]
    fn the_type_and_the_body() {
        assert_eq!(
            data("data:,Hello%2C%20World!").unwrap(),
            (
                "text/plain;charset=US-ASCII".to_owned(),
                b"Hello, World!".to_vec()
            )
        );
        assert_eq!(
            data("data:text/html;charset=utf-8,%3Cp%3E#frag").unwrap(),
            ("text/html;charset=utf-8".to_owned(), b"<p>".to_vec())
        );
        assert_eq!(
            data("data:;charset=utf-8,x").unwrap().0,
            "text/plain;charset=utf-8"
        );
        assert_eq!(data("data:text/plain"), None);
    }

    #[test]
    fn base64_bodies() {
        assert_eq!(
            data("data:image/gif;BASE64,R0lG").unwrap(),
            ("image/gif".to_owned(), b"GIF".to_vec())
        );
        assert_eq!(data("data:text/plain;  base64,YQ").unwrap().1, b"a");
        assert_eq!(data("data:;base64,%59%51").unwrap().1, b"a");
        assert_eq!(data("data:;base64,a"), None);
        // Not ;base64 at the end, so not decoded.
        assert_eq!(data("data:base64,YQ").unwrap().1, b"YQ");
        assert_eq!(data("data:x/y;base64x,YQ").unwrap().1, b"YQ");
    }

    #[test]
    fn without_base64_only_strips_the_marker() {
        assert_eq!(without_base64("text/plain;base64"), Some("text/plain"));
        assert_eq!(without_base64("text/plain ;  BaSe64"), Some("text/plain "));
        assert_eq!(without_base64(";base64"), Some(""));
        assert_eq!(without_base64("base64"), None);
        assert_eq!(without_base64("text/plain;charset=x;nobase64"), None);
        assert_eq!(without_base64("ase64"), None);
    }

    #[test]
    #[should_panic(expected = "a data: URL")]
    fn only_data_urls() {
        process(&Url::parse("https://example.com/").unwrap());
    }

    #[test]
    fn base64_round_trips() {
        let mut state = 0x9E37_79B9_u32;
        for length in 0..64 {
            let bytes: Vec<u8> = (0..length)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect();
            let encoded = encode(&bytes);
            assert_eq!(forgiving_base64_decode(&encoded).unwrap(), bytes);
            assert_eq!(
                forgiving_base64_decode(encoded.trim_end_matches('=')).unwrap(),
                bytes
            );
        }
    }

    /// RFC 4648 base64, padded, for the round trip.
    fn encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
}
