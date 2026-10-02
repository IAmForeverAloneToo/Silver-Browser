//! Connections kept for reuse, by origin: at most a few open to one
//! origin at once, and an idle one closed after a while.

use std::collections::HashMap;
use std::io::BufReader;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::address_space::AddressSpace;
use crate::error::{AbortHandle, NetworkError};

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

struct State {
    origins: HashMap<OriginKey, Origin>,
    reaper_running: bool,
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
                reaper_running: false,
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

impl Drop for Lease {
    fn drop(&mut self) {
        let connection = self.connection.take();
        let mut state = self.shared.lock();
        let origin = state.origins.entry(self.key.clone()).or_default();
        let closed = match connection {
            Some(connection) if self.keep => {
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
            let shared = Arc::downgrade(&self.shared);
            thread::spawn(move || reap(shared));
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
/// the pool is gone.
fn reap(shared: Weak<Shared>) {
    loop {
        let Some(strong) = shared.upgrade() else {
            return;
        };
        let wait = {
            let mut state = strong.lock();
            let now = Instant::now();
            let timeout = strong.idle_timeout;
            let mut next: Option<Instant> = None;
            for origin in state.origins.values_mut() {
                expire(origin, now, timeout);
                for (_, since) in &origin.idle {
                    let at = *since + timeout;
                    next = Some(next.map_or(at, |next| next.min(at)));
                }
            }
            state.origins.retain(|_, origin| origin.open > 0);
            match next {
                Some(at) => at.saturating_duration_since(now),
                None => {
                    state.reaper_running = false;
                    drop(state);
                    strong.changed.notify_all();
                    return;
                }
            }
        };
        strong.changed.notify_all();
        drop(strong);
        thread::sleep(wait.max(Duration::from_millis(10)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::stream::{Socket, Waiter, non_blocking};
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
