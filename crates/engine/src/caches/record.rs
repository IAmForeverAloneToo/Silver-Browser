//! The Cache API's records as the profile keeps them (M39): a cache's
//! name; an entry's cache, request, response head and body's length.
//! Versioned, every length checked against what is left on reading, a
//! record refused whole when anything is out of place.

use super::{StoredRequest, StoredResponse};

/// The layout this code writes and reads.
const VERSION: u8 = 1;

/// The most fields, or URLs, a record holds.
const MAX_ITEMS: usize = 4096;

/// An entry's record read back.
pub(super) struct EntryRecord {
    pub(super) cache: u64,
    pub(super) request: StoredRequest,
    pub(super) response: StoredResponse,
    pub(super) body_len: u64,
}

fn put_u32(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    put_u32(out, text.len());
    out.extend_from_slice(text.as_bytes());
}

fn put_pairs(out: &mut Vec<u8>, pairs: &[(String, String)]) {
    put_u32(out, pairs.len());
    for (name, value) in pairs {
        put_text(out, name);
        put_text(out, value);
    }
}

pub(super) fn encode_cache(name: &str) -> Vec<u8> {
    let mut out = vec![VERSION];
    put_text(&mut out, name);
    out
}

pub(super) fn decode_cache(bytes: &[u8]) -> Option<String> {
    let mut read = Reader(bytes);
    if read.u8()? != VERSION {
        return None;
    }
    let name = read.text()?;
    read.0.is_empty().then_some(name)
}

pub(super) fn encode_entry(
    cache: u64,
    request: &StoredRequest,
    response: &StoredResponse,
    body_len: u64,
) -> Vec<u8> {
    let mut out = vec![VERSION];
    out.extend_from_slice(&cache.to_le_bytes());
    put_text(&mut out, &request.method);
    put_text(&mut out, &request.url);
    put_pairs(&mut out, &request.headers);
    put_text(&mut out, &response.kind);
    put_u32(&mut out, response.urls.len());
    for url in &response.urls {
        put_text(&mut out, url);
    }
    out.extend_from_slice(&response.status.to_le_bytes());
    put_text(&mut out, &response.status_text);
    put_pairs(&mut out, &response.headers);
    out.extend_from_slice(&body_len.to_le_bytes());
    out
}

pub(super) fn decode_entry(bytes: &[u8]) -> Option<EntryRecord> {
    let mut read = Reader(bytes);
    if read.u8()? != VERSION {
        return None;
    }
    let cache = u64::from_le_bytes(read.array()?);
    let method = read.text()?;
    let url = read.text()?;
    let request_headers = read.pairs()?;
    let kind = read.text()?;
    if !matches!(kind.as_str(), "basic" | "cors" | "default" | "opaque") {
        return None;
    }
    let mut urls = Vec::new();
    for _ in 0..read.count()? {
        urls.push(read.text()?);
    }
    let status = u16::from_le_bytes(read.array()?);
    let status_text = read.text()?;
    let headers = read.pairs()?;
    let body_len = u64::from_le_bytes(read.array()?);
    if !read.0.is_empty() {
        return None;
    }
    Some(EntryRecord {
        cache,
        request: StoredRequest {
            method,
            url,
            headers: request_headers,
        },
        response: StoredResponse {
            kind,
            urls,
            status,
            status_text,
            headers,
        },
        body_len,
    })
}

/// What is left of a record to read.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(taken)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u32(&mut self) -> Option<usize> {
        usize::try_from(u32::from_le_bytes(self.array()?)).ok()
    }

    fn count(&mut self) -> Option<usize> {
        self.u32().filter(|&n| n <= MAX_ITEMS)
    }

    fn text(&mut self) -> Option<String> {
        let n = self.u32()?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }

    fn pairs(&mut self) -> Option<Vec<(String, String)>> {
        let mut pairs = Vec::new();
        for _ in 0..self.count()? {
            pairs.push((self.text()?, self.text()?));
        }
        Some(pairs)
    }
}

/// For fuzzing: an entry's record read, if it reads, writes back as it
/// was.
#[doc(hidden)]
pub fn fuzz_entry_record(data: &[u8]) {
    if let Some(read) = decode_entry(data) {
        assert_eq!(
            encode_entry(read.cache, &read.request, &read.response, read.body_len),
            data
        );
    }
    if let Some(name) = decode_cache(data) {
        assert_eq!(encode_cache(&name), data);
    }
}
