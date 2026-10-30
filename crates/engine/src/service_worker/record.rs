//! A registration as the profile keeps it (M39): its scope, its
//! `updateViaCache`, its last check and its active version's scripts,
//! events and worker type. Versioned, every length checked against what
//! is left on reading, a record refused whole when anything is out of
//! place; a record of the first layout is a classic script's.

use std::sync::Arc;

use net::Url;

use super::{MAX_IMPORTED_SCRIPTS, Registration, UpdateViaCache, Version};
use crate::caches::record::{Reader, put_text, put_u32};

/// The layout this code writes, and reads with the one before it, which
/// had no worker type.
const VERSION: u8 = 2;

/// A registration's record read back.
pub(super) struct Read {
    pub(super) scope: Url,
    pub(super) update_via_cache: UpdateViaCache,
    pub(super) last_check: Option<u64>,
    pub(super) script: Url,
    pub(super) events: Vec<String>,
    pub(super) resources: Vec<(Url, Arc<[u8]>)>,
    pub(super) module: bool,
}

pub(super) fn encode(registration: &Registration, active: &Version) -> Vec<u8> {
    let mut out = vec![VERSION];
    put_text(&mut out, registration.scope.as_str());
    out.push(match registration.update_via_cache {
        UpdateViaCache::Imports => 0,
        UpdateViaCache::All => 1,
        UpdateViaCache::None => 2,
    });
    match registration.last_check {
        Some(at) => {
            out.push(1);
            out.extend_from_slice(&at.to_le_bytes());
        }
        None => out.push(0),
    }
    put_text(&mut out, active.script.as_str());
    out.push(u8::from(active.module));
    put_u32(&mut out, active.events.len());
    for event in &active.events {
        put_text(&mut out, event);
    }
    put_u32(&mut out, active.resources.len());
    for (url, body) in &active.resources {
        put_text(&mut out, url.as_str());
        put_u32(&mut out, body.len());
        out.extend_from_slice(body);
    }
    out
}

pub(super) fn decode(bytes: &[u8]) -> Option<Read> {
    let mut read = Reader(bytes);
    let layout = read.u8()?;
    if layout != VERSION && layout != 1 {
        return None;
    }
    let scope = Url::parse(&read.text()?).ok()?;
    let update_via_cache = match read.u8()? {
        0 => UpdateViaCache::Imports,
        1 => UpdateViaCache::All,
        2 => UpdateViaCache::None,
        _ => return None,
    };
    let last_check = match read.u8()? {
        0 => None,
        1 => Some(u64::from_le_bytes(read.array()?)),
        _ => return None,
    };
    let script = Url::parse(&read.text()?).ok()?;
    let module = match layout {
        1 => false,
        _ => match read.u8()? {
            0 => false,
            1 => true,
            _ => return None,
        },
    };
    let mut events = Vec::new();
    for _ in 0..read.count()? {
        events.push(read.text()?);
    }
    let count = read.count()?;
    if count == 0 || count > MAX_IMPORTED_SCRIPTS + 1 {
        return None;
    }
    let mut resources = Vec::new();
    for _ in 0..count {
        let url = Url::parse(&read.text()?).ok()?;
        let body: Arc<[u8]> = Arc::from(read.bytes()?);
        resources.push((url, body));
    }
    if resources[0].0 != script || !read.0.is_empty() {
        return None;
    }
    Some(Read {
        scope,
        update_via_cache,
        last_check,
        script,
        events,
        resources,
        module,
    })
}
