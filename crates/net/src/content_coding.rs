//! Content codings (Fetch's *handle content codings*, RFC 9110 §8.4.1):
//! a body in `gzip` or `deflate` decoded, as a request's
//! `Accept-Encoding: gzip, deflate` asked for.

use std::io::Read;

use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};

use crate::error::NetworkError;
use crate::headers::HeaderList;

/// `body` with the codings its `Content-Encoding` lists removed, last
/// applied first. A list naming any coding this does not decode leaves
/// the body as it is; an empty body has nothing to decode. Decoding
/// stops past `limit` bytes, so a small body cannot grow without end.
pub(crate) fn decode(
    headers: &HeaderList,
    body: Vec<u8>,
    limit: usize,
) -> Result<Vec<u8>, NetworkError> {
    let Some(codings) = headers.get_decode_split(b"Content-Encoding") else {
        return Ok(body);
    };
    let codings: Vec<String> = codings.iter().map(|c| c.to_ascii_lowercase()).collect();
    let supported = |c: &String| matches!(c.as_str(), "gzip" | "x-gzip" | "deflate" | "identity");
    if body.is_empty() || !codings.iter().all(supported) {
        return Ok(body);
    }
    codings
        .iter()
        .rev()
        .try_fold(body, |body, coding| match coding.as_str() {
            "gzip" | "x-gzip" => read_limited(GzDecoder::new(&body[..]), limit),
            "deflate" if is_zlib(&body) => read_limited(ZlibDecoder::new(&body[..]), limit),
            // Some servers send `deflate` without the zlib wrapper RFC 9110
            // defines it with; browsers read it.
            "deflate" => read_limited(DeflateDecoder::new(&body[..]), limit),
            _ => Ok(body),
        })
}

/// Whether `body` starts with a zlib header: deflate as the method, a
/// window no larger than 32 KiB, and the check bits right.
fn is_zlib(body: &[u8]) -> bool {
    match body {
        [cmf, flg, ..] => {
            cmf & 0x0F == 8 && cmf >> 4 <= 7 && (u16::from(*cmf) << 8 | u16::from(*flg)) % 31 == 0
        }
        _ => false,
    }
}

/// Everything `decoder` gives, at most `limit` bytes.
fn read_limited(decoder: impl Read, limit: usize) -> Result<Vec<u8>, NetworkError> {
    let mut out = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    decoder
        .take(cap)
        .read_to_end(&mut out)
        .map_err(|_| NetworkError::ContentCoding)?;
    if out.len() > limit {
        return Err(NetworkError::BodyTooLarge);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use std::io::Write;

    fn coded(name: &str) -> HeaderList {
        let mut headers = HeaderList::new();
        headers.append(b"Content-Encoding", name.as_bytes());
        headers
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn gzip_and_deflate() {
        let text = b"hello, hello, hello".to_vec();
        assert_eq!(decode(&coded("gzip"), gzip(&text), 1000).unwrap(), text);
        assert_eq!(decode(&coded("X-GZIP"), gzip(&text), 1000).unwrap(), text);
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(&text).unwrap();
        assert_eq!(
            decode(&coded("deflate"), zlib.finish().unwrap(), 1000).unwrap(),
            text
        );
        let mut raw = DeflateEncoder::new(Vec::new(), Compression::default());
        raw.write_all(&text).unwrap();
        assert_eq!(
            decode(&coded("deflate"), raw.finish().unwrap(), 1000).unwrap(),
            text
        );
        // Codings applied one after another come off in reverse.
        let twice = gzip(&gzip(&text));
        assert_eq!(decode(&coded("gzip, gzip"), twice, 1000).unwrap(), text);
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(&gzip(&text)).unwrap();
        assert_eq!(
            decode(&coded("gzip, deflate"), zlib.finish().unwrap(), 1000).unwrap(),
            text
        );
        assert_eq!(
            decode(&coded("identity, gzip"), gzip(&text), 1000).unwrap(),
            text
        );
    }

    #[test]
    fn what_passes_through() {
        let body = b"not compressed".to_vec();
        assert_eq!(
            decode(&HeaderList::new(), body.clone(), 1000).unwrap(),
            body
        );
        assert_eq!(decode(&coded("br"), body.clone(), 1000).unwrap(), body);
        assert_eq!(
            decode(&coded("gzip, br"), body.clone(), 1000).unwrap(),
            body
        );
        assert_eq!(decode(&coded(""), body.clone(), 1000).unwrap(), body);
        assert_eq!(decode(&coded("gzip"), Vec::new(), 1000).unwrap(), b"");
    }

    #[test]
    fn a_body_that_does_not_decode_is_an_error() {
        assert_eq!(
            decode(&coded("gzip"), b"not gzip".to_vec(), 1000),
            Err(NetworkError::ContentCoding)
        );
        let mut cut = gzip(b"hello, hello, hello");
        cut.truncate(cut.len() - 4);
        assert_eq!(
            decode(&coded("gzip"), cut, 1000),
            Err(NetworkError::ContentCoding)
        );
    }

    #[test]
    fn a_small_body_cannot_grow_past_the_limit() {
        let bomb = gzip(&vec![0; 100_000]);
        assert!(bomb.len() < 1000);
        assert_eq!(
            decode(&coded("gzip"), bomb.clone(), 99_999),
            Err(NetworkError::BodyTooLarge)
        );
        assert_eq!(
            decode(&coded("gzip"), bomb, 100_000).unwrap().len(),
            100_000
        );
    }

    #[test]
    fn zlib_headers() {
        assert!(is_zlib(&[0x78, 0x9C]));
        assert!(is_zlib(&[0x78, 0x01]));
        assert!(!is_zlib(&[0x78, 0x9D]));
        assert!(!is_zlib(&[0x79, 0x9C]));
        // A window larger than 32 KiB, its check bits right.
        assert!(!is_zlib(&[0x88, 0x1C]));
        assert!(!is_zlib(&[0x78]));
    }
}
