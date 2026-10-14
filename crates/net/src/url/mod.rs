//! URLs as the WHATWG URL Standard parses and writes them (roadmap
//! M14.1, decision D17; see `docs/design/net.md`): the basic URL parser
//! with its host parser, the serializer, and origins.
//!
//! Validation errors are not reported: parsing gives a URL or a
//! [`ParseError`], which is all a browser acts on. The URL API's setters
//! and `application/x-www-form-urlencoded` come with their users,
//! scripts and form submission (M25, M27).

pub mod form;
mod host;
mod parser;
pub(crate) mod percent;

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use encoding::{Encoding, UTF_8};

pub use host::Host;
pub use percent::percent_decode;

/// Why a string is not a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// A relative reference with no base URL to resolve it against, or a
    /// base with an opaque path that only a fragment can follow.
    RelativeWithoutBase,
    /// A special URL, or one with credentials, with nothing for a host.
    EmptyHost,
    /// A host with a forbidden code point.
    InvalidHost,
    /// A domain that IDNA refuses or that is empty after it, or that holds
    /// a forbidden domain code point.
    InvalidDomain,
    /// A host that ends in a number but is not an IPv4 address.
    InvalidIpv4,
    /// A bracketed host that is not an IPv6 address.
    InvalidIpv6,
    /// A port that is not a number up to 65535.
    InvalidPort,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ParseError::RelativeWithoutBase => "a relative URL with nothing to resolve it against",
            ParseError::EmptyHost => "the URL needs a host and has none",
            ParseError::InvalidHost => "the host has a character no host may have",
            ParseError::InvalidDomain => "the domain is not a valid domain name",
            ParseError::InvalidIpv4 => "the host ends in a number but is not an IPv4 address",
            ParseError::InvalidIpv6 => "the host is not a valid IPv6 address",
            ParseError::InvalidPort => "the port is not a number from 0 to 65535",
        })
    }
}

impl std::error::Error for ParseError {}

/// A URL's path (URL Standard §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Path {
    /// An opaque path, as in `mailto:a@example.com`: one string.
    Opaque(String),
    /// A list of segments, each without `/`; a special URL's path is
    /// always one.
    Segments(Vec<String>),
}

impl Default for Path {
    fn default() -> Self {
        Path::Segments(Vec::new())
    }
}

/// The URL record the standard describes, as the parser builds it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Record {
    pub(crate) scheme: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) host: Option<Host>,
    pub(crate) port: Option<u16>,
    pub(crate) path: Path,
    pub(crate) query: Option<String>,
    pub(crate) fragment: Option<String>,
}

/// Whether `scheme` is special: `ftp`, `file`, `http`, `https`, `ws` or
/// `wss` (URL Standard §4.2).
pub(crate) fn is_special_scheme(scheme: &str) -> bool {
    matches!(scheme, "ftp" | "file" | "http" | "https" | "ws" | "wss")
}

/// A special scheme's default port.
pub(crate) fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "ftp" => Some(21),
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

/// A parsed URL: the record and its serialization, made once when the
/// URL is parsed.
///
/// Two URLs are equal when their serializations are (URL Standard
/// §4.6).
#[derive(Clone)]
pub struct Url {
    record: Record,
    serialization: String,
}

impl Url {
    /// Parses an absolute URL.
    pub fn parse(input: &str) -> Result<Url, ParseError> {
        Url::parse_with(input, None, UTF_8)
    }

    /// Parses `input` against this URL as its base, with UTF-8 for the
    /// query.
    pub fn join(&self, input: &str) -> Result<Url, ParseError> {
        Url::parse_with(input, Some(self), UTF_8)
    }

    /// The URL parser: `input` against an optional `base`, a query
    /// encoded with `encoding`'s output encoding where the URL is special
    /// and not `ws:` or `wss:`, as HTML parses a document's URLs with the
    /// document's encoding.
    pub fn parse_with(
        input: &str,
        base: Option<&Url>,
        encoding: Encoding,
    ) -> Result<Url, ParseError> {
        let record = parser::parse(input, base.map(|base| &base.record), encoding)?;
        Ok(Url::from_record(record))
    }

    fn from_record(record: Record) -> Url {
        let serialization = serialize(&record);
        Url {
            record,
            serialization,
        }
    }

    /// The `file:` URL of an absolute path, which `file:` fetching reads
    /// back as the same path. Each component is percent-encoded where
    /// the URL parser would read it otherwise: `%`, `#` and `?`,
    /// separators, `|`, which can stand for a drive letter's colon, and
    /// space and the C0 controls, which the parser strips or trims.
    /// `None` for a relative path, one that is not UTF-8, or on Windows
    /// one that does not begin with a drive letter.
    pub fn from_file_path(path: &std::path::Path) -> Option<Url> {
        use std::path::{Component, Prefix};
        if !path.is_absolute() {
            return None;
        }
        let mut input = String::from("file://");
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => match prefix.kind() {
                    Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                        input.push('/');
                        input.push(char::from(letter));
                        input.push(':');
                    }
                    _ => return None,
                },
                Component::RootDir => {}
                Component::CurDir => input.push_str("/."),
                Component::ParentDir => input.push_str("/.."),
                Component::Normal(segment) => {
                    input.push('/');
                    for c in segment.to_str()?.chars() {
                        if c <= ' ' || matches!(c, '%' | '#' | '?' | '/' | '\\' | '|') {
                            input.push_str(&format!("%{:02X}", u32::from(c)));
                        } else {
                            input.push(c);
                        }
                    }
                }
            }
        }
        Url::parse(&input).ok()
    }

    /// The serialization (URL Standard §4.5).
    pub fn as_str(&self) -> &str {
        &self.serialization
    }

    /// The serialization without the fragment.
    pub fn without_fragment(&self) -> &str {
        match &self.record.fragment {
            Some(fragment) => &self.serialization[..self.serialization.len() - fragment.len() - 1],
            None => &self.serialization,
        }
    }

    /// The scheme, lowercase, without its `:`.
    pub fn scheme(&self) -> &str {
        &self.record.scheme
    }

    /// Whether the scheme is special.
    pub fn is_special(&self) -> bool {
        is_special_scheme(&self.record.scheme)
    }

    /// The username, percent-encoded; empty when there is none.
    pub fn username(&self) -> &str {
        &self.record.username
    }

    /// The password, percent-encoded; empty when there is none.
    pub fn password(&self) -> &str {
        &self.record.password
    }

    /// Whether the URL has a username or a password.
    pub fn includes_credentials(&self) -> bool {
        !self.record.username.is_empty() || !self.record.password.is_empty()
    }

    /// The host, if any.
    pub fn host(&self) -> Option<&Host> {
        self.record.host.as_ref()
    }

    /// The port, or `None` when it is absent or the scheme's default.
    pub fn port(&self) -> Option<u16> {
        self.record.port
    }

    /// The port, or the scheme's default when it has one.
    pub fn port_or_default(&self) -> Option<u16> {
        self.record
            .port
            .or_else(|| default_port(&self.record.scheme))
    }

    /// The path.
    pub fn path(&self) -> &Path {
        &self.record.path
    }

    /// The URL path serializer's output: an opaque path as it is, a list
    /// of segments each after a `/`.
    pub fn path_str(&self) -> String {
        let mut out = String::new();
        write_path(&self.record.path, &mut out);
        out
    }

    /// The query, without its `?`, if any.
    pub fn query(&self) -> Option<&str> {
        self.record.query.as_deref()
    }

    /// The fragment, without its `#`, if any.
    pub fn fragment(&self) -> Option<&str> {
        self.record.fragment.as_deref()
    }

    /// This URL with its fragment replaced: `fragment` is put through the
    /// fragment percent-encode set, which leaves a fragment taken from
    /// another URL as it was.
    pub fn with_fragment(&self, fragment: Option<&str>) -> Url {
        let mut record = self.record.clone();
        record.fragment = fragment.map(parser::encode_fragment);
        Url::from_record(record)
    }

    /// Whether the URL is potentially trustworthy (Secure Contexts 3.2):
    /// `about:blank`, `about:srcdoc` and `data:`; `https:`, `wss:` and
    /// `file:`; a loopback address; `localhost` and the names under it.
    /// What a secure context, a `Secure` cookie and mixed content ask.
    pub fn is_potentially_trustworthy(&self) -> bool {
        if matches!(self.as_str(), "about:blank" | "about:srcdoc") {
            return true;
        }
        match self.scheme() {
            "data" | "https" | "wss" | "file" => true,
            _ => match self.host() {
                Some(Host::Ipv4(address)) => address.is_loopback(),
                Some(Host::Ipv6(address)) => address.is_loopback(),
                Some(Host::Domain(name)) => {
                    name == "localhost"
                        || name == "localhost."
                        || name.ends_with(".localhost")
                        || name.ends_with(".localhost.")
                }
                _ => false,
            },
        }
    }

    /// The URL's origin (URL Standard §4.7). `file:` URLs and schemes
    /// other than the network ones get a new opaque origin each time, as
    /// the standard says: such a URL is not same origin with itself.
    pub fn origin(&self) -> Origin {
        match self.scheme() {
            "ftp" | "http" | "https" | "ws" | "wss" => Origin::Tuple {
                scheme: self.record.scheme.clone(),
                host: self.record.host.clone().expect("a special URL has a host"),
                port: self.record.port,
            },
            "blob" => match Url::parse(&self.path_str()) {
                Ok(inner) if matches!(inner.scheme(), "http" | "https" | "file") => inner.origin(),
                _ => Origin::new_opaque(),
            },
            _ => Origin::new_opaque(),
        }
    }
}

/// The URL serializer (URL Standard §4.5).
fn serialize(record: &Record) -> String {
    let mut out = String::with_capacity(record.scheme.len() + 32);
    out.push_str(&record.scheme);
    out.push(':');
    if let Some(host) = &record.host {
        out.push_str("//");
        if !record.username.is_empty() || !record.password.is_empty() {
            out.push_str(&record.username);
            if !record.password.is_empty() {
                out.push(':');
                out.push_str(&record.password);
            }
            out.push('@');
        }
        out.push_str(&host.to_string());
        if let Some(port) = record.port {
            out.push(':');
            out.push_str(&port.to_string());
        }
    }
    if record.host.is_none()
        && let Path::Segments(segments) = &record.path
        && segments.len() > 1
        && segments[0].is_empty()
    {
        // Keeps web+demo:/.//not-a-host/ from serializing as a URL
        // with a host.
        out.push_str("/.");
    }
    write_path(&record.path, &mut out);
    if let Some(query) = &record.query {
        out.push('?');
        out.push_str(query);
    }
    if let Some(fragment) = &record.fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// The URL path serializer.
fn write_path(path: &Path, out: &mut String) {
    match path {
        Path::Opaque(path) => out.push_str(path),
        Path::Segments(segments) => {
            for segment in segments {
                out.push('/');
                out.push_str(segment);
            }
        }
    }
}

impl PartialEq for Url {
    fn eq(&self, other: &Url) -> bool {
        self.serialization == other.serialization
    }
}

impl Eq for Url {}

impl Hash for Url {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.serialization.hash(state);
    }
}

impl fmt::Debug for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Url({:?})", self.serialization)
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.serialization)
    }
}

/// An origin (HTML §7.1.1): a scheme, host and port, or an opaque origin
/// equal only to itself.
#[derive(Debug, Clone)]
pub enum Origin {
    /// A tuple origin.
    Tuple {
        /// The scheme.
        scheme: String,
        /// The host.
        host: Host,
        /// The port, `None` for the scheme's default.
        port: Option<u16>,
    },
    /// An opaque origin, named by a number no other has.
    Opaque(u64),
}

/// The number of the next opaque origin made in this process.
static NEXT_OPAQUE: AtomicU64 = AtomicU64::new(0);

/// This process's opaque origins are numbered from `start` on, if its
/// numbers have not passed it (M34.2): each process of the browser is
/// given a space of its own, so that two processes' opaque origins never
/// share a number.
pub fn start_opaque_origins_at(start: u64) {
    NEXT_OPAQUE.fetch_max(start, Ordering::Relaxed);
}

impl Origin {
    /// A new opaque origin, same origin with nothing but itself.
    pub fn new_opaque() -> Origin {
        Origin::Opaque(NEXT_OPAQUE.fetch_add(1, Ordering::Relaxed))
    }

    /// Whether the origin is opaque.
    pub fn is_opaque(&self) -> bool {
        matches!(self, Origin::Opaque(_))
    }

    /// The serialization of an origin: `scheme://host[:port]`, or
    /// `null` for an opaque one.
    pub fn serialization(&self) -> String {
        match self {
            Origin::Tuple { scheme, host, port } => match port {
                Some(port) => format!("{scheme}://{host}:{port}"),
                None => format!("{scheme}://{host}"),
            },
            Origin::Opaque(_) => "null".to_owned(),
        }
    }
}

impl Origin {
    /// The origin's site (HTML 7.1.1.1): an opaque origin itself, else
    /// its scheme and its host's registrable domain, or the host when it
    /// has none (an address, or a domain that is a public suffix).
    pub fn site(&self) -> Site {
        match self {
            Origin::Opaque(id) => Site::Opaque(*id),
            Origin::Tuple { scheme, host, .. } => Site::Schemeful {
                scheme: scheme.clone(),
                host: host
                    .registrable_domain()
                    .map_or_else(|| host.to_string(), str::to_owned),
            },
        }
    }

    /// Whether the two origins are same site (HTML 7.1.1.1): their
    /// sites equal, scheme and all.
    pub fn same_site(&self, other: &Origin) -> bool {
        self.site() == other.site()
    }

    /// Whether the two origins are schemelessly same site (HTML
    /// 7.1.1.1): the same opaque origin, hosts with the same registrable
    /// domain, or the same host where it has none.
    pub fn schemelessly_same_site(&self, other: &Origin) -> bool {
        match (self, other) {
            (Origin::Opaque(a), Origin::Opaque(b)) => a == b,
            (Origin::Tuple { host: a, .. }, Origin::Tuple { host: b, .. }) => {
                match (a.registrable_domain(), b.registrable_domain()) {
                    (Some(a), Some(b)) => a == b,
                    _ => a == b,
                }
            }
            _ => false,
        }
    }
}

/// A site (HTML 7.1.1.1): an opaque origin, or a scheme and a host's
/// registrable domain (or the host, serialized, when it has none).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Site {
    /// An opaque origin's site, itself.
    Opaque(u64),
    /// A scheme and a registrable domain or host.
    Schemeful {
        /// The scheme.
        scheme: String,
        /// The registrable domain, or the host's serialization.
        host: String,
    },
}

impl Site {
    /// The site as text: `scheme://host`, or `null` with its number for
    /// an opaque origin's, which is no other's (M33.2: what partitions
    /// are keyed by).
    pub fn serialization(&self) -> String {
        match self {
            Site::Opaque(id) => format!("null:{id}"),
            Site::Schemeful { scheme, host } => format!("{scheme}://{host}"),
        }
    }
}

/// Where a request's document is in its tab (M33.2): the top-level
/// document's origin, and whether the document, or one of its
/// ancestors, is of another site than it. A top-level document's
/// requests have none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub top: Origin,
    pub cross_site: bool,
}

impl Partition {
    /// The key its cookies of `Partitioned` are kept under (CHIPS): the
    /// top-level site.
    pub fn cookie_key(&self) -> String {
        self.top.site().serialization()
    }

    /// The key what it keeps apart is kept under: the top-level site,
    /// and whether the context is a cross-site one (the reference
    /// browser's storage key's ancestor bit).
    pub fn key(&self) -> String {
        let cross = if self.cross_site { "^cross" } else { "" };
        format!("{}{cross}", self.cookie_key())
    }
}

impl PartialEq for Origin {
    /// Same origin: two tuples with the same scheme, host and port, or
    /// an opaque origin and itself.
    fn eq(&self, other: &Origin) -> bool {
        match (self, other) {
            (
                Origin::Tuple { scheme, host, port },
                Origin::Tuple {
                    scheme: other_scheme,
                    host: other_host,
                    port: other_port,
                },
            ) => scheme == other_scheme && host == other_host && port == other_port,
            (Origin::Opaque(a), Origin::Opaque(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Origin {}

#[cfg(test)]
mod tests {
    use super::*;

    fn href(input: &str) -> String {
        Url::parse(input)
            .map(|u| u.to_string())
            .unwrap_or_else(|e| format!("error: {e:?}"))
    }

    #[test]
    fn a_file_path_becomes_the_file_url_that_names_it() {
        let (root, prefix) = if cfg!(windows) {
            ("C:\\", "file:///C:/")
        } else {
            ("/", "file:///")
        };
        let path = std::path::Path::new(root)
            .join("srv")
            .join("a b")
            .join("c#d%e?f|g\u{1}\u{e9}.html");
        assert_eq!(
            Url::from_file_path(&path).map(|url| url.to_string()),
            Some(format!("{prefix}srv/a%20b/c%23d%25e%3Ff%7Cg%01%C3%A9.html"))
        );
        // The directories . and .. are resolved as the URL parser does.
        let dotted = std::path::Path::new(root).join("a").join("..").join("b");
        assert_eq!(
            Url::from_file_path(&dotted).map(|url| url.to_string()),
            Some(format!("{prefix}b"))
        );
        assert_eq!(Url::from_file_path(std::path::Path::new("relative")), None);
        if cfg!(windows) {
            // A verbatim drive path is its drive path; a share is not a
            // file: URL this engine reads.
            assert_eq!(
                Url::from_file_path(std::path::Path::new(r"\\?\C:\a\b.html"))
                    .map(|url| url.to_string()),
                Some("file:///C:/a/b.html".to_owned())
            );
            assert_eq!(
                Url::from_file_path(std::path::Path::new(r"\\server\share\a.html")),
                None
            );
        }
    }

    #[test]
    fn components_of_the_standards_examples() {
        // URL Standard §4.1's table.
        let url = Url::parse("https://localhost:8000/search?q=text#hello").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host(), Some(&Host::Domain("localhost".into())));
        assert_eq!(url.port(), Some(8000));
        assert_eq!(url.path(), &Path::Segments(vec!["search".into()]));
        assert_eq!(
            (url.query(), url.fragment()),
            (Some("q=text"), Some("hello"))
        );
        assert_eq!(
            url.without_fragment(),
            "https://localhost:8000/search?q=text"
        );

        let url = Url::parse("urn:isbn:9780307476463").unwrap();
        assert_eq!(
            (url.host(), url.path()),
            (None, &Path::Opaque("isbn:9780307476463".into()))
        );

        let url = Url::parse("file:///ada/Analytical%20Engine/README.md").unwrap();
        assert_eq!(url.host(), Some(&Host::Empty));
        assert_eq!(url.path_str(), "/ada/Analytical%20Engine/README.md");
    }

    #[test]
    fn relative_references() {
        let base = Url::parse("http://example.com/a/b/c?q#f").unwrap();
        for (input, expected) in [
            ("d", "http://example.com/a/b/d"),
            ("../d", "http://example.com/a/d"),
            ("/d", "http://example.com/d"),
            ("//other/d", "http://other/d"),
            ("?x", "http://example.com/a/b/c?x"),
            ("#g", "http://example.com/a/b/c?q#g"),
            ("", "http://example.com/a/b/c?q"),
            ("https:d", "https://d/"),
        ] {
            assert_eq!(base.join(input).unwrap().as_str(), expected, "{input:?}");
        }
        assert_eq!(Url::parse("d"), Err(ParseError::RelativeWithoutBase));
    }

    #[test]
    fn special_details() {
        assert_eq!(href("HTTP://EXAMPLE.com:80/"), "http://example.com/");
        assert_eq!(href("https://example.com:0443/"), "https://example.com/");
        assert_eq!(href("http://example.com:65536/"), "error: InvalidPort");
        assert_eq!(href("http://u:p@[::1]:8/x"), "http://u:p@[::1]:8/x");
        assert_eq!(href("http://a\\b\\..\\c"), "http://a/c");
        assert_eq!(href("http:/\\/x"), "http://x/");
        assert_eq!(href("http://"), "error: EmptyHost");
        assert_eq!(href(" \t http://x/\na b\t "), "http://x/a%20b");
        assert_eq!(href("web+demo:/.//not-a-host/"), "web+demo:/.//not-a-host/");
        assert_eq!(href("file:///C|/x"), "file:///C:/x");
        assert_eq!(href("file://localhost/x"), "file:///x");
    }

    #[test]
    fn queries_take_the_documents_encoding_where_the_url_is_special() {
        let base = Url::parse("http://example.com/").unwrap();
        let special = Url::parse_with("?\u{E9}", Some(&base), encoding::WINDOWS_1252).unwrap();
        assert_eq!(special.query(), Some("%E9"));
        let ws = Url::parse_with("ws://x/?\u{E9}", None, encoding::WINDOWS_1252).unwrap();
        assert_eq!(ws.query(), Some("%C3%A9"));
        let other = Url::parse_with("x:/?\u{E9}", None, encoding::WINDOWS_1252).unwrap();
        assert_eq!(other.query(), Some("%C3%A9"));
        // UTF-16 documents encode URLs as UTF-8, their output encoding.
        let utf16 = Url::parse_with("?\u{E9}", Some(&base), encoding::UTF_16LE).unwrap();
        assert_eq!(utf16.query(), Some("%C3%A9"));
    }

    #[test]
    fn origins() {
        let a = Url::parse("https://example.com/a").unwrap().origin();
        let b = Url::parse("https://example.com:443/b").unwrap().origin();
        assert_eq!(a, b);
        assert_eq!(a.serialization(), "https://example.com");
        assert_ne!(a, Url::parse("http://example.com/").unwrap().origin());
        let blob = Url::parse("blob:https://whatwg.org/d0360e2f").unwrap();
        assert_eq!(blob.origin().serialization(), "https://whatwg.org");
        let file = Url::parse("file:///x").unwrap();
        assert!(file.origin().is_opaque());
        assert_ne!(file.origin(), file.origin());
        let opaque = Origin::new_opaque();
        assert_eq!(opaque.clone(), opaque);
        assert_eq!(opaque.serialization(), "null");
    }

    #[test]
    fn potentially_trustworthy_urls_are_secure_contexts_sources() {
        let trustworthy = |input: &str| Url::parse(input).unwrap().is_potentially_trustworthy();
        for input in [
            "about:blank",
            "about:srcdoc",
            "data:,x",
            "https://example.com/",
            "wss://example.com/",
            "file:///x",
            "http://127.0.0.1/",
            "http://127.200.0.1:8080/",
            "http://[::1]/",
            "http://localhost/",
            "http://localhost./",
            "http://a.localhost/",
            "ws://b.a.localhost./",
        ] {
            assert!(trustworthy(input), "{input}");
        }
        for input in [
            "http://example.com/",
            "http://10.0.0.1/",
            "http://[::2]/",
            "http://localhost.example/",
            "http://notlocalhost/",
            "about:silver",
            "ftp://localhost.example/",
        ] {
            assert!(!trustworthy(input), "{input}");
        }
    }

    #[test]
    fn hosts_have_public_suffixes_and_registrable_domains() {
        let host = |input: &str| {
            Url::parse(&format!("http://{input}/"))
                .unwrap()
                .host()
                .cloned()
                .unwrap()
        };
        let both = |input: &str| {
            let host = host(input);
            (
                host.public_suffix().map(str::to_owned),
                host.registrable_domain().map(str::to_owned),
            )
        };
        let some = |a: &str, b: &str| (Some(a.to_owned()), Some(b.to_owned()));
        assert_eq!(both("www.example.com"), some("com", "example.com"));
        assert_eq!(both("a.b.example.co.uk"), some("co.uk", "example.co.uk"));
        assert_eq!(both("co.uk"), (Some("co.uk".to_owned()), None));
        // The list's private section: each project its own site.
        assert_eq!(both("a.b.github.io"), some("github.io", "b.github.io"));
        // A name the list does not know: its last label, by the rule `*`.
        assert_eq!(both("a.localhost"), some("localhost", "a.localhost"));
        assert_eq!(both("localhost"), (Some("localhost".to_owned()), None));
        assert_eq!(both("example.com."), some("com.", "example.com."));
        assert_eq!(both("127.0.0.1"), (None, None));
        assert_eq!(both("[::1]"), (None, None));
    }

    #[test]
    fn sites_are_schemes_and_registrable_domains() {
        let origin = |input: &str| Url::parse(input).unwrap().origin();
        let same = |a: &str, b: &str| origin(a).same_site(&origin(b));
        assert!(same("http://a.example.com:8080/", "http://b.example.com/"));
        assert!(same("http://127.0.0.1:1/", "http://127.0.0.1:2/"));
        assert!(same("http://example.co.uk/", "http://www.example.co.uk/"));
        assert!(!same("http://example.com/", "https://example.com/"));
        assert!(!same("http://a.localhost/", "http://b.localhost/"));
        assert!(!same("http://127.0.0.1/", "http://localhost/"));
        assert!(!same("http://a.github.io/", "http://b.github.io/"));
        assert!(!same("http://a.co.uk/", "http://b.co.uk/"));
        assert_eq!(
            origin("https://www.example.com/").site(),
            Site::Schemeful {
                scheme: "https".to_owned(),
                host: "example.com".to_owned()
            }
        );
        // A public suffix as a host is its own site.
        assert!(!same("http://co.uk/", "http://a.co.uk/"));
        let opaque = Origin::new_opaque();
        assert!(opaque.same_site(&opaque.clone()));
        assert!(!opaque.same_site(&Origin::new_opaque()));
    }

    #[test]
    fn a_registrable_suffix_is_the_host_or_a_domain_it_ends_in() {
        let host = |input: &str| {
            Url::parse(&format!("http://{input}/"))
                .unwrap()
                .host()
                .cloned()
                .unwrap()
        };
        let www = host("www.example.com");
        for suffix in ["example.com", "www.example.com", "EXAMPLE.com"] {
            assert!(www.has_registrable_suffix(suffix), "{suffix}");
        }
        for suffix in [
            "com",
            "ample.com",
            "",
            "example.com.",
            "other.com",
            "x.www.example.com",
        ] {
            assert!(!www.has_registrable_suffix(suffix), "{suffix}");
        }
        assert!(!host("a.github.io").has_registrable_suffix("github.io"));
        // Under a wildcard rule, `*.kawasaki.jp`: a domain whose public
        // suffix the host's ends in is refused, though it is no public
        // suffix itself; the exception `!city.kawasaki.jp` is a domain.
        let wild = host("a.b.kawasaki.jp");
        assert!(!wild.has_registrable_suffix("kawasaki.jp"));
        assert!(!wild.has_registrable_suffix("b.kawasaki.jp"));
        assert!(host("www.city.kawasaki.jp").has_registrable_suffix("city.kawasaki.jp"));
        assert!(host("a.b.github.io").has_registrable_suffix("b.github.io"));
        let ip = host("127.0.0.1");
        assert!(ip.has_registrable_suffix("127.0.0.1"));
        assert!(!ip.has_registrable_suffix("0.0.1"));
        assert!(!ip.has_registrable_suffix("1"));
    }

    #[test]
    fn fragments_replaced() {
        let url = Url::parse("https://x/p#old").unwrap();
        assert_eq!(
            url.with_fragment(Some("new a")).as_str(),
            "https://x/p#new%20a"
        );
        assert_eq!(url.with_fragment(None).as_str(), "https://x/p");
        assert_eq!(url.with_fragment(Some("%20")).fragment(), Some("%20"));
    }
}
