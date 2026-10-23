//! What a demuxer reads: a file's bytes, all there (`decodeAudioData()`,
//! the tests) or coming from the network into a media element's buffer,
//! read as a file whose reads wait for them.

use std::io::{Read, Seek, SeekFrom};

/// Bytes read as a file. `len` is the whole resource's length when known;
/// a read past what has come may wait for it, and one past the end reads
/// nothing.
pub trait ByteSource: Read + Seek + Send + Sync {
    /// The resource's length in bytes, if known.
    fn length(&self) -> Option<u64>;
    /// Whether a seek to any position can be served.
    fn seekable(&self) -> bool;
}

/// Bytes all in memory.
pub struct Bytes {
    data: std::sync::Arc<[u8]>,
    at: u64,
}

impl Bytes {
    pub fn new(data: impl Into<std::sync::Arc<[u8]>>) -> Bytes {
        Bytes {
            data: data.into(),
            at: 0,
        }
    }
}

impl Read for Bytes {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let start = usize::try_from(self.at)
            .unwrap_or(usize::MAX)
            .min(self.data.len());
        let n = buf.len().min(self.data.len() - start);
        buf[..n].copy_from_slice(&self.data[start..start + n]);
        self.at += n as u64;
        Ok(n)
    }
}

impl Seek for Bytes {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let len = self.data.len() as i128;
        let at = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => len + i128::from(n),
            SeekFrom::Current(n) => i128::from(self.at) + i128::from(n),
        };
        if at < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a seek before the start",
            ));
        }
        self.at = u64::try_from(at).unwrap_or(u64::MAX);
        Ok(self.at)
    }
}

impl ByteSource for Bytes {
    fn length(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }

    fn seekable(&self) -> bool {
        true
    }
}

/// The most bytes a [`Tap`] keeps.
const MAX_TAPPED: usize = 16 << 20;

/// What a container's reader read while it was being opened, kept so
/// that what the reader does not understand can be looked for in it
/// (an MP4's AV1 configuration, which symphonia passes over).
pub(crate) type Tap = std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>;

/// A [`ByteSource`] as symphonia reads one.
pub(crate) struct Adapter {
    pub(crate) source: Box<dyn ByteSource>,
    pub(crate) tap: Tap,
}

impl Read for Adapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.source.read(buf)?;
        if let Ok(mut tap) = self.tap.lock()
            && let Some(kept) = tap.as_mut()
            && kept.len() + n <= MAX_TAPPED
        {
            kept.extend_from_slice(&buf[..n]);
        }
        Ok(n)
    }
}

impl Seek for Adapter {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.source.seek(to)
    }
}

impl symphonia::core::io::MediaSource for Adapter {
    fn is_seekable(&self) -> bool {
        self.source.seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.source.length()
    }
}
