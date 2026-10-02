//! Networking: URLs and resource fetching (roadmap M14; see
//! `docs/design/net.md`).
//!
//! [`url`] is the URL Standard's parser. [`mime`] parses MIME types and
//! sniffs a resource's type, [`headers`] holds Fetch's header lists,
//! [`data_url`] reads `data:` URLs, [`http`] writes HTTP/1.1 requests
//! and reads responses over any stream, and [`transport`] keeps the
//! connections they travel on, plain or over TLS. Fetching itself is
//! still a stub; it grows into the resource loader that `engine`
//! drives. Only `engine` orchestrates fetching.

mod address_space;
mod content_coding;
pub mod data_url;
mod error;
pub mod headers;
pub mod http;
mod infra;
pub mod mime;
#[cfg(any(test, feature = "test-server"))]
pub mod test_server;
pub mod transport;
pub mod url;

pub use address_space::AddressSpace;
pub use error::{AbortHandle, NetworkError, TlsFailure};
pub use headers::HeaderList;
pub use mime::MimeType;
pub use url::{Host, Origin, Url};

/// Fetches a resource.
///
/// Currently always returns [`NetworkError::NotYetImplemented`].
pub fn fetch(_url: &Url) -> Result<Vec<u8>, NetworkError> {
    Err(NetworkError::NotYetImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_is_explicitly_unimplemented() {
        let url = Url::parse("https://example.org/").expect("valid URL");
        assert_eq!(fetch(&url), Err(NetworkError::NotYetImplemented));
    }
}
