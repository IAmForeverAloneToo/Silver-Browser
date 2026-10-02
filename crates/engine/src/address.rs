//! What the user types into the address field, made a URL
//! (docs/design/navigation.md, "The address", M15.4).
//!
//! The input loses the whitespace at its ends and any line break inside
//! it. Then the first rule that applies decides:
//!
//! 1. nothing is left: nothing happens;
//! 2. a host, a colon and a port, then the end or a path, a query or a
//!    fragment (`localhost:8080`, `example.com:8443/a`) is a host,
//!    though it would parse as a URL whose scheme is the host;
//! 3. an absolute path on this computer is that file's `file:` URL;
//! 4. an absolute URL is that URL, if its scheme is one the browser
//!    opens, and refused otherwise, unless its scheme has a dot in it:
//!    that is a host with something after a colon that is no port
//!    (`example.com:port`), and not an address;
//! 5. something that looks like a host, before an optional port, path,
//!    query and fragment, gets `http://` when it is on this computer or
//!    the local network (`localhost`, a `.localhost` name, an IP
//!    address, a name of one label given with a port) and `https://`
//!    when it is a domain of two labels or more;
//! 6. anything else is not an address.
//!
//! There is no falling back to `http:` when `https:` fails, and no
//! search until the choice of search engine (M37.3).

use std::path::Path;

use net::Url;

/// What an input to the address field is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// A URL to navigate to.
    Url(Url),
    /// Nothing but whitespace: nothing happens.
    Empty,
    /// A URL of a scheme the browser does not open (`mailto:`,
    /// `javascript:`), named here.
    Unsupported(String),
    /// Not an address.
    NotAnAddress,
}

impl Address {
    /// What the user is told when the input goes nowhere.
    pub fn notice(&self) -> Option<String> {
        match self {
            Address::Url(_) | Address::Empty => None,
            Address::Unsupported(scheme) => Some(format!(
                "{scheme}: addresses open in another program, which this version cannot do"
            )),
            Address::NotAnAddress => Some(
                "Not an address: type a web address, such as example.com; searching comes \
                 in a later version"
                    .to_owned(),
            ),
        }
    }
}

/// The schemes the browser opens from the address field.
const SCHEMES: [&str; 5] = ["about", "data", "file", "http", "https"];

/// Makes `input`, as typed or pasted into the address field, a URL by
/// the rules above.
pub fn parse(input: &str) -> Address {
    let input: String = input
        .trim()
        .chars()
        .filter(|&c| c != '\n' && c != '\r')
        .collect();
    if input.is_empty() {
        return Address::Empty;
    }
    let as_host = Host::split(&input);
    let with_port = as_host.as_ref().is_some_and(|host| host.port);
    if !with_port {
        if Path::new(&input).is_absolute() {
            return match Url::from_file_path(Path::new(&input)) {
                Some(url) => Address::Url(url),
                None => Address::NotAnAddress,
            };
        }
        if let Ok(url) = Url::parse(&input) {
            return if SCHEMES.contains(&url.scheme()) {
                Address::Url(url)
            } else if url.scheme().contains('.') {
                Address::NotAnAddress
            } else {
                Address::Unsupported(url.scheme().to_owned())
            };
        }
    }
    let Some(host) = as_host else {
        return Address::NotAnAddress;
    };
    let scheme = match host.kind {
        Kind::Local | Kind::Ip => "http",
        Kind::Label if host.port => "http",
        Kind::Domain => "https",
        Kind::Label => return Address::NotAnAddress,
    };
    match Url::parse(&format!("{scheme}://{input}")) {
        Ok(url) => Address::Url(url),
        Err(_) => Address::NotAnAddress,
    }
}

/// What kind of host the start of an input names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `localhost`, or a name ending in `.localhost`.
    Local,
    /// An IPv4 address in dotted-decimal form, or an IPv6 address in
    /// brackets.
    Ip,
    /// A domain of two labels or more.
    Domain,
    /// A name of one label.
    Label,
}

/// An input that starts with a host: its kind, and whether a port
/// follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Host {
    kind: Kind,
    port: bool,
}

impl Host {
    /// The host at the start of `input`, if it starts with one and the
    /// rest is an optional port, then the end or a path, a query or a
    /// fragment; no whitespace and no `@` anywhere.
    fn split(input: &str) -> Option<Host> {
        if input.contains(|c: char| c.is_whitespace() || c == '@') {
            return None;
        }
        let (host, rest) = if input.starts_with('[') {
            let end = input.find(']')? + 1;
            input.split_at(end)
        } else {
            let end = input.find([':', '/', '?', '#']).unwrap_or(input.len());
            input.split_at(end)
        };
        let after_port = match rest.strip_prefix(':') {
            Some(rest) => {
                let digits = rest.find(['/', '?', '#']).unwrap_or(rest.len());
                if digits == 0 || !rest[..digits].bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                &rest[digits..]
            }
            None => rest,
        };
        if !(after_port.is_empty() || after_port.starts_with(['/', '?', '#'])) {
            return None;
        }
        Some(Host {
            kind: kind(host)?,
            port: rest.starts_with(':'),
        })
    }
}

/// What kind of host `host` is, if it is one.
fn kind(host: &str) -> Option<Kind> {
    if let Some(inner) = host.strip_prefix('[') {
        let inner = inner.strip_suffix(']')?;
        return inner.parse::<std::net::Ipv6Addr>().ok().map(|_| Kind::Ip);
    }
    let name = host.strip_suffix('.').unwrap_or(host);
    let lower = name.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return Some(Kind::Local);
    }
    if is_dotted_decimal(name) {
        return Some(Kind::Ip);
    }
    let labels: Vec<&str> = name.split('.').collect();
    if !labels.iter().all(|label| is_label(label)) {
        return None;
    }
    match labels.as_slice() {
        [_] => Some(Kind::Label),
        [.., last] if !last.bytes().all(|b| b.is_ascii_digit()) => Some(Kind::Domain),
        _ => None,
    }
}

/// Four decimal numbers of at most three digits, each at most 255,
/// joined by dots.
fn is_dotted_decimal(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|part| {
            (1..=3).contains(&part.len())
                && part.bytes().all(|b| b.is_ascii_digit())
                && part.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

/// A label of a name: 1 to 63 letters, digits, hyphens or non-ASCII
/// characters (a name IDNA maps), not starting or ending with a hyphen.
fn is_label(label: &str) -> bool {
    (1..=63).contains(&label.chars().count())
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || !c.is_ascii())
        && !label.starts_with('-')
        && !label.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(input: &str) -> Option<String> {
        match parse(input) {
            Address::Url(url) => Some(url.as_str().to_owned()),
            _ => None,
        }
    }

    #[test]
    fn hosts_get_a_scheme_by_where_they_are() {
        for (input, expected) in [
            ("example.com", "https://example.com/"),
            ("EXAMPLE.com/a", "https://example.com/a"),
            ("example.com.", "https://example.com./"),
            (
                "www.example.co.uk/path?q=1#f",
                "https://www.example.co.uk/path?q=1#f",
            ),
            ("example.com:8443/a", "https://example.com:8443/a"),
            ("bücher.example", "https://xn--bcher-kva.example/"),
            ("localhost", "http://localhost/"),
            ("localhost:8080", "http://localhost:8080/"),
            ("app.localhost/x", "http://app.localhost/x"),
            ("127.0.0.1", "http://127.0.0.1/"),
            ("192.168.1.1:8080/admin", "http://192.168.1.1:8080/admin"),
            ("[::1]:3000", "http://[::1]:3000/"),
            ("[2001:db8::1]", "http://[2001:db8::1]/"),
            ("devbox:3000", "http://devbox:3000/"),
            ("notes.txt", "https://notes.txt/"),
            ("  example.com \n", "https://example.com/"),
            ("exam\nple.com", "https://example.com/"),
        ] {
            assert_eq!(url(input).as_deref(), Some(expected), "{input:?}");
        }
    }

    #[test]
    fn urls_are_taken_as_they_are_and_other_schemes_refused() {
        for (input, expected) in [
            ("https://example.com", "https://example.com/"),
            ("http://example.com/a", "http://example.com/a"),
            ("about:silver", "about:silver"),
            ("about:blank", "about:blank"),
            ("data:text/plain,hi", "data:text/plain,hi"),
            ("file:///tmp/a.html", "file:///tmp/a.html"),
            ("HTTPS://Example.COM/", "https://example.com/"),
        ] {
            assert_eq!(url(input).as_deref(), Some(expected), "{input:?}");
        }
        for (input, scheme) in [
            ("mailto:someone@example.com", "mailto"),
            ("javascript:alert(1)", "javascript"),
            ("ftp://example.com/", "ftp"),
            ("view-source:https://example.com/", "view-source"),
        ] {
            assert_eq!(
                parse(input),
                Address::Unsupported(scheme.to_owned()),
                "{input:?}"
            );
            assert!(parse(input).notice().unwrap().contains(scheme));
        }
    }

    #[test]
    fn an_absolute_path_is_a_file() {
        let path = if cfg!(windows) {
            "C:\\pages\\a page.html"
        } else {
            "/pages/a page.html"
        };
        let address = url(path).expect("a file URL");
        assert!(address.starts_with("file:///"), "{address}");
        assert!(address.ends_with("/pages/a%20page.html"), "{address}");
    }

    #[test]
    fn anything_else_is_not_an_address() {
        for input in [
            "intranet",
            "two words",
            "example.com/a b",
            "someone@example.com",
            "1.2.3",
            "999.1.1.1",
            "-bad.example",
            "bad-.example",
            "a..b",
            "http//example.com",
            "example.com:port",
            "example.com:80x",
            "[not-ipv6]",
            "?",
            "#top",
            "relative/path",
        ] {
            assert_eq!(parse(input), Address::NotAnAddress, "{input:?}");
        }
        assert!(
            Address::NotAnAddress
                .notice()
                .unwrap()
                .starts_with("Not an address")
        );
    }

    #[test]
    fn nothing_typed_is_nothing() {
        for input in ["", "   ", "\n\t"] {
            assert_eq!(parse(input), Address::Empty, "{input:?}");
            assert_eq!(parse(input).notice(), None);
        }
    }
}
