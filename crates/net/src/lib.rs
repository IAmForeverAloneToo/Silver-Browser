//! Networking: URLs and resource fetching (roadmap M14; see
//! `docs/design/net.md`).
//!
//! [`url`] is the URL Standard's parser. [`mime`] parses MIME types and
//! sniffs a resource's type, [`headers`] holds Fetch's header lists,
//! [`data_url`] reads `data:` URLs, and [`http`] writes HTTP/1.1
//! requests and reads responses over any stream. Fetching is still a stub; it grows
//! into the resource loader, HTTP(S) with connection pooling, that
//! `engine` drives. Only `engine` orchestrates fetching.

pub mod data_url;
pub mod headers;
pub mod http;
mod infra;
pub mod mime;
pub mod url;

pub use headers::HeaderList;
pub use mime::MimeType;
pub use url::{Host, Origin, Url};

/// Errors produced by resource fetching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    /// Fetching is not implemented yet.
    NotYetImplemented,
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::NotYetImplemented => {
                write!(f, "resource fetching is not implemented yet")
            }
        }
    }
}

impl std::error::Error for NetworkError {}

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
