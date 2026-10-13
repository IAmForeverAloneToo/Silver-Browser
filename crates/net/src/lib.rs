//! Networking: URLs and resource fetching (roadmap M14; see
//! `docs/design/net.md`).
//!
//! [`url`] is the URL Standard's parser. [`mime`] parses MIME types and
//! sniffs a resource's type, [`headers`] holds Fetch's header lists,
//! [`data_url`] reads `data:` URLs, [`http`] writes HTTP/1.1 requests
//! and reads responses over any stream, and [`transport`] keeps the
//! connections they travel on, plain or over TLS. [`fetch`] puts them
//! together: a [`Client`] fetches a [`Request`] by its scheme, follows
//! its redirects and returns a [`Response`]. [`csp`] parses a document's
//! Content Security Policy and checks its scripts against it. Only
//! `engine` orchestrates fetching.

mod address_space;
pub mod cache;
mod content_coding;
pub mod cookies;
pub mod csp;
pub mod data_url;
mod error;
pub mod fetch;
pub mod headers;
pub mod hsts;
pub mod http;
mod infra;
pub mod mime;
pub mod referrer;
#[cfg(any(test, feature = "test-server"))]
pub mod test_server;
pub mod transport;
pub mod url;

pub use address_space::{AddressSpace, LocalNetworkAccess, LocalNetworkGate};
pub use cache::{Cache, CacheMode};
pub use error::{AbortHandle, NetworkError, TlsFailure};
pub use fetch::{
    COMPAT_USER_AGENT, Client, Credentials, Delivery, Destination, Exposed, Initiator, Method,
    Mode, Redirect, Request, Response, ResponseType, USER_AGENT,
};
pub use headers::HeaderList;
pub use hsts::Hsts;
pub use mime::MimeType;
pub use referrer::{Referrer, ReferrerPolicy};
pub use url::Partition;
pub use url::{Host, Origin, Url};
