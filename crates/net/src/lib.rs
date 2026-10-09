//! Networking: URLs and resource fetching (roadmap M14; see
//! `docs/design/net.md`).
//!
//! [`url`] is the URL Standard's parser. [`mime`] parses MIME types and
//! sniffs a resource's type, [`headers`] holds Fetch's header lists,
//! [`data_url`] reads `data:` URLs, [`http`] writes HTTP/1.1 requests
//! and reads responses over any stream, and [`transport`] keeps the
//! connections they travel on, plain or over TLS. [`fetch`] puts them
//! together: a [`Client`] fetches a [`Request`] by its scheme, follows
//! its redirects and returns a [`Response`]. Only `engine` orchestrates
//! fetching.

mod address_space;
mod content_coding;
pub mod data_url;
mod error;
pub mod fetch;
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
pub use fetch::{Client, Delivery, Destination, Initiator, Method, Mode, Request, Response};
pub use headers::HeaderList;
pub use mime::MimeType;
pub use url::{Host, Origin, Url};
