//! The registry of service workers (M39, docs/design/service-workers.md
//! "The registry"): registrations by storage key and scope, their
//! versions, and the jobs that change them, run as the standard writes
//! them (Service Workers, "Algorithms": Schedule Job, Register, Update,
//! Install, Activate, Unregister). What a job needs done outside, a
//! script fetched, a version run, an event dispatched, the registry
//! hands out as an [`Effect`] and is told how it went; it does nothing
//! by itself, so the context's driver, the tests and the fuzzer drive
//! it alike.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use net::Url;

mod clients;
mod driver;
#[cfg(test)]
mod driver_tests;
mod fuzz;
pub(crate) mod page;
mod record;
pub mod rules;
#[cfg(test)]
mod tests;

pub(crate) use clients::new_client_id;
pub use driver::{
    Done, Listener, ServiceWorkerHost, ServiceWorkers, Settled, Start, WeakServiceWorkers,
};
pub use fuzz::fuzz_registry;

/// The most registrations a storage key has.
pub const MAX_REGISTRATIONS: usize = 100;
/// The most scripts a version imports.
pub const MAX_IMPORTED_SCRIPTS: usize = 64;
/// The most a version's scripts hold together, in bytes.
pub const MAX_VERSION_BYTES: usize = 64 << 20;
/// How long after its last check a registration is stale, in seconds:
/// its scripts are then fetched past the HTTP cache.
pub const STALE_AFTER: u64 = 24 * 60 * 60;

/// A registration's `updateViaCache`: which of its scripts an update
/// may take from the HTTP cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UpdateViaCache {
    /// The imported scripts, not the script itself.
    #[default]
    Imports,
    All,
    None,
}

impl UpdateViaCache {
    pub fn parse(text: &str) -> Option<UpdateViaCache> {
        match text {
            "imports" => Some(UpdateViaCache::Imports),
            "all" => Some(UpdateViaCache::All),
            "none" => Some(UpdateViaCache::None),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            UpdateViaCache::Imports => "imports",
            UpdateViaCache::All => "all",
            UpdateViaCache::None => "none",
        }
    }
}

/// A version's state, as `ServiceWorker.state` says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Parsed,
    Installing,
    Installed,
    Activating,
    Activated,
    Redundant,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Parsed => "parsed",
            State::Installing => "installing",
            State::Installed => "installed",
            State::Activating => "activating",
            State::Activated => "activated",
            State::Redundant => "redundant",
        }
    }
}

/// Where in its registration a version is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Installing,
    Waiting,
    Active,
}

impl Slot {
    /// Its name, as the registration's attribute is named.
    pub fn name(self) -> &'static str {
        match self {
            Slot::Installing => "installing",
            Slot::Waiting => "waiting",
            Slot::Active => "active",
        }
    }
}

/// A service worker: one version of a registration's script.
#[derive(Debug)]
pub struct Version {
    pub id: u64,
    pub registration: u64,
    pub script: Url,
    /// Its scripts: its own first, then those it imported, in the order
    /// it imported them.
    pub resources: Vec<(Url, Arc<[u8]>)>,
    /// The events it listened for when its script ran.
    pub events: Vec<String>,
    pub state: State,
    pub skip_waiting: bool,
}

impl Version {
    /// Whether its script listened for `event` (Should Skip Event, the
    /// other way round).
    pub fn handles(&self, event: &str) -> bool {
        self.events.iter().any(|e| e == event)
    }
}

/// A registration: a scope of a storage key's URLs and its versions.
#[derive(Debug)]
pub struct Registration {
    pub id: u64,
    /// Its number in the profile: its id, unless another had that when
    /// it was read.
    pub row: u64,
    pub key: String,
    pub scope: Url,
    pub update_via_cache: UpdateViaCache,
    pub installing: Option<u64>,
    pub waiting: Option<u64>,
    pub active: Option<u64>,
    /// When its script was last fetched from the network, in seconds
    /// since the epoch.
    pub last_check: Option<u64>,
    /// Whether it is in the map: not unregistered, nor failed at its
    /// first install.
    pub listed: bool,
    pub partition: Option<net::Partition>,
}

impl Registration {
    /// Get Newest Worker.
    pub fn newest(&self) -> Option<u64> {
        self.installing.or(self.waiting).or(self.active)
    }

    fn slot(&mut self, slot: Slot) -> &mut Option<u64> {
        match slot {
            Slot::Installing => &mut self.installing,
            Slot::Waiting => &mut self.waiting,
            Slot::Active => &mut self.active,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Register,
    Update,
    Unregister,
}

/// A job, as Schedule Job takes it: the client's checks already passed
/// (`rules`).
#[derive(Clone, Debug)]
pub struct Job {
    pub kind: JobKind,
    pub key: String,
    pub scope: Url,
    /// The script; the scope's for an unregister job, which has none.
    pub script: Url,
    pub update_via_cache: UpdateViaCache,
    /// Soft Update's: the script fetched past the HTTP cache.
    pub force_bypass: bool,
    /// Where the storage key's documents are (M33.2), which its scripts
    /// are fetched in.
    pub partition: Option<net::Partition>,
}

impl Job {
    /// Whether `other` would do what it does (the standard's "equivalent").
    fn equivalent(&self, other: &Job) -> bool {
        self.kind == other.kind
            && self.scope == other.scope
            && (self.kind == JobKind::Unregister
                || (self.script == other.script && self.update_via_cache == other.update_via_cache))
    }

    /// What its errors begin with, as the reference browser words them.
    fn prefix(&self) -> String {
        let verb = match self.kind {
            JobKind::Update => "update",
            _ => "register",
        };
        format!(
            "Failed to {verb} a ServiceWorker for scope ('{}') with script ('{}'): ",
            self.scope, self.script
        )
    }
}

/// Why a job failed: a `DOMException`'s name, or `TypeError`, and its
/// message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobError {
    pub name: &'static str,
    pub message: String,
}

/// How a job's promise settles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A register or update job's: the registration.
    Registration(u64),
    /// An unregister job's: whether there was one.
    Unregistered(bool),
    Failed(JobError),
}

/// A script's response, as the driver fetched it.
#[derive(Clone, Debug)]
pub struct Fetched {
    pub status: u16,
    /// Its MIME type's essence, if it has one.
    pub mime: Option<String>,
    /// Whether that is a JavaScript MIME type.
    pub javascript: bool,
    /// Its `Service-Worker-Allowed` field, if any.
    pub allowed: Option<String>,
    pub body: Arc<[u8]>,
    /// Whether it came from the HTTP cache rather than the network.
    pub from_cache: bool,
}

/// Why a script's fetch gave no response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchFailure {
    Network,
    /// It was redirected, which a service worker's script may not be.
    Redirect,
}

/// What a version's script did when it ran.
#[derive(Clone, Debug, Default)]
pub struct Evaluated {
    /// The events it listened for.
    pub events: Vec<String>,
    /// The scripts it imported, in order.
    pub imports: Vec<(Url, Arc<[u8]>)>,
}

/// What the registry needs done, or has to say.
#[derive(Debug)]
pub enum Effect {
    /// Fetch job `job`'s script (`main`), or a script its registration's
    /// newest version imported, to compare it.
    Fetch {
        job: u64,
        url: Box<Url>,
        main: bool,
        bypass_cache: bool,
        partition: Option<net::Partition>,
    },
    /// Run version `version`'s script, which hears no event, and say how
    /// it went ([`Registry::evaluated`]).
    Evaluate { job: u64, version: u64 },
    /// Dispatch `install` at the version, and say how it went
    /// ([`Registry::installed`]).
    Install { version: u64 },
    /// Dispatch `activate` at the version, and say when it is done
    /// ([`Registry::activated`]).
    Activate { version: u64 },
    /// The version will not run again: stop it if it runs.
    Terminate { version: u64 },
    /// The promise of `ticket` settles.
    /// A registration's, with its view at that moment.
    Settle {
        ticket: u64,
        outcome: Outcome,
        view: Option<View>,
    },
    /// The clients of storage key `key` hear `notice`.
    /// With the registration's view at that moment.
    Notice {
        key: String,
        notice: Notice,
        view: View,
    },
}

/// What the clients of a storage key hear, in order: the tasks the
/// standard queues at their `ServiceWorkerRegistration` and
/// `ServiceWorker` objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// Update Registration State: the slot holds the version now.
    Slot {
        registration: u64,
        slot: Slot,
        version: Option<u64>,
    },
    /// Update Worker State.
    State { version: u64, state: State },
    /// `updatefound` at the registration.
    UpdateFound { registration: u64 },
}

/// A version as its clients see it: its number, script and state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub id: u64,
    pub script: Url,
    pub state: State,
}

/// A registration as its clients see it at a moment: its scope, its
/// `updateViaCache` and its slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub id: u64,
    pub scope: Url,
    pub update_via_cache: UpdateViaCache,
    pub installing: Option<Shown>,
    pub waiting: Option<Shown>,
    pub active: Option<Shown>,
}

/// Where a running job is.
enum Stage {
    Queued,
    /// Its script is being fetched.
    Fetching {
        registration: u64,
    },
    /// The newest version's imported scripts are being fetched, `at` the
    /// one asked for, the script itself unchanged.
    Imports {
        registration: u64,
        script: Arc<[u8]>,
        at: usize,
    },
    Evaluating {
        registration: u64,
        version: u64,
    },
    Installing {
        registration: u64,
        version: u64,
    },
}

struct Running {
    job: Job,
    /// Its promise's ticket and those of the jobs equivalent to it.
    tickets: Vec<u64>,
    settled: bool,
    stage: Stage,
}

/// A registration as the profile keeps it: its active version.
pub struct Written {
    pub key: String,
    /// Its row.
    pub id: u64,
    /// Its record, or none for one gone.
    pub record: Option<Vec<u8>>,
}

/// A registration as DevTools lists it: its number and scope, and its
/// versions' slots, scripts and states.
pub type Listed = (u64, String, Vec<(Slot, String, State)>);

/// The registrations and versions of a context, and their jobs.
#[derive(Default)]
pub struct Registry {
    registrations: HashMap<u64, Registration>,
    versions: HashMap<u64, Version>,
    /// The map: a storage key's registrations by scope.
    map: HashMap<(String, String), u64>,
    /// The keys whose registrations were read from the profile.
    loaded: BTreeSet<String>,
    jobs: HashMap<u64, Running>,
    /// Each scope's job queue, by its key and scope.
    queues: HashMap<(String, String), VecDeque<u64>>,
    /// The clients each registration controls, which keep a waiting
    /// version waiting and an unregistered registration alive.
    uses: HashMap<u64, usize>,
    effects: Vec<Effect>,
    next: u64,
    /// The clock, in seconds since the epoch.
    now: u64,
    writing: bool,
    /// The registrations changed since the profile was last written:
    /// by their key and row, each its number.
    dirty: BTreeMap<(String, u64), u64>,
}

impl Registry {
    /// Sets the clock: seconds since the epoch.
    pub fn set_now(&mut self, now: u64) {
        self.now = now;
    }

    /// Keeps what changes for the profile from now on.
    pub fn set_writing(&mut self, writing: bool) {
        self.writing = writing;
    }

    fn number(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    pub fn registration(&self, id: u64) -> Option<&Registration> {
        self.registrations.get(&id)
    }

    pub fn version(&self, id: u64) -> Option<&Version> {
        self.versions.get(&id)
    }

    /// Get Registration: the storage key's registration of `scope`.
    pub fn get(&self, key: &str, scope: &str) -> Option<u64> {
        self.map.get(&(key.to_owned(), scope.to_owned())).copied()
    }

    /// The storage key's registrations, in their scopes' order, as the
    /// reference browser lists them.
    pub fn registrations(&self, key: &str) -> Vec<u64> {
        let mut scopes: Vec<(&str, u64)> = self
            .map
            .iter()
            .filter(|((k, _), _)| k == key)
            .map(|((_, scope), id)| (scope.as_str(), *id))
            .collect();
        scopes.sort_unstable();
        scopes.into_iter().map(|(_, id)| id).collect()
    }

    /// Match Service Worker Registration: the storage key's registration
    /// whose scope is the longest prefix of `url`.
    pub fn matching(&self, key: &str, url: &Url) -> Option<u64> {
        let url = url.without_fragment();
        self.map
            .iter()
            .filter(|((k, scope), _)| k == key && url.starts_with(scope.as_str()))
            .max_by_key(|((_, scope), _)| scope.len())
            .map(|(_, id)| *id)
    }

    /// The effects to perform, in order.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Schedule Job: the ticket its promise settles by.
    pub fn schedule(&mut self, job: Job) -> u64 {
        let ticket = self.number();
        let queue_key = (job.key.clone(), job.scope.as_str().to_owned());
        let last = self.queues.get(&queue_key).and_then(VecDeque::back);
        if let Some(last) = last.and_then(|id| self.jobs.get_mut(id))
            && !last.settled
            && last.job.equivalent(&job)
        {
            last.tickets.push(ticket);
            return ticket;
        }
        let id = self.number();
        let queue = self.queues.entry(queue_key).or_default();
        queue.push_back(id);
        let first = queue.len() == 1;
        self.jobs.insert(
            id,
            Running {
                job,
                tickets: vec![ticket],
                settled: false,
                stage: Stage::Queued,
            },
        );
        if first {
            self.start(id);
        }
        ticket
    }

    /// Soft Update: an update of the registration's newest version, past
    /// the HTTP cache; nothing for one with none.
    pub fn soft_update(&mut self, registration: u64) {
        let Some(reg) = self.registrations.get(&registration) else {
            return;
        };
        let Some(newest) = reg.newest().and_then(|v| self.versions.get(&v)) else {
            return;
        };
        if !reg.listed {
            return;
        }
        let job = Job {
            kind: JobKind::Update,
            key: reg.key.clone(),
            scope: reg.scope.clone(),
            script: newest.script.clone(),
            update_via_cache: reg.update_via_cache,
            force_bypass: true,
            partition: reg.partition.clone(),
        };
        self.schedule(job);
    }

    fn start(&mut self, id: u64) {
        let Some(running) = self.jobs.get(&id) else {
            return;
        };
        match running.job.kind {
            JobKind::Register => self.register(id),
            JobKind::Update => self.update(id),
            JobKind::Unregister => self.unregister(id),
        }
    }

    /// Settles job `id`'s promise and those equivalent to it, once.
    fn settle(&mut self, id: u64, outcome: Outcome) {
        let Some(running) = self.jobs.get_mut(&id) else {
            return;
        };
        if running.settled {
            return;
        }
        running.settled = true;
        let tickets = running.tickets.clone();
        let view = match outcome {
            Outcome::Registration(registration) => self.view(registration),
            _ => None,
        };
        for ticket in tickets {
            self.effects.push(Effect::Settle {
                ticket,
                outcome: outcome.clone(),
                view: view.clone(),
            });
        }
    }

    /// A version as its clients see it now.
    fn shown(&self, version: Option<u64>) -> Option<Shown> {
        let v = self.versions.get(&version?)?;
        Some(Shown {
            id: v.id,
            script: v.script.clone(),
            state: v.state,
        })
    }

    /// The registration as its clients see it now.
    pub fn view(&self, registration: u64) -> Option<View> {
        let reg = self.registrations.get(&registration)?;
        Some(View {
            id: reg.id,
            scope: reg.scope.clone(),
            update_via_cache: reg.update_via_cache,
            installing: self.shown(reg.installing),
            waiting: self.shown(reg.waiting),
            active: self.shown(reg.active),
        })
    }

    /// The registration's clients hear `notice`, with its view now.
    fn notify(&mut self, registration: u64, notice: Notice) {
        let (Some(reg), Some(view)) = (
            self.registrations.get(&registration),
            self.view(registration),
        ) else {
            return;
        };
        let key = reg.key.clone();
        self.effects.push(Effect::Notice { key, notice, view });
    }

    fn reject(&mut self, id: u64, name: &'static str, words: &str) {
        let Some(running) = self.jobs.get(&id) else {
            return;
        };
        let message = format!("{}{words}", running.job.prefix());
        self.settle(id, Outcome::Failed(JobError { name, message }));
    }

    /// Finish Job: the next of its queue starts.
    fn finish(&mut self, id: u64) {
        let Some(running) = self.jobs.remove(&id) else {
            return;
        };
        let queue_key = (
            running.job.key.clone(),
            running.job.scope.as_str().to_owned(),
        );
        let Some(queue) = self.queues.get_mut(&queue_key) else {
            return;
        };
        queue.retain(|job| *job != id);
        match queue.front().copied() {
            Some(next) => self.start(next),
            None => {
                self.queues.remove(&queue_key);
            }
        }
    }

    fn register(&mut self, id: u64) {
        let job = self.jobs[&id].job.clone();
        let existing = self.get(&job.key, job.scope.as_str());
        if let Some(reg) = existing.and_then(|r| self.registrations.get(&r)) {
            let same = reg
                .newest()
                .and_then(|v| self.versions.get(&v))
                .is_some_and(|newest| newest.script == job.script)
                && reg.update_via_cache == job.update_via_cache;
            if same {
                let reg = reg.id;
                self.settle(id, Outcome::Registration(reg));
                self.finish(id);
                return;
            }
        }
        match existing {
            Some(reg) => {
                let reg = self.registrations.get_mut(&reg).expect("listed");
                reg.update_via_cache = job.update_via_cache;
                self.dirty.insert((reg.key.clone(), reg.row), reg.id);
            }
            None => {
                if self.registrations(&job.key).len() >= MAX_REGISTRATIONS {
                    self.reject(
                        id,
                        "QuotaExceededError",
                        "The origin has too many registrations.",
                    );
                    self.finish(id);
                    return;
                }
                let reg = self.number();
                self.registrations.insert(
                    reg,
                    Registration {
                        id: reg,
                        row: reg,
                        key: job.key.clone(),
                        scope: job.scope.clone(),
                        update_via_cache: job.update_via_cache,
                        installing: None,
                        waiting: None,
                        active: None,
                        last_check: None,
                        listed: true,
                        partition: job.partition.clone(),
                    },
                );
                self.map
                    .insert((job.key.clone(), job.scope.as_str().to_owned()), reg);
            }
        }
        self.update(id);
    }

    fn update(&mut self, id: u64) {
        let job = self.jobs[&id].job.clone();
        let Some(reg) = self
            .get(&job.key, job.scope.as_str())
            .and_then(|r| self.registrations.get(&r))
        else {
            self.reject(id, "TypeError", "The registration is not found.");
            self.finish(id);
            return;
        };
        let newest = reg.newest().and_then(|v| self.versions.get(&v));
        if job.kind == JobKind::Update && newest.is_some_and(|n| n.script != job.script) {
            self.reject(id, "TypeError", "The script URL changed.");
            self.finish(id);
            return;
        }
        let stale = reg
            .last_check
            .is_some_and(|at| self.now.saturating_sub(at) > STALE_AFTER);
        let bypass_cache = reg.update_via_cache != UpdateViaCache::All
            || job.force_bypass
            || (newest.is_some() && stale);
        let registration = reg.id;
        self.jobs.get_mut(&id).expect("running").stage = Stage::Fetching { registration };
        self.effects.push(Effect::Fetch {
            job: id,
            url: Box::new(job.script),
            main: true,
            bypass_cache,
            partition: job.partition,
        });
    }

    /// Job `job`'s fetch, of its script or of an imported one, went as
    /// `result`.
    pub fn fetched(&mut self, job: u64, result: Result<Fetched, FetchFailure>) {
        let Some(running) = self.jobs.get(&job) else {
            return;
        };
        match running.stage {
            Stage::Fetching { registration } => self.script_fetched(job, registration, result),
            Stage::Imports {
                registration, at, ..
            } => self.import_fetched(job, registration, at, result),
            _ => {}
        }
    }

    /// The job failed before its version installed: the registration
    /// leaves the map if it has no version.
    fn failed(&mut self, id: u64, registration: u64, name: &'static str, words: &str) {
        self.reject(id, name, words);
        if self
            .registrations
            .get(&registration)
            .is_some_and(|reg| reg.newest().is_none())
        {
            self.unlist(registration);
            self.clear(registration);
        }
        self.finish(id);
    }

    fn script_fetched(
        &mut self,
        id: u64,
        registration: u64,
        result: Result<Fetched, FetchFailure>,
    ) {
        let job = self.jobs[&id].job.clone();
        let response = match result {
            Ok(response) => response,
            Err(FetchFailure::Redirect) => {
                return self.failed(
                    id,
                    registration,
                    "SecurityError",
                    "The script resource is behind a redirect, which is disallowed.",
                );
            }
            Err(FetchFailure::Network) => {
                return self.failed(
                    id,
                    registration,
                    "TypeError",
                    "An unknown error occurred when fetching the script.",
                );
            }
        };
        if !response.from_cache
            && let Some(reg) = self.registrations.get_mut(&registration)
        {
            reg.last_check = Some(self.now);
            self.dirty.insert((reg.key.clone(), reg.row), reg.id);
        }
        if let Err((name, words)) = rules::check_script(&job.script, &job.scope, &response) {
            return self.failed(id, registration, name, &words);
        }
        let newest = self
            .registrations
            .get(&registration)
            .and_then(Registration::newest)
            .and_then(|v| self.versions.get(&v));
        if let Some(newest) = newest
            && newest
                .resources
                .first()
                .is_some_and(|(_, b)| *b == response.body)
        {
            if newest.resources.len() > 1 {
                self.jobs.get_mut(&id).expect("running").stage = Stage::Imports {
                    registration,
                    script: response.body,
                    at: 1,
                };
                return self.fetch_import(id, registration, 1);
            }
            return self.unchanged(id, registration);
        }
        self.new_version(id, registration, response.body);
    }

    /// Asks for the newest version's imported script `at`, past the cache
    /// as the registration says.
    fn fetch_import(&mut self, id: u64, registration: u64, at: usize) {
        let Some(reg) = self.registrations.get(&registration) else {
            return;
        };
        let stale = reg
            .last_check
            .is_some_and(|t| self.now.saturating_sub(t) > STALE_AFTER);
        let bypass_cache = reg.update_via_cache == UpdateViaCache::None || stale;
        let url = reg
            .newest()
            .and_then(|v| self.versions.get(&v))
            .and_then(|v| v.resources.get(at))
            .map(|(url, _)| url.clone());
        let partition = reg.partition.clone();
        match url {
            Some(url) => self.effects.push(Effect::Fetch {
                job: id,
                url: Box::new(url),
                main: false,
                bypass_cache,
                partition,
            }),
            None => self.unchanged(id, registration),
        }
    }

    fn import_fetched(
        &mut self,
        id: u64,
        registration: u64,
        at: usize,
        result: Result<Fetched, FetchFailure>,
    ) {
        let stored = self
            .registrations
            .get(&registration)
            .and_then(Registration::newest)
            .and_then(|v| self.versions.get(&v))
            .and_then(|v| v.resources.get(at))
            .map(|(_, body)| body.clone());
        // A bad import's response counts as unchanged (the standard's
        // "bad import script response").
        let changed = match &result {
            Ok(response) if response.status / 100 == 2 && response.javascript => {
                stored.is_none_or(|stored| stored != response.body)
            }
            _ => false,
        };
        if let Ok(response) = &result
            && !response.from_cache
            && let Some(reg) = self.registrations.get_mut(&registration)
        {
            reg.last_check = Some(self.now);
            self.dirty.insert((reg.key.clone(), reg.row), reg.id);
        }
        let Some(Running {
            stage: Stage::Imports { script, .. },
            ..
        }) = self.jobs.get_mut(&id)
        else {
            return;
        };
        if changed {
            let script = script.clone();
            return self.new_version(id, registration, script);
        }
        if let Some(Running {
            stage: Stage::Imports { at: next, .. },
            ..
        }) = self.jobs.get_mut(&id)
        {
            *next = at + 1;
        }
        self.fetch_import(id, registration, at + 1);
    }

    /// Nothing changed: the job resolves with the registration.
    fn unchanged(&mut self, id: u64, registration: u64) {
        self.settle(id, Outcome::Registration(registration));
        self.finish(id);
    }

    fn new_version(&mut self, id: u64, registration: u64, script: Arc<[u8]>) {
        let job = self.jobs[&id].job.clone();
        let version = self.number();
        self.versions.insert(
            version,
            Version {
                id: version,
                registration,
                script: job.script.clone(),
                resources: vec![(job.script, script)],
                events: Vec::new(),
                state: State::Parsed,
                skip_waiting: false,
            },
        );
        self.jobs.get_mut(&id).expect("running").stage = Stage::Evaluating {
            registration,
            version,
        };
        self.effects.push(Effect::Evaluate { job: id, version });
    }

    /// Job `job`'s version ran its script as `result` says: none when it
    /// threw or did not run.
    pub fn evaluated(&mut self, job: u64, result: Option<Evaluated>) {
        let Some(Running {
            stage:
                Stage::Evaluating {
                    registration,
                    version,
                },
            ..
        }) = self.jobs.get(&job)
        else {
            return;
        };
        let (registration, version) = (*registration, *version);
        let result = result.filter(|evaluated| {
            let size: usize = evaluated.imports.iter().map(|(_, b)| b.len()).sum();
            evaluated.imports.len() <= MAX_IMPORTED_SCRIPTS
                && size.saturating_add(self.versions[&version].resources[0].1.len())
                    <= MAX_VERSION_BYTES
        });
        let Some(evaluated) = result else {
            self.versions.remove(&version);
            self.effects.push(Effect::Terminate { version });
            return self.failed(
                job,
                registration,
                "TypeError",
                "ServiceWorker script evaluation failed",
            );
        };
        let worker = self.versions.get_mut(&version).expect("evaluated");
        worker.events = evaluated.events;
        worker.resources.extend(evaluated.imports);
        self.install(job, registration, version);
    }

    fn install(&mut self, id: u64, registration: u64, version: u64) {
        self.set_slot(registration, Slot::Installing, Some(version));
        self.set_state(version, State::Installing);
        self.settle(id, Outcome::Registration(registration));
        self.notify(registration, Notice::UpdateFound { registration });
        self.jobs.get_mut(&id).expect("running").stage = Stage::Installing {
            registration,
            version,
        };
        if self.versions[&version].handles("install") {
            self.effects.push(Effect::Install { version });
        } else {
            self.installed(version, true);
        }
    }

    /// The version's `install` went as `ok` says: its `waitUntil`
    /// promises all fulfilled, or not.
    pub fn installed(&mut self, version: u64, ok: bool) {
        let job = self
            .jobs
            .iter()
            .find_map(|(id, running)| match running.stage {
                Stage::Installing {
                    registration,
                    version: v,
                } if v == version => Some((*id, registration)),
                _ => None,
            });
        let Some((id, registration)) = job else {
            return;
        };
        if !ok {
            self.set_state(version, State::Redundant);
            self.set_slot(registration, Slot::Installing, None);
            self.effects.push(Effect::Terminate { version });
            self.versions.remove(&version);
            if self
                .registrations
                .get(&registration)
                .is_some_and(|reg| reg.newest().is_none())
            {
                self.unlist(registration);
                self.clear(registration);
            }
            self.finish(id);
            return;
        }
        if let Some(waiting) = self.registrations[&registration].waiting {
            self.effects.push(Effect::Terminate { version: waiting });
            self.set_state(waiting, State::Redundant);
            self.versions.remove(&waiting);
        }
        self.set_slot(registration, Slot::Waiting, Some(version));
        self.set_slot(registration, Slot::Installing, None);
        self.set_state(version, State::Installed);
        self.finish(id);
        self.try_activate(registration);
    }

    /// Try Activate.
    fn try_activate(&mut self, registration: u64) {
        let Some(reg) = self.registrations.get(&registration) else {
            return;
        };
        let Some(waiting) = reg.waiting.and_then(|v| self.versions.get(&v)) else {
            return;
        };
        let active = reg.active.and_then(|v| self.versions.get(&v));
        if active.is_some_and(|a| a.state == State::Activating) {
            return;
        }
        let unused = self.uses.get(&registration).copied().unwrap_or(0) == 0;
        if active.is_none() || unused || waiting.skip_waiting {
            self.activate(registration);
        }
    }

    /// Activate.
    fn activate(&mut self, registration: u64) {
        let Some(waiting) = self
            .registrations
            .get(&registration)
            .and_then(|r| r.waiting)
        else {
            return;
        };
        if let Some(active) = self.registrations[&registration].active {
            self.effects.push(Effect::Terminate { version: active });
            self.set_state(active, State::Redundant);
            self.versions.remove(&active);
        }
        self.set_slot(registration, Slot::Active, Some(waiting));
        self.set_slot(registration, Slot::Waiting, None);
        self.set_state(waiting, State::Activating);
        if self.versions[&waiting].handles("activate") {
            self.effects.push(Effect::Activate { version: waiting });
        } else {
            self.activated(waiting);
        }
    }

    /// The version's `activate` is done, its `waitUntil` promises settled.
    pub fn activated(&mut self, version: u64) {
        let Some(worker) = self.versions.get(&version) else {
            return;
        };
        if worker.state != State::Activating {
            return;
        }
        let registration = worker.registration;
        self.set_state(version, State::Activated);
        // A version that installed while this one activated may follow.
        self.try_activate(registration);
    }

    /// `skipWaiting()` of the version.
    pub fn skip_waiting(&mut self, version: u64) {
        let Some(worker) = self.versions.get_mut(&version) else {
            return;
        };
        worker.skip_waiting = true;
        let registration = worker.registration;
        self.try_activate(registration);
    }

    fn unregister(&mut self, id: u64) {
        let job = self.jobs[&id].job.clone();
        let Some(registration) = self.get(&job.key, job.scope.as_str()) else {
            self.settle(id, Outcome::Unregistered(false));
            self.finish(id);
            return;
        };
        self.unlist(registration);
        self.settle(id, Outcome::Unregistered(true));
        if self.uses.get(&registration).copied().unwrap_or(0) == 0 {
            self.clear(registration);
        }
        self.finish(id);
    }

    /// Takes the registration out of the map.
    fn unlist(&mut self, registration: u64) {
        let Some(reg) = self.registrations.get_mut(&registration) else {
            return;
        };
        reg.listed = false;
        self.map
            .remove(&(reg.key.clone(), reg.scope.as_str().to_owned()));
        self.dirty.insert((reg.key.clone(), reg.row), reg.id);
    }

    /// Clear Registration: its versions stopped and redundant, and it
    /// forgotten.
    fn clear(&mut self, registration: u64) {
        for slot in [Slot::Installing, Slot::Waiting, Slot::Active] {
            let Some(version) = self
                .registrations
                .get_mut(&registration)
                .and_then(|reg| *reg.slot(slot))
            else {
                continue;
            };
            self.effects.push(Effect::Terminate { version });
            self.set_state(version, State::Redundant);
            self.set_slot(registration, slot, None);
            self.versions.remove(&version);
        }
        if let Some(reg) = self.registrations.remove(&registration) {
            self.dirty.insert((reg.key, reg.row), reg.id);
        }
        self.uses.remove(&registration);
    }

    /// Every registration of `key` unregistered and cleared at once: the
    /// site's data cleared. Its jobs end, their promises rejected.
    pub fn clear_key(&mut self, key: &str) {
        let jobs: Vec<u64> = self
            .jobs
            .iter()
            .filter(|(_, running)| running.job.key == key)
            .map(|(id, _)| *id)
            .collect();
        for id in jobs {
            self.reject(id, "AbortError", "The site's data was cleared.");
            if let Some(running) = self.jobs.remove(&id)
                && let Stage::Evaluating { version, .. } | Stage::Installing { version, .. } =
                    running.stage
            {
                self.effects.push(Effect::Terminate { version });
                self.versions.remove(&version);
            }
        }
        self.queues.retain(|(k, _), _| k != key);
        let all: Vec<u64> = self
            .registrations
            .values()
            .filter(|reg| reg.key == key)
            .map(|reg| reg.id)
            .collect();
        for registration in all {
            self.unlist(registration);
            self.clear(registration);
        }
    }

    /// Update Registration State.
    fn set_slot(&mut self, registration: u64, slot: Slot, version: Option<u64>) {
        let Some(reg) = self.registrations.get_mut(&registration) else {
            return;
        };
        *reg.slot(slot) = version;
        if slot == Slot::Active {
            self.dirty.insert((reg.key.clone(), reg.row), registration);
        }
        self.notify(
            registration,
            Notice::Slot {
                registration,
                slot,
                version,
            },
        );
    }

    /// Update Worker State.
    fn set_state(&mut self, version: u64, state: State) {
        let Some(worker) = self.versions.get_mut(&version) else {
            return;
        };
        worker.state = state;
        let registration = worker.registration;
        let Some(reg) = self.registrations.get(&registration) else {
            return;
        };
        if reg.active == Some(version) {
            self.dirty.insert((reg.key.clone(), reg.row), registration);
        }
        self.notify(registration, Notice::State { version, state });
    }

    /// A client of the registration's came (`controlled`) or went.
    pub fn use_registration(&mut self, registration: u64, controlled: bool) {
        let uses = self.uses.entry(registration).or_default();
        if controlled {
            *uses += 1;
            return;
        }
        *uses = uses.saturating_sub(1);
        if *uses > 0 {
            return;
        }
        match self.registrations.get(&registration) {
            Some(reg) if !reg.listed => self.clear(registration),
            Some(_) => self.try_activate(registration),
            None => {}
        }
    }
}

/// The partition of the storage key `key`: a cross-site one's, whose
/// top-level site follows a `^` (`crate::storage::AreaKey`).
fn partition_of(key: &str) -> Option<net::Partition> {
    let (_, site) = key.split_once('^')?;
    Some(net::Partition {
        top: Url::parse(site).ok()?.origin(),
        cross_site: true,
    })
}

impl Registry {
    /// The storage keys with registrations.
    pub fn keys(&self) -> Vec<String> {
        let keys: BTreeSet<String> = self.map.keys().map(|(key, _)| key.clone()).collect();
        keys.into_iter().collect()
    }

    /// Whether the storage key's registrations were read.
    pub fn knows(&self, key: &str) -> bool {
        self.loaded.contains(key)
    }

    /// Takes in the storage key's registrations the profile kept: each
    /// with its active version, activated. Records that do not read are
    /// passed over.
    pub fn load(&mut self, key: &str, records: Vec<(u64, Vec<u8>)>) {
        self.loaded.insert(key.to_owned());
        for (id, bytes) in records {
            let Some(read) = record::decode(&bytes) else {
                continue;
            };
            if self
                .map
                .contains_key(&(key.to_owned(), read.scope.as_str().to_owned()))
            {
                continue;
            }
            let row = id;
            self.next = self.next.max(row);
            let id = if self.registrations.contains_key(&row) {
                self.number()
            } else {
                row
            };
            let version = self.number();
            self.versions.insert(
                version,
                Version {
                    id: version,
                    registration: id,
                    script: read.script,
                    resources: read.resources,
                    events: read.events,
                    state: State::Activated,
                    skip_waiting: false,
                },
            );
            self.map
                .insert((key.to_owned(), read.scope.as_str().to_owned()), id);
            self.registrations.insert(
                id,
                Registration {
                    id,
                    row,
                    key: key.to_owned(),
                    scope: read.scope,
                    update_via_cache: read.update_via_cache,
                    installing: None,
                    waiting: None,
                    active: Some(version),
                    last_check: read.last_check,
                    listed: true,
                    partition: partition_of(key),
                },
            );
        }
    }

    /// Whether there is anything to write.
    pub fn has_written(&self) -> bool {
        self.writing && !self.dirty.is_empty()
    }

    /// What changed since it was last taken, for the profile: each
    /// registration with an active version activated, or gone.
    pub fn take_written(&mut self) -> Vec<Written> {
        if !self.writing {
            self.dirty.clear();
            return Vec::new();
        }
        let dirty = std::mem::take(&mut self.dirty);
        dirty
            .into_iter()
            .map(|((key, row), id)| {
                let record = self.registrations.get(&id).and_then(|reg| {
                    let active = reg.active.and_then(|v| self.versions.get(&v))?;
                    (reg.listed && active.state == State::Activated)
                        .then(|| record::encode(reg, active))
                });
                Written {
                    key,
                    id: row,
                    record,
                }
            })
            .collect()
    }

    /// The storage key's registrations, for DevTools: each scope with
    /// its versions' scripts and states.
    pub fn summary(&self, key: &str) -> Vec<Listed> {
        self.registrations(key)
            .into_iter()
            .filter_map(|id| {
                let reg = self.registrations.get(&id)?;
                let versions = [
                    (Slot::Installing, reg.installing),
                    (Slot::Waiting, reg.waiting),
                    (Slot::Active, reg.active),
                ]
                .into_iter()
                .filter_map(|(slot, v)| {
                    let v = self.versions.get(&v?)?;
                    Some((slot, v.script.to_string(), v.state))
                })
                .collect();
                Some((id, reg.scope.to_string(), versions))
            })
            .collect()
    }
}
