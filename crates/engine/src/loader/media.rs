//! A media element's resource (M40 step 5, `docs/design/audio-video.md`,
//! "Fetching media"): fetched on a thread of its own, by ranges where the
//! server answers them, into a buffer of byte ranges in memory that the
//! element's pipeline reads as a file ([`MediaReader`], `media`'s
//! `ByteSource`), a read waiting for its bytes. A read far from what is
//! coming starts a ranged request there; a response that ends before the
//! resource does is taken up again from where it stopped. The buffer
//! keeps at most [`MAX_MEDIA_BUFFER`], the ranges farthest behind the
//! reader let go first; past it, the fetch waits for the reader.

use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use net::{AbortHandle, Delivery, NetworkError, Request, Response};

use super::Fetcher;

/// The most bytes an element's resource keeps in memory.
pub(crate) const MAX_MEDIA_BUFFER: usize = 64 << 20;

/// The most bytes one ranged request asks for: within what one response
/// may carry. The next range follows as one ends.
pub(crate) const MEDIA_RANGE: u64 = 16 << 20;

/// How far past what has come a read waits for the fetch under way
/// rather than starting a ranged request where it is.
const NEAR: u64 = 1 << 20;

/// How long a read waits between looks at whether its resource is gone.
const LOOK: Duration = Duration::from_millis(100);

/// The most times in a row a ranged fetch is taken up again after it
/// ended early without a byte.
const MAX_RESUMES: u32 = 3;

/// What became of a resource's fetching.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MediaError {
    /// The network failed it, or the server answered with an error.
    Network(String),
    /// The resource was let go.
    Closed,
}

#[derive(Default)]
struct State {
    /// The bytes come, by their first byte's position; never touching.
    ranges: BTreeMap<u64, Vec<u8>>,
    held: usize,
    /// The resource's length, once a response says it.
    length: Option<u64>,
    /// Whether the server answers ranges (a 206 with `Content-Range`).
    by_ranges: bool,
    /// The first response's head: its URL, type and fields.
    head: Option<Response>,
    /// The fetch under way: where it started, where its next byte goes,
    /// and its number; a fetch whose number is not the latest stops.
    flight: Option<(u64, u64, u64)>,
    abort: Option<AbortHandle>,
    generation: u64,
    /// Whether a whole (not ranged) response came to its end.
    whole: bool,
    failed: Option<MediaError>,
    /// Where the reader is, by which what is behind it goes first.
    reading: u64,
    resumes: u32,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    fetcher: Arc<dyn Fetcher>,
    request: Request,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An element's resource; its fetching stops when it is dropped.
pub(crate) struct MediaResource {
    shared: Arc<Shared>,
}

impl MediaResource {
    /// Starts fetching `request` (a media request, its page's policies
    /// on it) with `fetcher`, from the start.
    pub(crate) fn start(fetcher: Arc<dyn Fetcher>, request: Request) -> MediaResource {
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            changed: Condvar::new(),
            fetcher,
            request,
        });
        {
            let mut state = lock(&shared);
            begin(&shared, &mut state, 0);
        }
        MediaResource { shared }
    }

    /// A reader of the resource, at its start.
    pub(crate) fn reader(&self) -> MediaReader {
        MediaReader {
            shared: self.shared.clone(),
            at: 0,
        }
    }

    /// The first response's head, once it came.
    pub(crate) fn head(&self) -> Option<Response> {
        lock(&self.shared).head.clone()
    }

    /// Why fetching failed, if it did.
    pub(crate) fn failed(&self) -> Option<MediaError> {
        lock(&self.shared).failed.clone()
    }

    /// The byte ranges held, as (start, end) pairs: what has come and is
    /// kept.
    #[cfg(test)]
    pub(crate) fn held(&self) -> Vec<(u64, u64)> {
        lock(&self.shared)
            .ranges
            .iter()
            .map(|(start, bytes)| (*start, start + bytes.len() as u64))
            .collect()
    }

    /// Whether no fetch is under way: all came, or it failed, or it
    /// waits for a read to want more.
    pub(crate) fn idle(&self) -> bool {
        let state = lock(&self.shared);
        state.flight.is_none() || state.failed.is_some()
    }

    /// The bytes kept in memory.
    pub(crate) fn bytes_held(&self) -> usize {
        lock(&self.shared).held
    }
}

impl Drop for MediaResource {
    fn drop(&mut self) {
        let mut state = lock(&self.shared);
        state.failed = Some(MediaError::Closed);
        state.generation += 1;
        state.flight = None;
        if let Some(abort) = state.abort.take() {
            abort.abort();
        }
        drop(state);
        self.shared.changed.notify_all();
    }
}

/// Starts a fetch from `start`, ending the one under way.
fn begin(shared: &Arc<Shared>, state: &mut State, start: u64) {
    if state.failed.is_some() {
        return;
    }
    if let Some(abort) = state.abort.take() {
        abort.abort();
    }
    state.generation += 1;
    let generation = state.generation;
    let abort = AbortHandle::new();
    state.abort = Some(abort.clone());
    state.flight = Some((start, start, generation));
    let mut request = shared.request.clone();
    request.abort = abort;
    request.headers.delete(b"Range");
    request.headers.append(
        b"Range",
        format!("bytes={start}-{}", start + MEDIA_RANGE - 1).as_bytes(),
    );
    let shared = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("media fetch".to_owned())
        .spawn(move || fetch(&shared, &request, start, generation));
    if spawned.is_err() {
        state.failed = Some(MediaError::Network("no thread".to_owned()));
    }
}

/// The first byte's position and the whole length a `Content-Range`
/// says: `bytes a-b/n`, `n` perhaps `*`.
fn content_range(response: &Response) -> Option<(u64, Option<u64>)> {
    let value = response.headers.get(b"Content-Range")?;
    let value = std::str::from_utf8(&value).ok()?.trim();
    let rest = value.strip_prefix("bytes")?.trim_start();
    let (span, length) = rest.split_once('/')?;
    let (first, _) = span.split_once('-')?;
    let first = first.trim().parse().ok()?;
    let length = match length.trim() {
        "*" => None,
        n => Some(n.parse().ok()?),
    };
    Some((first, length))
}

fn fetch(shared: &Arc<Shared>, request: &Request, start: u64, generation: u64) {
    let current = |state: &State| state.generation == generation;
    let result = shared.fetcher.fetch_streaming(request, &mut |delivery| {
        let mut state = lock(shared);
        if !current(&state) {
            return;
        }
        match delivery {
            Delivery::Head(head) => {
                let at = match head.status {
                    206 => match content_range(head) {
                        Some((first, length)) if first == start => {
                            state.by_ranges = true;
                            state.length = length.or(state.length);
                            start
                        }
                        _ => {
                            state.failed = Some(MediaError::Network(
                                "a range not the one asked for".to_owned(),
                            ));
                            0
                        }
                    },
                    200 => {
                        // The server ignored the range: the whole, from
                        // its start.
                        state.by_ranges = false;
                        if let Some(length) = head
                            .headers
                            .get(b"Content-Length")
                            .and_then(|v| std::str::from_utf8(&v).ok()?.trim().parse().ok())
                        {
                            state.length = Some(length);
                        }
                        0
                    }
                    status => {
                        state.failed = Some(MediaError::Network(format!("status {status}")));
                        0
                    }
                };
                if state.head.is_none() {
                    state.head = Some(head.clone());
                }
                state.flight = Some((at, at, generation));
            }
            Delivery::Body(bytes) => {
                if state.failed.is_some() {
                    return;
                }
                let Some((from, at, _)) = state.flight else {
                    return;
                };
                state.resumes = 0;
                keep(&mut state, at, bytes);
                state.flight = Some((from, at + bytes.len() as u64, generation));
                // Past the bound, what is behind the reader goes; if what
                // is ahead alone fills it, the fetch waits for the reader.
                loop {
                    let reading = state.reading;
                    let held = state.held;
                    if held <= MAX_MEDIA_BUFFER || !current(&state) {
                        break;
                    }
                    if !evict_behind(&mut state, reading) {
                        state = shared
                            .changed
                            .wait_timeout(state, LOOK)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                    }
                }
            }
        }
        drop(state);
        shared.changed.notify_all();
    });
    let mut state = lock(shared);
    if !current(&state) {
        return;
    }
    let (_, at, _) = state.flight.unwrap_or((start, start, generation));
    state.flight = None;
    state.abort = None;
    match result {
        Ok(_) if state.failed.is_some() => {}
        Ok(_) if !state.by_ranges => {
            // A whole response's end is the resource's.
            state.whole = true;
            state.length = Some(state.length.unwrap_or(at).min(at));
        }
        // A range ended short of the resource's end: the next.
        Ok(_) if state.length.is_none_or(|length| at < length) && at > start => {
            begin(shared, &mut state, at);
        }
        Ok(_) => {}
        Err(_) if state.failed.is_some() => {}
        Err(NetworkError::Aborted) => {}
        Err(error) => {
            // A ranged response that stopped short is taken up again
            // where it stopped, a few times without progress at most.
            let short = state.by_ranges && state.length.is_none_or(|length| at < length);
            if short && state.resumes < MAX_RESUMES {
                state.resumes += 1;
                begin(shared, &mut state, at);
            } else {
                state.failed = Some(MediaError::Network(error.to_string()));
            }
        }
    }
    drop(state);
    shared.changed.notify_all();
}

/// Keeps `bytes` at `at`, merged with the ranges they touch; what was
/// already there stays.
fn keep(state: &mut State, at: u64, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let end = at + bytes.len() as u64;
    // Most often the bytes follow a range and touch no other: added to it.
    let follows = state
        .ranges
        .range(..at)
        .next_back()
        .filter(|(start, kept)| **start + kept.len() as u64 == at)
        .map(|(start, _)| *start);
    let clear = state.ranges.range(at..=end).next().is_none();
    if let (Some(start), true) = (follows, clear)
        && let Some(kept) = state.ranges.get_mut(&start)
    {
        kept.extend_from_slice(bytes);
        state.held += bytes.len();
        return;
    }
    // The ranges that overlap or touch [at, end), taken out and merged.
    let touching: Vec<u64> = state
        .ranges
        .range(..=end)
        .filter(|(start, kept)| **start + kept.len() as u64 >= at)
        .map(|(start, _)| *start)
        .collect();
    let mut first = at;
    let mut last = end;
    let mut pieces = Vec::new();
    for start in touching {
        let kept = state.ranges.remove(&start).unwrap_or_default();
        state.held -= kept.len();
        first = first.min(start);
        last = last.max(start + kept.len() as u64);
        pieces.push((start, kept));
    }
    let mut merged = vec![0u8; (last - first) as usize];
    let place = |merged: &mut Vec<u8>, start: u64, data: &[u8]| {
        let offset = (start - first) as usize;
        merged[offset..offset + data.len()].copy_from_slice(data);
    };
    place(&mut merged, at, bytes);
    // What was there before wins over the same bytes come again.
    for (start, kept) in &pieces {
        place(&mut merged, *start, kept);
    }
    state.held += merged.len();
    state.ranges.insert(first, merged);
}

/// Lets go of the range farthest behind `reading`, or of the part of one
/// behind it; whether anything went.
fn evict_behind(state: &mut State, reading: u64) -> bool {
    let Some((&start, kept)) = state.ranges.iter().next() else {
        return false;
    };
    let end = start + kept.len() as u64;
    if start >= reading {
        return false;
    }
    let kept = state.ranges.remove(&start).unwrap_or_default();
    if end <= reading {
        state.held -= kept.len();
    } else {
        let tail = kept[(reading - start) as usize..].to_vec();
        state.held -= kept.len() - tail.len();
        state.ranges.insert(reading, tail);
    }
    true
}

/// The resource read as a file, a read waiting for its bytes.
pub(crate) struct MediaReader {
    shared: Arc<Shared>,
    at: u64,
}

impl Read for MediaReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut state = lock(&self.shared);
        state.reading = self.at;
        loop {
            // Bytes there: as many as are.
            if let Some((start, kept)) = state.ranges.range(..=self.at).next_back() {
                let end = start + kept.len() as u64;
                if self.at < end {
                    let from = (self.at - start) as usize;
                    let n = buf.len().min(kept.len() - from);
                    buf[..n].copy_from_slice(&kept[from..from + n]);
                    self.at += n as u64;
                    drop(state);
                    self.shared.changed.notify_all();
                    return Ok(n);
                }
            }
            // A failure first: a failed response's end is no resource's.
            match &state.failed {
                Some(MediaError::Closed) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "the resource went",
                    ));
                }
                Some(MediaError::Network(why)) => return Err(io::Error::other(why.clone())),
                None => {}
            }
            if state.length.is_some_and(|length| self.at >= length) {
                return Ok(0);
            }
            let coming = state
                .flight
                .is_some_and(|(from, next, _)| from <= self.at && self.at <= next + NEAR);
            if !coming {
                // Not coming: by a range from here, or (bytes of a whole
                // response let go) the whole again.
                let start = if state.by_ranges { self.at } else { 0 };
                begin(&self.shared, &mut state, start);
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, LOOK)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            state.reading = self.at;
        }
    }
}

impl Seek for MediaReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let length = lock(&self.shared).length;
        let at = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.at) + i128::from(n),
            SeekFrom::End(n) => match length {
                Some(length) => i128::from(length) + i128::from(n),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "a seek from an end not known",
                    ));
                }
            },
        };
        if at < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a seek before the start",
            ));
        }
        self.at = u64::try_from(at).unwrap_or(u64::MAX);
        Ok(self.at)
    }
}

impl media::ByteSource for MediaReader {
    fn length(&self) -> Option<u64> {
        lock(&self.shared).length
    }

    fn seekable(&self) -> bool {
        let state = lock(&self.shared);
        // By ranges, or all of it held.
        state.by_ranges
            || state.whole
            || state.length.is_some_and(|length| {
                state
                    .ranges
                    .get(&0)
                    .is_some_and(|all| all.len() as u64 >= length)
            })
    }
}
