//! HTTP/1.1 messages (RFC 9112) over any byte stream: a request written,
//! a response read. Plain TCP and TLS are both just streams here, so the
//! same code is tested over memory and fuzzed. Connections, their reuse
//! and their time limits are the connection layer's.

mod read;

use std::fmt;
use std::io::{self, Write};

use crate::Url;
use crate::headers::{HeaderList, is_http_token_code_point};

pub use read::{BodySink, Malformed, ReadError, read_response, read_response_to};

/// The most a response's status lines and header fields may take,
/// interim responses and a chunked body's trailer fields included: 256
/// KiB. Browsers allow a few hundred KiB; real responses use a few KiB.
pub const HEADER_LIMIT: usize = 256 * 1024;

/// The most a response's body may hold: 64 MiB. A page and its
/// stylesheets are far smaller; this bounds what a server can make the
/// engine keep in memory, since a body is read whole.
pub const BODY_LIMIT: usize = 64 * 1024 * 1024;

/// What one response may take: [`HEADER_LIMIT`] and [`BODY_LIMIT`]
/// unless a test asks for less.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of status lines and header and trailer field lines, line
    /// endings included.
    pub header_bytes: usize,
    /// Bytes of body, after any chunked coding is removed. The chunked
    /// coding's own size lines and line endings may take as much again.
    pub body_bytes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            header_bytes: HEADER_LIMIT,
            body_bytes: BODY_LIMIT,
        }
    }
}

/// The HTTP version a response says it speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Http10,
    Http11,
}

/// A response, read whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub version: Version,
    pub status: u16,
    /// The reason phrase, as it came; nothing should depend on it.
    pub reason: Vec<u8>,
    pub headers: HeaderList,
    /// The body with any transfer coding removed. Content codings are
    /// left as they are.
    pub body: Vec<u8>,
    /// Whether the connection may carry another request: an HTTP/1.1
    /// response without `Connection: close`, whose end the message
    /// itself marked. Whether bytes are already waiting after it is for
    /// the connection's owner to check.
    pub persistent: bool,
}

/// A request to write.
#[derive(Debug, Clone, Copy)]
pub struct RequestMessage<'a> {
    /// The method, a token, normalized by the caller.
    pub method: &'a str,
    /// An `http:` or `https:` URL. The request names its path and query
    /// (the origin form) and its host and port in `Host`; never its
    /// fragment.
    pub url: &'a Url,
    /// The fields to send after `Host`, in order. `Host` and
    /// `Content-Length` are the writer's own and may not be among them.
    pub headers: &'a HeaderList,
    /// The body, sent with its `Content-Length`; `None` sends neither.
    pub body: Option<&'a [u8]>,
}

/// Writes `request` to `stream` as HTTP/1.1, in one write: the request
/// line, `Host`, the fields, `Content-Length` with a body, the empty
/// line and the body.
///
/// A method or field name that is not a token, a field value holding
/// NUL, CR or LF, a `Host` or `Content-Length` among the fields, or a
/// URL that is not `http:` or `https:` is refused as invalid input:
/// nothing a caller passes can split the message.
pub fn write_request(stream: &mut impl Write, request: &RequestMessage) -> io::Result<()> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_owned());
    if !is_token(request.method.as_bytes()) {
        return Err(invalid("the method is not a token"));
    }
    let url = request.url;
    let Some(host) = url
        .host()
        .filter(|_| matches!(url.scheme(), "http" | "https"))
    else {
        return Err(invalid("not an http: or https: URL"));
    };
    let mut message = Vec::with_capacity(512);
    message.extend_from_slice(request.method.as_bytes());
    message.push(b' ');
    // The origin form: the path, and the query with its '?'. Both are
    // ASCII, as the URL serializer writes them.
    message.extend_from_slice(url.path_str().as_bytes());
    if let Some(query) = url.query() {
        message.push(b'?');
        message.extend_from_slice(query.as_bytes());
    }
    message.extend_from_slice(b" HTTP/1.1\r\nHost: ");
    message.extend_from_slice(host.to_string().as_bytes());
    // A URL keeps no port that is its scheme's default.
    if let Some(port) = url.port() {
        message.extend_from_slice(format!(":{port}").as_bytes());
    }
    message.extend_from_slice(b"\r\n");
    for (name, value) in request.headers.iter() {
        if !is_token(name) {
            return Err(invalid("a field name is not a token"));
        }
        if name.eq_ignore_ascii_case(b"Host") || name.eq_ignore_ascii_case(b"Content-Length") {
            return Err(invalid("Host and Content-Length are the writer's"));
        }
        if value.iter().any(|&b| matches!(b, b'\0' | b'\r' | b'\n')) {
            return Err(invalid("a field value holds NUL, CR or LF"));
        }
        message.extend_from_slice(name);
        message.extend_from_slice(b": ");
        message.extend_from_slice(value);
        message.extend_from_slice(b"\r\n");
    }
    if let Some(body) = request.body {
        message.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    }
    message.extend_from_slice(b"\r\n");
    if let Some(body) = request.body {
        message.extend_from_slice(body);
    }
    stream.write_all(&message)?;
    stream.flush()
}

/// A non-empty run of token characters.
fn is_token(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes
            .iter()
            .all(|&b| is_http_token_code_point(char::from(b)))
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Version::Http10 => "HTTP/1.0",
            Version::Http11 => "HTTP/1.1",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(
        method: &str,
        url: &str,
        fields: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> io::Result<String> {
        let url = Url::parse(url).expect("a URL");
        let mut headers = HeaderList::new();
        for (name, value) in fields {
            headers.append(name.as_bytes(), value.as_bytes());
        }
        let mut out = Vec::new();
        write_request(
            &mut out,
            &RequestMessage {
                method,
                url: &url,
                headers: &headers,
                body,
            },
        )?;
        Ok(String::from_utf8(out).expect("ASCII"))
    }

    #[test]
    fn a_request_as_written() {
        assert_eq!(
            written(
                "GET",
                "http://example.com/a/b?c=d#frag",
                &[("Accept", "*/*")],
                None
            )
            .unwrap(),
            "GET /a/b?c=d HTTP/1.1\r\nHost: example.com\r\nAccept: */*\r\n\r\n"
        );
        assert_eq!(
            written("GET", "https://example.com", &[], None).unwrap(),
            "GET / HTTP/1.1\r\nHost: example.com\r\n\r\n"
        );
        // The port is written when it is not the scheme's own.
        assert_eq!(
            written("HEAD", "https://example.com:8443/", &[], None).unwrap(),
            "HEAD / HTTP/1.1\r\nHost: example.com:8443\r\n\r\n"
        );
        assert_eq!(
            written("GET", "http://example.com:80/", &[], None).unwrap(),
            "GET / HTTP/1.1\r\nHost: example.com\r\n\r\n"
        );
        assert_eq!(
            written("GET", "http://[::1]:8080/x?", &[], None).unwrap(),
            "GET /x? HTTP/1.1\r\nHost: [::1]:8080\r\n\r\n"
        );
        assert_eq!(
            written(
                "POST",
                "http://127.0.0.1/f",
                &[("Content-Type", "text/plain")],
                Some(b"hi")
            )
            .unwrap(),
            "POST /f HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nhi"
        );
        assert_eq!(
            written("POST", "http://a/", &[], Some(b"")).unwrap(),
            "POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn nothing_passed_in_can_split_the_message() {
        let refused = |result: io::Result<String>| {
            result.expect_err("refused").kind() == io::ErrorKind::InvalidInput
        };
        assert!(refused(written("GET /x", "http://a/", &[], None)));
        assert!(refused(written("", "http://a/", &[], None)));
        assert!(refused(written(
            "GET",
            "http://a/",
            &[("X", "a\r\nEvil: 1")],
            None
        )));
        assert!(refused(written("GET", "http://a/", &[("X", "a\nb")], None)));
        assert!(refused(written("GET", "http://a/", &[("X", "a\0b")], None)));
        assert!(refused(written("GET", "http://a/", &[("X Y", "1")], None)));
        assert!(refused(written("GET", "http://a/", &[("host", "b")], None)));
        assert!(refused(written(
            "GET",
            "http://a/",
            &[("content-length", "1")],
            None
        )));
        assert!(refused(written("GET", "ftp://a/", &[], None)));
        assert!(refused(written("GET", "data:,x", &[], None)));
    }
}
