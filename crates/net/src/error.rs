//! What can stop a fetch, named by the step that failed, and the handle
//! that stops one on purpose.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mio::Waker;

use crate::address_space::AddressSpace;
use crate::http::Malformed;

/// Why a fetch gave no response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkError {
    /// A scheme that is not fetched: anything but `about:blank`,
    /// `data:`, `file:`, `http:` and `https:`.
    BadScheme(String),
    /// One of Fetch's bad ports.
    BadPort(u16),
    /// A request from a page in one address space to a less public one.
    AddressSpace {
        initiator: AddressSpace,
        target: AddressSpace,
    },
    /// An `http:` request from an `https:` page.
    MixedContent,
    /// The host's name did not resolve.
    Resolve(String),
    /// No connection could be made.
    Connect(String),
    /// TLS failed, for this reason.
    Tls(TlsFailure),
    /// The response broke HTTP's syntax.
    Malformed(Malformed),
    /// A 101, switching to a protocol no request asked for.
    UnaskedUpgrade,
    /// The connection ended inside the response.
    Truncated,
    /// The connection failed inside the response.
    Io(String),
    /// The response's header was too large.
    HeadersTooLarge,
    /// The response's body was too large, before or after its content
    /// coding was removed.
    BodyTooLarge,
    /// A body in a content coding that would not decode.
    ContentCoding,
    /// A `data:` URL that is not one.
    DataUrl,
    /// A `file:` URL that names no readable file.
    File(String),
    /// Too long without a byte, or without a connection.
    Timeout,
    /// More redirects than Fetch allows.
    RedirectLimit,
    /// A redirect to somewhere a redirect may not go.
    BadRedirect,
    /// The request was aborted.
    Aborted,
    /// A CORS request the server did not allow, or a preflight it
    /// refused (M27.3): why, for the console.
    Cors(String),
    /// A response whose `Cross-Origin-Resource-Policy` refuses the
    /// document that asked (M27).
    CrossOriginResourcePolicy,
    /// A redirect, for a request whose redirect mode is error.
    Redirected,
    /// A request its mode refuses (M27): why.
    Refused(String),
    /// The document's Content Security Policy does not allow it: the
    /// directive, as the policy wrote it (M24.6).
    Csp(String),
    /// An `only-if-cached` request with nothing cached for it (M28.4).
    NotCached,
}

/// Why TLS failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsFailure {
    /// The certificate has expired.
    Expired,
    /// The certificate is not valid yet.
    NotYetValid,
    /// No trusted root issued the certificate.
    UnknownIssuer,
    /// The certificate is for another name.
    WrongName,
    /// Anything else, as rustls says it.
    Other(String),
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkError::BadScheme(scheme) => write!(f, "{scheme}: URLs are not fetched"),
            NetworkError::BadPort(port) => write!(f, "port {port} is not fetched"),
            NetworkError::AddressSpace { initiator, target } => {
                write!(f, "a {initiator} page may not reach a {target} address")
            }
            NetworkError::MixedContent => f.write_str("an http: request from an https: page"),
            NetworkError::Resolve(why) => write!(f, "the name did not resolve: {why}"),
            NetworkError::Connect(why) => write!(f, "no connection: {why}"),
            NetworkError::Tls(TlsFailure::Expired) => f.write_str("the certificate has expired"),
            NetworkError::Tls(TlsFailure::NotYetValid) => {
                f.write_str("the certificate is not valid yet")
            }
            NetworkError::Tls(TlsFailure::UnknownIssuer) => {
                f.write_str("the certificate's issuer is not trusted")
            }
            NetworkError::Tls(TlsFailure::WrongName) => {
                f.write_str("the certificate is for another name")
            }
            NetworkError::Tls(TlsFailure::Other(why)) => write!(f, "TLS failed: {why}"),
            NetworkError::Malformed(what) => write!(f, "malformed response: {what:?}"),
            NetworkError::UnaskedUpgrade => f.write_str("a protocol switch no request asked for"),
            NetworkError::Truncated => f.write_str("the connection closed inside the response"),
            NetworkError::Io(why) => write!(f, "the connection failed: {why}"),
            NetworkError::HeadersTooLarge => f.write_str("the response's header is too large"),
            NetworkError::BodyTooLarge => f.write_str("the response's body is too large"),
            NetworkError::ContentCoding => f.write_str("the body's content coding did not decode"),
            NetworkError::DataUrl => f.write_str("not a data: URL"),
            NetworkError::File(why) => write!(f, "the file could not be read: {why}"),
            NetworkError::Timeout => f.write_str("timed out"),
            NetworkError::RedirectLimit => f.write_str("too many redirects"),
            NetworkError::BadRedirect => f.write_str("a redirect to where none may go"),
            NetworkError::Aborted => f.write_str("aborted"),
            NetworkError::Cors(why) => write!(f, "refused by CORS: {why}"),
            NetworkError::CrossOriginResourcePolicy => {
                f.write_str("refused by its Cross-Origin-Resource-Policy")
            }
            NetworkError::Redirected => f.write_str("a redirect, which the request refuses"),
            NetworkError::NotCached => f.write_str("nothing cached, and only the cache asked"),
            NetworkError::Refused(why) => write!(f, "refused: {why}"),
            NetworkError::Csp(directive) => {
                write!(f, "blocked by Content-Security-Policy: {directive}")
            }
        }
    }
}

impl std::error::Error for NetworkError {}

/// Stops a fetch. Aborting sets a flag that every step of the fetch
/// looks at, and wakes the poll of the connection the fetch is using,
/// which ends a read or write waiting on it at once. Resolving a name,
/// connecting and waiting for a free connection look at the flag every
/// quarter of a second. Clones share the one abort.
#[derive(Debug, Clone, Default)]
pub struct AbortHandle(Arc<Abort>);

#[derive(Debug, Default)]
struct Abort {
    aborted: AtomicBool,
    watched: Mutex<Watched>,
}

/// The wakers of the connections fetches holding the handle are using,
/// each with the number its [`Watch`] goes by.
#[derive(Debug, Default)]
struct Watched {
    next: u64,
    wakers: Vec<(u64, Arc<Waker>)>,
}

impl AbortHandle {
    /// A handle not aborted yet.
    pub fn new() -> AbortHandle {
        AbortHandle::default()
    }

    /// Aborts every fetch holding a clone of this handle.
    pub fn abort(&self) {
        // The flag first: a wait the waker ends looks at it.
        self.0.aborted.store(true, Ordering::SeqCst);
        for (_, waker) in &self.watched().wakers {
            // A poll already gone has nothing to wake.
            let _ = waker.wake();
        }
    }

    /// Whether [`AbortHandle::abort`] was called.
    pub fn is_aborted(&self) -> bool {
        self.0.aborted.load(Ordering::SeqCst)
    }

    /// Watches a connection's poll while the returned guard lives: an
    /// abort wakes it, and one that came before wakes it at once.
    pub(crate) fn watch(&self, waker: &Arc<Waker>) -> Watch {
        let mut watched = self.watched();
        if self.is_aborted() {
            let _ = waker.wake();
        }
        let id = watched.next;
        watched.next += 1;
        watched.wakers.push((id, waker.clone()));
        Watch {
            abort: self.clone(),
            id,
        }
    }

    fn watched(&self) -> MutexGuard<'_, Watched> {
        self.0
            .watched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// A connection's poll that an [`AbortHandle`] wakes if it aborts, until
/// this is dropped.
#[derive(Debug)]
pub(crate) struct Watch {
    abort: AbortHandle,
    id: u64,
}

impl Drop for Watch {
    fn drop(&mut self) {
        let id = self.id;
        self.abort
            .watched()
            .wakers
            .retain(|(watched, _)| *watched != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let handle = AbortHandle::new();
        let clone = handle.clone();
        assert!(!clone.is_aborted());
        handle.abort();
        assert!(clone.is_aborted());
    }

    #[test]
    fn a_watch_lasts_as_long_as_its_guard() {
        let poll = mio::Poll::new().unwrap();
        let waker = Arc::new(Waker::new(poll.registry(), mio::Token(0)).unwrap());
        let handle = AbortHandle::new();
        let first = handle.watch(&waker);
        let second = handle.clone().watch(&waker);
        assert_eq!(handle.watched().wakers.len(), 2);
        drop(first);
        assert_eq!(handle.watched().wakers.len(), 1);
        drop(second);
        assert!(handle.watched().wakers.is_empty());
    }

    #[test]
    fn an_abort_wakes_the_polls_it_watches() {
        let mut poll = mio::Poll::new().unwrap();
        let waker = Arc::new(Waker::new(poll.registry(), mio::Token(7)).unwrap());
        let handle = AbortHandle::new();
        let _watch = handle.watch(&waker);
        handle.abort();
        let mut events = mio::Events::with_capacity(1);
        poll.poll(&mut events, Some(std::time::Duration::from_secs(5)))
            .unwrap();
        assert_eq!(
            events.iter().map(|e| e.token()).collect::<Vec<_>>(),
            [mio::Token(7)]
        );
        // A watch begun after the abort is woken at once. (A poll has
        // one waker at most.)
        let mut poll = mio::Poll::new().unwrap();
        let late = Arc::new(Waker::new(poll.registry(), mio::Token(8)).unwrap());
        let _late = handle.watch(&late);
        poll.poll(&mut events, Some(std::time::Duration::from_secs(5)))
            .unwrap();
        assert_eq!(
            events.iter().map(|e| e.token()).collect::<Vec<_>>(),
            [mio::Token(8)]
        );
    }

    #[test]
    fn every_error_says_something() {
        let errors = [
            NetworkError::BadPort(25),
            NetworkError::AddressSpace {
                initiator: AddressSpace::Public,
                target: AddressSpace::Local,
            },
            NetworkError::Tls(TlsFailure::WrongName),
            NetworkError::Malformed(Malformed::Chunk),
        ];
        let said: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "port 25 is not fetched",
                "a public page may not reach a local address",
                "the certificate is for another name",
                "malformed response: Chunk",
            ]
        );
    }
}
