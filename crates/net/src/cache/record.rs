//! An entry of the cache as the profile keeps it (M38): its key, its
//! response but the body, and its times, in a versioned record read
//! back with every length checked against what is left, so that a
//! damaged one is refused whole.

use std::time::{Duration, UNIX_EPOCH};

use super::{Entry, Partition, Stored};
use crate::url::Site;
use crate::{AddressSpace, HeaderList};

/// The record's layout this code writes and reads.
const VERSION: u8 = 1;

/// The most fields a record's head or `Vary` holds.
const MAX_FIELDS: usize = 1024;

/// What a record holds: the entry's key and the entry, its body not
/// read.
pub(super) struct Record {
    pub(super) key: (Partition, String),
    pub(super) entry: Entry,
}

/// `entry`'s record, kept under `key`; `None` for an opaque site's,
/// which no other browsing ever finds.
pub(super) fn encode(key: &(Partition, String), entry: &Entry) -> Option<Vec<u8>> {
    let mut out = vec![VERSION];
    for site in [&key.0.top, &key.0.frame] {
        let Site::Schemeful { scheme, host } = site else {
            return None;
        };
        put_bytes(&mut out, scheme.as_bytes());
        put_bytes(&mut out, host.as_bytes());
    }
    put_bytes(&mut out, key.1.as_bytes());
    put_u32(&mut out, entry.vary.len());
    for (name, value) in &entry.vary {
        put_bytes(&mut out, name);
        match value {
            Some(value) => {
                out.push(1);
                put_bytes(&mut out, value);
            }
            None => out.push(0),
        }
    }
    let stored = &entry.stored;
    out.extend_from_slice(&stored.status.to_le_bytes());
    put_bytes(&mut out, &stored.status_message);
    put_u32(&mut out, stored.headers.iter().count());
    for (name, value) in stored.headers.iter() {
        put_bytes(&mut out, name);
        put_bytes(&mut out, value);
    }
    out.push(match stored.address_space {
        None => 0,
        Some(AddressSpace::Loopback) => 1,
        Some(AddressSpace::Local) => 2,
        Some(AddressSpace::Public) => 3,
    });
    out.extend_from_slice(&(entry.body_len as u64).to_le_bytes());
    let since = entry
        .response_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    for duration in [since, entry.lifetime, entry.initial_age] {
        put_duration(&mut out, duration);
    }
    out.push(
        u8::from(entry.no_cache)
            | u8::from(entry.must_revalidate) << 1
            | u8::from(entry.immutable) << 2,
    );
    out.extend_from_slice(&entry.used.to_le_bytes());
    Some(out)
}

/// The record `bytes` read back, the entry numbered `id`; `None` if it
/// is not one this code wrote, whole.
pub(super) fn decode(id: u64, bytes: &[u8]) -> Option<Record> {
    let mut read = Reader(bytes);
    if read.u8()? != VERSION {
        return None;
    }
    let mut sites = Vec::with_capacity(2);
    for _ in 0..2 {
        let scheme = read.text()?;
        let host = read.text()?;
        sites.push(Site::Schemeful { scheme, host });
    }
    let frame = sites.pop()?;
    let top = sites.pop()?;
    let url = read.text()?;
    let mut vary = Vec::new();
    for _ in 0..read.count()? {
        let name = read.bytes()?.to_vec();
        let value = match read.u8()? {
            0 => None,
            1 => Some(read.bytes()?.to_vec()),
            _ => return None,
        };
        vary.push((name, value));
    }
    let status = u16::from_le_bytes(read.array()?);
    let status_message = read.bytes()?.to_vec();
    let mut headers = HeaderList::new();
    for _ in 0..read.count()? {
        let name = read.bytes()?;
        let value = read.bytes()?;
        headers.append(name, value);
    }
    let address_space = match read.u8()? {
        0 => None,
        1 => Some(AddressSpace::Loopback),
        2 => Some(AddressSpace::Local),
        3 => Some(AddressSpace::Public),
        _ => return None,
    };
    let body_len = usize::try_from(u64::from_le_bytes(read.array()?)).ok()?;
    if body_len > super::MAX_CACHE_ENTRY_BYTES {
        return None;
    }
    // A time the platform's clock cannot hold exactly (Windows keeps
    // 100 ns steps) is none the cache wrote: refused, as a record must
    // write back as it was (found by fuzzing).
    let since = read.duration()?;
    let response_time = UNIX_EPOCH.checked_add(since)?;
    if response_time.duration_since(UNIX_EPOCH).ok()? != since {
        return None;
    }
    let lifetime = read.duration()?;
    let initial_age = read.duration()?;
    let flags = read.u8()?;
    if flags > 0b111 {
        return None;
    }
    let used = u64::from_le_bytes(read.array()?);
    if !read.0.is_empty() {
        return None;
    }
    Some(Record {
        key: (Partition { top, frame }, url),
        entry: Entry {
            stored: Stored {
                status,
                status_message,
                headers,
                body: Vec::new(),
                address_space,
            },
            vary,
            response_time,
            lifetime,
            initial_age,
            no_cache: flags & 1 != 0,
            must_revalidate: flags & 2 != 0,
            immutable: flags & 4 != 0,
            used,
            id,
            body_len,
            in_memory: false,
            persisted: true,
        },
    })
}

fn put_u32(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(out, bytes.len());
    out.extend_from_slice(bytes);
}

fn put_duration(out: &mut Vec<u8>, duration: Duration) {
    out.extend_from_slice(&duration.as_secs().to_le_bytes());
    out.extend_from_slice(&duration.subsec_nanos().to_le_bytes());
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
        self.u32().filter(|&n| n <= MAX_FIELDS)
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()?;
        self.take(n)
    }

    fn text(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?.to_vec()).ok()
    }

    fn duration(&mut self) -> Option<Duration> {
        let secs = u64::from_le_bytes(self.array()?);
        let nanos = u32::from_le_bytes(self.array()?);
        (nanos < 1_000_000_000).then(|| Duration::new(secs, nanos))
    }
}
