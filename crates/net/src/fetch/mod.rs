//! Fetching (docs/design/net.md, "One way to fetch"): a request's URL
//! fetched by its scheme, as Fetch's scheme fetch has it, with its
//! redirects followed. Every check a fetch starts with (the scheme, the
//! port, mixed content, a script's Content Security Policy and the
//! address space) is made again on each hop, so a redirect cannot reach
//! what the first URL could not.

mod file;
mod ports;

use crate::csp::Csp;
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

/// The platform the reference browser names in its user agent string on
/// the system this is built for.
#[cfg(target_os = "windows")]
macro_rules! compat_platform {
    () => {
        "Windows NT 10.0; Win64; x64"
    };
}
#[cfg(target_os = "macos")]
macro_rules! compat_platform {
    () => {
        "Macintosh; Intel Mac OS X 10_15_7"
    };
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
macro_rules! compat_platform {
    () => {
        "X11; Linux x86_64"
    };
}

/// The `User-Agent` requests send unless the user asks otherwise. It
/// begins as Fetch encourages and names nothing but the browser: D12,
/// decided at M25, is an honest, minimal string.
pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; SilverBrowser/",
    env!("CARGO_PKG_VERSION"),
    ")"
);

/// The `User-Agent` of D12's compatibility escape hatch, for a page that
/// refuses an engine it does not recognize: what the reference browser
/// on the same system sends, with this browser named after it. Never the
/// default.
pub const COMPAT_USER_AGENT: &str = concat!(
    "Mozilla/5.0 (",
    compat_platform!(),
    ") AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36 SilverBrowser/",
    env!("CARGO_PKG_VERSION")
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
    /// A classic script (M24).
    Script,
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
            Destination::Script => b"*/*",
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
    /// The body's `Content-Type`, sent with it (M26.9).
    pub content_type: Option<String>,
    pub destination: Destination,
    pub mode: Mode,
    /// The document that asked, or `None` for a load the user asked for,
    /// which may go anywhere.
    pub initiator: Option<Initiator>,
    /// Stops the fetch.
    pub abort: AbortHandle,
    /// The most the response's body may hold, when less than the
    /// client's limit: an image's bound (M23).
    pub body_limit: Option<usize>,
    /// The policies of the document that asked, for a script: Fetch's
    /// "should request be blocked by Content Security Policy" at each
    /// hop (M24.6).
    pub csp: Option<Csp>,
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
            content_type: None,
            destination,
            mode: match destination {
                Destination::Document => Mode::Navigate,
                Destination::Style | Destination::Image | Destination::Script => Mode::NoCors,
            },
            initiator: None,
            abort: AbortHandle::new(),
            body_limit: None,
            csp: None,
        }
    }

    /// A `POST` of `body` to `url` for `destination`, of `content_type`
    /// (M26.9): a form's submission.
    pub fn post(url: Url, destination: Destination, body: Vec<u8>, content_type: &str) -> Request {
        Request {
            method: Method::Post,
            body: Some(body),
            content_type: Some(content_type.to_owned()),
            ..Request::get(url, destination)
        }
    }

    /// The request, checked against `csp` at each hop if it is for a
    /// script.
    pub fn with_csp(mut self, csp: Csp) -> Request {
        self.csp = Some(csp);
        self
    }

    /// The request, its response's body held to `limit` bytes.
    pub fn with_body_limit(mut self, limit: usize) -> Request {
        self.body_limit = Some(limit);
        self
    }

    /// The body limit of this request under a client's `limit`.
    fn limit(&self, limit: usize) -> usize {
        self.body_limit.map_or(limit, |own| own.min(limit))
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
    /// The `User-Agent` its requests send.
    user_agent: &'static str,
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
            user_agent: USER_AGENT,
        }
    }

    /// The client, its requests sending `user_agent`: [`USER_AGENT`] or
    /// [`COMPAT_USER_AGENT`].
    pub fn with_user_agent(mut self, user_agent: &'static str) -> Client {
        self.user_agent = user_agent;
        self
    }

    /// Fetches `request`, blocking until the response is whole or the
    /// fetch fails.
    pub fn fetch(&self, request: &Request) -> Result<Response, NetworkError> {
        self.fetch_to(request, None)
    }

    /// [`Client::fetch`], handing the final response to `deliver` as it
    /// is read (M24.8): its head first, its body empty, then its body a
    /// piece at a time, its content codings removed. The response
    /// returned has an empty body. A redirect's response is not handed
    /// over; what it redirects to is.
    pub fn fetch_streaming(
        &self,
        request: &Request,
        deliver: &mut dyn FnMut(Delivery<'_>),
    ) -> Result<Response, NetworkError> {
        self.fetch_to(request, Some(deliver))
    }

    fn fetch_to<'d>(
        &self,
        request: &Request,
        mut deliver: Option<&mut (dyn FnMut(Delivery<'_>) + 'd)>,
    ) -> Result<Response, NetworkError> {
        let mut url = request.url.clone();
        let mut method = request.method;
        let mut body = request.body.clone();
        let mut url_list = vec![url.clone()];
        loop {
            if request.abort.is_aborted() {
                return Err(NetworkError::Aborted);
            }
            if let Some(upgraded) = upgrade_mixed_content(request, &url) {
                url = upgraded;
                *url_list.last_mut().expect("never empty") = url.clone();
            }
            check(request, &url, url_list.len() - 1)?;
            let mut response = self.scheme_fetch(
                request,
                &url,
                &url_list,
                method,
                body.as_deref(),
                deliver.as_deref_mut(),
            )?;
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

    /// One hop's fetch, by `url`'s scheme, the response handed to
    /// `deliver` as it is read when there is one (`url_list` the URLs so
    /// far, this one last).
    #[allow(clippy::too_many_arguments)]
    fn scheme_fetch<'d>(
        &self,
        request: &Request,
        url: &Url,
        url_list: &[Url],
        method: Method,
        body: Option<&[u8]>,
        deliver: Option<&mut (dyn FnMut(Delivery<'_>) + 'd)>,
    ) -> Result<Response, NetworkError> {
        match url.scheme() {
            "about" => whole(about(url), deliver),
            "data" => whole(data(url, request.limit(self.body_limit)), deliver),
            "file" => whole(file::fetch(url, request.limit(self.body_limit)), deliver),
            "http" | "https" => {
                // A body's type goes with it: not after a redirect
                // turned the request into a GET.
                let content_type = body.and(request.content_type.as_deref());
                let headers = request_headers(request.destination, self.user_agent, content_type);
                let exchange = Exchange {
                    method: method.as_str(),
                    url,
                    headers: &headers,
                    body,
                    initiator: request
                        .initiator
                        .as_ref()
                        .map(|initiator| initiator.address_space),
                    abort: &request.abort,
                    body_limit: request.body_limit,
                };
                let received = match deliver {
                    None => self.transport.exchange(&exchange)?,
                    Some(deliver) => {
                        let mut stream = Streaming {
                            url,
                            url_list,
                            deliver,
                        };
                        self.transport.exchange_streaming(&exchange, &mut stream)?
                    }
                };
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

/// A response read whole, handed to `deliver` if there is one as a
/// streamed one would be: its head, then its body.
fn whole(
    response: Result<Response, NetworkError>,
    deliver: Option<&mut (dyn FnMut(Delivery<'_>) + '_)>,
) -> Result<Response, NetworkError> {
    let mut response = response?;
    if let Some(deliver) = deliver {
        let body = std::mem::take(&mut response.body);
        deliver(Delivery::Head(&response));
        if !body.is_empty() {
            deliver(Delivery::Body(&body));
        }
    }
    Ok(response)
}

/// What a streamed fetch hands over as the response is read (M24.8).
pub enum Delivery<'a> {
    /// The final response, its body empty: what [`Client::fetch`] would
    /// return but for the body.
    Head(&'a Response),
    /// A piece of its body, its content codings removed.
    Body(&'a [u8]),
}

/// A hop's response streamed to a fetch's `deliver`, unless it is a
/// redirect, which is followed instead.
struct Streaming<'a, 'd> {
    url: &'a Url,
    url_list: &'a [Url],
    deliver: &'a mut (dyn FnMut(Delivery<'_>) + 'd),
}

impl transport::ResponseStream for Streaming<'_, '_> {
    fn head(
        &mut self,
        status: u16,
        reason: &[u8],
        headers: &HeaderList,
        space: AddressSpace,
    ) -> bool {
        let response = Response {
            url_list: self.url_list.to_vec(),
            status,
            status_message: reason.to_vec(),
            mime_type: mime::extract_mime_type(headers),
            headers: headers.clone(),
            body: Vec::new(),
            address_space: Some(space),
        };
        // A redirect, or a broken one, is the fetch's to follow or fail.
        if !matches!(location(&response, self.url), Ok(None)) {
            return false;
        }
        (self.deliver)(Delivery::Head(&response));
        true
    }

    fn body(&mut self, bytes: &[u8]) {
        (self.deliver)(Delivery::Body(bytes));
    }
}

/// The checks a fetch makes before each hop, the `redirects`th: the
/// scheme the request may use, the port, mixed content, and a script's
/// Content Security Policy. The address space is checked by the
/// transport, on the address it is about to connect to.
fn check(request: &Request, url: &Url, redirects: usize) -> Result<(), NetworkError> {
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
            && !url.is_potentially_trustworthy()
        {
            return Err(NetworkError::MixedContent);
        }
    }
    if request.destination == Destination::Script
        && let Some(csp) = &request.csp
    {
        csp.check_script_url(url, redirects)
            .map_err(|violation| NetworkError::Csp(violation.to_string()))?;
    }
    Ok(())
}

/// Mixed Content section 4.1: an `http:` image of an `https:` page is
/// fetched over `https:` instead, unless its host is an IP address or
/// counts as trustworthy already. Other mixed content stays blocked.
fn upgrade_mixed_content(request: &Request, url: &Url) -> Option<Url> {
    let initiator = request.initiator.as_ref()?;
    if request.destination != Destination::Image
        || initiator.url.scheme() != "https"
        || url.scheme() != "http"
        || url.is_potentially_trustworthy()
        || matches!(url.host(), Some(Host::Ipv4(_) | Host::Ipv6(_)))
    {
        return None;
    }
    // The default port goes with the scheme; another is kept.
    Url::parse(&format!("https{}", &url.as_str()["http".len()..])).ok()
}

/// The fields every HTTP request sends besides `Host`, which the
/// message writer adds, and a body's `Content-Type` (M26.9). Not sent:
/// `Referer` (M33.3), `Cookie` (M27.4), `Origin` (M27) and the
/// `Sec-Fetch-*` fields (M27).
fn request_headers(
    destination: Destination,
    user_agent: &str,
    content_type: Option<&str>,
) -> HeaderList {
    let mut headers = HeaderList::new();
    headers.append(b"User-Agent", user_agent.as_bytes());
    headers.append(b"Accept", destination.accept());
    headers.append(b"Accept-Language", ACCEPT_LANGUAGE);
    headers.append(b"Accept-Encoding", ACCEPT_ENCODING);
    if let Some(content_type) = content_type {
        headers.append(b"Content-Type", content_type.as_bytes());
    }
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
fn data(url: &Url, limit: usize) -> Result<Response, NetworkError> {
    let data_url = data_url::process(url).ok_or(NetworkError::DataUrl)?;
    if data_url.body.len() > limit {
        return Err(NetworkError::BodyTooLarge);
    }
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
