//! The network service (M34.1, D31, docs/design/processes.md): the
//! browser's client, with the cookie jar, the cache, the HSTS hosts and
//! the Local Network Access gate, serving every renderer. It takes no
//! renderer's word for what decides security: a request must come from a
//! frame the renderer was given, its origins must be of the renderer's
//! site or opaque, its partition and cookies' context are the browser's,
//! a referrer of another site is dropped, and `file:` is served only to
//! the `file:` renderer. What goes back never carries `Set-Cookie`, and
//! the body of an opaque response only when Opaque Response Blocking
//! lets it through.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::SystemTime;

use net::cookies::{Api, Context, Jar};
use net::{
    AbortHandle, Client, Delivery, NetworkError, Origin, Partition, Referrer, Request, Response,
    ResponseType, Url,
};

use super::handover::{HandoverNotice, Handovers};
use super::orb;
use crate::ipc::{Answer, Bytes, MAX_CHUNK, Sender, ToBrowser, ToRenderer};

/// The most fetches one renderer has in flight; one past it is answered
/// with an error.
pub const MAX_RENDERER_FETCHES: usize = 256;

/// What a renderer is locked to, once it shows a document (M34.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Lock {
    /// The documents of a site: a scheme and a registrable domain, or a
    /// host that has none.
    Site { scheme: String, host: String },
    /// `file:` documents.
    File,
    /// Documents of opaque origins alone.
    Opaque,
}

impl std::fmt::Display for Lock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lock::Site { scheme, host } => write!(f, "{scheme}://{host}"),
            Lock::File => write!(f, "file:"),
            Lock::Opaque => write!(f, "opaque"),
        }
    }
}

impl Lock {
    /// The lock of a renderer showing `url`'s document.
    pub fn of(url: &Url) -> Lock {
        if url.scheme() == "file" {
            return Lock::File;
        }
        match url.origin().site() {
            net::Site::Schemeful { scheme, host } => Lock::Site { scheme, host },
            net::Site::Opaque(_) => Lock::Opaque,
        }
    }

    /// Whether a renderer of this lock may speak for `origin`: one of its
    /// site, or an opaque one.
    pub fn admits(&self, origin: &Origin) -> bool {
        match (self, origin.site()) {
            (_, net::Site::Opaque(_)) => true,
            (Lock::Site { scheme, host }, net::Site::Schemeful { scheme: s, host: h }) => {
                *scheme == s && *host == h
            }
            _ => false,
        }
    }

    /// Whether `url` is of this lock's site.
    pub fn admits_url(&self, url: &Url) -> bool {
        match self {
            Lock::File => url.scheme() == "file",
            _ => Lock::of(url) == *self,
        }
    }
}

/// What the browser knows of a frame a renderer shows: the origin of the
/// tab's top-level document, and whether the frame or one of its
/// ancestors is of another site than it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameGrant {
    pub top: Origin,
    pub cross_site: bool,
}

/// What the browser lets a renderer do: its lock and the frames it
/// shows, by the numbers the browser gave them.
#[derive(Debug, Clone, Default)]
pub struct Grant {
    pub lock: Option<Lock>,
    pub frames: HashMap<u32, FrameGrant>,
}

/// Every renderer's grant, by its number.
#[derive(Default)]
pub struct Grants(Mutex<HashMap<u32, Grant>>);

impl Grants {
    pub fn new() -> Arc<Grants> {
        Arc::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<u32, Grant>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Locks renderer `renderer` to `lock`; false if it is locked to
    /// another already, which it stays.
    pub fn set_lock(&self, renderer: u32, lock: Lock) -> bool {
        let mut grants = self.lock();
        let grant = grants.entry(renderer).or_default();
        match &grant.lock {
            Some(held) => *held == lock,
            None => {
                grant.lock = Some(lock);
                true
            }
        }
    }

    /// Gives renderer `renderer` frame `frame`.
    pub fn grant_frame(&self, renderer: u32, frame: u32, grant: FrameGrant) {
        self.lock()
            .entry(renderer)
            .or_default()
            .frames
            .insert(frame, grant);
    }

    /// Takes frame `frame` from renderer `renderer`.
    pub fn revoke_frame(&self, renderer: u32, frame: u32) {
        if let Some(grant) = self.lock().get_mut(&renderer) {
            grant.frames.remove(&frame);
        }
    }

    /// Frame `frame` of renderer `renderer` commits `url`'s document: a
    /// top-level frame's origin is the top's from now on, and a renderer
    /// not yet locked is locked to its site. A lock never changes: a
    /// document of another site is handed over before it commits.
    pub(crate) fn commit(&self, renderer: u32, frame: u32, url: &Url) {
        let mut grants = self.lock();
        let Some(grant) = grants.get_mut(&renderer) else {
            return;
        };
        let Some(framed) = grant.frames.get_mut(&frame) else {
            return;
        };
        if !framed.cross_site {
            framed.top = url.origin();
            if grant.lock.is_none() {
                grant.lock = Some(Lock::of(url));
            }
        }
    }

    /// Renderer `renderer`'s lock, if it has one.
    pub fn lock_of(&self, renderer: u32) -> Option<Lock> {
        self.lock()
            .get(&renderer)
            .and_then(|grant| grant.lock.clone())
    }

    /// Forgets renderer `renderer`.
    pub fn forget(&self, renderer: u32) {
        self.lock().remove(&renderer);
    }

    /// Renderer `renderer`'s lock and its grant for `frame`.
    pub(crate) fn frame(&self, renderer: u32, frame: u32) -> Result<(Lock, FrameGrant), String> {
        let grants = self.lock();
        let grant = grants
            .get(&renderer)
            .ok_or_else(|| format!("renderer {renderer} was given nothing"))?;
        let lock = grant
            .lock
            .clone()
            .ok_or_else(|| format!("renderer {renderer} is not locked"))?;
        let frame_grant = grant
            .frames
            .get(&frame)
            .cloned()
            .ok_or_else(|| format!("frame {frame} is not renderer {renderer}'s"))?;
        Ok((lock, frame_grant))
    }
}

/// The browser's network, for every renderer.
pub struct NetworkService {
    client: Mutex<Arc<Client>>,
    jar: Arc<Mutex<Jar>>,
    grants: Arc<Grants>,
    /// The fetches in flight, by renderer and the renderer's number for
    /// them, with what aborts each.
    fetches: Arc<Mutex<HashMap<(u32, u64), AbortHandle>>>,
    /// Where a top-level document's response of another site than its
    /// renderer's waits, and who is told of it (M34.1); none hands nothing
    /// over.
    handovers: Mutex<Option<Arc<Handovers>>>,
    notify: Mutex<Option<Notify>>,
    /// The tab's word on frames (M34): whether one is shown by another
    /// renderer, whose documents then go there whatever their site; and
    /// who hears of each document committed.
    remote: Mutex<Option<Remote>>,
    committed: Mutex<Option<Committed>>,
}

/// Whether the frame the browser numbers so is shown by another renderer.
type Remote = Arc<dyn Fn(u32) -> bool + Send + Sync>;

/// Who hears that a frame of a renderer commits a document of a URL, and
/// whether the frame is a top-level one.
type Committed = Arc<dyn Fn(u32, u32, &Url, bool) + Send + Sync>;

/// What the browser's tab is told a response waits for it by.
type Notify = Arc<dyn Fn(HandoverNotice) + Send + Sync>;

impl NetworkService {
    /// The service of `client`, whose jar, cache, HSTS hosts and gate are
    /// the browser's, checking renderers against `grants`.
    pub fn new(client: Client, grants: Arc<Grants>) -> Arc<NetworkService> {
        Arc::new(NetworkService {
            jar: client.cookies().clone(),
            client: Mutex::new(Arc::new(client)),
            grants,
            fetches: Arc::default(),
            handovers: Mutex::new(None),
            notify: Mutex::new(None),
            remote: Mutex::new(None),
            committed: Mutex::new(None),
        })
    }

    /// The client's cookie jar.
    pub fn jar(&self) -> &Arc<Mutex<Jar>> {
        &self.jar
    }

    /// Fetches from now on with `client`, which keeps the same jar:
    /// another user agent (D12). Fetches in flight end with the old.
    pub fn replace_client(&self, client: Client) {
        *self.client.lock().unwrap_or_else(PoisonError::into_inner) = Arc::new(client);
    }

    /// Hands top-level documents of another site than their renderer's
    /// over through `handovers` from now on.
    pub fn hand_over_with(&self, handovers: Arc<Handovers>) {
        *self
            .handovers
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(handovers);
    }

    /// Tells `notify` of each response handed over.
    /// Asks `remote` whether a frame is shown by another renderer, and
    /// tells `committed` of each document committed (M34).
    pub(crate) fn watch_frames(&self, remote: Remote, committed: Committed) {
        *self.remote.lock().unwrap_or_else(PoisonError::into_inner) = Some(remote);
        *self
            .committed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(committed);
    }

    pub fn on_handover(&self, notify: Notify) {
        *self.notify.lock().unwrap_or_else(PoisonError::into_inner) = Some(notify);
    }

    /// How many fetches renderer `renderer` has in flight.
    pub fn in_flight(&self, renderer: u32) -> usize {
        self.fetches()
            .keys()
            .filter(|(of, _)| *of == renderer)
            .count()
    }

    fn fetches(&self) -> MutexGuard<'_, HashMap<(u32, u64), AbortHandle>> {
        self.fetches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Serves `message` of renderer `renderer`, answering through `reply`
    /// if it is the network's; gives it back otherwise. Err for what the
    /// protocol does not allow, for which the renderer is killed.
    pub fn serve(
        &self,
        renderer: u32,
        message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        match message {
            ToBrowser::Fetch {
                id,
                frame,
                streamed,
                request,
            } => {
                let (lock, grant) = self.grants.frame(renderer, frame)?;
                let request = checked(*request, &lock, &grant)?;
                self.start((renderer, frame), id, streamed, request, &lock, reply)?;
                Ok(None)
            }
            ToBrowser::FetchHandedOver {
                id,
                frame,
                handover,
                request,
            } => {
                self.grants.frame(renderer, frame)?;
                let handovers = self
                    .handovers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
                    .ok_or("a handover, where none are")?;
                handovers.take(handover, (renderer, frame), id, &request.url, reply)?;
                Ok(None)
            }
            ToBrowser::CancelFetch { id } => {
                if let Some(abort) = self.fetches().remove(&(renderer, id)) {
                    abort.abort();
                }
                Ok(None)
            }
            ToBrowser::Cookies {
                call,
                frame,
                url,
                context,
            } => {
                let context = self.cookie_context(renderer, frame, &url, context)?;
                let cookies = self
                    .jar()
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .header(&url, Api::Script, context, SystemTime::now());
                let _ = reply.send(&ToRenderer::Answer {
                    call,
                    answer: Answer::Cookies {
                        cookies: cookies.map(Bytes),
                    },
                });
                Ok(None)
            }
            ToBrowser::SetCookie {
                frame,
                url,
                value,
                context,
            } => {
                let context = self.cookie_context(renderer, frame, &url, context)?;
                self.jar()
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .store(&value.0, &url, Api::Script, context, SystemTime::now());
                Ok(None)
            }
            other => Ok(Some(other)),
        }
    }

    /// The context `document.cookie` of `frame` reads and writes `url`'s
    /// cookies in: the browser's, never laxer for the renderer's word.
    fn cookie_context(
        &self,
        renderer: u32,
        frame: u32,
        url: &Url,
        claimed: Context,
    ) -> Result<Context, String> {
        let (lock, grant) = self.grants.frame(renderer, frame)?;
        if !matches!(url.scheme(), "http" | "https") || !lock.admits_url(url) {
            return Err(format!("the cookies of {url}, not of its site"));
        }
        Ok(Context {
            same_site: claimed.same_site && !grant.cross_site,
            top_level: claimed.top_level && !grant.cross_site,
            safe_method: claimed.safe_method,
            partition: Some(grant.top.site().serialization()),
        })
    }

    /// Starts fetching `request`, renderer `renderer`'s fetch `id`, on a
    /// thread of its own, which answers through `reply`.
    fn start(
        &self,
        (renderer, frame): (u32, u32),
        id: u64,
        streamed: bool,
        mut request: Request,
        lock: &Lock,
        reply: &Sender<ToRenderer>,
    ) -> Result<(), String> {
        {
            let mut fetches = self.fetches();
            if fetches.contains_key(&(renderer, id)) {
                return Err(format!("fetch {id} again"));
            }
            let theirs = fetches.keys().filter(|(r, _)| *r == renderer).count();
            if theirs >= MAX_RENDERER_FETCHES {
                drop(fetches);
                send_done(
                    reply,
                    id,
                    Err(NetworkError::Refused("too many fetches".into())),
                    Vec::new(),
                );
                return Ok(());
            }
            fetches.insert((renderer, id), request.abort.clone());
        }
        if request.url.scheme() == "file" && *lock != Lock::File {
            self.fetches().remove(&(renderer, id));
            send_done(
                reply,
                id,
                Err(NetworkError::Refused("file".into())),
                Vec::new(),
            );
            return Ok(());
        }
        // The violations of the request's policies are collected here and
        // go back with its end.
        request.csp = request.csp.map(net::csp::Csp::with_sink);
        let client = self
            .client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let fetches = self.fetches.clone();
        let grants = self.grants.clone();
        // A top-level document's response decides, before the renderer
        // has it, what its frame's requests are from then on, or that it
        // goes to the renderer of its site (M34.1).
        let top_level = request.destination == net::Destination::Document;
        let navigation = matches!(
            request.destination,
            net::Destination::Document | net::Destination::Iframe
        ) && request.mode == net::Mode::Navigate;
        let shown_elsewhere = self
            .remote
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .is_some_and(|remote| !top_level && remote(frame));
        let committed = self
            .committed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let handovers = self
            .handovers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let notify = self
            .notify
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let lock = lock.clone();
        let answer = reply.clone();
        let spawned = thread::Builder::new()
            .name(format!("renderer-{renderer}-fetch"))
            .spawn(move || {
                let reply = answer;
                let abort = request.abort.clone();
                let mut moved: Option<u64> = None;
                let result = match streamed {
                    true => client
                        .fetch_streaming(&request, &mut |delivery| match delivery {
                            Delivery::Head(head) => {
                                if navigation
                                    && commits(head)
                                    && let Some(url) = head.url_list.last()
                                {
                                    let target = Lock::of(url);
                                    // A frame shown by another renderer
                                    // stays so for its parent (M34).
                                    if (shown_elsewhere || super::frames::elsewhere(&target, &lock))
                                        && let (Some(handovers), Some(notify)) =
                                            (&handovers, &notify)
                                    {
                                        let held = handovers.hold(url.clone(), renderer_head(head));
                                        moved = Some(held);
                                        // The renderer that asked is done with it.
                                        fetches
                                            .lock()
                                            .unwrap_or_else(PoisonError::into_inner)
                                            .remove(&(renderer, id));
                                        send_done(&reply, id, Err(NetworkError::Moved), Vec::new());
                                        notify(HandoverNotice {
                                            handover: held,
                                            renderer,
                                            frame,
                                            url: url.clone(),
                                            lock: target,
                                        });
                                        return;
                                    }
                                    if top_level {
                                        grants.commit(renderer, frame, url);
                                    }
                                    if let Some(committed) = &committed {
                                        committed(renderer, frame, url, top_level);
                                    }
                                }
                                let _ = reply.send(&ToRenderer::FetchHead {
                                    id,
                                    response: renderer_head(head),
                                });
                            }
                            Delivery::Body(bytes) => match (moved, &handovers) {
                                (Some(held), Some(handovers)) => {
                                    if !handovers.body(held, bytes) {
                                        abort.abort();
                                    }
                                }
                                _ => send_body(&reply, id, bytes),
                            },
                        })
                        .map(|response| renderer_head(&response)),
                    false => client.fetch(&request).map(|response| {
                        let response = filtered(response);
                        let _ = reply.send(&ToRenderer::FetchHead {
                            id,
                            response: renderer_head(&response),
                        });
                        send_body(&reply, id, &response.body);
                        renderer_head(&response)
                    }),
                };
                let violations = request
                    .csp
                    .as_ref()
                    .map(net::csp::Csp::take_violations)
                    .unwrap_or_default();
                fetches
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&(renderer, id));
                match (moved, &handovers) {
                    (Some(held), Some(handovers)) => handovers.end(held, result, violations),
                    _ => send_done(&reply, id, result, violations),
                }
            });
        if spawned.is_err() {
            self.fetches().remove(&(renderer, id));
            send_done(
                reply,
                id,
                Err(NetworkError::Io("no thread".into())),
                Vec::new(),
            );
        }
        Ok(())
    }
}

impl super::Serve for NetworkService {
    fn serve(
        &self,
        renderer: u32,
        message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        NetworkService::serve(self, renderer, message, reply)
    }
}

/// `request` as the browser makes it for a frame of `grant` in a renderer
/// locked to `lock`; Err for one the renderer may not ask for.
fn checked(mut request: Request, lock: &Lock, grant: &FrameGrant) -> Result<Request, String> {
    if let Some(origin) = &request.origin
        && !lock.admits(origin)
    {
        return Err(format!("a request of {}", origin.serialization()));
    }
    if let Some(initiator) = &request.initiator {
        if !lock.admits(&initiator.origin) {
            return Err(format!(
                "a request initiated by {}",
                initiator.origin.serialization()
            ));
        }
        if !initiator.origin.is_opaque() && initiator.url.origin() != initiator.origin {
            return Err("an initiator whose URL is not of its origin".into());
        }
    }
    // A frame asks in the name of the tab's top-level origin (M33.4).
    if let Some(access) = &request.local_network
        && !lock.admits(&access.origin)
        && access.origin != grant.top
    {
        return Err(format!(
            "local network access for {}",
            access.origin.serialization()
        ));
    }
    if let Referrer::Url(referrer) = &request.referrer
        && !referrer.origin().is_opaque()
        && !lock.admits(&referrer.origin())
    {
        request.referrer = Referrer::None;
    }
    request.partition = partition(request.partition.take(), grant)?;
    Ok(request)
}

/// The partition a frame of `grant` is in, the renderer having `claimed`
/// one: its top-level origin the browser's, cross-site if either says
/// so; Err for a claim of another top.
pub(crate) fn partition(
    claimed: Option<Partition>,
    grant: &FrameGrant,
) -> Result<Option<Partition>, String> {
    Ok(match claimed {
        Some(partition) if partition.top != grant.top => {
            return Err(format!(
                "a request in the partition of {}",
                partition.top.serialization()
            ));
        }
        Some(partition) => Some(Partition {
            top: partition.top,
            cross_site: partition.cross_site || grant.cross_site,
        }),
        None if grant.cross_site => Some(Partition {
            top: grant.top.clone(),
            cross_site: true,
        }),
        None => None,
    })
}

/// `response` as a renderer may have it: an opaque one that Opaque
/// Response Blocking stops loses its body and every field but its type.
fn filtered(mut response: Response) -> Response {
    if response.response_type == ResponseType::Opaque && !orb::allowed(&response) {
        let mut headers = net::HeaderList::new();
        if let Some(content_type) = response.headers.get(b"Content-Type") {
            headers.append(b"Content-Type", &content_type);
        }
        response.headers = headers;
        response.body.clear();
    }
    response
}

/// Whether a navigation's response `head` makes a document: not a
/// download, nor a 204 or 205 (the engine's `outcome`).
fn commits(head: &Response) -> bool {
    !matches!(
        crate::loader::outcome(head),
        crate::loader::Outcome::Download(_) | crate::loader::Outcome::Nothing
    )
}

/// `response`'s head as a renderer may have it: no `Set-Cookie`, which is
/// the jar's alone, and no body, which goes in pieces.
fn renderer_head(response: &Response) -> Response {
    let mut head = Response {
        url_list: response.url_list.clone(),
        status: response.status,
        status_message: response.status_message.clone(),
        headers: response.headers.clone(),
        body: Vec::new(),
        mime_type: response.mime_type.clone(),
        address_space: response.address_space,
        response_type: response.response_type,
        cors_exposed: response.cors_exposed.clone(),
    };
    head.headers.delete(b"Set-Cookie");
    head.headers.delete(b"Set-Cookie2");
    head
}

fn send_body(reply: &Sender<ToRenderer>, id: u64, bytes: &[u8]) {
    for piece in bytes.chunks(MAX_CHUNK) {
        let _ = reply.send(&ToRenderer::FetchBody {
            id,
            bytes: Bytes(piece.to_vec()),
        });
    }
}

fn send_done(
    reply: &Sender<ToRenderer>,
    id: u64,
    result: Result<Response, NetworkError>,
    violations: Vec<net::csp::Violation>,
) {
    let _ = reply.send(&ToRenderer::FetchDone {
        id,
        result,
        violations,
    });
}
