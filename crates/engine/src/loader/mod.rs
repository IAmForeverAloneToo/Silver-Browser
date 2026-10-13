//! The engine's loader (M14.6, docs/design/net.md, "The engine's
//! loader"): documents and their stylesheets fetched on threads of their
//! own, their results taken on the engine's thread.

#[cfg(test)]
mod body_tests;
#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod cookie_tests;
#[cfg(test)]
mod csp_tests;
#[cfg(test)]
mod dom_tests;
#[cfg(test)]
mod editing_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod fetch_tests;
#[cfg(test)]
mod focus_tests;
#[cfg(test)]
mod frame_script_tests;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod mixed_tests;
#[cfg(test)]
mod origin_tests;
#[cfg(test)]
mod partition_tests;
#[cfg(test)]
mod pointer_tests;
pub(crate) mod pool;
#[cfg(test)]
mod referrer_tests;
#[cfg(test)]
mod request_tests;
mod rules;
#[cfg(test)]
mod sandbox_tests;
#[cfg(test)]
mod script_tests;
#[cfg(test)]
mod security_tests;
mod sheets;
#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod submit_tests;
#[cfg(test)]
mod task_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod validity_tests;
#[cfg(test)]
mod worker_tests;
#[cfg(test)]
mod xhr_tests;

use std::collections::HashMap;
use std::sync::Mutex;

/// The state of a CORS settings attribute (HTML 2.5.4, M27.3): no CORS
/// when it is absent; credentials for `use-credentials`, in any case;
/// same-origin ones for `anonymous`, the empty value and any other.
pub(crate) fn cors_settings(value: Option<&str>) -> Option<net::Credentials> {
    Some(match value? {
        value if value.eq_ignore_ascii_case("use-credentials") => net::Credentials::Include,
        _ => net::Credentials::SameOrigin,
    })
}

/// `request` in CORS mode when a CORS settings attribute of `value` says
/// so, as it is otherwise.
pub(crate) fn with_cors_settings(request: net::Request, value: Option<&str>) -> net::Request {
    match cors_settings(value) {
        Some(credentials) => request.with_cors(credentials),
        None => request,
    }
}
use std::fmt;
use std::sync::Arc;

use dom::{Document, NodeId};
use encoding::Encoding;
use net::{AbortHandle, Client, Delivery, NetworkError, Request, Response};

pub(crate) use pool::{JobId, Lost, Pool, Waker};
pub(crate) use sheets::has_token;
pub(crate) use sheets::{Creator, Page, is_stylesheet, may_touch_sheets};

/// The most fetches in flight at once; further ones wait in the order
/// they were made (docs/design/net.md, "Threads, not an asynchronous
/// runtime").
pub const MAX_FETCH_THREADS: usize = 32;

/// The most threads a frame's engine fetches on (M33.2): a frame is one
/// of up to `MAX_FRAMES` a tab, each with its own.
pub const MAX_FRAME_FETCH_THREADS: usize = 6;

/// A page's number: what its fetches' results are matched to.
pub(crate) type PageId = u64;

/// A fetch's number.
pub(crate) type FetchId = u64;

/// Where the latest navigation stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    /// A document is on its way, or the document shown still has
    /// stylesheets on their way.
    Loading,
    /// The document shown is all there: each of its stylesheets arrived,
    /// or failed and is left out.
    Loaded,
    /// The navigation failed, and the page shown is the error page that
    /// says so, under the URL that failed (M15).
    Failed(LoadError),
}

/// Why a navigation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The fetch gave no response.
    Network(NetworkError),
    /// No thread could be started to fetch on, or the fetch's thread
    /// failed.
    Lost,
    /// A frame's response refused to be shown in it (M33.2): its
    /// `frame-ancestors` or `X-Frame-Options`.
    Framed,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Network(error) => error.fmt(f),
            LoadError::Lost => f.write_str("the fetch's thread failed"),
            LoadError::Framed => f.write_str("the page refused to be shown in a frame"),
        }
    }
}

impl std::error::Error for LoadError {}

/// What a fetch is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A page's document.
    Document,
    /// A sheet of `owner`, the element: its own at an empty `path`, and
    /// else the import at each index in turn.
    Sheet { owner: NodeId, path: Vec<usize> },
    /// A page's icon.
    Icon,
    /// An image a page shows, by its URL (M23).
    Image { url: String },
    /// A script's, from a `script` element's `src` (M24.3).
    Script,
    /// A script's `fetch()` (M27.1).
    Fetch,
    /// A violation's report (M33.1), whose answer nobody reads.
    Report,
}

/// A fetch in flight.
struct Flight {
    page: PageId,
    target: Target,
    abort: AbortHandle,
}

/// What a fetch's thread sends back: a streamed document's head and
/// the pieces of its body as they arrive (M24.8), and the result.
pub(crate) enum Fetched {
    Head(Response),
    Body(Vec<u8>),
    Done(Result<Response, NetworkError>),
}

/// What [`Network::take`] gives: a fetch's result, or a piece of a
/// document's response on its way.
pub(crate) enum Taken {
    Arrived(Arrived),
    /// The head of a document's response, its body to come.
    Head {
        page: PageId,
        response: Response,
    },
    /// A piece of a document's body.
    Body {
        page: PageId,
        bytes: Vec<u8>,
    },
}

/// A fetch's result, with what it was for.
pub(crate) struct Arrived {
    pub(crate) id: FetchId,
    pub(crate) page: PageId,
    pub(crate) target: Target,
    pub(crate) result: Result<Response, LoadError>,
}

/// The engine's fetches: the client they share, the threads they run
/// on, and those in flight, each of which can be aborted.
pub(crate) struct Network {
    pool: Pool<Fetched>,
    /// Made at the first fetch: a document given to the engine needs
    /// none.
    client: Option<Arc<Client>>,
    /// The cookies every client of the engine keeps (M27.4), and
    /// `document.cookie` reads and writes.
    cookies: Arc<Mutex<net::cookies::Jar>>,
    /// The HTTP cache every client of the engine keeps (M28.4).
    cache: Arc<net::Cache>,
    flights: HashMap<FetchId, Flight>,
    next: FetchId,
    /// Where each page's document is in its tab (M33.2): what a frame's
    /// requests but its navigations are made in.
    partitions: HashMap<PageId, net::Partition>,
    /// Each page's document's policies (M33.1): what its requests are
    /// checked against, unless they carry their own.
    csps: HashMap<PageId, net::csp::Csp>,
    /// Each page's document's referrer policy (M33.3), for its requests
    /// that name none.
    referrer_policies: HashMap<PageId, net::ReferrerPolicy>,
}

impl Network {
    pub(crate) fn new() -> Network {
        Network {
            pool: Pool::new(MAX_FETCH_THREADS),
            client: None,
            cookies: Arc::default(),
            cache: Arc::default(),
            flights: HashMap::new(),
            next: 0,
            partitions: HashMap::new(),
            csps: HashMap::new(),
            referrer_policies: HashMap::new(),
        }
    }

    /// Page `page`'s requests that name no referrer policy use `policy`
    /// from now on (M33.3); none once the page is left.
    pub(crate) fn set_referrer_policy(
        &mut self,
        page: PageId,
        policy: Option<net::ReferrerPolicy>,
    ) {
        match policy {
            Some(policy) => self.referrer_policies.insert(page, policy),
            None => self.referrer_policies.remove(&page),
        };
    }

    /// Page `page`'s requests are checked against `csp` from now on
    /// (M33.1); none once the page is left.
    pub(crate) fn set_csp(&mut self, page: PageId, csp: Option<net::csp::Csp>) {
        match csp {
            Some(csp) => self.csps.insert(page, csp),
            None => self.csps.remove(&page),
        };
    }

    /// Page `page`'s requests are made in `partition` from now on (M33.2);
    /// none for a top-level document's.
    pub(crate) fn set_partition(&mut self, page: PageId, partition: Option<net::Partition>) {
        match partition {
            Some(partition) => self.partitions.insert(page, partition),
            None => self.partitions.remove(&page),
        };
    }

    /// Fetches from now on with `client`; fetches in flight finish with
    /// the client they began with.
    pub(crate) fn set_client(&mut self, client: Client) {
        self.client = Some(Arc::new(self.shared(client)));
    }

    /// `client` keeping the engine's cookies and cache.
    fn shared(&self, client: Client) -> Client {
        client
            .with_cookies(self.cookies.clone())
            .with_cache(self.cache.clone())
    }

    /// The client fetches are made with, made now if none is yet.
    pub(crate) fn client(&mut self) -> Arc<Client> {
        if self.client.is_none() {
            self.client = Some(Arc::new(self.shared(Client::default())));
        }
        self.client.clone().expect("made")
    }

    /// The engine's cookie jar.
    pub(crate) fn cookies(&self) -> Arc<Mutex<net::cookies::Jar>> {
        self.cookies.clone()
    }

    /// The fetches of a frame's engine (M33.2): threads of its own, at
    /// most [`MAX_FRAME_FETCH_THREADS`], and this network's client, so
    /// its cookies and cache.
    pub(crate) fn for_frame(&mut self) -> Network {
        let mut network = Network::new();
        network.pool = Pool::new(MAX_FRAME_FETCH_THREADS);
        network.client = Some(self.client());
        network.cookies = self.cookies.clone();
        network.cache = self.cache.clone();
        network
    }

    /// Fetches from now on with `client`, a frame's parent's (M33.2).
    pub(crate) fn share_client(&mut self, client: Arc<Client>) {
        self.client = Some(client);
    }

    pub(crate) fn set_waker(&self, waker: Waker) {
        self.pool.set_waker(waker);
    }

    /// What wakes the engine's owner, if it was given one (M31: a
    /// worker's message).
    pub(crate) fn waker(&self) -> Option<Waker> {
        self.pool.waker()
    }

    /// Starts fetching `request` for `target` of `page`; a document's
    /// response comes in pieces as it arrives.
    pub(crate) fn fetch(&mut self, page: PageId, target: Target, mut request: Request) -> FetchId {
        if request.partition.is_none() && request.mode != net::Mode::Navigate {
            request.partition = self.partitions.get(&page).cloned();
        }
        if request.csp.is_none() && request.mode != net::Mode::Navigate {
            request.csp = self.csps.get(&page).cloned();
        }
        if request.referrer_policy.is_none() {
            request.referrer_policy = self.referrer_policies.get(&page).copied();
        }
        let streamed = target == Target::Document;
        let client = self.client();
        let id = self.next;
        self.next += 1;
        self.flights.insert(
            id,
            Flight {
                page,
                target,
                abort: request.abort.clone(),
            },
        );
        self.pool.run(id, move |report| {
            if !streamed {
                return Fetched::Done(client.fetch(&request));
            }
            Fetched::Done(
                client.fetch_streaming(&request, &mut |delivery| match delivery {
                    Delivery::Head(response) => report.send(Fetched::Head(response.clone())),
                    Delivery::Body(bytes) => report.send(Fetched::Body(bytes.to_vec())),
                }),
            )
        });
        id
    }

    /// Aborts and forgets the fetches of `page`: all of them, or those
    /// for `owner`'s sheets. Their results are not taken, even those that
    /// have finished and wait to be. Says whether there were any.
    pub(crate) fn cancel(&mut self, page: PageId, owner: Option<NodeId>) -> bool {
        let before = self.flights.len();
        self.flights.retain(|_, flight| {
            let cancelled = flight.page == page
                && owner.is_none_or(
                    |owner| matches!(&flight.target, Target::Sheet { owner: o, .. } if *o == owner),
                );
            if cancelled {
                flight.abort.abort();
            }
            !cancelled
        });
        self.flights.len() != before
    }

    /// Aborts and forgets fetch `id`, if it is in flight.
    pub(crate) fn abort(&mut self, id: FetchId) {
        if let Some(flight) = self.flights.remove(&id) {
            flight.abort.abort();
        }
    }

    /// Aborts and forgets every fetch.
    pub(crate) fn cancel_all(&mut self) {
        for flight in self.flights.values() {
            flight.abort.abort();
        }
        self.flights.clear();
    }

    /// A finished fetch still wanted, or a piece of a document's
    /// response on its way, if one has come.
    pub(crate) fn take(&mut self) -> Option<Taken> {
        while let Some((id, result)) = self.pool.try_take() {
            let page = |flights: &HashMap<FetchId, Flight>| flights.get(&id).map(|f| f.page);
            let taken = match result {
                Ok(Fetched::Head(response)) => {
                    page(&self.flights).map(|page| Taken::Head { page, response })
                }
                Ok(Fetched::Body(bytes)) => {
                    page(&self.flights).map(|page| Taken::Body { page, bytes })
                }
                Ok(Fetched::Done(result)) => self.arrived(id, Ok(result)).map(Taken::Arrived),
                Err(Lost) => self.arrived(id, Err(Lost)).map(Taken::Arrived),
            };
            if taken.is_some() {
                return taken;
            }
        }
        None
    }

    fn arrived(
        &mut self,
        id: FetchId,
        result: Result<Result<Response, NetworkError>, Lost>,
    ) -> Option<Arrived> {
        let flight = self.flights.remove(&id)?;
        Some(Arrived {
            id,
            page: flight.page,
            target: flight.target,
            result: match result {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(error)) => Err(LoadError::Network(error)),
                Err(Lost) => Err(LoadError::Lost),
            },
        })
    }

    /// How many fetches are in flight.
    pub(crate) fn in_flight(&self) -> usize {
        self.flights.len()
    }

    #[cfg(test)]
    pub(crate) fn pool(&self) -> &Pool<Fetched> {
        &self.pool
    }
}

/// A fetched document, parsed.
pub(crate) struct Parsed {
    pub(crate) document: Document,
    /// The encoding its bytes were decoded with.
    pub(crate) encoding: Encoding,
    /// The encoding it turned out to be in, when the engine cannot
    /// decode that one yet (a legacy CJK encoding): its bytes were
    /// decoded as UTF-8 instead.
    pub(crate) undecodable: Option<Encoding>,
    /// The language the response's `Content-Language` names, if one.
    pub(crate) language: Option<String>,
    /// Its type when it is no HTML document (M32).
    pub(crate) content_type: Option<String>,
}

/// What a navigation's response becomes (docs/design/navigation.md,
/// "What a response becomes"), in HTML's order.
pub(crate) enum Outcome {
    /// A document to show: HTML, or text made into a document.
    Document(Parsed),
    /// An error status with an empty body: the browser's page naming it.
    Status,
    /// An image, audio or video, of this essence, which this version
    /// cannot show yet: the browser's page saying so.
    Unshowable(String),
    /// A download, of this essence, which this version cannot save: the
    /// page shown stays.
    Download(String),
    /// A 204 or 205: nothing is shown, and the page shown stays.
    Nothing,
}

/// What `response`, to a navigation, becomes:
///
/// - a `Content-Disposition` of type `attachment` is a download;
/// - a 204 or a 205 shows nothing;
/// - an error status with an empty body has nothing to show;
/// - otherwise its computed MIME type (MIME Sniffing §7) decides, as
///   HTML's "load a document" does: HTML, and XHTML through the HTML
///   parser, is parsed; text, JavaScript, JSON and the other XML types
///   are a text document; images, audio and video cannot be shown yet;
///   anything else is handed to external software, a download.
pub(crate) fn outcome(response: &Response) -> Outcome {
    if is_attachment(response) {
        return Outcome::Download(computed_type(response).essence().to_owned());
    }
    if matches!(response.status, 204 | 205) {
        return Outcome::Nothing;
    }
    if !response.ok() && response.body.is_empty() {
        return Outcome::Status;
    }
    let computed = computed_type(response);
    let transport = transport_encoding(response);
    let essence = computed.essence();
    if computed.is_html() || essence == "application/xhtml+xml" {
        let decoded = html::parse_document_from_bytes(&response.body, transport);
        Outcome::Document(Parsed {
            document: decoded.document,
            encoding: decoded.encoding,
            undecodable: decoded.undecodable,
            language: content_language(response),
            content_type: None,
        })
    } else if essence == "image/svg+xml" {
        // An SVG document (M32): the XML parser's, or the page naming its
        // first error, as the reference browser shows one.
        let (document, content_type) = match html::xml::parse_xml(&response.body) {
            Ok(document) => (document, Some(essence.to_owned())),
            Err(error) => (
                crate::pages::xml_error(response.url(), &error.to_string()),
                None,
            ),
        };
        Outcome::Document(Parsed {
            document,
            encoding: encoding::UTF_8,
            undecodable: None,
            language: content_language(response),
            content_type,
        })
    } else if computed.is_xml()
        || computed.is_javascript()
        || computed.is_json()
        || matches!(essence, "text/css" | "text/plain" | "text/vtt")
    {
        // HTML feeds a text document's bytes to its parser, so they are
        // decoded as a document's are.
        let (encoding, _) = html::encoding_sniffing::sniff(&response.body, transport);
        let (text, decoded_with) = encoding::decode(&response.body, encoding);
        Outcome::Document(Parsed {
            document: crate::pages::text(&text),
            encoding: decoded_with,
            undecodable: (!encoding.can_decode()).then_some(encoding),
            language: content_language(response),
            content_type: None,
        })
    } else if computed.is_image() || computed.is_audio_or_video() {
        Outcome::Unshowable(essence.to_owned())
    } else {
        Outcome::Download(essence.to_owned())
    }
}

/// How many bytes of a body MIME sniffing reads, its resource header
/// (MIME Sniffing section 7.1): a navigation's response waits for them,
/// or for its end, before it is known to be HTML and parsed as it comes
/// (M24.8).
pub(crate) const RESOURCE_HEADER: usize = 1445;

/// The encoding the response's `Content-Type` names, if one.
pub(crate) fn transport_encoding(response: &Response) -> Option<Encoding> {
    response
        .mime_type
        .as_ref()
        .and_then(|mime_type| mime_type.parameter("charset"))
        .and_then(Encoding::for_label)
}

/// Whether `response`, its body what has arrived of it, its resource
/// header at least or the whole, is a document [`outcome`] would parse as
/// HTML: then it is parsed as it arrives, with the transport's encoding.
pub(crate) fn streams_as_html(response: &Response) -> Option<Option<Encoding>> {
    if is_attachment(response)
        || matches!(response.status, 204 | 205)
        || (!response.ok() && response.body.is_empty())
    {
        return None;
    }
    let computed = computed_type(response);
    (computed.is_html() || computed.essence() == "application/xhtml+xml")
        .then(|| transport_encoding(response))
}

/// A document parsed as its body arrives (M24.8): its load, and its
/// response's head, which a restart in another encoding makes its page
/// from again.
pub(crate) struct Parsing {
    pub(crate) load: html::Load,
    pub(crate) head: Response,
    /// The document's scripts until it is shown, when they go with it
    /// (M24).
    pub(crate) scripts: crate::scripts::Scripts,
    /// What a frame's local document takes from its creator (M33.2),
    /// again when its parse starts over, and its sandboxing flags.
    pub(crate) creator: Option<Creator>,
    pub(crate) sandbox: crate::sandbox::Sandbox,
    /// The referrer of the request that brought it (M33.3).
    pub(crate) referrer: String,
}

/// The language the response's `Content-Language` gives its document,
/// HTML's language "from a higher-level protocol": the one language tag
/// it names; none when it names none, or more than one, which HTML
/// takes to leave the language unknown.
pub(crate) fn content_language(response: &Response) -> Option<String> {
    let values = response.headers.get_decode_split(b"Content-Language")?;
    let mut tags = values.into_iter().filter(|value| !value.is_empty());
    let tag = tags.next()?;
    tags.next().is_none().then_some(tag)
}

/// The response's computed MIME type (MIME Sniffing §7).
pub(crate) fn computed_type(response: &Response) -> net::MimeType {
    net::mime::sniff::computed_type(
        response.mime_type.as_ref(),
        net::mime::sniff::check_for_apache_bug(&response.headers),
        net::mime::determine_nosniff(&response.headers),
        &response.body,
    )
}

/// Whether the response's `Content-Disposition` has the disposition
/// type `attachment` (RFC 6266 §4.2, matched without regard to case).
fn is_attachment(response: &Response) -> bool {
    response
        .headers
        .get(b"Content-Disposition")
        .is_some_and(|value| {
            let kind = value.split(|&byte| byte == b';').next().unwrap_or_default();
            kind.trim_ascii().eq_ignore_ascii_case(b"attachment")
        })
}

/// A load whose document is not shown yet.
pub(crate) enum Incoming {
    /// Its document is being fetched.
    Fetching(PageId),
    /// Its document's response has begun: its head, and its body so far
    /// (M24.8), which is not known to be HTML until its resource header
    /// is in; `sniffed` once it is known not to be, when it is taken
    /// whole.
    Receiving {
        id: PageId,
        head: Box<Response>,
        body: Vec<u8>,
        sniffed: bool,
    },
    /// Its document is being parsed as its body arrives, and is shown
    /// once it has a body and its blocking sheets have come, or it is
    /// parsed whole (M24.8).
    Parsing {
        page: Box<Page>,
        document: Box<Document>,
        parsing: Box<Parsing>,
    },
    /// Its document has arrived, and waits for its blocking sheets.
    Waiting {
        page: Box<Page>,
        document: Box<Document>,
    },
}

impl Incoming {
    pub(crate) fn id(&self) -> PageId {
        match self {
            Incoming::Fetching(id) | Incoming::Receiving { id, .. } => *id,
            Incoming::Waiting { page, .. } | Incoming::Parsing { page, .. } => page.id(),
        }
    }
}
