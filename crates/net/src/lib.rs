//! Networking: URLs and resource fetching (stub).
//!
//! Will contain the resource loader: HTTP(S) fetching, connection pooling,
//! caching, and content-type dispatch back to `engine`. Only `engine`
//! orchestrates fetching — no other layer performs I/O.

pub use url::Url;

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
