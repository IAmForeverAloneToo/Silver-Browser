//! Fetching (docs/design/net.md, "One way to fetch"): a request's URL
//! fetched by its scheme, as Fetch's scheme fetch has it, with its
//! redirects followed. Every check a fetch starts with (the scheme, the
//! port, mixed content, a script's Content Security Policy and the
//! address space) is made again on each hop, so a redirect cannot reach
//! what the first URL could not. Since M27 (docs/design/fetch.md) a
//! request has Fetch's modes, credentials and redirect mode, and its
//! response a tainting: [`Response::exposed`] is all a script may see.

mod cors;
mod file;
mod ports;

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::cache::{Cache, CacheMode, Stored};
use crate::cookies::{self, Jar};
use crate::csp::Csp;
use crate::data_url;
use crate::error::{AbortHandle, NetworkError};
use crate::headers::{self, HeaderList};
use crate::mime::{self, MimeType};
use crate::transport::{self, Exchange, Transport};
use crate::{AddressSpace, Host, Origin, Partition, Url};

pub use cors::{DEFAULT_PREFLIGHT_AGE, MAX_PREFLIGHT_AGE, MAX_PREFLIGHT_ENTRIES};
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

/// A request's method (Fetch 2.2.1): a token, as it goes on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Method(Cow<'static, str>);

impl Method {
    pub const GET: Method = Method(Cow::Borrowed("GET"));
    pub const HEAD: Method = Method(Cow::Borrowed("HEAD"));
    pub const POST: Method = Method(Cow::Borrowed("POST"));
    pub const OPTIONS: Method = Method(Cow::Borrowed("OPTIONS"));

    /// `token` as a method (Fetch 2.2.1): `None` when it is not a token;
    /// `DELETE`, `GET`, `HEAD`, `OPTIONS`, `POST` and `PUT` in upper case
    /// whatever their case, the rest as they came (`patch` stays
    /// `patch`, as the reference browser sends it).
    pub fn parse(token: &str) -> Option<Method> {
        if !headers::is_header_name(token.as_bytes()) {
            return None;
        }
        Some(Method(
            ["DELETE", "GET", "HEAD", "OPTIONS", "POST", "PUT"]
                .into_iter()
                .find(|normal| token.eq_ignore_ascii_case(normal))
                .map_or_else(|| Cow::Owned(token.to_owned()), Cow::Borrowed),
        ))
    }

    /// The method as it goes on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether it is a CORS-safelisted method: `GET`, `HEAD` or `POST`.
    pub fn is_cors_safelisted(&self) -> bool {
        matches!(self.as_str(), "GET" | "HEAD" | "POST")
    }

    /// Whether it is a forbidden method: `CONNECT`, `TRACE` or `TRACK`.
    pub fn is_forbidden(&self) -> bool {
        headers::is_forbidden_method(self.0.as_bytes())
    }
}

/// What the response is for (Fetch's request destination).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// A document to show.
    Document,
    /// A frame's document (M33.2): an `iframe`'s navigation.
    Iframe,
    /// A stylesheet.
    Style,
    /// An image: a document's icon, for now (M15).
    Image,
    /// A classic script (M24).
    Script,
    /// A dedicated worker's script (M31).
    Worker,
    /// No destination: a script's `fetch` or `XMLHttpRequest` (M27).
    Empty,
}

impl Destination {
    /// The destination's name, as `Sec-Fetch-Dest` says it.
    pub fn name(self) -> &'static str {
        match self {
            Destination::Document => "document",
            Destination::Iframe => "iframe",
            Destination::Style => "style",
            Destination::Image => "image",
            Destination::Script => "script",
            Destination::Worker => "worker",
            Destination::Empty => "empty",
        }
    }

    /// The `Accept` value Fetch lists for the destination.
    fn accept(self) -> &'static [u8] {
        match self {
            Destination::Document | Destination::Iframe => {
                b"text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
            }
            Destination::Style => b"text/css,*/*;q=0.1",
            Destination::Image => b"image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5",
            Destination::Script | Destination::Worker | Destination::Empty => b"*/*",
        }
    }
}

/// Fetch's request mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A navigation: the load of a document.
    Navigate,
    /// Same origin or nothing: another origin is a network error.
    SameOrigin,
    /// A subresource with no CORS: another origin's response is opaque.
    NoCors,
    /// CORS: another origin's response only when it says so (M27.3).
    Cors,
}

impl Mode {
    /// The mode's name, as `Sec-Fetch-Mode` says it.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Navigate => "navigate",
            Mode::SameOrigin => "same-origin",
            Mode::NoCors => "no-cors",
            Mode::Cors => "cors",
        }
    }
}

/// Fetch's credentials mode: whether cookies go with the request and
/// are taken from its response (M27.4), and what a CORS response must
/// allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credentials {
    Omit,
    /// Only while the request is same origin.
    SameOrigin,
    Include,
}

/// Fetch's redirect mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redirect {
    Follow,
    /// A redirect is a network error.
    Error,
    /// A redirect is the answer: an opaque-redirect response.
    Manual,
}

/// The document a request comes from.
#[derive(Debug, Clone)]
pub struct Initiator {
    /// The document's URL.
    pub url: Url,
    /// The address space of the server the document came from;
    /// loopback for a `file:` document.
    pub address_space: AddressSpace,
    /// The document's origin (M27.2): the request's origin.
    pub origin: Origin,
}

/// A response's type (Fetch 2.2.6), from its request's tainting: what
/// [`Response::exposed`] lets a script see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseType {
    /// Same origin, or no document asked: everything but its cookies.
    Basic,
    /// Another origin's, allowed by CORS: its status, its body and the
    /// headers it exposes.
    Cors,
    /// Another origin's, with no CORS: nothing at all.
    Opaque,
    /// A redirect a request in manual redirect mode stopped at: nothing
    /// but that it happened.
    OpaqueRedirect,
}

/// A request to fetch.
#[derive(Debug, Clone)]
pub struct Request {
    pub url: Url,
    pub method: Method,
    /// The author's header list: a form's `Content-Type` (M26.9), what a
    /// script set (M27); `net` adds its own fields around them.
    pub headers: HeaderList,
    /// The body.
    pub body: Option<Vec<u8>>,
    pub destination: Destination,
    pub mode: Mode,
    pub credentials: Credentials,
    pub redirect: Redirect,
    /// The origin of the document that asked (Fetch's request origin),
    /// or `None` for a load the user asked for: what the tainting,
    /// `Origin`, CORS, `Sec-Fetch-Site` and cookies' same-site rules go
    /// by. A navigation a document starts has one but no initiator.
    pub origin: Option<Origin>,
    /// Whether the user's activation started it: `Sec-Fetch-User` on a
    /// navigation.
    pub user_activated: bool,
    /// Fetch's use-CORS-preflight flag: an XHR whose upload is listened
    /// to is preflighted even when it is safelisted.
    pub preflight: bool,
    /// The document that asked, for a subresource's checks (its scheme,
    /// mixed content, its address space), or `None` for a load the user
    /// asked for or a navigation, which may go anywhere.
    pub initiator: Option<Initiator>,
    /// Stops the fetch.
    pub abort: AbortHandle,
    /// The most the response's body may hold, when less than the
    /// client's limit: an image's bound (M23).
    pub body_limit: Option<usize>,
    /// The policies of the document that asked: Fetch's "should request
    /// be blocked by Content Security Policy" at each hop, by its
    /// destination's fetch directive (M24.6, M33.1), the violations
    /// recorded for that document.
    pub csp: Option<Csp>,
    /// The nonce of the element that asked, a script's or a stylesheet
    /// link's, empty for none; and whether the parser inserted that
    /// element (Fetch's cryptographic nonce and parser metadata, M33.1).
    pub nonce: String,
    pub parser_inserted: bool,
    /// How it uses the HTTP cache (M28.4).
    pub cache: CacheMode,
    /// Where its document is in its tab (M33.2): a frame's requests are
    /// kept apart by its top-level site; a top-level document's have
    /// none.
    pub partition: Option<Partition>,
}

impl Request {
    /// A `GET` of `url` for `destination`, as the user asks: in navigate
    /// mode for a document, CORS for no destination, same-origin for a
    /// worker's script and no-cors otherwise, with credentials
    /// (same-origin ones for a script's fetch and a worker's script),
    /// following redirects, with no origin or initiator, and an abort
    /// handle of its own.
    pub fn get(url: Url, destination: Destination) -> Request {
        Request {
            url,
            method: Method::GET,
            headers: HeaderList::new(),
            body: None,
            destination,
            mode: match destination {
                Destination::Document | Destination::Iframe => Mode::Navigate,
                Destination::Empty => Mode::Cors,
                Destination::Worker => Mode::SameOrigin,
                Destination::Style | Destination::Image | Destination::Script => Mode::NoCors,
            },
            credentials: match destination {
                Destination::Empty | Destination::Worker => Credentials::SameOrigin,
                _ => Credentials::Include,
            },
            redirect: Redirect::Follow,
            origin: None,
            user_activated: destination == Destination::Document,
            preflight: false,
            initiator: None,
            abort: AbortHandle::new(),
            body_limit: None,
            csp: None,
            nonce: String::new(),
            parser_inserted: false,
            cache: CacheMode::Default,
            partition: None,
        }
    }

    /// A `POST` of `body` to `url` for `destination`, of `content_type`
    /// (M26.9): a form's submission.
    pub fn post(url: Url, destination: Destination, body: Vec<u8>, content_type: &str) -> Request {
        let mut headers = HeaderList::new();
        headers.append(b"Content-Type", content_type.as_bytes());
        Request {
            method: Method::POST,
            headers,
            body: Some(body),
            ..Request::get(url, destination)
        }
    }

    /// The request, a navigation the document of `origin` started, by
    /// the user's activation or not.
    pub fn from_document(mut self, origin: Origin, activated: bool) -> Request {
        self.origin = Some(origin);
        self.user_activated = activated;
        self
    }

    /// The request, checked against `csp` at each hop.
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

    /// The request in CORS mode with `credentials`, as a CORS settings
    /// attribute asks (HTML 2.5.4, M27.3).
    pub fn with_cors(mut self, credentials: Credentials) -> Request {
        self.mode = Mode::Cors;
        self.credentials = credentials;
        self
    }

    /// The request, made by `initiator`'s document, of its origin.
    pub fn initiated_by(mut self, initiator: Initiator) -> Request {
        self.origin = Some(initiator.origin.clone());
        self.user_activated = false;
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
    /// Its type, from the request's tainting (M27).
    pub response_type: ResponseType,
    /// The header names a CORS response exposes beyond the safelisted
    /// ones (Fetch's CORS-exposed header-name list).
    pub cors_exposed: Vec<Vec<u8>>,
}

/// What a script may see of a response (Fetch 2.2.6's filtered
/// responses), from [`Response::exposed`].
#[derive(Debug, Clone)]
pub struct Exposed<'a> {
    pub response_type: ResponseType,
    /// 0 for an opaque or opaque-redirect response.
    pub status: u16,
    pub status_message: &'a [u8],
    pub headers: HeaderList,
    /// Empty for an opaque response.
    pub url_list: &'a [Url],
    /// `None` for an opaque or opaque-redirect response.
    pub body: Option<&'a [u8]>,
}

impl Response {
    /// Whether the fetch was redirected: its URL list holds more than one.
    pub fn redirected(&self) -> bool {
        self.url_list.len() > 1
    }

    /// The response as a script may see it (Fetch 2.2.6): a basic one
    /// without its `Set-Cookie` fields; a CORS one with its status, its
    /// body and its CORS-safelisted and exposed headers; an opaque one
    /// with nothing, no URL either; an opaque redirect with its URL
    /// alone. A realm is given nothing else (docs/design/fetch.md).
    pub fn exposed(&self) -> Exposed<'_> {
        let keep = |keep: &dyn Fn(&[u8]) -> bool| {
            let mut headers = HeaderList::new();
            for (name, value) in self.headers.iter().filter(|(name, _)| keep(name)) {
                headers.append(name, value);
            }
            headers
        };
        let (status, status_message, headers, url_list, body) = match self.response_type {
            ResponseType::Basic => (
                self.status,
                self.status_message.as_slice(),
                keep(&|name| !headers::is_forbidden_response_header_name(name)),
                self.url_list.as_slice(),
                Some(self.body.as_slice()),
            ),
            ResponseType::Cors => (
                self.status,
                self.status_message.as_slice(),
                keep(&|name| {
                    headers::is_cors_safelisted_response_header_name(name, &self.cors_exposed)
                }),
                self.url_list.as_slice(),
                Some(self.body.as_slice()),
            ),
            ResponseType::Opaque => (0, &[][..], HeaderList::new(), &[][..], None),
            ResponseType::OpaqueRedirect => (
                0,
                &[][..],
                HeaderList::new(),
                self.url_list.as_slice(),
                None,
            ),
        };
        Exposed {
            response_type: self.response_type,
            status,
            status_message,
            headers,
            url_list,
            body,
        }
    }

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
    /// What preflights allowed (M27.3).
    preflights: cors::PreflightCache,
    /// The cookies every fetch reads and writes (M27.4), which an engine
    /// shares with `document.cookie`.
    cookies: Arc<Mutex<Jar>>,
    /// The HTTP cache (M28.4), which an engine's clients share.
    cache: Arc<Cache>,
    /// The time, by which the cache reckons ages.
    clock: WallClock,
}

/// A clock of the time of day, the system's unless a test gives its own.
pub type WallClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

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
            preflights: cors::PreflightCache::default(),
            cookies: Arc::default(),
            cache: Arc::default(),
            clock: Arc::new(SystemTime::now),
        }
    }

    /// The client, its requests sending `user_agent`: [`USER_AGENT`] or
    /// [`COMPAT_USER_AGENT`].
    pub fn with_user_agent(mut self, user_agent: &'static str) -> Client {
        self.user_agent = user_agent;
        self
    }

    /// The client, its fetches keeping their cookies in `jar` (M27.4):
    /// an engine's, which outlives the clients it is given.
    pub fn with_cookies(mut self, jar: Arc<Mutex<Jar>>) -> Client {
        self.cookies = jar;
        self
    }

    /// The client's cookie jar.
    pub fn cookies(&self) -> &Arc<Mutex<Jar>> {
        &self.cookies
    }

    /// The client, its fetches keeping their responses in `cache`
    /// (M28.4): an engine's, which outlives the clients it is given.
    pub fn with_cache(mut self, cache: Arc<Cache>) -> Client {
        self.cache = cache;
        self
    }

    /// The client's HTTP cache.
    pub fn cache(&self) -> &Arc<Cache> {
        &self.cache
    }

    /// The client, its cache reckoning ages by `clock`.
    pub fn with_clock(mut self, clock: WallClock) -> Client {
        self.clock = clock;
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
        let mut hop = Hop {
            method: request.method.clone(),
            author: request.headers.clone(),
            body: request.body.clone(),
            url_list: vec![url.clone()],
            tainting: ResponseType::Basic,
            tainted_origin: false,
        };
        loop {
            if request.abort.is_aborted() {
                return Err(NetworkError::Aborted);
            }
            if let Some(upgraded) = upgrade_mixed_content(request, &url) {
                url = upgraded;
                *hop.url_list.last_mut().expect("never empty") = url.clone();
            }
            check(request, &url, hop.url_list.len() - 1)?;
            hop.tainting = tainting(request, &url, hop.tainting)?;
            if hop.tainting == ResponseType::Cors && self.wants_preflight(request, &url, &hop) {
                self.preflight(request, &url, &hop)?;
            }
            let mut response = self.scheme_fetch(request, &url, &hop, deliver.as_deref_mut())?;
            match hop.tainting {
                ResponseType::Cors => cors_check(request, &response.headers, hop.tainted_origin)?,
                ResponseType::Opaque => resource_policy_check(request, &response.headers, &url)?,
                _ => {}
            }
            let Some(location) = location(&response, &url)? else {
                response.cors_exposed = match hop.tainting {
                    ResponseType::Cors => cors_exposed(request, &response.headers),
                    _ => Vec::new(),
                };
                response.response_type = hop.tainting;
                response.url_list = hop.url_list;
                return Ok(response);
            };
            match request.redirect {
                Redirect::Follow => {}
                Redirect::Error => return Err(NetworkError::Redirected),
                Redirect::Manual => {
                    response.response_type = ResponseType::OpaqueRedirect;
                    response.url_list = hop.url_list;
                    return Ok(response);
                }
            }
            if hop.url_list.len() > MAX_REDIRECTS {
                return Err(NetworkError::RedirectLimit);
            }
            hop.redirect(request, &url, &location, response.status)?;
            url = location;
            hop.url_list.push(url.clone());
        }
    }

    /// One hop's fetch, by `url`'s scheme, the response handed to
    /// `deliver` as it is read when there is one.
    fn scheme_fetch<'d>(
        &self,
        request: &Request,
        url: &Url,
        hop: &Hop,
        deliver: Option<&mut (dyn FnMut(Delivery<'_>) + 'd)>,
    ) -> Result<Response, NetworkError> {
        let url_list = hop.url_list.as_slice();
        match url.scheme() {
            "about" => whole(about(url), deliver),
            "data" => whole(
                headless(hop, data(url, request.limit(self.body_limit))),
                deliver,
            ),
            "file" => whole(
                headless(hop, file::fetch(url, request.limit(self.body_limit))),
                deliver,
            ),
            "http" | "https" => {
                let headers = self.request_headers(request, url, hop);
                // The HTTP cache (M28.4): for a GET fetched whole, by the
                // site the request is for.
                let mode = cache_mode(request, hop);
                let cached = hop.method.as_str() == "GET" && deliver.is_none();
                let site = partition_key(request, hop).1;
                if cached && let Some(stale) = mode.reads() {
                    let now = (self.clock)();
                    if let Some(stored) = self.cache.lookup(&site, url, &headers, now, stale) {
                        return Ok(Response {
                            url_list: vec![url.clone()],
                            status: stored.status,
                            status_message: stored.status_message,
                            mime_type: mime::extract_mime_type(&stored.headers),
                            headers: stored.headers,
                            body: stored.body,
                            address_space: stored.address_space,
                            response_type: ResponseType::Basic,
                            cors_exposed: Vec::new(),
                        });
                    }
                    if mode == CacheMode::OnlyIfCached {
                        return Err(NetworkError::NotCached);
                    }
                }
                let request_time = (self.clock)();
                // A POST or a PUT without a body says its length is 0.
                let body = match (&hop.body, hop.method.as_str()) {
                    (Some(body), _) => Some(body.as_slice()),
                    (None, "POST" | "PUT") => Some(&[][..]),
                    (None, _) => None,
                };
                let partition = request.partition.as_ref().map(Partition::key);
                let exchange = Exchange {
                    partition: partition.as_deref(),
                    method: hop.method.as_str(),
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
                // Set-Cookie is taken as soon as the head comes, a
                // redirect's too: before a document's scripts run.
                let take_cookies =
                    |headers: &HeaderList| self.take_cookies(request, url, hop, headers);
                let received = match deliver {
                    None => {
                        let received = self.transport.exchange(&exchange)?;
                        take_cookies(&received.response.headers);
                        received
                    }
                    Some(deliver) => {
                        let mut stream = Streaming {
                            url,
                            url_list,
                            deliver,
                            take_cookies: &take_cookies,
                        };
                        self.transport.exchange_streaming(&exchange, &mut stream)?
                    }
                };
                let response = received.response;
                if cached && mode != CacheMode::NoStore {
                    let stored = Stored {
                        status: response.status,
                        status_message: response.reason.clone(),
                        headers: response.headers.clone(),
                        body: response.body.clone(),
                        address_space: Some(received.address_space),
                    };
                    let now = (self.clock)();
                    self.cache
                        .store(&site, url, &headers, stored, request_time, now);
                }
                // A change of what is there makes what is stored for it
                // stale (RFC 9111 4.4).
                if !matches!(hop.method.as_str(), "GET" | "HEAD" | "OPTIONS" | "TRACE")
                    && (200..400).contains(&response.status)
                {
                    self.cache.invalidate(&site, url);
                }
                Ok(Response {
                    url_list: vec![url.clone()],
                    status: response.status,
                    status_message: response.reason,
                    mime_type: mime::extract_mime_type(&response.headers),
                    headers: response.headers,
                    body: response.body,
                    address_space: Some(received.address_space),
                    response_type: ResponseType::Basic,
                    cors_exposed: Vec::new(),
                })
            }
            other => Err(NetworkError::BadScheme(other.to_owned())),
        }
    }

    /// Whether a hop of a CORS request to `url` waits for a preflight
    /// (Fetch 4.4): its method is not safelisted, or the request asks
    /// for one, and no cached preflight allowed the method; or one of
    /// its CORS-unsafe header names none allowed.
    fn wants_preflight(&self, request: &Request, url: &Url, hop: &Hop) -> bool {
        let origin = serialized_origin(request, hop.tainted_origin);
        let asking = cors::Asking {
            origin: &origin,
            url,
            credentials: request.credentials == Credentials::Include,
        };
        self.preflights.wants(
            &asking,
            hop.method.as_str().as_bytes(),
            hop.method.is_cors_safelisted(),
            request.preflight,
            &headers::cors_unsafe_request_header_names(&hop.author),
        )
    }

    /// The CORS-preflight fetch (Fetch 4.8) of a hop to `url`: an
    /// `OPTIONS` request naming the method and the CORS-unsafe header
    /// names, without credentials, whose answer must pass the CORS check
    /// with an ok status and allow the method (exactly, or by `*`
    /// without credentials; a safelisted one always) and every name (in
    /// any case, or by `*` without credentials, never `Authorization`).
    /// What it allowed is cached for its `Access-Control-Max-Age`.
    fn preflight(&self, request: &Request, url: &Url, hop: &Hop) -> Result<(), NetworkError> {
        let refused = |why: &str| NetworkError::Cors(format!("the preflight {why}"));
        let names = headers::cors_unsafe_request_header_names(&hop.author);
        let mut author = HeaderList::new();
        author.append(b"Accept", b"*/*");
        author.append(
            b"Access-Control-Request-Method",
            hop.method.as_str().as_bytes(),
        );
        if !names.is_empty() {
            author.append(b"Access-Control-Request-Headers", &names.join(&b","[..]));
        }
        // Its method, fields and body are the hop's; it has no
        // credentials.
        let preflight = Request {
            credentials: Credentials::Omit,
            ..request.clone()
        };
        let preflight_hop = Hop {
            method: Method::OPTIONS,
            author,
            body: None,
            url_list: hop.url_list.clone(),
            tainting: ResponseType::Cors,
            tainted_origin: hop.tainted_origin,
        };
        let response = self.scheme_fetch(&preflight, url, &preflight_hop, None)?;
        cors_check(request, &response.headers, hop.tainted_origin)?;
        if !response.ok() {
            return Err(refused(&format!("was answered {}", response.status)));
        }
        let list = |name: &[u8]| match headers::token_list(&response.headers, name) {
            Some(Err(())) => Err(refused("answered a list that is not one")),
            found => Ok(found.and_then(Result::ok)),
        };
        let methods = match list(b"Access-Control-Allow-Methods")? {
            None if request.preflight => vec![hop.method.as_str().as_bytes().to_vec()],
            found => found.unwrap_or_default(),
        };
        let allowed_names = list(b"Access-Control-Allow-Headers")?.unwrap_or_default();
        let include = request.credentials == Credentials::Include;
        let method = hop.method.as_str().as_bytes();
        let star = |list: &[Vec<u8>]| !include && list.iter().any(|item| item == b"*");
        if !hop.method.is_cors_safelisted()
            && !methods.iter().any(|allowed| allowed == method)
            && !star(&methods)
        {
            return Err(refused(&format!("does not allow {}", hop.method.as_str())));
        }
        for name in &names {
            let named = allowed_names
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(name));
            if !named && (name == b"authorization" || !star(&allowed_names)) {
                return Err(refused(&format!(
                    "does not allow {}",
                    String::from_utf8_lossy(name)
                )));
            }
        }
        let origin = serialized_origin(request, hop.tainted_origin);
        let asking = cors::Asking {
            origin: &origin,
            url,
            credentials: include,
        };
        let age = cors::max_age(response.headers.get(b"Access-Control-Max-Age").as_deref());
        self.preflights
            .store(&asking, age, &methods, &allowed_names);
        Ok(())
    }

    /// Stores the cookies of a hop's response from `url` (RFC 6265bis
    /// 5.7), when the request includes credentials.
    fn take_cookies(&self, request: &Request, url: &Url, hop: &Hop, headers: &HeaderList) {
        if !includes_credentials(request, hop) {
            return;
        }
        let mut jar = self.cookies.lock().expect("never poisoned");
        let now = SystemTime::now();
        for (_, line) in headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(b"Set-Cookie"))
        {
            jar.store(
                line,
                url,
                cookies::Api::Http,
                cookie_context(request, hop),
                now,
            );
        }
    }

    /// The fields a hop to `url` sends besides `Host`, which the message
    /// writer adds: `User-Agent`, `Accept` and `Accept-Language` unless
    /// the author set them, `Accept-Encoding`, the author's fields, then
    /// `Origin`, the `Sec-Fetch-*` fields and `Cookie`, when the request
    /// includes credentials. Not sent: `Referer` (M33.3).
    fn request_headers(&self, request: &Request, url: &Url, hop: &Hop) -> HeaderList {
        let mut headers = HeaderList::new();
        let author = &hop.author;
        if !author.contains(b"User-Agent") {
            headers.append(b"User-Agent", self.user_agent.as_bytes());
        }
        if !author.contains(b"Accept") {
            headers.append(b"Accept", request.destination.accept());
        }
        if !author.contains(b"Accept-Language") {
            headers.append(b"Accept-Language", ACCEPT_LANGUAGE);
        }
        headers.append(b"Accept-Encoding", ACCEPT_ENCODING);
        for (name, value) in author.iter() {
            headers.append(name, value);
        }
        if let Some(origin) = origin_field(request, url, hop) {
            headers.append(b"Origin", origin.as_bytes());
        }
        // What a cache mode asks of the caches on the way (Fetch 4.6).
        match cache_mode(request, hop) {
            CacheMode::NoStore | CacheMode::Reload => {
                if !author.contains(b"Pragma") {
                    headers.append(b"Pragma", b"no-cache");
                }
                if !author.contains(b"Cache-Control") {
                    headers.append(b"Cache-Control", b"no-cache");
                }
            }
            CacheMode::NoCache if !author.contains(b"Cache-Control") => {
                headers.append(b"Cache-Control", b"max-age=0");
            }
            _ => {}
        }
        // Fetch Metadata: to potentially trustworthy URLs alone.
        if url.is_potentially_trustworthy() {
            headers.append(b"Sec-Fetch-Dest", request.destination.name().as_bytes());
            headers.append(b"Sec-Fetch-Mode", request.mode.name().as_bytes());
            headers.append(
                b"Sec-Fetch-Site",
                sec_fetch_site(request, &hop.url_list).as_bytes(),
            );
            if request.mode == Mode::Navigate && request.user_activated {
                headers.append(b"Sec-Fetch-User", b"?1");
            }
        }
        if includes_credentials(request, hop) {
            let mut jar = self.cookies.lock().expect("never poisoned");
            let context = cookie_context(request, hop);
            if let Some(cookie) = jar.header(url, cookies::Api::Http, context, SystemTime::now()) {
                headers.append(b"Cookie", &cookie);
            }
        }
        headers
    }
}

/// The cache mode a hop goes by (Fetch 4.6): the request's, but
/// `no-store` for a `default` one with a conditional field its author
/// set, whose answer is for the author alone.
fn cache_mode(request: &Request, hop: &Hop) -> CacheMode {
    let conditional = [
        &b"If-Modified-Since"[..],
        b"If-None-Match",
        b"If-Unmodified-Since",
        b"If-Match",
        b"If-Range",
    ];
    if request.cache == CacheMode::Default
        && conditional.iter().any(|name| hop.author.contains(name))
    {
        CacheMode::NoStore
    } else {
        request.cache
    }
}

/// Where a fetch stands from hop to hop.
struct Hop {
    method: Method,
    /// The author's fields, less those a redirect dropped.
    author: HeaderList,
    body: Option<Vec<u8>>,
    /// The URLs so far, this hop's last.
    url_list: Vec<Url>,
    tainting: ResponseType,
    /// Fetch's tainted origin flag: a redirect left another origin for a
    /// third, and the request's origin is `null` from then on.
    tainted_origin: bool,
}

impl Hop {
    /// Fetch's HTTP-redirect fetch, from `url` to `location` on a
    /// `status`: a CORS request may not go to a URL with credentials; a
    /// POST answered by 301 or 302, or anything but GET or HEAD
    /// answered by 303, goes on as a GET without its body or the body's
    /// fields; `Authorization` stays with its origin; and the origin is
    /// tainted when the redirect leaves an origin other than the
    /// request's for another.
    fn redirect(
        &mut self,
        request: &Request,
        url: &Url,
        location: &Url,
        status: u16,
    ) -> Result<(), NetworkError> {
        let to = location.origin();
        if location.includes_credentials()
            && (self.tainting == ResponseType::Cors
                || (request.mode == Mode::Cors && request.origin.as_ref() != Some(&to)))
        {
            return Err(NetworkError::Cors(
                "a redirect to a URL with credentials".to_owned(),
            ));
        }
        let to_get = match status {
            301 | 302 => self.method == Method::POST,
            303 => !matches!(self.method.as_str(), "GET" | "HEAD"),
            _ => false,
        };
        if to_get {
            self.method = Method::GET;
            self.body = None;
            for name in [
                b"Content-Encoding".as_slice(),
                b"Content-Language",
                b"Content-Location",
                b"Content-Type",
            ] {
                self.author.delete(name);
            }
        }
        let from = url.origin();
        if from != to {
            self.author.delete(b"Authorization");
            if request
                .origin
                .as_ref()
                .is_some_and(|origin| *origin != from)
            {
                self.tainted_origin = true;
            }
        }
        Ok(())
    }
}

/// A hop's response tainting (Fetch 4.1, main fetch's step 12), the
/// tainting so far `previous`: basic for a request no document made,
/// while every URL has been the document's origin, for a `data:` URL
/// and for a navigation; else a network error in same-origin mode,
/// opaque in no-cors mode (which must follow redirects), and CORS for
/// an HTTP URL.
fn tainting(
    request: &Request,
    url: &Url,
    previous: ResponseType,
) -> Result<ResponseType, NetworkError> {
    let Some(origin) = &request.origin else {
        return Ok(ResponseType::Basic);
    };
    if (previous == ResponseType::Basic && *origin == url.origin())
        || url.scheme() == "data"
        || request.mode == Mode::Navigate
    {
        return Ok(ResponseType::Basic);
    }
    match request.mode {
        Mode::SameOrigin => Err(NetworkError::Refused(
            "a same-origin request to another origin".to_owned(),
        )),
        Mode::NoCors if request.redirect != Redirect::Follow => Err(NetworkError::Refused(
            "a no-cors request that does not follow redirects".to_owned(),
        )),
        Mode::NoCors => Ok(ResponseType::Opaque),
        _ if !matches!(url.scheme(), "http" | "https") => Err(NetworkError::Refused(format!(
            "a CORS request for a {}: URL",
            url.scheme()
        ))),
        _ => Ok(ResponseType::Cors),
    }
}

/// Whether a hop includes credentials (Fetch 4.6's includeCredentials):
/// always for a request in include mode; in same-origin mode while it
/// is basic; never in omit mode.
fn includes_credentials(request: &Request, hop: &Hop) -> bool {
    match request.credentials {
        Credentials::Include => true,
        Credentials::SameOrigin => hop.tainting == ResponseType::Basic,
        Credentials::Omit => false,
    }
}

/// A hop as cookies' rules see it (RFC 6265bis 5.2): same-site when no
/// document asked (the user did) or every URL so far is same site with
/// the asking document; a navigation's is a top-level one, by a safe
/// method or not.
fn cookie_context(request: &Request, hop: &Hop) -> cookies::Context {
    // A frame's request is same-site only in a same-site context, to its
    // top-level site (M33.2, RFC 6265bis's "site for cookies").
    let in_frame = request.partition.as_ref().is_none_or(|partition| {
        !partition.cross_site
            && hop
                .url_list
                .iter()
                .all(|url| partition.top.same_site(&url.origin()))
    });
    cookies::Context {
        partition: Some(partition_key(request, hop).0),
        same_site: in_frame
            && request.origin.as_ref().is_none_or(|origin| {
                hop.url_list
                    .iter()
                    .all(|url| origin.same_site(&url.origin()))
            }),
        // A frame's navigation is no top-level one (M33.2).
        top_level: request.destination == Destination::Document,
        safe_method: matches!(hop.method.as_str(), "GET" | "HEAD" | "OPTIONS" | "TRACE"),
    }
}

/// What a request is kept apart by (M33.2): the key of its top-level
/// site (its partition's, else its origin's, else the first URL's, for
/// a load the user asked for), and the cache's key, that site and the
/// site of the document asking, or the top-level site itself.
fn partition_key(request: &Request, hop: &Hop) -> (String, crate::cache::Partition) {
    let first = || hop.url_list[0].origin();
    let asking = request.origin.clone().unwrap_or_else(first);
    let top = request
        .partition
        .as_ref()
        .map_or_else(|| asking.clone(), |partition| partition.top.clone());
    let cache = crate::cache::Partition {
        top: top.site(),
        frame: asking.site(),
    };
    (top.site().serialization(), cache)
}

/// The request's origin as `Origin` and CORS name it: `null` once
/// tainted, or for a request no document made.
fn serialized_origin(request: &Request, tainted: bool) -> String {
    match &request.origin {
        Some(origin) if !tainted => origin.serialization(),
        _ => "null".to_owned(),
    }
}

/// The `Origin` a hop sends (Fetch 3.2): on a CORS request, and on any
/// whose method is neither `GET` nor `HEAD`, where it is `null` from an
/// `https:` document to a URL that is not (the default referrer
/// policy's downgrade). None for a request no document made.
fn origin_field(request: &Request, url: &Url, hop: &Hop) -> Option<String> {
    let origin = request.origin.as_ref()?;
    let serialized = serialized_origin(request, hop.tainted_origin);
    if hop.tainting == ResponseType::Cors {
        return Some(serialized);
    }
    if matches!(hop.method.as_str(), "GET" | "HEAD") {
        return None;
    }
    let downgrade = matches!(origin, Origin::Tuple { scheme, .. } if scheme == "https")
        && url.scheme() != "https";
    Some(if downgrade {
        "null".to_owned()
    } else {
        serialized
    })
}

/// `Sec-Fetch-Site` (Fetch Metadata): `none` for a request the user
/// made; else `same-origin` while every URL is the request's origin,
/// `same-site` while every one is its site, `cross-site` otherwise.
fn sec_fetch_site(request: &Request, url_list: &[Url]) -> &'static str {
    let Some(origin) = &request.origin else {
        return "none";
    };
    let mut site = "same-origin";
    for url in url_list {
        let other = url.origin();
        if other == *origin {
            continue;
        }
        if !origin.same_site(&other) {
            return "cross-site";
        }
        site = "same-site";
    }
    site
}

/// The CORS check (Fetch 4.10) of a response to a CORS request: exactly
/// one `Access-Control-Allow-Origin`, `*` for a request without
/// credentials, or else the request's origin serialized; and with
/// credentials, `Access-Control-Allow-Credentials: true`.
fn cors_check(request: &Request, headers: &HeaderList, tainted: bool) -> Result<(), NetworkError> {
    let refused = |why: &str| Err(NetworkError::Cors(why.to_owned()));
    let Some(allowed) = headers.get(b"Access-Control-Allow-Origin") else {
        return refused("no Access-Control-Allow-Origin");
    };
    let include = request.credentials == Credentials::Include;
    if !include && allowed == b"*" {
        return Ok(());
    }
    if allowed != serialized_origin(request, tainted).as_bytes() {
        return refused("Access-Control-Allow-Origin names another origin");
    }
    if !include || headers.get(b"Access-Control-Allow-Credentials").as_deref() == Some(b"true") {
        return Ok(());
    }
    refused("credentials, without Access-Control-Allow-Credentials: true")
}

/// The header names a CORS response exposes (Fetch 4.1): its
/// `Access-Control-Expose-Headers`, where `*` without credentials is
/// every name the response has.
fn cors_exposed(request: &Request, headers: &HeaderList) -> Vec<Vec<u8>> {
    let Some(Ok(names)) = headers::token_list(headers, b"Access-Control-Expose-Headers") else {
        return Vec::new();
    };
    if request.credentials != Credentials::Include && names.iter().any(|name| name == b"*") {
        headers
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect()
    } else {
        names
    }
}

/// `Cross-Origin-Resource-Policy` (Fetch 3.7) on an opaque response:
/// `same-origin` refuses it to another origin; `same-site` to a
/// document that is not schemelessly same site with it, or that is not
/// `https:` where the response is.
fn resource_policy_check(
    request: &Request,
    headers: &HeaderList,
    url: &Url,
) -> Result<(), NetworkError> {
    let Some(origin) = &request.origin else {
        return Ok(());
    };
    let target = url.origin();
    let blocked = match headers.get(b"Cross-Origin-Resource-Policy").as_deref() {
        Some(b"same-origin") => *origin != target,
        Some(b"same-site") => {
            !origin.schemelessly_same_site(&target)
                || (!matches!(origin, Origin::Tuple { scheme, .. } if scheme == "https")
                    && url.scheme() == "https")
        }
        _ => false,
    };
    match blocked {
        true => Err(NetworkError::CrossOriginResourcePolicy),
        false => Ok(()),
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
    /// Stores the head's cookies, first of all.
    take_cookies: &'a dyn Fn(&HeaderList),
}

impl transport::ResponseStream for Streaming<'_, '_> {
    fn head(
        &mut self,
        status: u16,
        reason: &[u8],
        headers: &HeaderList,
        space: AddressSpace,
    ) -> bool {
        (self.take_cookies)(headers);
        let response = Response {
            url_list: self.url_list.to_vec(),
            status,
            status_message: reason.to_vec(),
            mime_type: mime::extract_mime_type(headers),
            headers: headers.clone(),
            body: Vec::new(),
            address_space: Some(space),
            response_type: ResponseType::Basic,
            cors_exposed: Vec::new(),
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
    if let Some(csp) = &request.csp {
        // A report names the first URL after a redirect to another
        // origin (M33.1).
        let reported = match redirects > 0 && url.origin() != request.url.origin() {
            true => &request.url,
            false => url,
        };
        let violations = csp.request_violations(
            request.destination,
            (url, reported),
            redirects,
            &request.nonce,
            request.parser_inserted,
        );
        csp.record(&violations);
        crate::csp::enforced(violations)
            .map_err(|violation| NetworkError::Csp(violation.to_string()))?;
    }
    Ok(())
}

/// Mixed Content section 4.1: an `http:` image of an `https:` page is
/// fetched over `https:` instead, unless its host is an IP address or
/// counts as trustworthy already. Other mixed content stays blocked,
/// a CORS-mode image's too (Mixed Content 3, M27.5).
fn upgrade_mixed_content(request: &Request, url: &Url) -> Option<Url> {
    let initiator = request.initiator.as_ref()?;
    if request.destination != Destination::Image
        || request.mode == Mode::Cors
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

/// Fetch's main fetch (4.1, step 22): a response to `HEAD` has no
/// body, whatever the scheme gave; HTTP's has none to read already.
fn headless(hop: &Hop, result: Result<Response, NetworkError>) -> Result<Response, NetworkError> {
    result.map(|mut response| {
        if hop.method == Method::HEAD {
            response.body.clear();
        }
        response
    })
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
        response_type: ResponseType::Basic,
        cors_exposed: Vec::new(),
    }
}

#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod tests;
