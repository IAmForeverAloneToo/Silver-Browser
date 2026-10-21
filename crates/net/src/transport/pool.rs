//! Connections kept for reuse, by origin: at most a few open to one
//! origin at once, and an idle one closed after a while.

use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::address_space::AddressSpace;
use crate::error::{AbortHandle, NetworkError};
use crate::h2::conn::Connection as H2Connection;

use super::ABORT_POLL;
use super::stream::Stream;

/// An open connection, ready for a request.
pub(crate) struct Connection {
    pub(crate) reader: BufReader<Stream>,
    /// The address space of the address it is connected to.
    pub(crate) space: AddressSpace,
}

/// What connections are pooled by: an origin's scheme, host and port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct OriginKey {
    pub(crate) scheme: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    /// The partition a frame's requests are made in (M33.2): its
    /// connections are its own.
    pub(crate) partition: Option<String>,
}

/// Each origin's connections.
#[derive(Default)]
struct Origin {
    /// Connections waiting for a request, with when they began to wait.
    idle: Vec<(Connection, Instant)>,
    /// Connections open, idle or in use, and places taken for ones
    /// being opened.
    open: usize,
}

/// The most origins the pool remembers speak HTTP/1.1 alone: past it,
/// it forgets them all.
const MAX_KNOWN_ORIGINS: usize = 1024;

struct State {
    origins: HashMap<OriginKey, Origin>,
    /// Each origin's HTTP/2 connection (M38), every fetch to it sharing
    /// it; the origins a first connection is being made to, whose
    /// fetches wait for what its TLS chose; and those that chose
    /// HTTP/1.1.
    h2: HashMap<OriginKey, Arc<H2Connection>>,
    probing: HashSet<OriginKey>,
    h1: HashSet<OriginKey>,
    reaper_running: bool,
    /// The pool was dropped: its idle connections were closed, none is
    /// parked any more, and its reaper ends, woken.
    closed: bool,
    /// How many times its reapers looked at the idle connections, for
    /// the tests.
    #[cfg(test)]
    reaper_passes: usize,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    max_per_origin: usize,
    idle_timeout: Duration,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The pool.
pub(crate) struct Pool(Arc<Shared>);

/// A connection taken from the pool, or a place for a new one. Dropped,
/// it goes back: a connection marked to keep waits for the next
/// request, and anything else gives its place up and is closed.
pub(crate) struct Lease {
    shared: Arc<Shared>,
    key: OriginKey,
    pub(crate) connection: Option<Connection>,
    pub(crate) keep: bool,
}

impl Pool {
    pub(crate) fn new(max_per_origin: usize, idle_timeout: Duration) -> Pool {
        Pool(Arc::new(Shared {
            state: Mutex::new(State {
                origins: HashMap::new(),
                h2: HashMap::new(),
                probing: HashSet::new(),
                h1: HashSet::new(),
                reaper_running: false,
                closed: false,
                #[cfg(test)]
                reaper_passes: 0,
            }),
            changed: Condvar::new(),
            max_per_origin,
            idle_timeout,
        }))
    }

    /// An idle connection to the origin that is still open and quiet, or
    /// a place to open a new one, waiting while the origin has all it
    /// may have open. With `fresh`, never an idle connection: one is
    /// closed to make room if need be.
    pub(crate) fn checkout(
        &self,
        key: &OriginKey,
        fresh: bool,
        abort: &AbortHandle,
    ) -> Result<Lease, NetworkError> {
        let shared = &self.0;
        let mut state = shared.lock();
        loop {
            if abort.is_aborted() {
                return Err(NetworkError::Aborted);
            }
            let origin = state.origins.entry(key.clone()).or_default();
            expire(origin, Instant::now(), shared.idle_timeout);
            let lease = |connection| Lease {
                shared: shared.clone(),
                key: key.clone(),
                connection,
                keep: false,
            };
            if !fresh {
                while let Some((mut connection, _)) = origin.idle.pop() {
                    if connection.reader.get_mut().is_idle_and_open() {
                        return Ok(lease(Some(connection)));
                    }
                    // Closed by the server, or not quiet: it gives its
                    // place up.
                    origin.open -= 1;
                }
            } else if origin.open >= shared.max_per_origin && !origin.idle.is_empty() {
                // The oldest idle connection gives its place up.
                origin.idle.remove(0);
                origin.open -= 1;
            }
            if origin.open < shared.max_per_origin {
                origin.open += 1;
                return Ok(lease(None));
            }
            state = shared
                .changed
                .wait_timeout(state, ABORT_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// What a fetch to an `https:` origin uses (M38): its HTTP/2
    /// connection; or, when nothing is known of it, the first
    /// connection, made by this fetch, whose choice the others wait
    /// for; or HTTP/1.1.
    pub(crate) fn speaks(
        &self,
        key: &OriginKey,
        abort: &AbortHandle,
    ) -> Result<Speaks, NetworkError> {
        let shared = &self.0;
        let mut state = shared.lock();
        loop {
            if abort.is_aborted() {
                return Err(NetworkError::Aborted);
            }
            if let Some(connection) = state.h2.get(key) {
                if connection.usable() {
                    return Ok(Speaks::H2(connection.clone()));
                }
                state.h2.remove(key);
            }
            if state.h1.contains(key) {
                return Ok(Speaks::H1);
            }
            if state.probing.insert(key.clone()) {
                return Ok(Speaks::Probe(Probe {
                    shared: shared.clone(),
                    key: key.clone(),
                }));
            }
            state = shared
                .changed
                .wait_timeout(state, ABORT_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Keeps `connection` as the origin's HTTP/2 connection.
    pub(crate) fn keep_h2(&self, key: &OriginKey, connection: Arc<H2Connection>) {
        let mut state = self.0.lock();
        state.h1.remove(key);
        state.h2.insert(key.clone(), connection);
        drop(state);
        self.0.changed.notify_all();
    }

    /// Whether another host of `key`'s scheme, port and partition has an
    /// HTTP/2 connection, which a request to it might share.
    pub(crate) fn may_coalesce(&self, key: &OriginKey) -> bool {
        self.0.lock().h2.keys().any(|other| same_reach(other, key))
    }

    /// An HTTP/2 connection to another host of `key`'s scheme, port and
    /// partition that may carry `key`'s requests (RFC 9113 9.1.1): one
    /// connected to an address of `host`'s, `addresses`, whose
    /// certificate is valid for `host`; `key`'s from now on.
    pub(crate) fn coalesce(
        &self,
        key: &OriginKey,
        host: &crate::Host,
        addresses: &[std::net::SocketAddr],
    ) -> Option<Arc<H2Connection>> {
        let mut state = self.0.lock();
        let found = state
            .h2
            .iter()
            .find(|(other, connection)| {
                same_reach(other, key)
                    && connection.usable()
                    && addresses
                        .iter()
                        .any(|address| address.ip() == connection.peer.ip())
                    && connection.covers(host)
            })
            .map(|(_, connection)| connection.clone())?;
        state.h2.insert(key.clone(), found.clone());
        drop(state);
        self.0.changed.notify_all();
        Some(found)
    }

    /// Forgets `key`'s HTTP/2 connection: one another host's, which
    /// answered that it may not carry `key`'s requests.
    pub(crate) fn forget_h2(&self, key: &OriginKey) {
        self.0.lock().h2.remove(key);
    }

    /// The origin speaks HTTP/1.1 alone.
    pub(crate) fn speaks_h1(&self, key: &OriginKey) {
        let mut state = self.0.lock();
        if state.h1.len() >= MAX_KNOWN_ORIGINS {
            state.h1.clear();
        }
        state.h1.insert(key.clone());
        drop(state);
        self.0.changed.notify_all();
    }

    /// How many connections to the origin are open, and how many of
    /// them idle.
    #[cfg(test)]
    pub(crate) fn counts(&self, key: &OriginKey) -> (usize, usize) {
        let state = self.0.lock();
        state
            .origins
            .get(key)
            .map_or((0, 0), |origin| (origin.open, origin.idle.len()))
    }
}

/// Whether two origins' requests may share a connection: of one scheme,
/// port and partition.
fn same_reach(one: &OriginKey, other: &OriginKey) -> bool {
    one.scheme == other.scheme && one.port == other.port && one.partition == other.partition
}

/// What a fetch to an origin is to use.
pub(crate) enum Speaks {
    H2(Arc<H2Connection>),
    /// A first connection, whose choice the others wait for.
    Probe(Probe),
    H1,
}

/// A first connection to an origin being made: dropped, the fetches
/// waiting for it go on, as they would after a choice of HTTP/1.1 had
/// none been made (the connection failed).
pub(crate) struct Probe {
    shared: Arc<Shared>,
    key: OriginKey,
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.shared.lock().probing.remove(&self.key);
        self.shared.changed.notify_all();
    }
}

impl Drop for Pool {
    /// Closes the idle connections now rather than when their time runs
    /// out, and wakes the reaper to end: a dropped pool, an engine's,
    /// leaves no thread behind it (M35). Its HTTP/2 connections end
    /// once idle.
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.closed = true;
        state.h2.clear();
        let mut idle = Vec::new();
        for origin in state.origins.values_mut() {
            origin.open -= origin.idle.len();
            idle.append(&mut origin.idle);
        }
        state.origins.retain(|_, origin| origin.open > 0);
        drop(state);
        self.0.changed.notify_all();
        drop(idle);
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let connection = self.connection.take();
        let mut state = self.shared.lock();
        let parks = self.keep && !state.closed;
        let origin = state.origins.entry(self.key.clone()).or_default();
        let closed = match connection {
            Some(connection) if parks => {
                origin.idle.push((connection, Instant::now()));
                None
            }
            other => {
                origin.open -= 1;
                other
            }
        };
        if origin.open == 0 {
            state.origins.remove(&self.key);
        }
        if !state.reaper_running && state.origins.values().any(|o| !o.idle.is_empty()) {
            state.reaper_running = true;
            let shared = self.shared.clone();
            thread::spawn(move || reap(&shared));
        }
        drop(state);
        self.shared.changed.notify_all();
        drop(closed);
    }
}

/// Closes the origin's connections that have been idle for `timeout`.
fn expire(origin: &mut Origin, now: Instant, timeout: Duration) {
    let before = origin.idle.len();
    origin
        .idle
        .retain(|(_, since)| now.duration_since(*since) < timeout);
    origin.open -= before - origin.idle.len();
}

/// Closes idle connections as their time runs out, until none is idle or
/// the pool is dropped. It waits on the pool's changes rather than
/// sleeping, so that a pool dropped wakes it to end at once: asleep
/// until the next idle connection's time ran out, each engine dropped
/// left a thread behind for as long as that, and a test making
/// thousands of engines thousands of them (M35).
fn reap(shared: &Shared) {
    let mut state = shared.lock();
    loop {
        if state.closed {
            state.reaper_running = false;
            return;
        }
        let now = Instant::now();
        let timeout = shared.idle_timeout;
        let mut next: Option<Instant> = None;
        for origin in state.origins.values_mut() {
            expire(origin, now, timeout);
            for (_, since) in &origin.idle {
                let at = *since + timeout;
                next = Some(next.map_or(at, |next| next.min(at)));
            }
        }
        state.origins.retain(|_, origin| origin.open > 0);
        #[cfg(test)]
        {
            state.reaper_passes += 1;
        }
        let Some(at) = next else {
            state.reaper_running = false;
            drop(state);
            shared.changed.notify_all();
            return;
        };
        shared.changed.notify_all();
        let wait = at
            .saturating_duration_since(now)
            .max(Duration::from_millis(10));
        state = shared
            .changed
            .wait_timeout(state, wait)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::stream::{Socket, Waiter, non_blocking};
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};

    /// A pooled connection to a listener of its own, and the server's
    /// end.
    fn connection() -> (Connection, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let mut client = non_blocking(client).unwrap();
        let waiter = Waiter::new(&mut client).unwrap();
        let stream = Stream::new(Socket::Plain(client), waiter, Duration::from_secs(60));
        let connection = Connection {
            reader: BufReader::new(stream),
            space: AddressSpace::Loopback,
        };
        (connection, server)
    }

    fn key(host: &str) -> OriginKey {
        OriginKey {
            scheme: "http".into(),
            host: host.into(),
            port: 80,
            partition: None,
        }
    }

    /// Takes a place for `key`, puts a connection in it and parks it.
    fn park(pool: &Pool, key: &OriginKey) -> TcpStream {
        let mut lease = pool.checkout(key, false, &AbortHandle::new()).unwrap();
        assert!(lease.connection.is_none());
        let (connection, server) = connection();
        lease.connection = Some(connection);
        lease.keep = true;
        server
    }

    /// An abort handle that aborts itself after `after`.
    fn aborted_after(after: Duration) -> AbortHandle {
        let abort = AbortHandle::new();
        let aborter = abort.clone();
        thread::spawn(move || {
            thread::sleep(after);
            aborter.abort();
        });
        abort
    }

    #[test]
    fn a_parked_connection_is_taken_again() {
        let pool = Pool::new(2, Duration::from_secs(30));
        let a = key("a");
        let _server = park(&pool, &a);
        assert_eq!(pool.counts(&a), (1, 1));
        let lease = pool.checkout(&a, false, &AbortHandle::new()).unwrap();
        assert!(lease.connection.is_some());
        assert_eq!(pool.counts(&a), (1, 0));
        // Not kept, it gives its place up.
        drop(lease);
        assert_eq!(pool.counts(&a), (0, 0));
    }

    /// Whether the server's end of a connection hears it close within
    /// a few seconds.
    fn closes(mut server: TcpStream) -> bool {
        server
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        matches!(server.read(&mut [0; 1]), Ok(0))
    }

    #[test]
    fn a_dropped_pools_reaper_ends_at_once() {
        // M35: a pool with an idle connection has a reaper waiting for
        // its time to run out, thirty seconds here; dropped, the pool
        // wakes it to end, rather than leaving a thread behind for that
        // long.
        let pool = Pool::new(2, Duration::from_secs(30));
        let idle = park(&pool, &key("a"));
        // The reaper has looked, and waits.
        let started = Instant::now();
        while pool.0.lock().reaper_passes == 0 {
            assert!(started.elapsed() < Duration::from_secs(5), "no reaper");
            thread::sleep(Duration::from_millis(10));
        }
        let shared = Arc::downgrade(&pool.0);
        let dropped = Instant::now();
        drop(pool);
        while shared.upgrade().is_some() {
            assert!(
                dropped.elapsed() < Duration::from_secs(2),
                "the reaper outlives its pool"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(closes(idle));
    }

    #[test]
    fn a_dropped_pool_closes_its_idle_connections_and_parks_none() {
        // M35: with connections still in use as it goes, the idle one
        // closes at once, and one that comes back to be kept is closed
        // rather than parked, though another is still out.
        let pool = Pool::new(4, Duration::from_secs(30));
        let a = key("a");
        let idle = park(&pool, &a);
        let mut out = Vec::new();
        let mut servers = Vec::new();
        for _ in 0..2 {
            let mut lease = pool.checkout(&a, true, &AbortHandle::new()).unwrap();
            let (connection, server) = connection();
            lease.connection = Some(connection);
            lease.keep = true;
            out.push(lease);
            servers.push(server);
        }
        drop(pool);
        assert!(closes(idle), "the idle connection closes");
        drop(out.remove(0));
        assert!(closes(servers.remove(0)), "one coming back closes");
    }

    #[test]
    fn a_closed_idle_connection_gives_its_place_up() {
        let pool = Pool::new(1, Duration::from_secs(30));
        let a = key("a");
        drop(park(&pool, &a));
        thread::sleep(Duration::from_millis(100));
        let lease = pool.checkout(&a, false, &AbortHandle::new()).unwrap();
        assert!(lease.connection.is_none());
        assert_eq!(pool.counts(&a), (1, 0));
    }

    #[test]
    fn a_fresh_checkout_closes_an_idle_connection_to_make_room() {
        let pool = Pool::new(1, Duration::from_secs(30));
        let a = key("a");
        let _server = park(&pool, &a);
        let abort = aborted_after(Duration::from_secs(2));
        let lease = pool.checkout(&a, true, &abort).unwrap();
        assert!(lease.connection.is_none());
        assert_eq!(pool.counts(&a), (1, 0));
    }

    #[test]
    fn a_full_origin_waits_for_a_place() {
        let pool = Pool::new(1, Duration::from_secs(30));
        let (a, b) = (key("a"), key("b"));
        let held = pool.checkout(&a, false, &AbortHandle::new()).unwrap();
        // Another origin does not wait.
        let other = pool.checkout(&b, false, &AbortHandle::new()).unwrap();
        drop(other);
        // An abort ends the wait.
        let started = Instant::now();
        let waited = pool.checkout(&a, false, &aborted_after(Duration::from_millis(300)));
        assert_eq!(waited.err(), Some(NetworkError::Aborted));
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
        // A place given up goes to the one waiting.
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(held);
        });
        let abort = aborted_after(Duration::from_secs(5));
        let lease = pool.checkout(&a, false, &abort).unwrap();
        assert!(lease.connection.is_none());
        assert_eq!(pool.counts(&a), (1, 0));
    }
}
