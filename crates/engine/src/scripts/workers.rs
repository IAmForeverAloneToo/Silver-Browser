//! Dedicated workers (M31, `docs/design/workers.md`): each a thread of
//! its own running a realm of its own, which shares nothing with the
//! realm that made it but the messages between them. Every crossing is
//! a [`WorkerMessage`] on a channel, followed by a wake: the seam M34's
//! processes turn into their IPC (M31.3), the messages already strings.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use js::{
    Answer, Call, ConsoleLevel, DomError, ErrorKind, Limits, MessageTarget, QuickJs, Realm,
    RealmKind, Script, ScriptEngine, ScriptError, ScriptingHost, Task,
};
use net::{Credentials, Destination, Initiator, Origin, Request, Url};

use crate::loader::Fetcher;

use super::canvas::{Canvases, names_context, needs_source};
use super::host::{
    PLATFORM, Timers, image_bitmap, location_parts, offscreen_element, offscreen_source,
};
use super::ports::{SharedRouter, lock, port_call};
use super::{ConsoleEntry, ConsoleSource, MAX_SCRIPT_BYTES, cut};
use crate::loader::Waker;
use crate::loader::pool::Pool;

/// A document's live workers, its workers' own included: past it, a
/// new one waits for one to end, as the standard allows.
pub(crate) const MAX_WORKERS: usize = 16;
/// How deep workers may make workers, from the document: a worker made
/// deeper fails to load.
pub(crate) const MAX_WORKER_DEPTH: u32 = 8;
/// The most a message's serialized form may hold, in bytes.
pub(crate) const MAX_MESSAGE_BYTES: usize = 256 << 20;
/// The most the messages waiting in one realm's inbox may hold, in
/// bytes.
pub(crate) const MAX_QUEUED_BYTES: usize = 512 << 20;
/// A worker thread's stack: the engine's own.
const WORKER_STACK: usize = 32 << 20;
/// The threads a worker's fetches run on at once.
const MAX_WORKER_FETCH_THREADS: usize = 6;
/// How often a worker's animation frames run: sixty times a second.
const FRAME_PERIOD: Duration = Duration::from_micros(16_667);

/// What crosses between a worker and the realm that made it (M31.3).
pub(crate) enum WorkerMessage {
    /// A message for the worker's global, from its owner.
    ToWorker(String),
    /// What worker `id` sends its owner.
    FromWorker(u32, FromWorker),
    /// Stops the worker's loop: its owner terminated it or went.
    Stop,
    /// One of the worker's own fetches finished (M31.2).
    Wake,
    /// A message for port `n`, which the realm holds (M31.5).
    Port(u32, String),
    /// A worker's frame for placeholder `n` of the document (M31.4): its
    /// offscreen's size and pixels.
    Frame(super::canvas::Commit),
    /// What the service workers' host tells the realm (M39), as JSON: a
    /// task of `js::Task::ServiceWorker`.
    ServiceWorker(String),
    /// An extendable event for a service worker's global (M39): its
    /// type, a message event's data, and what hears whether its promises
    /// all fulfilled.
    Extendable(String, Option<String>, crate::service_worker::Done<bool>),
    /// A `FetchEvent` for a service worker's global (M39 step 5): its
    /// request as JSON, and what hears how it was answered.
    Fetch(
        String,
        crate::service_worker::Done<crate::service_worker::Responded>,
    ),
}

impl WorkerMessage {
    /// The bytes of data it holds, which an inbox counts.
    fn bytes(&self) -> usize {
        match self {
            WorkerMessage::ToWorker(data)
            | WorkerMessage::FromWorker(_, FromWorker::Message(data))
            | WorkerMessage::Port(_, data)
            | WorkerMessage::ServiceWorker(data) => data.len(),
            _ => 0,
        }
    }
}

/// What a worker sends its owner.
pub(crate) enum FromWorker {
    /// A message for its `Worker` object.
    Message(String),
    /// An exception it did not handle, as the console words it, or
    /// `None` when its script could not be fetched or run.
    Error(Option<ScriptError>),
    /// A console message of its or of its workers, for its document's
    /// console.
    Console(Box<ConsoleEntry>),
    /// It ended: it closed itself, failed to load, or was stopped.
    Ended,
}

/// A realm's inbox as others send to it: the channel, the bytes of
/// data waiting in it, and what wakes its loop.
#[derive(Clone)]
pub(crate) struct Inbox {
    sender: Sender<WorkerMessage>,
    queued: Arc<AtomicUsize>,
    /// What wakes the receiver, shared by every clone: one given after a
    /// clone was taken wakes for it too.
    wake: Arc<std::sync::OnceLock<Waker>>,
    /// Gone once the realm's mailbox is.
    alive: std::sync::Weak<()>,
}

impl Inbox {
    /// Sends `message` and wakes the receiver; false, sending nothing,
    /// when its data would take the inbox past [`MAX_QUEUED_BYTES`]. A
    /// message to a receiver that is gone is dropped.
    pub(crate) fn send(&self, message: WorkerMessage) -> bool {
        let bytes = message.bytes();
        if self.queued.fetch_add(bytes, Ordering::SeqCst) + bytes > MAX_QUEUED_BYTES {
            self.queued.fetch_sub(bytes, Ordering::SeqCst);
            return false;
        }
        if self.sender.send(message).is_err() {
            // A realm that went hears nothing more: dropped, not refused.
            self.queued.fetch_sub(bytes, Ordering::SeqCst);
            return true;
        }
        if let Some(wake) = self.wake.get() {
            wake();
        }
        true
    }

    /// Whether the realm it sends to is still there.
    pub(crate) fn alive(&self) -> bool {
        self.alive.strong_count() > 0
    }
}

/// A realm's inbox as it takes from it.
pub(crate) struct Mailbox {
    receiver: Receiver<WorkerMessage>,
    queued: Arc<AtomicUsize>,
    _alive: Arc<()>,
}

/// What waiting on a mailbox gave.
enum Waited {
    Message(WorkerMessage),
    TimedOut,
    Gone,
}

impl Mailbox {
    /// A message, if one waits.
    pub(crate) fn try_take(&self) -> Option<WorkerMessage> {
        match self.receiver.try_recv() {
            Ok(message) => Some(self.taken(message)),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    /// The next message, waiting for it until `timeout` if given.
    fn take(&self, timeout: Option<Duration>) -> Waited {
        let message = match timeout {
            Some(timeout) => match self.receiver.recv_timeout(timeout) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => return Waited::TimedOut,
                Err(RecvTimeoutError::Disconnected) => return Waited::Gone,
            },
            None => match self.receiver.recv() {
                Ok(message) => message,
                Err(_) => return Waited::Gone,
            },
        };
        Waited::Message(self.taken(message))
    }

    fn taken(&self, message: WorkerMessage) -> WorkerMessage {
        self.queued.fetch_sub(message.bytes(), Ordering::SeqCst);
        message
    }
}

/// A new inbox: how others send to it, and how its realm takes.
pub(crate) fn inbox(wake: Option<Waker>) -> (Inbox, Mailbox) {
    let (sender, receiver) = channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let alive = Arc::new(());
    let woken = Arc::new(std::sync::OnceLock::new());
    if let Some(wake) = wake {
        let _ = woken.set(wake);
    }
    (
        Inbox {
            sender,
            queued: queued.clone(),
            wake: woken,
            alive: Arc::downgrade(&alive),
        },
        Mailbox {
            receiver,
            queued,
            _alive: alive,
        },
    )
}

/// What a worker starts from (M31.2): its script's URL, already the
/// creator's origin or `data:`, its options, and what it shares with
/// its creator.
pub(crate) struct Start {
    pub(crate) url: Url,
    pub(crate) name: String,
    /// `type: "module"`, which fails to load here (module scripts are
    /// G41).
    pub(crate) module: bool,
    pub(crate) credentials: Credentials,
    /// The client its fetches share with the engine's: its cookies and
    /// its cache.
    pub(crate) client: Arc<dyn Fetcher>,
    /// Its creator, which its script's request is made by.
    pub(crate) initiator: Initiator,
    /// Whether its creator is a secure context, which it then is.
    pub(crate) secure: bool,
    /// What its canvases measure text with: the window's viewport and
    /// fonts, as its creator had them.
    pub(crate) viewport: (u32, u32),
    pub(crate) fonts: Option<text::FontCollection>,
    /// Whether the user asked for D12's compatibility user agent.
    pub(crate) compat: bool,
    /// Where its creator is in its tab (M33.2), which its requests are
    /// made in.
    pub(crate) partition: Option<net::Partition>,
    /// Its creator's policies (M33.1): its script's request passes their
    /// `worker-src`, and a worker of a local URL has them as its own.
    pub(crate) csp: net::csp::Csp,
    /// Its creator's referrer policy (M33.3), which its script's request
    /// is made with.
    pub(crate) referrer_policy: net::ReferrerPolicy,
    /// What lets its creator's requests ask their way into a more private
    /// address space (M33.4), which its requests do too.
    pub(crate) local_network: Option<net::LocalNetworkAccess>,
    /// The storage its creator reaches and its storage key (M39): the
    /// Cache API's caches it shares; none when its creator's storage is
    /// refused.
    pub(crate) storage: Option<(Arc<dyn crate::storage::Areas>, crate::storage::AreaKey)>,
}

/// A worker its owner knows of: how to reach it and stop it.
struct Known {
    to: Inbox,
    stop: Arc<AtomicBool>,
    /// Whether it counts among the document's live workers: started, or
    /// waiting to be.
    counted: bool,
}

/// What a worker is given to run.
struct Setup {
    id: u32,
    start: Start,
    owner: Inbox,
    mailbox: Mailbox,
    me: Inbox,
    stop: Arc<AtomicBool>,
    depth: u32,
    live: Arc<AtomicUsize>,
    router: SharedRouter,
    document: u32,
}

/// The workers a realm made (M31.1), by number, and those waiting for
/// a slot.
pub(crate) struct Workers {
    /// Where its workers send to: its realm's inbox.
    inbox: Inbox,
    next: u32,
    known: HashMap<u32, Known>,
    waiting: VecDeque<Setup>,
    /// The document's live workers, its workers' included.
    live: Arc<AtomicUsize>,
    /// How deep its realm is: 0 for a document's.
    depth: u32,
    /// The ports of the document and its workers.
    router: SharedRouter,
    /// The document's realm in the router, which its workers' frames go
    /// to.
    document: u32,
}

/// What a worker's message is for its owner's realm.
pub(crate) enum Received {
    /// A task for the realm: an event at the worker's object.
    Task(Task),
    /// A line for the document's console.
    Console(ConsoleEntry),
    Nothing,
}

impl Workers {
    /// The workers of a realm `depth` deep, sending to `inbox`, counted
    /// in `live`.
    pub(crate) fn new(
        inbox: Inbox,
        live: Arc<AtomicUsize>,
        depth: u32,
        router: SharedRouter,
        document: u32,
    ) -> Workers {
        Workers {
            inbox,
            next: 1,
            known: HashMap::new(),
            waiting: VecDeque::new(),
            live,
            depth,
            router,
            document,
        }
    }

    /// Starts a worker, or queues it when the document has
    /// [`MAX_WORKERS`] live; its number. A module worker, or one past
    /// [`MAX_WORKER_DEPTH`], fails to load.
    pub(crate) fn start(&mut self, start: Start) -> u32 {
        let id = self.next;
        self.next += 1;
        let (to, mailbox) = inbox(None);
        let stop = Arc::new(AtomicBool::new(false));
        if start.module || self.depth >= MAX_WORKER_DEPTH {
            self.known.insert(
                id,
                Known {
                    to,
                    stop,
                    counted: false,
                },
            );
            self.inbox
                .send(WorkerMessage::FromWorker(id, FromWorker::Error(None)));
            self.inbox
                .send(WorkerMessage::FromWorker(id, FromWorker::Ended));
            return id;
        }
        self.known.insert(
            id,
            Known {
                to: to.clone(),
                stop: stop.clone(),
                counted: true,
            },
        );
        self.waiting.push_back(Setup {
            id,
            start,
            owner: self.inbox.clone(),
            mailbox,
            me: to,
            stop,
            depth: self.depth + 1,
            live: self.live.clone(),
            router: self.router.clone(),
            document: self.document,
        });
        self.start_waiting();
        id
    }

    /// Starts the workers waiting, while the document has room.
    pub(crate) fn start_waiting(&mut self) {
        while !self.waiting.is_empty() && self.live.load(Ordering::SeqCst) < MAX_WORKERS {
            let Some(setup) = self.waiting.pop_front() else {
                return;
            };
            self.live.fetch_add(1, Ordering::SeqCst);
            let (id, owner) = (setup.id, setup.owner.clone());
            let spawned = thread::Builder::new()
                .name(format!("worker {id}"))
                .stack_size(WORKER_STACK)
                .spawn(move || run(setup));
            if spawned.is_err() {
                owner.send(WorkerMessage::FromWorker(id, FromWorker::Error(None)));
                owner.send(WorkerMessage::FromWorker(id, FromWorker::Ended));
            }
        }
    }

    /// Posts `data` to worker `id`'s global; nothing for a worker that
    /// ended. A `DataCloneError` past the bounds.
    pub(crate) fn post(&mut self, id: u32, data: String) -> Result<(), DomError> {
        if data.len() > MAX_MESSAGE_BYTES {
            return Err(too_large());
        }
        if let Some(known) = self.known.get(&id)
            && !known.to.send(WorkerMessage::ToWorker(data))
            && !known.stop.load(Ordering::SeqCst)
        {
            return Err(too_large());
        }
        Ok(())
    }

    /// Stops worker `id` at once (`terminate()`): what it runs is
    /// interrupted, and nothing it sent that is not yet taken is.
    pub(crate) fn terminate(&mut self, id: u32) {
        let Some(known) = self.known.remove(&id) else {
            return;
        };
        known.stop.store(true, Ordering::SeqCst);
        known.to.send(WorkerMessage::Stop);
        if let Some(at) = self.waiting.iter().position(|setup| setup.id == id) {
            self.waiting.remove(at);
            if known.counted {
                // It never started: nothing to count down but its slot.
                return;
            }
        }
        if known.counted {
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
        self.start_waiting();
    }

    /// Stops every worker: the realm goes.
    pub(crate) fn terminate_all(&mut self) {
        let ids: Vec<u32> = self.known.keys().copied().collect();
        for id in ids {
            self.terminate(id);
        }
    }

    /// What worker `id`'s `event` is for the realm; nothing for a
    /// worker it terminated.
    pub(crate) fn receive(&mut self, id: u32, event: FromWorker) -> Received {
        if !self.known.contains_key(&id) {
            return Received::Nothing;
        }
        match event {
            FromWorker::Message(data) => Received::Task(Task::Message {
                target: MessageTarget::Worker(id),
                data,
                origin: String::new(),
            }),
            FromWorker::Error(error) => Received::Task(Task::WorkerError { worker: id, error }),
            FromWorker::Console(entry) => Received::Console(*entry),
            FromWorker::Ended => {
                if let Some(known) = self.known.remove(&id)
                    && known.counted
                {
                    self.live.fetch_sub(1, Ordering::SeqCst);
                }
                self.start_waiting();
                Received::Task(Task::WorkerEnded(id))
            }
        }
    }
}

impl Workers {
    /// Whether the realm has a worker that has not ended.
    pub(crate) fn any(&self) -> bool {
        !self.known.is_empty()
    }

    /// The document's live workers, for the tests.
    #[cfg(test)]
    pub(crate) fn live(&self) -> Arc<AtomicUsize> {
        self.live.clone()
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.terminate_all();
    }
}

/// The `DataCloneError` of a message past the bounds.
fn too_large() -> DomError {
    DomError::new("DataCloneError", "Data cannot be cloned, out of memory.")
}

/// A document's workers and its inbox, which they and its ports send
/// to, and the router of its ports and its workers'.
pub(crate) struct DocumentWorkers {
    pub(crate) workers: Workers,
    pub(crate) mailbox: Mailbox,
    inbox: Inbox,
    router: SharedRouter,
    realm: u32,
    /// Whether the document hears the service workers' notices (M39).
    pub(crate) service_listening: Arc<AtomicBool>,
    /// Its id as their client.
    pub(crate) service_id: String,
    /// Whether it was left for another document: no client any more.
    pub(crate) service_gone: Arc<AtomicBool>,
}

impl Default for DocumentWorkers {
    fn default() -> DocumentWorkers {
        DocumentWorkers::with_router(SharedRouter::default())
    }
}

impl DocumentWorkers {
    /// A document's, its realm one of `router`'s: the tab's (M33.2).
    pub(crate) fn with_router(router: SharedRouter) -> DocumentWorkers {
        let (inbox, mailbox) = inbox(None);
        let realm = lock(&router).add_realm(inbox.clone());
        DocumentWorkers {
            workers: Workers::new(inbox.clone(), Arc::default(), 0, router.clone(), realm),
            mailbox,
            inbox,
            router,
            realm,
            service_listening: Arc::default(),
            service_id: crate::service_worker::new_client_id(),
            service_gone: Arc::default(),
        }
    }
}

impl DocumentWorkers {
    /// Wakes the engine's owner with `wake` when a worker or a port
    /// sends.
    pub(crate) fn set_wake(&mut self, wake: Option<Waker>) {
        if let Some(wake) = wake {
            // Every clone of the inbox shares it: the workers', the
            // router's, the service workers' (M39).
            let _ = self.inbox.wake.set(wake);
        }
    }

    /// The document as the service workers' client (M39): of storage key
    /// `key`, its page `page`, its URL and base, nested in a frame or
    /// not; their tasks come to its inbox.
    pub(crate) fn service_client(
        &self,
        key: &crate::storage::AreaKey,
        page: u64,
        url: Url,
        base: Url,
        nested: bool,
    ) -> crate::service_worker::page::Client {
        let inbox = self.inbox();
        let watched = inbox.clone();
        let gone = self.service_gone.clone();
        crate::service_worker::page::Client {
            key: key.serialization(),
            partition: key.partition.clone(),
            origin: key.origin.clone(),
            base,
            url,
            page,
            id: self.service_id.clone(),
            nested,
            version: None,
            alive: Arc::new(move || watched.alive() && !gone.load(Ordering::SeqCst)),
            tell: Arc::new(move |text| {
                inbox.send(WorkerMessage::ServiceWorker(text));
                inbox.alive()
            }),
            listening: self.service_listening.clone(),
        }
    }

    /// Its id as the service workers' client is `id` (M39 step 6): the one
    /// its navigation reserved.
    pub(crate) fn set_service_id(&mut self, id: String) {
        self.service_id = id;
    }

    /// Whether its ports' router is `router`.
    pub(crate) fn uses(&self, router: &SharedRouter) -> bool {
        Arc::ptr_eq(&self.router, router)
    }

    /// The document's inbox, which the service workers' host tells (M39).
    pub(crate) fn inbox(&self) -> Inbox {
        self.inbox.clone()
    }

    /// A port's call of the document's (M31.5).
    pub(crate) fn port_call(&self, call: &js::Call) -> Option<Result<Answer, DomError>> {
        port_call(&self.router, self.realm, call)
    }
}

/// The URL of a worker's script, `url` as its creator parsed it
/// (`parsed`), if it may run for a creator of `origin` (HTML's "Worker
/// constructor" steps): the creator's origin, or `data:`; else the
/// reference browser's `SecurityError`, or a `SyntaxError` for a URL
/// that does not parse.
pub(crate) fn worker_url(url: &str, parsed: Option<Url>, origin: &Origin) -> Result<Url, DomError> {
    let Some(parsed) = parsed else {
        return Err(DomError::new(
            "SyntaxError",
            format!("The URL '{url}' is invalid."),
        ));
    };
    if parsed.scheme() == "data" || parsed.origin() == *origin {
        return Ok(parsed);
    }
    Err(DomError::new(
        "SecurityError",
        format!(
            "Script at '{parsed}' cannot be accessed from origin '{}'.",
            origin.serialization()
        ),
    ))
}

/// A worker's thread: its script fetched and run, then its event loop
/// until it closes, is stopped or its owner goes; its owner told it
/// ended.
fn run(setup: Setup) {
    let Setup {
        id,
        start,
        owner,
        mailbox,
        me,
        stop,
        depth,
        live,
        router,
        document,
    } = setup;
    let ended = |error: Option<Option<ScriptError>>| {
        if let Some(error) = error {
            owner.send(WorkerMessage::FromWorker(id, FromWorker::Error(error)));
        }
        owner.send(WorkerMessage::FromWorker(id, FromWorker::Ended));
    };
    let Some((source, url, csp)) = fetch_script(&start) else {
        return ended(Some(None));
    };
    if stop.load(Ordering::SeqCst) {
        return;
    }
    let kind = RealmKind::DedicatedWorker {
        name: start.name.clone(),
    };
    let limits = Limits {
        secure: start.secure,
        ..Limits::worker()
    };
    let Ok(mut realm) = QuickJs.realm_of::<WorkerHost>(kind, limits, Some(stop.clone())) else {
        return ended(Some(None));
    };
    let origin = if url.scheme() == "data" {
        Origin::new_opaque()
    } else {
        url.origin()
    };
    let realm_id = lock(&router).add_realm(me.clone());
    let pool = Pool::new(MAX_WORKER_FETCH_THREADS);
    let woken = me.clone();
    pool.set_waker(Arc::new(move || {
        woken.send(WorkerMessage::Wake);
    }));
    let mut host = WorkerHost {
        id,
        owner: Some(owner.clone()),
        pool: Some(pool),
        viewport: start.viewport,
        fonts: start.fonts.clone(),
        compat: start.compat,
        initiator: Some(Initiator {
            url: url.clone(),
            address_space: start.initiator.address_space,
            origin: origin.clone(),
        }),
        client: Some(start.client.clone()),
        storage: start.storage.clone(),
        origin,
        url: Some(url.clone()),
        secure: start.secure,
        clock: Instant::now(),
        epoch_ms: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0,
        router: Some(router.clone()),
        realm: realm_id,
        document,
        partition: start.partition.clone(),
        csp: Some(csp),
        local_network: start.local_network.clone(),
        workers: Some(Workers::new(me, live, depth, router.clone(), document)),
        ..WorkerHost::default()
    };
    let script = Script {
        source: &source,
        url: url.as_str(),
        line: 1,
        muted: false,
    };
    // Its policies may refuse or report code from strings (M33.1).
    if host
        .csp
        .as_ref()
        .is_some_and(|csp| !csp.eval_violations("").is_empty())
    {
        let _ = realm.enter(&Task::GuardCode, &mut host);
    }
    let result = realm.run(&script, &mut host).map(|_| false);
    host.finish(&mut *realm, result, &stop);
    work(&mut *realm, &mut host, &mailbox, &stop);
    lock(&router).remove_realm(realm_id);
    if !stop.load(Ordering::SeqCst) {
        ended(None);
    }
}

/// What a service worker's version is given to run (M39): its script,
/// where it fetches from, its storage, its mailbox, and its own part of
/// its host.
pub(crate) struct ServiceSetup {
    pub(crate) script: Url,
    pub(crate) source: Arc<[u8]>,
    pub(crate) address_space: net::AddressSpace,
    pub(crate) partition: Option<net::Partition>,
    pub(crate) client: Arc<dyn Fetcher>,
    pub(crate) storage: Option<(Arc<dyn crate::storage::Areas>, crate::storage::AreaKey)>,
    pub(crate) mailbox: Mailbox,
    pub(crate) me: Inbox,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) service: super::service_workers::Service,
    /// Told what its script did when it is evaluated for its job; none
    /// for a version started again for an event.
    pub(crate) evaluated:
        Option<crate::service_worker::Done<Option<crate::service_worker::Evaluated>>>,
}

/// A service worker's thread (M39): its script run in a realm of its
/// own, what it did told when it is evaluated for its job, then its
/// event loop until it is stopped or retires.
pub(crate) fn run_service(setup: ServiceSetup) {
    let ServiceSetup {
        script,
        source,
        address_space,
        partition,
        client,
        storage,
        mailbox,
        me,
        stop,
        service,
        mut evaluated,
    } = setup;
    let mut tell = |result: Option<crate::service_worker::Evaluated>| {
        if let Some(done) = evaluated.take() {
            done(result);
        }
    };
    let limits = Limits {
        secure: true,
        ..Limits::worker()
    };
    let Ok(mut realm) =
        QuickJs.realm_of::<WorkerHost>(RealmKind::ServiceWorker, limits, Some(stop.clone()))
    else {
        return tell(None);
    };
    let origin = script.origin();
    let router = SharedRouter::default();
    let realm_id = lock(&router).add_realm(me.clone());
    let pool = Pool::new(MAX_WORKER_FETCH_THREADS);
    let woken = me.clone();
    pool.set_waker(Arc::new(move || {
        woken.send(WorkerMessage::Wake);
    }));
    let mut host = WorkerHost {
        pool: Some(pool),
        initiator: Some(Initiator {
            url: script.clone(),
            address_space,
            origin: origin.clone(),
        }),
        client: Some(client),
        storage,
        origin: origin.clone(),
        url: Some(script.clone()),
        secure: true,
        clock: Instant::now(),
        epoch_ms: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0,
        router: Some(router.clone()),
        realm: realm_id,
        partition,
        // Its script's policies are not kept with it: it has none.
        csp: Some(net::csp::Csp::none(origin)),
        service: Some(service),
        ..WorkerHost::default()
    };
    let text = utf8_decode(&source);
    let run = Script {
        source: &text,
        url: script.as_str(),
        line: 1,
        muted: false,
    };
    let result = realm.run(&run, &mut host).map(|_| false);
    let threw = result.is_err();
    host.finish(&mut *realm, result, &stop);
    if threw || realm.dead().is_some() {
        tell(None);
    } else {
        // The events it listens for, its imports kept.
        let ask = Task::ServiceWorker(r#"{"type":"listened"}"#.to_owned());
        let result = realm.enter(&ask, &mut host);
        host.finish(&mut *realm, result, &stop);
        let service = host.service.as_mut().expect("a service worker's");
        let events = service.take_listened();
        tell(Some(crate::service_worker::Evaluated {
            events,
            imports: service.end_recording(),
        }));
        if !stop.load(Ordering::SeqCst) {
            work(&mut *realm, &mut host, &mailbox, &stop);
        }
    }
    lock(&router).remove_realm(realm_id);
    if let Some(service) = host.service.as_mut() {
        service.end();
    }
}

/// A worker's event loop (HTML section 10.2.4): its timers, and the
/// messages of its owner and of its own workers, in the order they are
/// due, each a task with its microtask checkpoint, until it closes or
/// is stopped.
fn work(
    realm: &mut dyn Realm<WorkerHost>,
    host: &mut WorkerHost,
    mailbox: &Mailbox,
    stop: &AtomicBool,
) {
    loop {
        if host.closing || stop.load(Ordering::SeqCst) {
            return;
        }
        let now = host.now();
        let frame_due = host
            .frame_wanted
            .then(|| host.last_frame.map_or(now, |last| last + FRAME_PERIOD));
        if frame_due.is_some_and(|due| due <= now) {
            host.frame_wanted = false;
            host.last_frame = Some(now);
            let at = now.as_secs_f64() * 1000.0;
            let result = realm.enter(&Task::AnimationFrames(at), host);
            host.finish(realm, result, stop);
            continue;
        }
        if host.timers.next_due().is_some_and(|due| due <= now) {
            let Some((id, level)) = host.timers.take_next() else {
                continue;
            };
            host.nesting = level;
            let result = realm.enter(&Task::Timer(id), host);
            host.nesting = 0;
            host.finish(realm, result, stop);
            continue;
        }
        // A service worker's own bounds (M39): an event past its limit
        // stops it, and so does its idleness.
        let bound = host
            .service
            .as_ref()
            .map(super::service_workers::Service::due);
        if bound.is_some_and(|due| due <= Instant::now()) {
            match super::service_workers::retire(host, mailbox) {
                Some(message) => {
                    host.handle(realm, message, stop);
                    continue;
                }
                None => return,
            }
        }
        let wait = [
            host.timers.next_due().map(|due| due.saturating_sub(now)),
            frame_due.map(|due| due.saturating_sub(now)),
            bound.map(|due| due.saturating_duration_since(Instant::now())),
        ]
        .into_iter()
        .flatten()
        .min();
        let message = match mailbox.take(wait) {
            Waited::Message(message) => message,
            Waited::TimedOut => continue,
            Waited::Gone => return,
        };
        if matches!(message, WorkerMessage::Stop) {
            return;
        }
        host.handle(realm, message, stop);
    }
}

impl WorkerHost {
    /// Runs what `message` asks of the worker's realm.
    fn handle(
        &mut self,
        realm: &mut dyn Realm<WorkerHost>,
        message: WorkerMessage,
        stop: &AtomicBool,
    ) {
        let host = self;
        let task = match message {
            WorkerMessage::Stop => return,
            // Frames are the document's alone.
            WorkerMessage::Frame(_) => return,
            WorkerMessage::Wake => {
                for task in host.take_fetched() {
                    if host.closing || stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let result = realm.enter(&task, host);
                    host.finish(realm, result, stop);
                }
                return;
            }
            WorkerMessage::ServiceWorker(text) => Task::ServiceWorker(text),
            WorkerMessage::Fetch(data, done) => match host.service.as_mut() {
                Some(service) => Task::ServiceWorker(service.fetch_event(&data, done)),
                None => return,
            },
            WorkerMessage::Extendable(kind, data, done) => match host.service.as_mut() {
                Some(service) => Task::ServiceWorker(service.event(&kind, data.as_deref(), done)),
                None => return,
            },
            WorkerMessage::ToWorker(data) => Task::Message {
                target: MessageTarget::Global,
                data,
                origin: String::new(),
            },
            WorkerMessage::Port(port, data) => Task::Message {
                target: MessageTarget::Port(port),
                data,
                origin: String::new(),
            },
            WorkerMessage::FromWorker(child, event) => {
                let received = match host.workers.as_mut() {
                    Some(workers) => workers.receive(child, event),
                    None => Received::Nothing,
                };
                match received {
                    Received::Task(task) => task,
                    Received::Console(entry) => {
                        host.tell(FromWorker::Console(Box::new(entry)));
                        return;
                    }
                    Received::Nothing => return,
                }
            }
        };
        let result = realm.enter(&task, host);
        host.finish(realm, result, stop);
    }
}

/// The worker's script (HTML's "fetch a classic worker script"): its
/// text, decoded as UTF-8, and the URL it came from; none when the
/// fetch fails, the status is not ok or the type is not a script's.
fn fetch_script(start: &Start) -> Option<(String, Url, net::csp::Csp)> {
    if start.module {
        return None;
    }
    let mut request = Request::get(start.url.clone(), Destination::Worker)
        .initiated_by(start.initiator.clone())
        .with_body_limit(MAX_SCRIPT_BYTES)
        .with_csp(start.csp.clone());
    request.partition.clone_from(&start.partition);
    request.referrer_policy = Some(start.referrer_policy);
    request.local_network.clone_from(&start.local_network);
    request.credentials = start.credentials;
    let response = start.client.fetch(&request).ok()?;
    if !response.ok()
        || net::mime::blocked_as_script(response.mime_type.as_ref(), &response.headers)
    {
        return None;
    }
    // Its own policies (M33.1): its response's, or its creator's for a
    // local URL.
    let csp = match start.url.scheme() {
        "data" | "blob" => start.csp.inherited(),
        _ => net::csp::Csp::from_headers(&response.headers, response.url()).with_sink(),
    };
    Some((utf8_decode(&response.body), response.url().clone(), csp))
}

/// Encoding's "UTF-8 decode": a UTF-8 byte order mark taken away and the
/// rest decoded as UTF-8, whatever another mark says (a worker's and an
/// imported script's text, HTML section 10.2.4).
fn utf8_decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// A worker's credentials mode, from WorkerOptions' `credentials`,
/// which the bindings checked.
pub(crate) fn credentials_mode(text: &str) -> Credentials {
    match text {
        "omit" => Credentials::Omit,
        "include" => Credentials::Include,
        _ => Credentials::SameOrigin,
    }
}

/// The host a worker's scripts reach the engine through: no document,
/// its own timers and workers, and its owner, which its messages,
/// console and errors go to.
pub(crate) struct WorkerHost {
    id: u32,
    owner: Option<Inbox>,
    /// What its requests are made by: its URL and origin.
    initiator: Option<Initiator>,
    client: Option<Arc<dyn Fetcher>>,
    /// The storage its creator reaches and its storage key, for the Cache
    /// API (M39).
    storage: Option<(Arc<dyn crate::storage::Areas>, crate::storage::AreaKey)>,
    origin: Origin,
    url: Option<Url>,
    secure: bool,
    /// Its clock: real time from when it started.
    clock: Instant,
    epoch_ms: f64,
    timers: Timers,
    /// The timer nesting level of the task it runs.
    nesting: u32,
    /// The threads its fetches run on, which wake it through its inbox:
    /// each fetch by its job, with the number the bindings gave it, its
    /// URL and its abort handle.
    pool: Option<Pool<Result<net::Response, net::NetworkError>>>,
    fetching: HashMap<u64, (u32, Url, net::AbortHandle)>,
    next_job: u64,
    /// Its canvases (an `OffscreenCanvas`'s, an `ImageBitmap`'s), and
    /// what they measure text with.
    canvases: Canvases,
    viewport: (u32, u32),
    fonts: Option<text::FontCollection>,
    compat: bool,
    /// `close()` was called: the task ends, and nothing more runs.
    closing: bool,
    /// An animation frame callback waits, and when the last ran.
    frame_wanted: bool,
    last_frame: Option<Duration>,
    workers: Option<Workers>,
    /// The router of its document's ports, and its realm's number there,
    /// and its document's.
    router: Option<SharedRouter>,
    document: u32,
    /// Where its creator is in its tab (M33.2).
    partition: Option<net::Partition>,
    /// Its own policies (M33.1), which its requests pass.
    csp: Option<net::csp::Csp>,
    /// What lets its requests ask their way into a more private address
    /// space (M33.4): its creator's.
    local_network: Option<net::LocalNetworkAccess>,
    realm: u32,
    work: u64,
    /// A service worker's own (M39); none for a dedicated worker.
    pub(super) service: Option<super::service_workers::Service>,
}

impl Default for WorkerHost {
    fn default() -> WorkerHost {
        WorkerHost {
            id: 0,
            owner: None,
            initiator: None,
            client: None,
            storage: None,
            origin: Origin::Opaque(0),
            url: None,
            secure: false,
            clock: Instant::now(),
            epoch_ms: 0.0,
            timers: Timers::default(),
            nesting: 0,
            frame_wanted: false,
            last_frame: None,
            pool: None,
            fetching: HashMap::new(),
            next_job: 0,
            canvases: Canvases::default(),
            viewport: (0, 0),
            fonts: None,
            compat: false,
            closing: false,
            workers: None,
            router: None,
            realm: 0,
            document: 0,
            partition: None,
            csp: None,
            local_network: None,
            work: 0,
            service: None,
        }
    }
}

impl ScriptingHost for WorkerHost {
    fn console(&mut self, level: ConsoleLevel, message: String) {
        self.say(ConsoleSource::Worker, level, message);
    }

    fn console_record(&mut self, record: js::ConsoleRecord) {
        let entry = ConsoleEntry::recorded(ConsoleSource::Worker, record);
        self.tell(FromWorker::Console(Box::new(entry)));
    }

    fn title(&mut self) -> String {
        String::new()
    }

    fn set_title(&mut self, _title: String) {}

    fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        self.work += 1;
        if let Some(router) = &self.router
            && let Some(answer) = port_call(router, self.realm, &call)
        {
            return answer;
        }
        match call {
            Call::SetTimer { id, delay } => {
                let now = self.now();
                self.timers.set(id, delay, self.nesting, now)?;
                Ok(Answer::Nothing)
            }
            Call::ClearTimer(id) => {
                self.timers.clear(id);
                Ok(Answer::Nothing)
            }
            Call::Now => {
                let tenths = self.now().as_micros() / 100;
                Ok(Answer::Number(tenths as f64 / 10.0))
            }
            Call::TimeOrigin => Ok(Answer::Number(self.epoch_ms)),
            Call::Origin => Ok(Answer::String(self.origin.serialization())),
            Call::SecureContext => Ok(Answer::Bool(self.secure)),
            // The worker's URL is the base its URLs are parsed against
            // (HTML section 10.2.1.1), and its location's.
            Call::ResolveUrl(text) | Call::ResolveUrlFromDocument(text) => Ok(Answer::MaybeString(
                Url::parse_with(&text, self.url.as_ref(), encoding::UTF_8)
                    .ok()
                    .map(|url| url.to_string()),
            )),
            Call::DocumentUrl => Ok(Answer::MaybeString(self.url.as_ref().map(Url::to_string))),
            Call::Location => Ok(Answer::Strings(location_parts(self.url.as_ref()))),
            Call::NavigatorInfo => {
                let (agent, mode) = if self.compat {
                    (net::COMPAT_USER_AGENT, "chrome")
                } else {
                    (net::USER_AGENT, "gecko")
                };
                Ok(Answer::Strings(vec![
                    agent.to_owned(),
                    PLATFORM.to_owned(),
                    mode.to_owned(),
                ]))
            }
            Call::UrlHasCredentials(url) => Ok(Answer::Bool(
                Url::parse(&url).is_ok_and(|url| url.includes_credentials()),
            )),
            Call::SameOriginUrl(url) => Ok(Answer::Bool(
                Url::parse(&url).is_ok_and(|url| url.origin() == self.origin),
            )),
            Call::Decode(bytes, label) => Ok(Answer::MaybeString(
                encoding::Encoding::for_label(&label)
                    .map(|encoding| encoding::decode(&bytes, encoding).0),
            )),
            Call::PlatformNewline => Ok(Answer::String(
                if cfg!(windows) { "\r\n" } else { "\n" }.to_owned(),
            )),
            Call::Fetch(call) => {
                let mut request = super::fetch::request(self.initiator.clone(), &call)?;
                request.partition.clone_from(&self.partition);
                request.csp.clone_from(&self.csp);
                request.local_network.clone_from(&self.local_network);
                if self.fetching.len() >= super::fetch::MAX_SCRIPT_FETCHES {
                    return Err(DomError::new("TypeError", "Too many requests in flight."));
                }
                let (Some(client), Some(pool)) = (self.client.clone(), self.pool.as_ref()) else {
                    return Err(DomError::new("TypeError", "Failed to fetch"));
                };
                let job = self.next_job;
                self.next_job += 1;
                self.fetching
                    .insert(job, (call.id, request.url.clone(), request.abort.clone()));
                pool.run(job, move |_| client.fetch(&request));
                Ok(Answer::Nothing)
            }
            Call::FetchNow(call) => {
                let mut request = super::fetch::request(self.initiator.clone(), &call)?;
                request.partition.clone_from(&self.partition);
                request.csp.clone_from(&self.csp);
                request.local_network.clone_from(&self.local_network);
                let Some(client) = &self.client else {
                    return Ok(Answer::Fetched(None));
                };
                let result = client.fetch(&request);
                self.report_violations();
                Ok(Answer::Fetched(match result {
                    Ok(response) => Some(Box::new(super::fetch::answer(&response))),
                    Err(error) => {
                        let text = super::fetch::failure_text(&request.url, error);
                        self.say(ConsoleSource::Network, ConsoleLevel::Error, text);
                        None
                    }
                }))
            }
            Call::AbortFetch(id) => {
                let jobs: Vec<u64> = self
                    .fetching
                    .iter()
                    .filter(|(_, (of, ..))| *of == id)
                    .map(|(job, _)| *job)
                    .collect();
                for job in jobs {
                    if let Some((_, _, abort)) = self.fetching.remove(&job) {
                        abort.abort();
                    }
                }
                Ok(Answer::Nothing)
            }
            Call::ImportScript(url) => Ok(Answer::Strings(
                self.import_script(&url)
                    .map_or_else(Vec::new, |(text, url)| vec![text, url]),
            )),
            Call::ServiceWorker(request) => match self.service.as_mut() {
                Some(service) => Ok(Answer::String(service.call(&request))),
                None => Err(DomError::not_supported()),
            },
            Call::ServiceWorkerRespond(head, body) => match self.service.as_mut() {
                Some(service) => {
                    service.respond(&head, body);
                    Ok(Answer::Nothing)
                }
                None => Err(DomError::not_supported()),
            },
            Call::Canvas(call) => self.canvas(*call),
            Call::PostToOwner(data) => {
                if data.len() > MAX_MESSAGE_BYTES {
                    return Err(too_large());
                }
                let sent = self.owner.as_ref().is_some_and(|owner| {
                    owner.send(WorkerMessage::FromWorker(
                        self.id,
                        FromWorker::Message(data),
                    ))
                });
                if !sent {
                    return Err(too_large());
                }
                Ok(Answer::Nothing)
            }
            Call::CloseWorker => {
                self.closing = true;
                Ok(Answer::Nothing)
            }
            // A dedicated worker's animation frames (HTML section 8.10).
            Call::CodeAllowed { kind, source, .. } => {
                let Some(csp) = &self.csp else {
                    return Ok(Answer::String(String::new()));
                };
                let violations = match kind.as_str() {
                    "wasm" => csp.wasm_violations(),
                    "eval" => csp.eval_violations(&source),
                    _ => Vec::new(),
                };
                let refused = net::csp::enforced(violations.clone()).err();
                for violation in violations {
                    let level = super::violation_level(&violation);
                    self.say(
                        ConsoleSource::Policy,
                        level,
                        super::violation_text(&violation),
                    );
                }
                Ok(Answer::String(
                    refused.map_or_else(String::new, |violation| {
                        super::host::code_refusal(&kind, &violation)
                    }),
                ))
            }
            Call::WantFrame => {
                self.frame_wanted = true;
                Ok(Answer::Nothing)
            }
            Call::Hidden => Ok(Answer::Bool(false)),
            Call::WorkerStart {
                url,
                name,
                module,
                credentials,
            } => {
                let parsed = Url::parse_with(&url, self.url.as_ref(), encoding::UTF_8).ok();
                let url = worker_url(&url, parsed, &self.origin)?;
                let (Some(client), Some(initiator), Some(workers)) =
                    (&self.client, &self.initiator, self.workers.as_mut())
                else {
                    return Err(DomError::not_supported());
                };
                Ok(Answer::Number(f64::from(
                    workers.start(Start {
                        url,
                        name,
                        module,
                        credentials: credentials_mode(&credentials),
                        client: client.clone(),
                        initiator: initiator.clone(),
                        secure: self.secure,
                        viewport: self.viewport,
                        fonts: self.fonts.clone(),
                        compat: self.compat,
                        partition: self.partition.clone(),
                        csp: self
                            .csp
                            .clone()
                            .unwrap_or_else(|| net::csp::Csp::none(self.origin.clone())),
                        referrer_policy: net::ReferrerPolicy::default(),
                        local_network: self.local_network.clone(),
                        // A nested worker reaches its creator's caches.
                        storage: self.storage.clone(),
                    }),
                )))
            }
            Call::WorkerPost { worker, data } => {
                if let Some(workers) = self.workers.as_mut() {
                    workers.post(worker, data)?;
                }
                Ok(Answer::Nothing)
            }
            Call::WorkerTerminate(worker) => {
                if let Some(workers) = self.workers.as_mut() {
                    workers.terminate(worker);
                }
                Ok(Answer::Nothing)
            }
            Call::Caches(request, bytes) => match &self.storage {
                Some((areas, key)) => {
                    let (answer, bytes) = areas.caches(key, &request, &bytes);
                    Ok(Answer::Pair(answer, bytes))
                }
                None => Err(DomError::new(
                    "SecurityError",
                    "Cache storage is disabled for this worker's origin.",
                )),
            },
            _ => Err(DomError::not_supported()),
        }
    }

    fn take_work(&mut self) -> u64 {
        std::mem::take(&mut self.work)
    }

    fn report(&mut self, error: ScriptError) {
        self.uncaught(error);
    }
}

impl WorkerHost {
    /// Writes `text` at `level` on its document's console, as `source`.
    fn say(&self, source: ConsoleSource, level: ConsoleLevel, text: String) {
        let entry = ConsoleEntry::of(source, level, cut(text));
        self.tell(FromWorker::Console(Box::new(entry)));
    }

    /// Says on its document's console what its policies refused, or
    /// would have, at its requests' hops (M33.1).
    fn report_violations(&self) {
        let Some(csp) = &self.csp else {
            return;
        };
        for violation in csp.take_violations() {
            let level = super::violation_level(&violation);
            self.say(
                ConsoleSource::Policy,
                level,
                super::violation_text(&violation),
            );
        }
    }

    /// The answers of the worker's fetches that finished, as tasks; why
    /// one failed goes to the console. Those aborted are dropped.
    fn take_fetched(&mut self) -> Vec<Task> {
        self.report_violations();
        let mut tasks = Vec::new();
        while let Some((job, result)) = self.pool.as_ref().and_then(Pool::try_take) {
            let Some((id, url, _)) = self.fetching.remove(&job) else {
                continue;
            };
            let answer = match result {
                Ok(Ok(response)) => Some(Box::new(super::fetch::answer(&response))),
                Ok(Err(error)) => {
                    let text = super::fetch::failure_text(&url, error);
                    self.say(ConsoleSource::Network, ConsoleLevel::Error, text);
                    None
                }
                Err(_) => None,
            };
            tasks.push(Task::Fetched { id, answer });
        }
        tasks
    }

    /// A script `importScripts` runs (HTML's "fetch a classic
    /// worker-imported script"): fetched as a script, which needs an ok
    /// status and a JavaScript type, decoded as UTF-8; its text and URL.
    fn import_script(&mut self, url: &str) -> Option<(String, String)> {
        let url = Url::parse(url).ok()?;
        // A service worker's (M39): fetched while it is evaluated and kept
        // as its version's, after that only those kept.
        if let Some(service) = &self.service
            && !service.recording()
        {
            let body = service.stored(&url)?;
            return Some((utf8_decode(&body), url.to_string()));
        }
        let mut request = Request::get(url, Destination::Script)
            .initiated_by(self.initiator.clone()?)
            .with_body_limit(MAX_SCRIPT_BYTES);
        request.partition.clone_from(&self.partition);
        request.csp.clone_from(&self.csp);
        let result = self.client.as_ref()?.fetch(&request);
        self.report_violations();
        let response = result.ok()?;
        if !response.ok()
            || !response
                .mime_type
                .as_ref()
                .is_some_and(net::MimeType::is_javascript)
        {
            return None;
        }
        if let Some(service) = self.service.as_mut() {
            service.record(response.url().clone(), &response.body);
        }
        Some((utf8_decode(&response.body), response.url().to_string()))
    }

    /// A canvas's call in a worker (M31): an `OffscreenCanvas`'s or an
    /// `ImageBitmap`'s, there being no element.
    fn canvas(&mut self, call: js::CanvasCall) -> Result<Answer, DomError> {
        let illegal = || DomError::new("TypeError", "Illegal invocation");
        if call.op == "imageState" {
            return Err(illegal());
        }
        if call.op == "imageBitmap" {
            let answer = image_bitmap(&mut self.canvases, &call, None, &mut self.work);
            self.work += self.canvases.take_work();
            return answer;
        }
        let element = if names_context(&call.op) && call.numbers.first() == Some(&0.0) {
            let handle = call.numbers.get(1).copied().unwrap_or(f64::NAN);
            if !((-f64::from(u32::MAX)..0.0).contains(&handle) && handle.fract() == 0.0) {
                return Err(illegal());
            }
            let source = if needs_source(&call.op) {
                let kind = call.numbers.get(2).copied().unwrap_or(-1.0);
                let which = call.numbers.get(3).copied().unwrap_or(-1.0);
                offscreen_source(&mut self.canvases, kind, which)
                    .unwrap_or_else(|| Err(illegal()))?
            } else {
                None
            };
            Some(offscreen_element(
                handle,
                self.viewport,
                self.fonts.clone(),
                source,
            ))
        } else {
            None
        };
        let answer = self.canvases.call(call, element);
        self.work += self.canvases.take_work();
        answer
    }

    /// The worker's time: since it started.
    fn now(&self) -> Duration {
        self.clock.elapsed()
    }

    /// Sends the owner `event`.
    fn tell(&self, event: FromWorker) {
        if let Some(owner) = &self.owner {
            owner.send(WorkerMessage::FromWorker(self.id, event));
        }
    }

    /// An error the worker's own `error` event did not handle: an
    /// exception goes to its owner, which fires it at the worker's
    /// object; a rejection, which stays in the worker, to the console.
    fn uncaught(&self, error: ScriptError) {
        match error.kind {
            ErrorKind::Exception => self.tell(FromWorker::Error(Some(error))),
            ErrorKind::Rejection | ErrorKind::Heap => {
                if let Some(entry) = ConsoleEntry::of_error(&error) {
                    self.tell(FromWorker::Console(Box::new(entry)));
                }
            }
            ErrorKind::Budget | ErrorKind::Dead | ErrorKind::Handled => {}
        }
    }

    /// After a task: the microtask checkpoint, and the errors of both,
    /// unless the worker was stopped.
    fn finish(
        &mut self,
        realm: &mut dyn Realm<WorkerHost>,
        result: Result<bool, ScriptError>,
        stop: &AtomicBool,
    ) {
        let errors = realm.checkpoint(self);
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if let Err(error) = result {
            self.uncaught(error);
        }
        for error in errors {
            self.uncaught(error);
        }
        if realm.dead().is_some() {
            self.closing = true;
        }
        // What its offscreen canvases with placeholders drew in the task
        // goes to the document, which shows it at its next frame.
        if let Some(router) = &self.router {
            for commit in self.canvases.commits() {
                lock(router).send_to(self.document, WorkerMessage::Frame(commit));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_to_a_realm_that_went_is_dropped_not_refused() {
        let (to, mailbox) = inbox(None);
        drop(mailbox);
        assert!(to.send(WorkerMessage::ToWorker("late".to_owned())));
        assert_eq!(to.queued.load(Ordering::SeqCst), 0);
    }
}
