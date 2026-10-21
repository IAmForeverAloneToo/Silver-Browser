//! HTTP/2 (M38, docs/design/http.md): its frames (RFC 9113) and its
//! header compression (RFC 7541), written in the crate as HTTP/1.1 is.
//! `hpack` and `frame` take bytes and give values, and back.

pub mod frame;
pub mod hpack;
mod huffman_table;

#[cfg(test)]
mod tests;
