//! Finding a host's addresses and connecting to one of them.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::Host;
use crate::error::{AbortHandle, NetworkError};

use super::ABORT_POLL;

/// RFC 8305's connection attempt delay: how long one attempt has alone
/// before the next address is tried beside it.
pub(crate) const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// The addresses `host` names, with `port`. `localhost` and the names
/// under `.localhost` are this machine without asking a resolver, as
/// Fetch's *resolve an origin* has it; an IP address is itself; any
/// other name goes to the operating system's resolver, which may block
/// on its own time limit.
pub(crate) fn resolve(host: &Host, port: u16) -> Result<Vec<SocketAddr>, NetworkError> {
    let addresses = match host {
        Host::Ipv4(address) => vec![SocketAddr::new(IpAddr::V4(*address), port)],
        Host::Ipv6(address) => vec![SocketAddr::new(IpAddr::V6(*address), port)],
        Host::Domain(name) if name == "localhost" || name.ends_with(".localhost") => vec![
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        ],
        Host::Domain(name) => (name.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| NetworkError::Resolve(e.to_string()))?
            .collect(),
        Host::Opaque(_) | Host::Empty => {
            return Err(NetworkError::Resolve("not a network host".to_owned()));
        }
    };
    if addresses.is_empty() {
        return Err(NetworkError::Resolve("no addresses".to_owned()));
    }
    Ok(addresses)
}

/// [`resolve`] on a thread of its own, so that an abort is not kept
/// waiting by the operating system's resolver, which cannot be
/// interrupted: the fetch gives up, and the lookup finishes by itself.
pub(crate) fn resolve_abortable(
    host: &Host,
    port: u16,
    abort: &AbortHandle,
) -> Result<Vec<SocketAddr>, NetworkError> {
    let Host::Domain(name) = host else {
        return resolve(host, port);
    };
    if name == "localhost" || name.ends_with(".localhost") {
        return resolve(host, port);
    }
    let (sender, receiver) = mpsc::channel();
    let host = host.clone();
    thread::spawn(move || {
        let _ = sender.send(resolve(&host, port));
    });
    loop {
        if abort.is_aborted() {
            return Err(NetworkError::Aborted);
        }
        match receiver.recv_timeout(ABORT_POLL) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(NetworkError::Resolve("the lookup stopped".to_owned()));
            }
        }
    }
}

/// The addresses in the resolver's order, but alternating families from
/// the first one's, as RFC 8305 §4 sorts them.
fn interleave(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let Some(first_is_v6) = addresses.first().map(SocketAddr::is_ipv6) else {
        return addresses;
    };
    let (mut first, mut second): (Vec<_>, Vec<_>) = addresses
        .into_iter()
        .partition(|address| address.is_ipv6() == first_is_v6);
    let mut sorted = Vec::with_capacity(first.len() + second.len());
    first.reverse();
    second.reverse();
    loop {
        match (first.pop(), second.pop()) {
            (None, None) => return sorted,
            (a, b) => sorted.extend(a.into_iter().chain(b)),
        }
    }
}

/// Connects to the first of `addresses` that answers, tried in their
/// order but alternating families. A new attempt starts
/// [`ATTEMPT_DELAY`] after the last one began, or at once when one
/// fails; together they get `timeout`. `allowed` is asked about each
/// address just before it is tried, and an address it refuses is
/// skipped, so a name that resolves somewhere else than it did a moment
/// ago is checked where it now points.
///
/// Attempts still running when one succeeds finish on their own threads
/// and close what they connect.
pub(crate) fn connect(
    addresses: &[SocketAddr],
    allowed: impl Fn(IpAddr) -> Result<(), NetworkError>,
    timeout: Duration,
    abort: &AbortHandle,
) -> Result<(TcpStream, SocketAddr), NetworkError> {
    let attempt: Attempt =
        Arc::new(|address, timeout| TcpStream::connect_timeout(&address, timeout));
    connect_with(addresses, allowed, timeout, abort, attempt)
}

/// How one address is tried: [`TcpStream::connect_timeout`], or what a
/// test puts in its place to see when the attempts start.
type Attempt = Arc<dyn Fn(SocketAddr, Duration) -> io::Result<TcpStream> + Send + Sync>;

/// [`connect`], trying each address with `attempt`.
fn connect_with(
    addresses: &[SocketAddr],
    allowed: impl Fn(IpAddr) -> Result<(), NetworkError>,
    timeout: Duration,
    abort: &AbortHandle,
    attempt: Attempt,
) -> Result<(TcpStream, SocketAddr), NetworkError> {
    let addresses = interleave(addresses.to_vec());
    let deadline = Instant::now() + timeout;
    let (sender, receiver) = mpsc::channel();
    let mut next = 0;
    let mut running = 0;
    let mut next_start = Instant::now();
    let mut refused = None;
    let mut failed = None;
    loop {
        if abort.is_aborted() {
            return Err(NetworkError::Aborted);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(NetworkError::Timeout);
        }
        if next < addresses.len() && now >= next_start {
            let address = addresses[next];
            next += 1;
            if let Err(e) = allowed(address.ip()) {
                refused = Some(e);
                continue;
            }
            let (sender, attempt) = (sender.clone(), attempt.clone());
            let remaining = deadline - now;
            thread::spawn(move || {
                let result = attempt(address, remaining);
                // Nobody listens once another attempt has won; the
                // stream is then dropped, and closed, here.
                let _ = sender.send((address, result));
            });
            running += 1;
            next_start = now + ATTEMPT_DELAY;
            continue;
        }
        if running == 0 && next >= addresses.len() {
            return Err(failed
                .or(refused)
                .unwrap_or_else(|| NetworkError::Connect("no address to try".to_owned())));
        }
        let mut wait = ABORT_POLL.min(deadline - now);
        if next < addresses.len() {
            wait = wait.min(next_start.saturating_duration_since(now));
        }
        match receiver.recv_timeout(wait) {
            Ok((address, Ok(stream))) => return Ok((stream, address)),
            Ok((_, Err(e))) => {
                running -= 1;
                failed = Some(NetworkError::Connect(e.to_string()));
                next_start = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => unreachable!("a sender is held here"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn addresses(list: &[&str]) -> Vec<SocketAddr> {
        list.iter().map(|a| a.parse().expect(a)).collect()
    }

    #[test]
    fn families_alternate_from_the_first() {
        let sorted = interleave(addresses(&[
            "[::1]:1",
            "[::2]:1",
            "[::3]:1",
            "10.0.0.1:1",
            "10.0.0.2:1",
        ]));
        assert_eq!(
            sorted,
            addresses(&["[::1]:1", "10.0.0.1:1", "[::2]:1", "10.0.0.2:1", "[::3]:1"])
        );
        let sorted = interleave(addresses(&["10.0.0.1:1", "[::1]:1", "[::2]:1"]));
        assert_eq!(sorted, addresses(&["10.0.0.1:1", "[::1]:1", "[::2]:1"]));
    }

    #[test]
    fn the_families_take_turns() {
        let tried = Arc::new(std::sync::Mutex::new(Vec::new()));
        let attempt: Attempt = {
            let tried = tried.clone();
            Arc::new(move |address, _| {
                tried.lock().unwrap().push(address);
                Err(io::ErrorKind::ConnectionRefused.into())
            })
        };
        let given = addresses(&["10.0.0.1:1", "10.0.0.2:1", "[::1]:1"]);
        let abort = AbortHandle::new();
        let result = connect_with(&given, |_| Ok(()), Duration::from_secs(10), &abort, attempt);
        assert!(
            matches!(result, Err(NetworkError::Connect(_))),
            "{result:?}"
        );
        assert_eq!(
            *tried.lock().unwrap(),
            addresses(&["10.0.0.1:1", "[::1]:1", "10.0.0.2:1"])
        );
        assert!(interleave(Vec::new()).is_empty());
    }

    #[test]
    fn localhost_and_ip_addresses_resolve_here() {
        let localhost = addresses(&["[::1]:8080", "127.0.0.1:8080"]);
        assert_eq!(
            resolve(&Host::Domain("localhost".into()), 8080).unwrap(),
            localhost
        );
        assert_eq!(
            resolve(&Host::Domain("a.b.localhost".into()), 8080).unwrap(),
            localhost
        );
        assert_eq!(
            resolve(&Host::Ipv4(Ipv4Addr::new(192, 0, 2, 1)), 80).unwrap(),
            addresses(&["192.0.2.1:80"])
        );
        assert!(resolve(&Host::Opaque("x".into()), 80).is_err());
        assert!(matches!(
            resolve(&Host::Domain("nowhere.invalid".into()), 80),
            Err(NetworkError::Resolve(_))
        ));
    }

    #[test]
    fn the_first_that_answers_is_taken() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap();
        let dead = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        // The dead address is refused at once, as a closed port is on
        // most systems (Windows takes seconds to say so).
        let attempt: Attempt = Arc::new(move |address, timeout| {
            if address == dead {
                return Err(io::ErrorKind::ConnectionRefused.into());
            }
            TcpStream::connect_timeout(&address, timeout)
        });
        let started = Instant::now();
        let abort = AbortHandle::new();
        let (_, address) = connect_with(
            &[dead, live],
            |_| Ok(()),
            Duration::from_secs(10),
            &abort,
            attempt,
        )
        .unwrap();
        assert_eq!(address, live);
        // A failure starts the next attempt without waiting its delay.
        assert!(started.elapsed() < ATTEMPT_DELAY, "{:?}", started.elapsed());
    }

    #[test]
    fn a_refused_address_is_skipped_and_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap();
        let refusal = NetworkError::Connect("refused by the test".into());
        let result = connect(
            &[live],
            |_| Err(refusal.clone()),
            Duration::from_secs(10),
            &AbortHandle::new(),
        );
        assert_eq!(result.unwrap_err(), refusal);
        let dead = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let result = connect(
            &[dead],
            |_| Ok(()),
            Duration::from_secs(10),
            &AbortHandle::new(),
        );
        assert!(
            matches!(result, Err(NetworkError::Connect(_))),
            "{result:?}"
        );
    }

    #[test]
    fn a_slow_address_has_company_after_the_attempt_delay() {
        let slow = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let quick_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let quick = quick_listener.local_addr().unwrap();
        let started = Instant::now();
        let tried = Arc::new(std::sync::Mutex::new(Vec::new()));
        let attempt: Attempt = {
            let tried = tried.clone();
            Arc::new(move |address, timeout| {
                tried.lock().unwrap().push(address);
                if address == slow {
                    thread::sleep(Duration::from_secs(3));
                }
                TcpStream::connect_timeout(&address, timeout)
            })
        };
        let abort = AbortHandle::new();
        let (_, address) = connect_with(
            &[slow, quick],
            |_| Ok(()),
            Duration::from_secs(10),
            &abort,
            attempt,
        )
        .unwrap();
        // The second attempt waited its delay, and not for the first.
        assert_eq!(address, quick);
        assert!(
            started.elapsed() >= ATTEMPT_DELAY,
            "{:?}",
            started.elapsed()
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(*tried.lock().unwrap(), [slow, quick]);
    }

    #[test]
    fn the_attempts_share_one_time_limit() {
        let address = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let attempt: Attempt = Arc::new(|address, timeout| {
            thread::sleep(Duration::from_secs(3));
            TcpStream::connect_timeout(&address, timeout)
        });
        let started = Instant::now();
        let limit = Duration::from_millis(400);
        let result = connect_with(
            &[address, address],
            |_| Ok(()),
            limit,
            &AbortHandle::new(),
            attempt,
        );
        assert_eq!(result.unwrap_err(), NetworkError::Timeout);
        assert!(started.elapsed() >= limit);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn an_abort_ends_the_wait_for_a_resolver() {
        let abort = AbortHandle::new();
        abort.abort();
        let host = Host::Domain("nowhere.invalid".into());
        let started = Instant::now();
        assert_eq!(
            resolve_abortable(&host, 80, &abort),
            Err(NetworkError::Aborted)
        );
        assert!(started.elapsed() < Duration::from_millis(1500));
        // Names that need no resolver are not waited for at all.
        let local = Host::Domain("localhost".into());
        assert!(resolve_abortable(&local, 80, &abort).is_ok());
    }

    #[test]
    fn an_abort_ends_the_attempts() {
        let abort = AbortHandle::new();
        abort.abort();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let result = connect(
            &[listener.local_addr().unwrap()],
            |_| Ok(()),
            Duration::from_secs(10),
            &abort,
        );
        assert_eq!(result.unwrap_err(), NetworkError::Aborted);
    }
}
