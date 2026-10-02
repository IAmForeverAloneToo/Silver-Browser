//! `file:` URLs (docs/design/net.md, "Schemes"). Fetch leaves them to
//! the user agent; here a `file:` URL with an empty host reads the file
//! its path names on this machine.

use std::fs;
use std::path::PathBuf;

use crate::error::NetworkError;
use crate::headers::HeaderList;
use crate::mime::{self, MimeType};
use crate::url::Path;
use crate::url::percent::percent_decode;
use crate::{AddressSpace, Host, Url};

use super::Response;

/// The file `url` names, read whole: at most `limit` bytes, typed by its
/// extension or else sniffed.
pub(super) fn fetch(url: &Url, limit: usize) -> Result<Response, NetworkError> {
    let path = path(url)?;
    let unreadable = |e: std::io::Error| NetworkError::File(e.to_string());
    let metadata = fs::metadata(&path).map_err(unreadable)?;
    if metadata.is_dir() {
        return Err(NetworkError::File("it is a directory".to_owned()));
    }
    if metadata.len() > u64::try_from(limit).unwrap_or(u64::MAX) {
        return Err(NetworkError::BodyTooLarge);
    }
    let body = fs::read(&path).map_err(unreadable)?;
    // The file may have grown since it was looked at.
    if body.len() > limit {
        return Err(NetworkError::BodyTooLarge);
    }
    let mime_type = content_type(&path, &body);
    let mut headers = HeaderList::new();
    headers.append(b"Content-Type", &mime_type.to_bytes());
    Ok(Response {
        url_list: vec![url.clone()],
        status: 200,
        status_message: b"OK".to_vec(),
        headers,
        body,
        mime_type: Some(mime_type),
        address_space: Some(AddressSpace::Loopback),
    })
}

/// The path `url` names. Its host must be empty: a file on another
/// machine is not read. Each segment is percent-decoded, and must not
/// decode to a separator or NUL, which would name another path than the
/// URL shows.
fn path(url: &Url) -> Result<PathBuf, NetworkError> {
    if !matches!(url.host(), None | Some(Host::Empty)) {
        return Err(NetworkError::File(
            "the file is on another machine".to_owned(),
        ));
    }
    let Path::Segments(segments) = url.path() else {
        return Err(NetworkError::File("not a path".to_owned()));
    };
    let mut decoded = Vec::with_capacity(segments.len());
    for segment in segments {
        let bytes = percent_decode(segment.as_bytes());
        if bytes.iter().any(|&b| b == b'/' || b == b'\\' || b == 0) {
            return Err(NetworkError::File(
                "a path segment holds a separator".to_owned(),
            ));
        }
        let segment = String::from_utf8(bytes)
            .map_err(|_| NetworkError::File("the path is not UTF-8".to_owned()))?;
        decoded.push(segment);
    }
    native(&decoded)
}

/// The path from its decoded segments: `/a/b` on Unix.
#[cfg(not(windows))]
fn native(segments: &[String]) -> Result<PathBuf, NetworkError> {
    Ok(PathBuf::from(format!("/{}", segments.join("/"))))
}

/// The path from its decoded segments: on Windows a `file:` path begins
/// with a drive letter, which the URL parser has normalized to `C:`, and
/// `/C:/a/b` is `C:\a\b`.
#[cfg(windows)]
fn native(segments: &[String]) -> Result<PathBuf, NetworkError> {
    match segments.first().map(String::as_bytes) {
        Some([letter, b':']) if letter.is_ascii_alphabetic() => Ok(PathBuf::from(format!(
            "{}\\{}",
            segments[0],
            segments[1..].join("\\")
        ))),
        _ => Err(NetworkError::File(
            "the path has no drive letter".to_owned(),
        )),
    }
}

/// The type of the file at `path`: by its extension for the types the
/// engine knows, by MIME Sniffing's rules for an unknown type otherwise.
fn content_type(path: &std::path::Path, body: &[u8]) -> MimeType {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let known = match extension.as_deref() {
        Some("html" | "htm") => Some("text/html"),
        Some("css") => Some("text/css"),
        Some("txt") => Some("text/plain"),
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("avif") => Some("image/avif"),
        Some("svg") => Some("image/svg+xml"),
        _ => None,
    };
    match known {
        Some(essence) => MimeType::known(essence),
        None => mime::sniff::identify_unknown(body, true),
    }
}
