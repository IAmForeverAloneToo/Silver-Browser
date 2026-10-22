//! What does the registry's effects (M39, D35): a thread of the
//! context's, which takes the jobs scheduled and what was done for them
//! in the order they come, gives them to the registry in the storage,
//! and does what it then asks: a script fetched on a thread of its own,
//! a version run by the [`ServiceWorkerHost`], a promise settled, a
//! notice told to the listeners.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::thread;

use net::{CacheMode, Destination, NetworkError, Redirect, Request, Url};

use super::{Effect, Evaluated, FetchFailure, Fetched, Job, Notice, Outcome, View};
use crate::loader::Fetcher;
use crate::scripts::MAX_SCRIPT_BYTES;
use crate::storage::Storage;

/// What is called when something asked for is done.
pub type Done<T> = Box<dyn FnOnce(T) + Send>;

/// What a listener hears: a storage key's notice, with its
/// registration's view; it is dropped once it answers false.
pub type Listener = Arc<dyn Fn(&str, &Notice, &View) -> bool + Send + Sync>;

/// Whether what it is asked of is still there.
pub type Alive = Arc<dyn Fn() -> bool + Send + Sync>;

/// How a version answered a fetch (M39 step 5): not at all, so the
/// request goes to the network; with a network error; or with a
/// response.
#[derive(Debug)]
pub enum Responded {
    Network,
    Failed,
    Given(Box<net::Response>),
}

/// How a job's promise settled, and its registration's view then.
#[derive(Clone, Debug)]
pub struct Settled {
    pub outcome: Outcome,
    pub view: Option<View>,
}

/// A version to run: its numbers, its registration's storage key and
/// scope, and its script.
pub struct Start {
    pub version: u64,
    pub registration: u64,
    pub key: String,
    pub scope: Url,
    pub script: Url,
    pub source: Arc<[u8]>,
    pub partition: Option<net::Partition>,
}

/// What runs versions and dispatches their events: the engine's threads
/// in one process, a worker renderer of the site once there are
/// processes (D35).
pub trait ServiceWorkerHost: Send + Sync {
    /// Runs the version's script, which hears no event, and keeps it
    /// running for its events; what it did comes to `done`, none when it
    /// threw or did not run.
    fn evaluate(&self, start: Start, done: Done<Option<Evaluated>>);
    /// Dispatches `install` at the version: `done` hears whether its
    /// `waitUntil` promises all fulfilled.
    fn install(&self, version: u64, done: Done<bool>);
    /// Dispatches `activate` at the version: `done` is called once its
    /// `waitUntil` promises settled.
    fn activate(&self, version: u64, done: Done<()>);
    /// Stops the version, if it runs.
    fn terminate(&self, version: u64);
    /// The versions of storage key `key` hear `notice`: their
    /// registration objects change.
    fn notice(&self, _key: &str, _notice: &Notice, _view: &View) {}
    /// Dispatches a client's message at the version (M39, an
    /// `ExtendableMessageEvent`): `data` its message, its origin and its
    /// source, as JSON; `done` hears whether its promises fulfilled.
    fn message(&self, _version: u64, _data: String, done: Done<bool>) {
        done(false);
    }
    /// Dispatches a `FetchEvent` at the version (M39 step 5): `data` its
    /// request and its clients' ids, as JSON; `done` hears how it
    /// answered.
    fn fetch(&self, _version: u64, _data: String, done: Done<Responded>) {
        done(Responded::Network);
    }
}

/// The host when none was given: no script runs, so no version
/// evaluates.
struct NoHost;

impl ServiceWorkerHost for NoHost {
    fn evaluate(&self, _start: Start, done: Done<Option<Evaluated>>) {
        done(None);
    }

    fn install(&self, _version: u64, done: Done<bool>) {
        done(false);
    }

    fn activate(&self, _version: u64, done: Done<()>) {
        done(());
    }

    fn terminate(&self, _version: u64) {}
}

/// What the driver's thread is told, in order.
enum Message {
    Schedule(Box<Job>, Done<Settled>),
    Fetched(u64, Result<Fetched, FetchFailure>),
    Evaluated(u64, Option<Evaluated>),
    Installed(u64, bool),
    Activated(u64),
    SkipWaiting(u64),
    Use(u64, bool),
    SoftUpdate(u64),
    /// The registry changed outside: its effects are done.
    Pump,
}

/// The context's service workers: the jobs scheduled through it, the
/// host its versions run in, and the listeners of its notices. Clones
/// are the same.
#[derive(Clone)]
pub struct ServiceWorkers {
    inner: Arc<Inner>,
}

/// The context's service workers, not kept alive by this.
#[derive(Clone)]
pub struct WeakServiceWorkers {
    inner: Weak<Inner>,
}

impl WeakServiceWorkers {
    pub fn upgrade(&self) -> Option<ServiceWorkers> {
        self.inner.upgrade().map(|inner| ServiceWorkers { inner })
    }
}

struct Inner {
    storage: Arc<Mutex<Storage>>,
    fetcher: Arc<dyn Fetcher>,
    host: Mutex<Arc<dyn ServiceWorkerHost>>,
    listeners: Mutex<Vec<Listener>>,
    /// The listeners that live while their client does.
    living: Mutex<Vec<(Alive, Listener)>>,
    /// The clients, by their ids.
    clients: Mutex<HashMap<String, super::clients::ClientEntry>>,
    /// The thread's channel, the thread started at the first message.
    sender: OnceLock<Sender<Message>>,
}

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ServiceWorkers {
    /// The service workers of the registry in `storage`, their scripts
    /// fetched by `fetcher`; no host until one is given.
    pub(crate) fn new(storage: Arc<Mutex<Storage>>, fetcher: Arc<dyn Fetcher>) -> ServiceWorkers {
        let inner = Arc::new(Inner {
            storage,
            fetcher,
            host: Mutex::new(Arc::new(NoHost)),
            listeners: Mutex::new(Vec::new()),
            living: Mutex::new(Vec::new()),
            clients: Mutex::new(HashMap::new()),
            sender: OnceLock::new(),
        });
        // The registry changed outside: the thread, started if it was not,
        // does its effects. Called with the storage locked, so it locks
        // nothing.
        let weak = Arc::downgrade(&inner);
        let mut storage = lock(&inner.storage);
        storage.set_service_workers(WeakServiceWorkers {
            inner: weak.clone(),
        });
        storage.set_registry_wake(Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                ServiceWorkers { inner }.send(Message::Pump);
            }
        }));
        drop(storage);
        ServiceWorkers { inner }
    }

    /// A handle that does not keep them alive.
    pub fn downgrade(&self) -> WeakServiceWorkers {
        WeakServiceWorkers {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Runs versions with `host` from now on.
    pub fn set_host(&self, host: Arc<dyn ServiceWorkerHost>) {
        *lock(&self.inner.host) = host;
    }

    /// Has `listener` hear every notice from now on.
    pub fn listen(&self, listener: Listener) {
        lock(&self.inner.listeners).push(listener);
    }

    /// Has `listener` hear every notice while `alive` holds: the
    /// listeners of clients gone are dropped as one comes.
    pub(crate) fn listen_while(
        &self,
        alive: Arc<dyn Fn() -> bool + Send + Sync>,
        listener: Listener,
    ) {
        let mut listeners = lock(&self.inner.living);
        listeners.retain(|(alive, listener)| {
            let keep = alive();
            if !keep {
                lock(&self.inner.listeners).retain(|l| !Arc::ptr_eq(l, listener));
            }
            keep
        });
        listeners.push((alive, listener.clone()));
        drop(listeners);
        self.listen(listener);
    }

    /// What `query` finds in the registry, `key`'s registrations read
    /// from the profile if they were not.
    pub fn query<T>(&self, key: &str, query: impl FnOnce(&super::Registry) -> T) -> T {
        query(lock(&self.inner.storage).service_workers(key))
    }

    /// What `query` finds in the registry as it is, no key read.
    pub fn query_any<T>(&self, query: impl FnOnce(&super::Registry) -> T) -> T {
        query(lock(&self.inner.storage).registry())
    }

    fn send(&self, message: Message) {
        let sender = self.inner.sender.get_or_init(|| {
            let (sender, receiver) = channel();
            let weak = Arc::downgrade(&self.inner);
            let _ = thread::Builder::new()
                .name("service workers".to_owned())
                .spawn(move || run(&weak, &receiver));
            sender
        });
        let _ = sender.send(message);
    }

    /// Schedule Job: `done` hears how its promise settles.
    pub fn schedule(&self, job: Job, done: Done<Settled>) {
        self.send(Message::Schedule(Box::new(job), done));
    }

    /// `skipWaiting()` of the version.
    pub fn skip_waiting(&self, version: u64) {
        self.send(Message::SkipWaiting(version));
    }

    /// A client of the registration's came (`controlled`) or went.
    pub fn use_registration(&self, registration: u64, controlled: bool) {
        self.send(Message::Use(registration, controlled));
    }

    /// The clients, locked.
    pub(crate) fn clients(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, super::clients::ClientEntry>> {
        lock(&self.inner.clients)
    }

    /// A client's message for `version`, as [`ServiceWorkerHost::message`]
    /// takes it.
    pub(crate) fn post_message(&self, version: u64, data: String) {
        host(&self.inner).message(version, data, Box::new(|_| {}));
    }

    /// A `FetchEvent` for `version`, as [`ServiceWorkerHost::fetch`]
    /// takes it.
    pub(crate) fn fetch_event(&self, version: u64, data: String, done: Done<Responded>) {
        host(&self.inner).fetch(version, data, done);
    }

    /// Soft Update of the registration.
    pub fn soft_update(&self, registration: u64) {
        self.send(Message::SoftUpdate(registration));
    }
}

/// The driver's thread: each message given to the registry, then its
/// effects done, until the context goes.
fn run(weak: &Weak<Inner>, receiver: &Receiver<Message>) {
    let mut waiting: HashMap<u64, Done<Settled>> = HashMap::new();
    while let Ok(message) = receiver.recv() {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let effects = {
            let mut storage = lock(&inner.storage);
            match message {
                Message::Schedule(job, done) => {
                    let ticket = storage.service_workers(&job.key).schedule(*job);
                    waiting.insert(ticket, done);
                }
                Message::Fetched(job, result) => storage.registry().fetched(job, result),
                Message::Evaluated(job, result) => storage.registry().evaluated(job, result),
                Message::Installed(version, ok) => storage.registry().installed(version, ok),
                Message::Activated(version) => storage.registry().activated(version),
                Message::SkipWaiting(version) => storage.registry().skip_waiting(version),
                Message::Use(registration, controlled) => {
                    storage
                        .registry()
                        .use_registration(registration, controlled);
                }
                Message::SoftUpdate(registration) => storage.registry().soft_update(registration),
                Message::Pump => {}
            }
            storage.registry().take_effects()
        };
        let Some(sender) = inner.sender.get().cloned() else {
            return;
        };
        for effect in effects {
            perform(&inner, &sender, effect, &mut waiting);
        }
    }
}

/// Does `effect`, what comes of it sent to the thread by `sender`.
fn perform(
    inner: &Arc<Inner>,
    sender: &Sender<Message>,
    effect: Effect,
    waiting: &mut HashMap<u64, Done<Settled>>,
) {
    match effect {
        Effect::Fetch {
            job,
            url,
            main,
            bypass_cache,
            partition,
        } => {
            let fetcher = inner.fetcher.clone();
            let fetched = sender.clone();
            let spawned = thread::Builder::new()
                .name("service worker script".to_owned())
                .spawn(move || {
                    let result = fetch_script(&*fetcher, *url, main, bypass_cache, partition);
                    let _ = fetched.send(Message::Fetched(job, result));
                });
            if spawned.is_err() {
                let _ = sender.send(Message::Fetched(job, Err(FetchFailure::Network)));
            }
        }
        Effect::Evaluate { job, version } => {
            let start = {
                let mut storage = lock(&inner.storage);
                let registry = storage.registry();
                registry.version(version).and_then(|v| {
                    let reg = registry.registration(v.registration)?;
                    Some(Start {
                        version,
                        registration: reg.id,
                        key: reg.key.clone(),
                        scope: reg.scope.clone(),
                        script: v.script.clone(),
                        source: v.resources.first()?.1.clone(),
                        partition: reg.partition.clone(),
                    })
                })
            };
            let sender = sender.clone();
            let done: Done<Option<Evaluated>> = Box::new(move |result| {
                let _ = sender.send(Message::Evaluated(job, result));
            });
            match start {
                Some(start) => host(inner).evaluate(start, done),
                None => done(None),
            }
        }
        Effect::Install { version } => {
            let sender = sender.clone();
            host(inner).install(
                version,
                Box::new(move |ok| {
                    let _ = sender.send(Message::Installed(version, ok));
                }),
            );
        }
        Effect::Activate { version } => {
            let sender = sender.clone();
            host(inner).activate(
                version,
                Box::new(move |()| {
                    let _ = sender.send(Message::Activated(version));
                }),
            );
        }
        Effect::Terminate { version } => host(inner).terminate(version),
        Effect::Settle {
            ticket,
            outcome,
            view,
        } => {
            if let Some(done) = waiting.remove(&ticket) {
                done(Settled { outcome, view });
            }
        }
        Effect::Notice { key, notice, view } => {
            host(inner).notice(&key, &notice, &view);
            // Activate: the registration's clients have the new version
            // for their controller.
            if let Notice::Slot {
                registration,
                slot: super::Slot::Active,
                version: Some(_),
            } = notice
                && let Some(shown) = &view.active
            {
                ServiceWorkers {
                    inner: inner.clone(),
                }
                .new_active(registration, shown);
            }
            let listeners = lock(&inner.listeners).clone();
            let gone: Vec<Listener> = listeners
                .into_iter()
                .filter(|listener| !listener(&key, &notice, &view))
                .collect();
            if !gone.is_empty() {
                lock(&inner.listeners)
                    .retain(|listener| !gone.iter().any(|g| Arc::ptr_eq(g, listener)));
            }
        }
    }
}

fn host(inner: &Arc<Inner>) -> Arc<dyn ServiceWorkerHost> {
    lock(&inner.host).clone()
}

/// A service worker's script (Update's "perform the fetch"), or one its
/// newest version imported: destination `serviceworker` with the
/// `Service-Worker: script` field and redirects an error for the
/// script, `script` for an import; of the script's origin, past the
/// HTTP cache when `bypass_cache`.
fn fetch_script(
    fetcher: &dyn Fetcher,
    url: Url,
    main: bool,
    bypass_cache: bool,
    partition: Option<net::Partition>,
) -> Result<Fetched, FetchFailure> {
    let origin = url.origin();
    let destination = match main {
        true => Destination::ServiceWorker,
        false => Destination::Script,
    };
    let mut request = Request::get(url, destination).with_body_limit(MAX_SCRIPT_BYTES);
    request.origin = Some(origin);
    request.partition = partition;
    if main {
        request.headers.append(b"Service-Worker", b"script");
        request.redirect = Redirect::Error;
    }
    if bypass_cache {
        request.cache = CacheMode::NoCache;
    }
    let response = fetcher.fetch(&request).map_err(|error| match error {
        NetworkError::Redirected => FetchFailure::Redirect,
        _ => FetchFailure::Network,
    })?;
    let allowed = response
        .headers
        .get(b"Service-Worker-Allowed")
        .map(|value| String::from_utf8_lossy(&value).into_owned());
    Ok(Fetched {
        status: response.status,
        mime: response
            .mime_type
            .as_ref()
            .map(|mime| mime.essence().to_owned()),
        javascript: response
            .mime_type
            .as_ref()
            .is_some_and(net::MimeType::is_javascript),
        allowed,
        body: Arc::from(response.body),
        // The HTTP cache does not say whether it answered: every check
        // counts as the network's.
        from_cache: false,
    })
}
