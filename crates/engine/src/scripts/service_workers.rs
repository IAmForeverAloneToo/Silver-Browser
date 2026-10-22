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
    Done, Evaluated, Notice, Responded, ServiceWorkerHost, Start, View, WeakServiceWorkers,
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
    /// The fetch events not yet answered, by the same numbers, each with
    /// its request's URL.
    fetches: HashMap<u32, (Done<Responded>, Option<Url>)>,
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
    upstream: Arc<dyn Upstream>,
    /// What its registration's methods ask as.
    client: Client,
    /// Told when its thread ends.
    ended: Option<Box<dyn FnOnce() + Send>>,
}

/// Where a running version's own requests go: its registry's driver, in
/// this process or the browser's.
pub(crate) trait Upstream: Send + Sync {
    /// `skipWaiting()` of `version`.
    fn skip_waiting(&self, version: u64);
    /// A request of its registration's methods, as `client`'s.
    fn serve(&self, client: &Client, request: &str) -> String;
}

/// The registry's driver in this process.
struct OneProcess(WeakServiceWorkers);

impl Upstream for OneProcess {
    fn skip_waiting(&self, version: u64) {
        if let Some(workers) = self.0.upgrade() {
            workers.skip_waiting(version);
        }
    }

    fn serve(&self, client: &Client, request: &str) -> String {
        match self.0.upgrade() {
            Some(workers) => crate::service_worker::page::serve(&workers, client, request),
            None => crate::storage::NO_SERVICE_WORKERS.to_owned(),
        }
    }
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

    /// The task of the extendable event `kind`, a message's with `data`,
    /// whose end `done` hears.
    pub(crate) fn event(&mut self, kind: &str, data: Option<&str>, done: Done<bool>) -> String {
        self.next_event += 1;
        let id = self.next_event;
        self.events.insert(id, (Instant::now(), done));
        let data = data
            .and_then(|data| Json::parse(data).ok())
            .unwrap_or(Json::Null);
        Json::object([
            ("type", Json::from("event")),
            ("kind", Json::from(kind)),
            ("id", Json::from(id)),
            ("data", data),
        ])
        .write()
    }

    /// The task of a `FetchEvent` of `data` (M39 step 5), whose answer
    /// `done` hears; it keeps its version as any extendable event does.
    pub(crate) fn fetch_event(&mut self, data: &str, done: Done<Responded>) -> String {
        self.next_event += 1;
        let id = self.next_event;
        self.events.insert(id, (Instant::now(), Box::new(|_| {})));
        let data = Json::parse(data).unwrap_or(Json::Null);
        let url = data
            .get("request")
            .and_then(|request| request.get("url"))
            .and_then(Json::as_str)
            .and_then(|url| Url::parse(url).ok());
        self.fetches.insert(id, (done, url));
        Json::object([
            ("type", Json::from("event")),
            ("kind", Json::from("fetch")),
            ("id", Json::from(id)),
            ("data", data),
        ])
        .write()
    }

    /// A `FetchEvent`'s answer: `{id, answer}`, the answer `network`,
    /// `error`, or `response` with its head, its body `body`.
    pub(crate) fn respond(&mut self, head: &str, body: Vec<u8>) {
        let Ok(head) = Json::parse(head) else {
            return;
        };
        let Some((done, url)) = head
            .get("id")
            .and_then(Json::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .and_then(|id| self.fetches.remove(&id))
        else {
            return;
        };
        let responded = match head.get("answer").and_then(Json::as_str) {
            Some("response") => match head
                .get("response")
                .and_then(|response| crate::service_worker::fetch::given(response, body))
            {
                // A response made by script has its request's URL.
                Some(mut response) => {
                    if response.url_list.is_empty() {
                        response.url_list.extend(url);
                    }
                    Responded::Given(Box::new(response))
                }
                None => Responded::Failed,
            },
            Some("error") => Responded::Failed,
            _ => Responded::Network,
        };
        done(responded);
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
                self.upstream.skip_waiting(self.version);
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
            _ => self.upstream.serve(&self.client, request),
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
        if let Some(ended) = self.ended.take() {
            ended();
        }
    }

    /// Fails its events: it stops.
    fn fail_events(&mut self) {
        for (_, (_, done)) in self.events.drain() {
            done(false);
        }
        // A fetch it never answered goes to the network.
        for (_, (done, _)) in self.fetches.drain() {
            done(Responded::Network);
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

/// A version to start: what it is, its scripts, its own view, where its
/// requests go, and who hears what its evaluation did and when it ends.
pub(crate) struct Launch {
    pub(crate) start: Start,
    /// Its version's scripts kept, which it imports from: none when it
    /// is evaluated.
    pub(crate) stored: Vec<(Url, Arc<[u8]>)>,
    /// `{view, worker}`, what `registration` and `serviceWorker` are.
    pub(crate) own: String,
    pub(crate) client: Arc<dyn Fetcher>,
    pub(crate) areas: Arc<dyn Areas>,
    pub(crate) upstream: Arc<dyn Upstream>,
    pub(crate) evaluated: Option<Done<Option<Evaluated>>>,
    pub(crate) ended: Option<Box<dyn FnOnce() + Send>>,
}

/// The versions running in this process, each on a thread of its own.
#[derive(Clone, Default)]
pub(crate) struct Versions {
    running: RunningMap,
}

impl Versions {
    /// Whether version `version` runs.
    #[cfg(test)]
    pub(crate) fn runs(&self, version: u64) -> bool {
        lock(&self.running).contains_key(&version)
    }

    /// Starts `launch`'s version, stopping the one given an event longest
    /// ago past [`MAX_RUNNING_SERVICE_WORKERS`]; false if it could not.
    pub(crate) fn launch(&self, launch: Launch) -> bool {
        let Launch {
            start,
            stored,
            own,
            client,
            areas,
            upstream,
            evaluated,
            ended,
        } = launch;
        let (me, mailbox) = inbox(None);
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut running = lock(&self.running);
            if let Some(gone) = running.remove(&start.version) {
                gone.stop.store(true, Ordering::SeqCst);
                gone.to.send(WorkerMessage::Stop);
            }
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
            running: self.running.clone(),
            own,
            events: HashMap::new(),
            fetches: HashMap::new(),
            next_event: 0,
            idle_since: Instant::now(),
            listened: None,
            recording: evaluated.is_some(),
            imports: Vec::new(),
            stored,
            upstream,
            client: Client {
                key: start.key.clone(),
                partition: start.partition.clone(),
                origin: origin.clone(),
                base: start.script.clone(),
                url: start.script.clone(),
                page: 0,
                id: String::new(),
                nested: false,
                version: Some(start.version),
                alive: Arc::new(|| true),
                tell: Arc::new(move |text| {
                    tell.send(WorkerMessage::ServiceWorker(text));
                    tell.alive()
                }),
                // Its host tells it the notices already.
                listening: Arc::new(AtomicBool::new(true)),
            },
            ended,
        };
        let setup = super::workers::ServiceSetup {
            script: start.script.clone(),
            source: start.source.clone(),
            address_space: space_of(&start.script),
            partition: start.partition.clone(),
            client,
            storage: Some((
                areas,
                AreaKey {
                    origin,
                    partition: start.partition.clone(),
                },
            )),
            mailbox,
            me,
            stop,
            service,
            evaluated,
        };
        let spawned = thread::Builder::new()
            .name(format!("service worker {}", start.version))
            .stack_size(SERVICE_WORKER_STACK)
            .spawn(move || super::workers::run_service(setup));
        if spawned.is_err() {
            lock(&self.running).remove(&start.version);
            return false;
        }
        true
    }

    /// Gives version `version` the extendable event `kind`; `done` back
    /// when it does not run.
    pub(crate) fn give(
        &self,
        version: u64,
        kind: &str,
        data: Option<String>,
        done: Done<bool>,
    ) -> Result<(), Done<bool>> {
        let mut running = lock(&self.running);
        match running.get_mut(&version) {
            Some(r) => {
                r.used = Instant::now();
                r.to.send(WorkerMessage::Extendable(kind.to_owned(), data, done));
                Ok(())
            }
            None => Err(done),
        }
    }

    /// Gives version `version` the `FetchEvent` of `data`; `done` back
    /// when it does not run.
    pub(crate) fn give_fetch(
        &self,
        version: u64,
        data: String,
        done: Done<Responded>,
    ) -> Result<(), Done<Responded>> {
        let mut running = lock(&self.running);
        match running.get_mut(&version) {
            Some(r) => {
                r.used = Instant::now();
                r.to.send(WorkerMessage::Fetch(data, done));
                Ok(())
            }
            None => Err(done),
        }
    }

    /// Stops version `version`, if it runs.
    pub(crate) fn terminate(&self, version: u64) {
        if let Some(gone) = lock(&self.running).remove(&version) {
            gone.stop.store(true, Ordering::SeqCst);
            gone.to.send(WorkerMessage::Stop);
        }
    }

    /// Version `version`, if it runs, hears the task `text`.
    pub(crate) fn tell_version(&self, version: u64, text: &str) {
        if let Some(r) = lock(&self.running).get(&version) {
            r.to.send(WorkerMessage::ServiceWorker(text.to_owned()));
        }
    }

    /// The running versions of storage key `key` hear the task `text`.
    pub(crate) fn tell(&self, key: &str, text: &str) {
        for r in lock(&self.running).values() {
            if r.key == key {
                r.to.send(WorkerMessage::ServiceWorker(text.to_owned()));
            }
        }
    }
}

/// A version's own view: its registration's and itself, as the bindings
/// read them.
pub(crate) fn own_json(
    registry: &crate::service_worker::Registry,
    start: &Start,
) -> Option<String> {
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
}

/// A version's scripts, by their URLs.
pub(crate) type Scripts = Vec<(Url, Arc<[u8]>)>;

/// What starts version `version` again: its start and its scripts kept.
pub(crate) fn restart_of(
    registry: &crate::service_worker::Registry,
    version: u64,
) -> Option<(Start, Scripts)> {
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
}

/// The versions of a context's service workers, run on threads of this
/// process.
#[derive(Clone)]
pub(crate) struct InProcess {
    shared: Arc<Shared>,
}

struct Shared {
    versions: Versions,
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
                versions: Versions::default(),
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
        self.shared.versions.runs(version)
    }

    /// Starts `start`'s version: evaluated for its job when `evaluated`
    /// is given, else started again from its scripts kept (`stored`).
    fn start(
        &self,
        start: Start,
        stored: Vec<(Url, Arc<[u8]>)>,
        evaluated: Option<Done<Option<Evaluated>>>,
    ) -> bool {
        let Some(workers) = lock(&self.shared.workers).clone() else {
            return false;
        };
        let Some(own) = workers
            .upgrade()
            .and_then(|driver| driver.query(&start.key, |registry| own_json(registry, &start)))
        else {
            return false;
        };
        self.shared.versions.launch(Launch {
            start,
            stored,
            own,
            client: self.shared.client.clone(),
            areas: self.shared.areas.clone(),
            upstream: Arc::new(OneProcess(workers)),
            evaluated,
            ended: None,
        })
    }

    /// Gives version `version` the extendable event `kind`, starting it
    /// again from its scripts if it stopped.
    fn dispatch(&self, version: u64, kind: &str, data: Option<String>, done: Done<bool>) {
        let Err(done) = self.shared.versions.give(version, kind, data.clone(), done) else {
            return;
        };
        let restart = lock(&self.shared.workers)
            .clone()
            .and_then(|workers| workers.upgrade())
            .and_then(|driver| driver.query_any(|registry| restart_of(registry, version)));
        let Some((start, stored)) = restart else {
            return done(false);
        };
        if !self.start(start, stored, None) {
            return done(false);
        }
        if let Err(done) = self.shared.versions.give(version, kind, data, done) {
            done(false);
        }
    }
}

impl ServiceWorkerHost for InProcess {
    fn evaluate(&self, start: Start, done: Done<Option<Evaluated>>) {
        // What the thread did not tell, it did not evaluate.
        let done = Arc::new(Mutex::new(Some(done)));
        let once = done.clone();
        let started = self.start(
            start,
            Vec::new(),
            Some(Box::new(move |result| {
                if let Some(done) = lock(&once).take() {
                    done(result);
                }
            })),
        );
        if !started && let Some(done) = lock(&done).take() {
            done(None);
        }
    }

    fn install(&self, version: u64, done: Done<bool>) {
        self.dispatch(version, "install", None, done);
    }

    fn activate(&self, version: u64, done: Done<()>) {
        self.dispatch(version, "activate", None, Box::new(move |_| done(())));
    }

    fn message(&self, version: u64, data: String, done: Done<bool>) {
        self.dispatch(version, "message", Some(data), done);
    }

    fn fetch(&self, version: u64, data: String, done: Done<Responded>) {
        let Err(done) = self.shared.versions.give_fetch(version, data.clone(), done) else {
            return;
        };
        let restart = lock(&self.shared.workers)
            .clone()
            .and_then(|workers| workers.upgrade())
            .and_then(|driver| driver.query_any(|registry| restart_of(registry, version)));
        let Some((start, stored)) = restart else {
            return done(Responded::Network);
        };
        if !self.start(start, stored, None) {
            return done(Responded::Network);
        }
        if let Err(done) = self.shared.versions.give_fetch(version, data, done) {
            done(Responded::Network);
        }
    }

    fn terminate(&self, version: u64) {
        self.shared.versions.terminate(version);
    }

    fn notice(&self, key: &str, notice: &Notice, view: &View) {
        self.shared.versions.tell(key, &notice_json(notice, view));
    }
}
