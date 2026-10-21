//! TLS for `https:` (decision D4): rustls over the aws-lc-rs provider,
//! TLS 1.2 and 1.3, ALPN offering `http/1.1` alone, no 0-RTT, sessions
//! resumed from memory, and certificates checked against Mozilla's root
//! program. A certificate that fails is an error with its reason; there
//! is no way to go on anyway.

use std::io;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mio::net::TcpStream;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{CertificateError, ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::Host;
use crate::error::{AbortHandle, NetworkError, TlsFailure};

use super::stream::Waiter;

/// The client configuration every `https:` connection shares, so that
/// its session cache is shared too. `extra_roots` are trusted beside
/// Mozilla's: the tests' own root, and nothing else.
pub(crate) fn client_config(extra_roots: &[CertificateDer<'static>]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    for root in extra_roots {
        roots.add(root.clone()).expect("a root the tests made");
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .expect("the provider supports both versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    // HTTP/2 first (M38), as browsers offer it.
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.enable_early_data = false;
    Arc::new(config)
}

/// The name a certificate must be for, sent by SNI when it is a domain:
/// an IP address is checked against the certificate but not sent.
fn server_name(host: &Host) -> Result<ServerName<'static>, NetworkError> {
    match host {
        Host::Domain(name) => ServerName::try_from(name.clone())
            .map_err(|_| NetworkError::Tls(TlsFailure::Other(format!("{name} is not a DNS name")))),
        Host::Ipv4(address) => Ok(ServerName::IpAddress(IpAddr::V4(*address).into())),
        Host::Ipv6(address) => Ok(ServerName::IpAddress(IpAddr::V6(*address).into())),
        Host::Opaque(_) | Host::Empty => Err(NetworkError::Tls(TlsFailure::Other(
            "not a network host".to_owned(),
        ))),
    }
}

/// Runs the TLS handshake over `tcp`, non-blocking and watched by
/// `waiter`. `timeout` without progress ends it, and `abort` wakes the
/// wait.
pub(crate) fn handshake(
    config: &Arc<ClientConfig>,
    host: &Host,
    mut tcp: TcpStream,
    waiter: &mut Waiter,
    timeout: Duration,
    abort: &AbortHandle,
) -> Result<StreamOwned<ClientConnection, TcpStream>, NetworkError> {
    let mut connection = ClientConnection::new(config.clone(), server_name(host)?)
        .map_err(|e| NetworkError::Tls(failure(&e)))?;
    let _watch = abort.watch(waiter.waker());
    let mut deadline = Instant::now() + timeout;
    while connection.is_handshaking() {
        if abort.is_aborted() {
            return Err(NetworkError::Aborted);
        }
        match connection.complete_io(&mut tcp) {
            Ok(_) => deadline = Instant::now() + timeout,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(NetworkError::Timeout);
                }
                waiter
                    .wait(deadline - now)
                    .map_err(|e| NetworkError::Io(e.to_string()))?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_failure(e)),
        }
    }
    Ok(StreamOwned::new(connection, tcp))
}

/// A failed handshake's error: TLS's own when rustls gave one.
fn io_failure(error: io::Error) -> NetworkError {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(tls) => NetworkError::Tls(failure(tls)),
        None => NetworkError::Tls(TlsFailure::Other(error.to_string())),
    }
}

/// Why rustls refused, in the words the user is told.
pub(crate) fn failure(error: &rustls::Error) -> TlsFailure {
    match error {
        rustls::Error::InvalidCertificate(certificate) => match certificate {
            CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                TlsFailure::Expired
            }
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                TlsFailure::NotYetValid
            }
            CertificateError::UnknownIssuer => TlsFailure::UnknownIssuer,
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                TlsFailure::WrongName
            }
            other => TlsFailure::Other(other.to_string()),
        },
        other => TlsFailure::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::stream::non_blocking;
    use std::net::{Ipv4Addr, TcpListener};
    use std::thread;

    /// A handshake with a listener the kernel connects to and nobody
    /// answers.
    fn silent(timeout: Duration, abort: &AbortHandle) -> Result<(), NetworkError> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut tcp = non_blocking(tcp).unwrap();
        let mut waiter = Waiter::new(&mut tcp).unwrap();
        let host = Host::Ipv4(Ipv4Addr::LOCALHOST);
        handshake(&client_config(&[]), &host, tcp, &mut waiter, timeout, abort).map(drop)
    }

    #[test]
    fn a_silent_server_times_the_handshake_out() {
        let started = Instant::now();
        let result = silent(Duration::from_millis(500), &AbortHandle::new());
        assert_eq!(result, Err(NetworkError::Timeout));
        assert!(started.elapsed() >= Duration::from_millis(500));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn an_abort_ends_a_handshake_at_once() {
        // Before it starts...
        let abort = AbortHandle::new();
        abort.abort();
        let started = Instant::now();
        assert_eq!(
            silent(Duration::from_secs(60), &abort),
            Err(NetworkError::Aborted)
        );
        assert!(started.elapsed() < Duration::from_millis(1500));
        // ...and while it waits.
        let abort = AbortHandle::new();
        let aborter = abort.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            aborter.abort();
        });
        let started = Instant::now();
        assert_eq!(
            silent(Duration::from_secs(60), &abort),
            Err(NetworkError::Aborted)
        );
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
    }
}
