//! The browser's DevTools (roadmap M36, decision D11; see
//! `docs/design/devtools.md`): the protocol the panel and the browser
//! speak, a JSON of their own, and what the panel is made of.
//!
//! A leaf crate below `engine`, which knows no other: the protocol's
//! messages are JSON text, read and written here, bounded so that no
//! message a page's content fills can make the parser use more than
//! [`MAX_MESSAGE_BYTES`] or nest past [`MAX_JSON_DEPTH`].

#![forbid(unsafe_code)]

pub mod frontend;
pub mod json;
pub mod protocol;

pub use frontend::{PANEL_CSP, PANEL_URL};
pub use json::{Json, JsonError, MAX_JSON_DEPTH, MAX_MESSAGE_BYTES};
pub use protocol::{Request, RequestError, error, event, result};
