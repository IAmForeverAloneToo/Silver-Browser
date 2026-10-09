//! Content codings (Fetch's *handle content codings*, RFC 9110 §8.4.1):
//! a body in `gzip` or `deflate` decoded, as a request's
//! `Accept-Encoding: gzip, deflate` asked for: whole, or a piece at a
//! time as it arrives (M24.8), the same bytes coming out.

use std::io::{self, Write};

use flate2::write::{DeflateDecoder, GzDecoder, ZlibDecoder};

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
    let mut decoder = Decoder::new(headers, limit);
    if decoder.passes_through() {
        return Ok(body);
    }
    let mut out = Vec::new();
    decoder.push(&body, &mut |bytes| out.extend_from_slice(bytes))?;
    decoder.finish(&mut |bytes| out.extend_from_slice(bytes))?;
    Ok(out)
}

/// The codings of a body removed as it arrives: each piece pushed in,
/// what it decodes to handed on, and the end said by `finish`.
pub(crate) struct Decoder {
    /// The codings to undo, the last applied first; none when the body
    /// passes through as it is.
    stages: Vec<Stage>,
    limit: usize,
    /// Bytes handed on so far.
    out: usize,
    /// Whether any byte has been pushed: an empty body decodes to
    /// nothing whatever its codings.
    started: bool,
}

/// One coding being undone, writing what it decodes to into its own
/// buffer.
enum Stage {
    Gzip(GzDecoder<Vec<u8>>),
    Zlib(ZlibDecoder<Vec<u8>>),
    Deflate(DeflateDecoder<Vec<u8>>),
    /// `deflate` before its first two bytes say whether they are zlib's
    /// wrapper: some servers send it raw, and browsers read it.
    Undecided(Vec<u8>),
    Identity,
}

impl Stage {
    /// What `input` decodes to. Bytes after the end of a coded stream
    /// are dropped, as reading one stream drops them.
    fn push(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        fn feed<W: Write>(writer: &mut W, mut input: &[u8]) -> io::Result<()> {
            while !input.is_empty() {
                match writer.write(input)? {
                    0 => break,
                    n => input = &input[n..],
                }
            }
            Ok(())
        }
        match self {
            Stage::Gzip(decoder) => {
                feed(decoder, input)?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            Stage::Zlib(decoder) => {
                feed(decoder, input)?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            Stage::Deflate(decoder) => {
                feed(decoder, input)?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            Stage::Undecided(start) => {
                start.extend_from_slice(input);
                if start.len() < 2 {
                    return Ok(Vec::new());
                }
                let start = std::mem::take(start);
                self.decide(&start);
                self.push(&start)
            }
            Stage::Identity => Ok(input.to_vec()),
        }
    }

    /// `deflate`'s decoder, by its first bytes.
    fn decide(&mut self, start: &[u8]) {
        *self = if is_zlib(start) {
            Stage::Zlib(ZlibDecoder::new(Vec::new()))
        } else {
            Stage::Deflate(DeflateDecoder::new(Vec::new()))
        };
    }

    /// The last of what it decodes to, its stream checked whole.
    fn finish(&mut self) -> io::Result<Vec<u8>> {
        match self {
            Stage::Gzip(decoder) => {
                decoder.try_finish()?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            Stage::Zlib(decoder) => {
                decoder.try_finish()?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            Stage::Deflate(decoder) => {
                decoder.try_finish()?;
                Ok(std::mem::take(decoder.get_mut()))
            }
            Stage::Undecided(start) => {
                let start = std::mem::take(start);
                self.decide(&start);
                let mut out = self.push(&start)?;
                out.extend(self.finish()?);
                Ok(out)
            }
            Stage::Identity => Ok(Vec::new()),
        }
    }
}

impl Decoder {
    /// A decoder for a body with `headers`, handing on at most `limit`
    /// bytes.
    pub(crate) fn new(headers: &HeaderList, limit: usize) -> Decoder {
        let codings: Vec<String> = headers
            .get_decode_split(b"Content-Encoding")
            .unwrap_or_default()
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let supported =
            |c: &String| matches!(c.as_str(), "gzip" | "x-gzip" | "deflate" | "identity");
        let stages = if codings.iter().all(supported) {
            codings
                .iter()
                .rev()
                .map(|coding| match coding.as_str() {
                    "gzip" | "x-gzip" => Stage::Gzip(GzDecoder::new(Vec::new())),
                    "deflate" => Stage::Undecided(Vec::new()),
                    _ => Stage::Identity,
                })
                .collect()
        } else {
            Vec::new()
        };
        Decoder {
            stages,
            limit,
            out: 0,
            started: false,
        }
    }

    /// Whether the body passes through untouched.
    pub(crate) fn passes_through(&self) -> bool {
        self.stages.is_empty()
    }

    /// Hands `bytes` on to `out`, or past the limit fails.
    fn hand_on(&mut self, bytes: &[u8], out: &mut dyn FnMut(&[u8])) -> Result<(), NetworkError> {
        self.out += bytes.len();
        if self.out > self.limit {
            return Err(NetworkError::BodyTooLarge);
        }
        if !bytes.is_empty() {
            out(bytes);
        }
        Ok(())
    }

    /// Decodes the next piece of the body, handing what it gives to
    /// `out`.
    pub(crate) fn push(
        &mut self,
        bytes: &[u8],
        out: &mut dyn FnMut(&[u8]),
    ) -> Result<(), NetworkError> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.started = true;
        let mut data = bytes.to_vec();
        for stage in &mut self.stages {
            data = stage.push(&data).map_err(|_| NetworkError::ContentCoding)?;
        }
        self.hand_on(&data, out)
    }

    /// Ends the body: each coding's last bytes, its stream checked
    /// whole, handed to `out`.
    pub(crate) fn finish(&mut self, out: &mut dyn FnMut(&[u8])) -> Result<(), NetworkError> {
        if !self.started {
            return Ok(());
        }
        for i in 0..self.stages.len() {
            let mut data = self.stages[i]
                .finish()
                .map_err(|_| NetworkError::ContentCoding)?;
            for stage in &mut self.stages[i + 1..] {
                data = stage.push(&data).map_err(|_| NetworkError::ContentCoding)?;
            }
            self.hand_on(&data, out)?;
        }
        Ok(())
    }
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

    /// `body` decoded pushed in pieces of `size`.
    fn streamed(
        headers: &HeaderList,
        body: &[u8],
        size: usize,
        limit: usize,
    ) -> Result<Vec<u8>, NetworkError> {
        let mut decoder = Decoder::new(headers, limit);
        let mut out = Vec::new();
        for piece in body.chunks(size) {
            decoder.push(piece, &mut |bytes| out.extend_from_slice(bytes))?;
        }
        decoder.finish(&mut |bytes| out.extend_from_slice(bytes))?;
        Ok(out)
    }

    #[test]
    fn a_body_decodes_in_pieces_as_it_does_whole() {
        let text: Vec<u8> = (0..20_000u32)
            .flat_map(|i| (i % 251).to_le_bytes())
            .collect();
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(&text).unwrap();
        let zlib = zlib.finish().unwrap();
        let mut raw = DeflateEncoder::new(Vec::new(), Compression::default());
        raw.write_all(&text).unwrap();
        let raw = raw.finish().unwrap();
        let mut gzip_deflated = ZlibEncoder::new(Vec::new(), Compression::default());
        gzip_deflated.write_all(&gzip(&text)).unwrap();
        let samples = [
            (coded("gzip"), gzip(&text)),
            (coded("deflate"), zlib),
            (coded("deflate"), raw),
            (coded("gzip, deflate"), gzip_deflated.finish().unwrap()),
            (coded("identity"), text.clone()),
        ];
        for (headers, body) in &samples {
            assert_eq!(decode(headers, body.clone(), 1 << 20).unwrap(), text);
            for size in [1, 2, 7, 4096] {
                assert_eq!(
                    streamed(headers, body, size, 1 << 20).unwrap(),
                    text,
                    "{size}"
                );
            }
        }
        // The limit holds as it goes, and a cut stream fails at the end.
        let bomb = gzip(&vec![0; 100_000]);
        assert_eq!(
            streamed(&coded("gzip"), &bomb, 10, 99_999),
            Err(NetworkError::BodyTooLarge)
        );
        let mut cut = gzip(b"hello, hello, hello");
        cut.truncate(cut.len() - 4);
        assert_eq!(
            streamed(&coded("gzip"), &cut, 3, 1000),
            Err(NetworkError::ContentCoding)
        );
        assert_eq!(streamed(&coded("gzip"), b"", 3, 1000), Ok(Vec::new()));
        // Bytes after a coded stream's end are dropped.
        let mut trailing = gzip(b"hello");
        trailing.extend_from_slice(b"junk");
        assert_eq!(
            streamed(&coded("gzip"), &trailing, 2, 1000).unwrap(),
            b"hello"
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
