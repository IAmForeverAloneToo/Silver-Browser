//! Fetching (docs/design/net.md, "One way to fetch"): a request's URL
//! fetched by its scheme, as Fetch's scheme fetch has it, with its
//! redirects followed. Every check a fetch starts with (the scheme, the
//! port, mixed content and the address space) is made again on each
//! hop, so a redirect cannot reach what the first URL could not.

mod file;
mod ports;

use crate::data_url;
use crate::error::{AbortHandle, NetworkError};
use crate::headers::HeaderList;
use crate::mime::{self, MimeType};
use crate::transport::{self, Exchange, Transport};
use crate::{AddressSpace, Host, Url};

pub use ports::{BAD_PORTS, is_bad as is_bad_port};

/// How many redirects a fetch follows (Fetch's HTTP-redirect fetch): the
/// twenty-first is an error.
pub const MAX_REDIRECTS: usize = 20;

/// The `User-Agent` every request sends. It begins as Fetch encourages
/// and names nothing but the browser; D12 settles the policy at M25.
pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; SilverBrowser/",
    env!("CARGO_PKG_VERSION"),
    ")"
);

/// The `Accept-Language` every request sends, until settings choose
/// (M37.3).
const ACCEPT_LANGUAGE: &[u8] = b"en-US,en;q=0.9";

/// The content codings [`crate::transport`] removes.
const ACCEPT_ENCODING: &[u8] = b"gzip, deflate";

/// A request's method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Post,
}

impl Method {
    /// The method as it goes on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
        }
    }
}

/// What the response is for (Fetch's request destination).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// A document to show.
    Document,
    /// A stylesheet.
    Style,
    /// An image: a document's icon, for now (M15).
    Image,
}

impl Destination {
    /// The `Accept` value Fetch lists for the destination.
    fn accept(self) -> &'static [u8] {
        match self {
            Destination::Document => {
                b"text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
            }
            Destination::Style => b"text/css,*/*;q=0.1",
            Destination::Image => b"image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5",
        }
    }
}

/// Fetch's request mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A navigation: the load of a document.
    Navigate,
    /// A subresource with no CORS: a stylesheet, for now.
    NoCors,
}

/// The document a request comes from.
#[derive(Debug, Clone)]
pub struct Initiator {
    /// The document's URL.
    pub url: Url,
    /// The address space of the server the document came from;
    /// loopback for a `file:` document.
    pub address_space: AddressSpace,
}

/// A request to fetch.
#[derive(Debug, Clone)]
pub struct Request {
    pub url: Url,
    pub method: Method,
    /// The body of a `POST`.
    pub body: Option<Vec<u8>>,
    pub destination: Destination,
    pub mode: Mode,
    /// The document that asked, or `None` for a load the user asked for,
    /// which may go anywhere.
    pub initiator: Option<Initiator>,
    /// Stops the fetch.
    pub abort: AbortHandle,
}

impl Request {
    /// A `GET` of `url` for `destination`, in navigate mode for a
    /// document and no-cors otherwise, with no initiator and an abort
    /// handle of its own.
    pub fn get(url: Url, destination: Destination) -> Request {
        Request {
            url,
            method: Method::Get,
            body: None,
            destination,
            mode: match destination {
                Destination::Document => Mode::Navigate,
                Destination::Style | Destination::Image => Mode::NoCors,
            },
            initiator: None,
            abort: AbortHandle::new(),
        }
    }

    /// The request, made by `initiator`'s document.
    pub fn initiated_by(mut self, initiator: Initiator) -> Request {
        self.initiator = Some(initiator);
        self
    }
}

/// A response, whole: its body is read to the end before the fetch
/// returns.
#[derive(Debug, Clone)]
pub struct Response {
    /// Every URL the fetch went through: the request's first, then each
    /// redirect's.
    pub url_list: Vec<Url>,
    pub status: u16,
    /// The status message, as it came; nothing should depend on it.
    pub status_message: Vec<u8>,
    pub headers: HeaderList,
    /// The body, its content codings removed.
    pub body: Vec<u8>,
    /// The MIME type extracted from `Content-Type` (Fetch §3.5).
    pub mime_type: Option<MimeType>,
    /// The address space of the server that answered: loopback for a
    /// `file:` URL, and none for `about:` and `data:`, which no server
    /// answers.
    pub address_space: Option<AddressSpace>,
}

impl Response {
    /// The URL that answered: the last of the URL list.
    pub fn url(&self) -> &Url {
        self.url_list.last().expect("a response has a URL")
    }

    /// Whether the status is an ok status, 200 to 299.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Fetches. One client serves every fetch, and its connections are
/// shared among them.
pub struct Client {
    transport: Transport,
    body_limit: usize,
}

impl Default for Client {
    fn default() -> Client {
        Client::new(transport::Config::default())
    }
}

impl Client {
    pub fn new(config: transport::Config) -> Client {
        Client {
            body_limit: config.limits.body_bytes,
            transport: Transport::new(config),
        }
    }

    /// Fetches `request`, blocking until the response is whole or the
    /// fetch fails.
    pub fn fetch(&self, request: &Request) -> Result<Response, NetworkError> {
        let mut url = request.url.clone();
        let mut method = request.method;
        let mut body = request.body.clone();
        let mut url_list = vec![url.clone()];
        loop {
            if request.abort.is_aborted() {
                return Err(NetworkError::Aborted);
            }
            check(request, &url)?;
            let mut response = self.scheme_fetch(request, &url, method, body.as_deref())?;
            let Some(location) = location(&response, &url)? else {
                response.url_list = url_list;
                return Ok(response);
            };
            if url_list.len() > MAX_REDIRECTS {
                return Err(NetworkError::RedirectLimit);
            }
            // A POST answered by 301 or 302, or anything but GET or HEAD
            // answered by 303, goes on as a GET without its body.
            let to_get = match response.status {
                301 | 302 => method == Method::Post,
                303 => !matches!(method, Method::Get | Method::Head),
                _ => false,
            };
            if to_get {
                method = Method::Get;
                body = None;
            }
            url = location;
            url_list.push(url.clone());
        }
    }

    /// One hop's fetch, by `url`'s scheme.
    fn scheme_fetch(
        &self,
        request: &Request,
        url: &Url,
        method: Method,
        body: Option<&[u8]>,
    ) -> Result<Response, NetworkError> {
        match url.scheme() {
            "about" => about(url),
            "data" => data(url),
            "file" => file::fetch(url, self.body_limit),
            "http" | "https" => {
                let headers = request_headers(request.destination);
                let received = self.transport.exchange(&Exchange {
                    method: method.as_str(),
                    url,
                    headers: &headers,
                    body,
                    initiator: request
                        .initiator
                        .as_ref()
                        .map(|initiator| initiator.address_space),
                    abort: &request.abort,
                })?;
                let response = received.response;
                Ok(Response {
                    url_list: vec![url.clone()],
                    status: response.status,
                    status_message: response.reason,
                    mime_type: mime::extract_mime_type(&response.headers),
                    headers: response.headers,
                    body: response.body,
                    address_space: Some(received.address_space),
                })
            }
            other => Err(NetworkError::BadScheme(other.to_owned())),
        }
    }
}

/// The checks a fetch makes before each hop: the scheme the request may
/// use, the port, and mixed content. The address space is checked by
/// the transport, on the address it is about to connect to.
fn check(request: &Request, url: &Url) -> Result<(), NetworkError> {
    let scheme = url.scheme();
    let allowed = match &request.initiator {
        // A load the user asked for.
        None => matches!(scheme, "about" | "data" | "file" | "http" | "https"),
        // A page's subresource; file: only from a file: page.
        Some(initiator) => {
            matches!(scheme, "data" | "http" | "https")
                || (scheme == "file" && initiator.url.scheme() == "file")
        }
    };
    if !allowed {
        return Err(NetworkError::BadScheme(scheme.to_owned()));
    }
    if matches!(scheme, "http" | "https") {
        let port = url.port_or_default().expect("http: and https: have a port");
        if ports::is_bad(port) {
            return Err(NetworkError::BadPort(port));
        }
        if let Some(initiator) = &request.initiator
            && initiator.url.scheme() == "https"
            && scheme == "http"
            && !is_potentially_trustworthy_host(url)
        {
            return Err(NetworkError::MixedContent);
        }
    }
    Ok(())
}

/// Whether an `http:` URL's host is one Secure Contexts counts as
/// potentially trustworthy all the same: a loopback address, or
/// `localhost` and the names under it.
fn is_potentially_trustworthy_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(name)) => name == "localhost" || name.ends_with(".localhost"),
        _ => false,
    }
}

/// The fields every HTTP request sends besides `Host`, which the
/// message writer adds. Not sent: `Referer` (M33.3), `Cookie` (M27.4),
/// `Origin` (M27) and the `Sec-Fetch-*` fields (M27).
fn request_headers(destination: Destination) -> HeaderList {
    let mut headers = HeaderList::new();
    headers.append(b"User-Agent", USER_AGENT.as_bytes());
    headers.append(b"Accept", destination.accept());
    headers.append(b"Accept-Language", ACCEPT_LANGUAGE);
    headers.append(b"Accept-Encoding", ACCEPT_ENCODING);
    headers
}

/// Where a redirect goes (Fetch's location URL): `None` when the
/// response is not a redirect, or names no location and is the answer
/// itself. Several `Location` fields, one that does not parse, or one
/// that is not `http:` or `https:`, end the fetch.
fn location(response: &Response, url: &Url) -> Result<Option<Url>, NetworkError> {
    if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let mut values = response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(b"Location"))
        .map(|(_, value)| value);
    let (Some(value), None) = (values.next(), values.next()) else {
        return match response.headers.contains(b"Location") {
            true => Err(NetworkError::BadRedirect),
            false => Ok(None),
        };
    };
    // Browsers read the field as UTF-8, where Fetch is silent on how.
    let value = String::from_utf8_lossy(value);
    let mut location = url.join(&value).map_err(|_| NetworkError::BadRedirect)?;
    if location.fragment().is_none() && url.fragment().is_some() {
        location = location.with_fragment(url.fragment());
    }
    if !matches!(location.scheme(), "http" | "https") {
        return Err(NetworkError::BadRedirect);
    }
    Ok(Some(location))
}

/// `about:blank`, an empty HTML document; any other `about:` URL is an
/// error.
fn about(url: &Url) -> Result<Response, NetworkError> {
    if url.path_str() != "blank" {
        return Err(NetworkError::BadScheme(url.scheme().to_owned()));
    }
    let mime_type = MimeType::known("text/html;charset=utf-8");
    Ok(basic(url, mime_type, Vec::new()))
}

/// A `data:` URL's body and type.
fn data(url: &Url) -> Result<Response, NetworkError> {
    let data_url = data_url::process(url).ok_or(NetworkError::DataUrl)?;
    Ok(basic(url, data_url.mime_type, data_url.body))
}

/// A `200 OK` with `Content-Type` alone among its fields, which no
/// server sent.
fn basic(url: &Url, mime_type: MimeType, body: Vec<u8>) -> Response {
    let mut headers = HeaderList::new();
    headers.append(b"Content-Type", &mime_type.to_bytes());
    Response {
        url_list: vec![url.clone()],
        status: 200,
        status_message: b"OK".to_vec(),
        headers,
        body,
        mime_type: Some(mime_type),
        address_space: None,
    }
}

#[cfg(test)]
mod tests;
