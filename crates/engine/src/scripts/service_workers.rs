//! Service workers' versions in one process (M39, D35): each running
//! version a thread with a realm of `RealmKind::ServiceWorker`, built on
//! the dedicated workers' machinery (M31): its event loop, timers,
//! fetches, imported scripts and ports. A version runs while it is
//! needed: started for its evaluation or an event, stopped after
//! [`SERVICE_WORKER_IDLE`] with no event pending, or once an event
//! outlasts [`SERVICE_WORKER_EVENT_LIMIT`]; what it keeps, it keeps in
//! the Cache API or IndexedDB, its global gone with it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use devtools::Json;
use net::{AddressSpace, Url};

use super::workers::{Inbox, Mailbox, WorkerHost, WorkerMessage, inbox};
use crate::loader::Fetcher;
use crate::service_worker::page::{Client, notice_json, view_json};
use crate::service_worker::{
    Done, Evaluated, Notice, ServiceWorkerHost, Start, View, WeakServiceWorkers,
};
use crate::storage::{AreaKey, Areas};

/// How long a version runs with no event pending before it is stopped:
/// the reference browser's (a second in the tests).
#[cfg(not(test))]
pub(crate) const SERVICE_WORKER_IDLE: Duration = Duration::from_secs(30);
#[cfg(test)]
pub(crate) const SERVICE_WORKER_IDLE: Duration = Duration::from_secs(1);
/// How long an event may keep its version: past it, the version is
/// stopped and the event fails (three seconds in the tests).
#[cfg(not(test))]
pub(crate) const SERVICE_WORKER_EVENT_LIMIT: Duration = Duration::from_secs(300);
#[cfg(test)]
pub(crate) const SERVICE_WORKER_EVENT_LIMIT: Duration = Duration::from_secs(3);
/// The most versions a context runs at once: past it, the one given an
/// event longest ago is stopped.
pub(crate) const MAX_RUNNING_SERVICE_WORKERS: usize = 32;
/// A running version's thread's stack: the engine's own.
const SERVICE_WORKER_STACK: usize = 32 << 20;

/// A version running: how to reach it and stop it, its storage key, and
/// when it was last given an event.
struct Running {
    to: Inbox,
    stop: Arc<AtomicBool>,
    key: String,
    used: Instant,
}

type RunningMap = Arc<Mutex<HashMap<u64, Running>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A service worker's own part of its host: its version, its events
/// pending, its scripts, and what reaches its registry.
pub(crate) struct Service {
    version: u64,
    stop: Arc<AtomicBool>,
    running: RunningMap,
    /// `{view, worker}`: its registration and itself when it started.
    own: String,
    events: HashMap<u32, (Instant, Done<bool>)>,
    next_event: u32,
    idle_since: Instant,
    /// The event types its global listens for, once asked.
    listened: Option<Vec<String>>,
    /// Whether the scripts it imports are fetched and kept: while it is
    /// evaluated for its job.
    recording: bool,
    imports: Vec<(Url, Arc<[u8]>)>,
    /// Its version's scripts, which it imports from once evaluated.
    stored: Vec<(Url, Arc<[u8]>)>,
    workers: WeakServiceWorkers,
    /// What its registration's methods ask as.
    client: Client,
}

impl Service {
    /// When it is next due to stop: its idleness's end, or its oldest
    /// event's limit.
    pub(crate) fn due(&self) -> Instant {
        match self.events.values().map(|(at, _)| *at).min() {
            Some(oldest) => oldest + SERVICE_WORKER_EVENT_LIMIT,
            None => self.idle_since + SERVICE_WORKER_IDLE,
        }
    }

    /// The task of the extendable event `kind`, whose end `done` hears.
    pub(crate) fn event(&mut self, kind: &str, done: Done<bool>) -> String {
        self.next_event += 1;
        let id = self.next_event;
        self.events.insert(id, (Instant::now(), done));
        Json::object([
            ("type", Json::from("event")),
            ("kind", Json::from(kind)),
            ("id", Json::from(id)),
        ])
        .write()
    }

    pub(crate) fn recording(&self) -> bool {
        self.recording
    }

    /// The body of its version's script `url`, if it has one.
    pub(crate) fn stored(&self, url: &Url) -> Option<Arc<[u8]>> {
        self.stored
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, body)| body.clone())
    }

    /// Keeps script `url` it imported while evaluated.
    pub(crate) fn record(&mut self, url: Url, body: &[u8]) {
        if self.recording {
            self.imports.push((url, Arc::from(body)));
        }
    }

    /// Its global's request of the host, as JSON.
    pub(crate) fn call(&mut self, request: &str) -> String {
        let Ok(parsed) = Json::parse(request) else {
            return r#"{"error":{"name":"TypeError","message":"Invalid request."}}"#.to_owned();
        };
        match parsed.get("op").and_then(Json::as_str) {
            Some("self") => format!(r#"{{"result":{}}}"#, self.own),
            Some("eventDone") => {
                let id = parsed.get("id").and_then(Json::as_u64).unwrap_or(0);
                let ok = parsed.get("ok").and_then(Json::as_bool).unwrap_or(false);
                if let Some((_, done)) = u32::try_from(id)
                    .ok()
                    .and_then(|id| self.events.remove(&id))
                {
                    done(ok);
                }
                if self.events.is_empty() {
                    self.idle_since = Instant::now();
                }
                r#"{"result":null}"#.to_owned()
            }
            Some("skipWaiting") => {
                if let Some(workers) = self.workers.upgrade() {
                    workers.skip_waiting(self.version);
                }
                r#"{"result":null}"#.to_owned()
            }
            Some("listened") => {
                let types = parsed
                    .get("types")
                    .and_then(Json::as_array)
                    .map(|types| {
                        types
                            .iter()
                            .filter_map(Json::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                self.listened = Some(types);
                r#"{"result":null}"#.to_owned()
            }
            // Its registration's own methods, as a client's.
            _ => match self.workers.upgrade() {
                Some(workers) => {
                    crate::service_worker::page::serve(&workers, &self.client, request)
                }
                None => crate::storage::NO_SERVICE_WORKERS.to_owned(),
            },
        }
    }

    /// The event types its global listens for, once asked.
    pub(crate) fn take_listened(&mut self) -> Vec<String> {
        self.listened.take().unwrap_or_default()
    }

    /// Its evaluation is over: the scripts it imported, kept as its
    /// version's from now on.
    pub(crate) fn end_recording(&mut self) -> Vec<(Url, Arc<[u8]>)> {
        self.recording = false;
        self.stored.clone_from(&self.imports);
        std::mem::take(&mut self.imports)
    }

    /// Its thread ends: it leaves the running versions and its events
    /// fail.
    pub(crate) fn end(&mut self) {
        let running = self.running.clone();
        self.leave(&mut lock(&running));
        self.fail_events();
    }

    /// Fails its events: it stops.
    fn fail_events(&mut self) {
        for (_, (_, done)) in self.events.drain() {
            done(false);
        }
    }

    /// Takes it out of the running versions, if it is still the one
    /// there.
    fn leave(&self, running: &mut HashMap<u64, Running>) {
        if running
            .get(&self.version)
            .is_some_and(|r| Arc::ptr_eq(&r.stop, &self.stop))
        {
            running.remove(&self.version);
        }
    }
}

/// The version is due to stop: it leaves the running versions, unless a
/// message came first, which it then runs. It leaves under the lock its
/// senders hold, so that nothing is sent to it once it has looked; its
/// events fail as its thread ends.
pub(super) fn retire(host: &mut WorkerHost, mailbox: &Mailbox) -> Option<WorkerMessage> {
    let service = host.service.as_mut()?;
    let running = service.running.clone();
    let mut running = lock(&running);
    if let Some(message) = mailbox.try_take() {
        // Its idleness ends with the message.
        service.idle_since = Instant::now();
        return Some(message);
    }
    service.leave(&mut running);
    None
}

/// The address space a version's requests are made from: its script's
/// host's, which a name under `localhost` or a loopback address makes
/// loopback.
fn space_of(url: &Url) -> AddressSpace {
    let host = url.host().map(ToString::to_string).unwrap_or_default();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<std::net::IpAddr>() {
        Ok(address) => AddressSpace::of(address),
        Err(_) if bare == "localhost" || bare.ends_with(".localhost") => AddressSpace::Loopback,
        Err(_) => AddressSpace::Public,
    }
}

/// The service workers of the registry in `storage`, their scripts
/// fetched with `client`, their versions run on threads of this process
/// and reaching `storage`'s caches.
pub(crate) fn in_process(
    storage: Arc<Mutex<crate::storage::Storage>>,
    client: Arc<dyn Fetcher>,
) -> crate::service_worker::ServiceWorkers {
    in_process_with_host(storage, client).0
}

/// [`in_process`]'s, and their host.
pub(crate) fn in_process_with_host(
    storage: Arc<Mutex<crate::storage::Storage>>,
    client: Arc<dyn Fetcher>,
) -> (crate::service_worker::ServiceWorkers, InProcess) {
    let workers = crate::service_worker::ServiceWorkers::new(storage.clone(), client.clone());
    let host = InProcess::new(
        client,
        Arc::new(crate::storage::TabAreas { storage, tab: 0 }),
    );
    host.set_workers(workers.downgrade());
    workers.set_host(Arc::new(host.clone()));
    (workers, host)
}

/// The versions of a context's service workers, run on its threads.
#[derive(Clone)]
pub(crate) struct InProcess {
    shared: Arc<Shared>,
}

struct Shared {
    running: RunningMap,
    client: Arc<dyn Fetcher>,
    areas: Arc<dyn Areas>,
    /// The context's service workers, for `skipWaiting()` and to start a
    /// version again from its scripts.
    workers: Mutex<Option<WeakServiceWorkers>>,
}

impl InProcess {
    /// Versions whose scripts fetch with `client` and reach `areas`.
    pub(crate) fn new(client: Arc<dyn Fetcher>, areas: Arc<dyn Areas>) -> InProcess {
        InProcess {
            shared: Arc::new(Shared {
                running: Arc::default(),
                client,
                areas,
                workers: Mutex::new(None),
            }),
        }
    }

    /// The context's service workers, which its versions reach.
    pub(crate) fn set_workers(&self, workers: WeakServiceWorkers) {
        *lock(&self.shared.workers) = Some(workers);
    }

    /// Whether version `version` runs, for the tests.
    #[cfg(test)]
    pub(crate) fn runs(&self, version: u64) -> bool {
        lock(&self.shared.running).contains_key(&version)
    }

    fn workers(&self) -> Option<WeakServiceWorkers> {
        lock(&self.shared.workers).clone()
    }

    /// Starts `start`'s version: evaluated for its job when `evaluated`
    /// is given, else started again from its scripts kept (`stored`).
    fn start(
        &self,
        start: &Start,
        stored: Vec<(Url, Arc<[u8]>)>,
        evaluated: Option<Done<Option<Evaluated>>>,
    ) -> Option<Inbox> {
        let workers = self.workers()?;
        let driver = workers.upgrade()?;
        let own = driver.query(&start.key, |registry| {
            let view = registry.view(start.registration)?;
            let worker = registry.version(start.version).map(|v| {
                Json::object([
                    ("id", Json::from(v.id)),
                    ("scriptURL", Json::from(v.script.as_str())),
                    ("state", Json::from(v.state.name())),
                ])
            });
            Some(
                Json::object([
                    ("view", view_json(&view)),
                    ("worker", worker.unwrap_or(Json::Null)),
                ])
                .write(),
            )
        })?;
        let (me, mailbox) = inbox(None);
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut running = lock(&self.shared.running);
            if running.len() >= MAX_RUNNING_SERVICE_WORKERS
                && let Some(oldest) = running.iter().min_by_key(|(_, r)| r.used).map(|(v, _)| *v)
                && let Some(gone) = running.remove(&oldest)
            {
                gone.stop.store(true, Ordering::SeqCst);
                gone.to.send(WorkerMessage::Stop);
            }
            running.insert(
                start.version,
                Running {
                    to: me.clone(),
                    stop: stop.clone(),
                    key: start.key.clone(),
                    used: Instant::now(),
                },
            );
        }
        let origin = start.script.origin();
        let tell = me.clone();
        let service = Service {
            version: start.version,
            stop: stop.clone(),
            running: self.shared.running.clone(),
            own,
            events: HashMap::new(),
            next_event: 0,
            idle_since: Instant::now(),
            listened: None,
            recording: evaluated.is_some(),
            imports: Vec::new(),
            stored,
            workers,
            client: Client {
                key: start.key.clone(),
                partition: start.partition.clone(),
                origin: origin.clone(),
                base: start.script.clone(),
                url: start.script.clone(),
                tell: Arc::new(move |text| {
                    tell.send(WorkerMessage::ServiceWorker(text));
                    tell.alive()
                }),
                // Its host tells it the notices already.
                listening: Arc::new(AtomicBool::new(true)),
            },
        };
        let setup = super::workers::ServiceSetup {
            script: start.script.clone(),
            source: start.source.clone(),
            address_space: space_of(&start.script),
            partition: start.partition.clone(),
            client: self.shared.client.clone(),
            storage: Some((
                self.shared.areas.clone(),
                AreaKey {
                    origin,
                    partition: start.partition.clone(),
                },
            )),
            mailbox,
            me: me.clone(),
            stop,
            service,
            evaluated,
        };
        let spawned = thread::Builder::new()
            .name(format!("service worker {}", start.version))
            .stack_size(SERVICE_WORKER_STACK)
            .spawn(move || super::workers::run_service(setup));
        match spawned {
            Ok(_) => Some(me),
            Err(_) => {
                lock(&self.shared.running).remove(&start.version);
                None
            }
        }
    }

    /// Gives version `version` the extendable event `kind`, starting it
    /// again from its scripts if it stopped.
    fn dispatch(&self, version: u64, kind: &str, done: Done<bool>) {
        {
            let mut running = lock(&self.shared.running);
            if let Some(r) = running.get_mut(&version) {
                r.used = Instant::now();
                r.to.send(WorkerMessage::Extendable(kind.to_owned(), done));
                return;
            }
        }
        let Some(started) = self.restart(version) else {
            return done(false);
        };
        let running = lock(&self.shared.running);
        match running.get(&version) {
            Some(r) => {
                r.to.send(WorkerMessage::Extendable(kind.to_owned(), done));
            }
            None => {
                drop(running);
                drop(started);
                done(false);
            }
        }
    }

    /// Starts version `version` again from what its registry keeps.
    fn restart(&self, version: u64) -> Option<Inbox> {
        let driver = self.workers()?.upgrade()?;
        let (start, stored) = driver.query_any(|registry| {
            let v = registry.version(version)?;
            let reg = registry.registration(v.registration)?;
            let (_, source) = v.resources.first()?;
            Some((
                Start {
                    version,
                    registration: reg.id,
                    key: reg.key.clone(),
                    scope: reg.scope.clone(),
                    script: v.script.clone(),
                    source: source.clone(),
                    partition: reg.partition.clone(),
                },
                v.resources[1..].to_vec(),
            ))
        })?;
        self.start(&start, stored, None)
    }
}

impl ServiceWorkerHost for InProcess {
    fn evaluate(&self, start: Start, done: Done<Option<Evaluated>>) {
        // What the thread did not tell, it did not evaluate.
        let done = Arc::new(Mutex::new(Some(done)));
        let once = done.clone();
        let started = self.start(
            &start,
            Vec::new(),
            Some(Box::new(move |result| {
                if let Some(done) = lock(&once).take() {
                    done(result);
                }
            })),
        );
        if started.is_none()
            && let Some(done) = lock(&done).take()
        {
            done(None);
        }
    }

    fn install(&self, version: u64, done: Done<bool>) {
        self.dispatch(version, "install", done);
    }

    fn activate(&self, version: u64, done: Done<()>) {
        self.dispatch(version, "activate", Box::new(move |_| done(())));
    }

    fn terminate(&self, version: u64) {
        if let Some(gone) = lock(&self.shared.running).remove(&version) {
            gone.stop.store(true, Ordering::SeqCst);
            gone.to.send(WorkerMessage::Stop);
        }
    }

    fn notice(&self, key: &str, notice: &Notice, view: &View) {
        let text = notice_json(notice, view);
        for r in lock(&self.shared.running).values() {
            if r.key == key {
                r.to.send(WorkerMessage::ServiceWorker(text.clone()));
            }
        }
    }
}
