//! Service workers' versions in worker renderers (M39, D35): a renderer
//! of each site whose versions run, locked to it and sandboxed as any
//! renderer is, started when one of the site's versions must run and
//! shut down once none has for [`WORKER_RENDERER_IDLE`]. Each running
//! version is a frame of its renderer to the browser's network and
//! storage services, which check its fetches and its caches as they
//! check a document's; the browser keeps the registry, and a renderer
//! answers only for the versions the browser started in it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use super::network::{FrameGrant, Grants, Lock, NetworkService};
use super::process::{Event, Launcher, Renderer, Serve, Services};
use super::storage::StorageService;
use crate::ipc::{Bytes, ToBrowser, ToRenderer};
use crate::scripts::service_workers::{own_json, restart_of};
use crate::service_worker::{
    Done, Evaluated, Notice, Responded, ServiceWorkerHost, Start, View, WeakServiceWorkers,
};

/// How long a worker renderer lives with no version running in it.
#[cfg(not(test))]
pub const WORKER_RENDERER_IDLE: Duration = Duration::from_secs(30);
#[cfg(test)]
pub const WORKER_RENDERER_IDLE: Duration = Duration::from_secs(2);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A site's worker renderer: the browser's hold on it, its number, and
/// since when no version runs in it.
struct Site {
    renderer: Renderer,
    number: u32,
    empty_since: Option<Instant>,
}

/// A version running in a worker renderer: the renderer's number, the
/// frame the browser numbered it, and its storage key.
#[derive(Clone)]
struct Placed {
    renderer: u32,
    frame: u32,
    key: String,
}

/// An event sent and not yet done: its version, its type and data,
/// whether it was sent again after its version started again, and who
/// hears its end.
struct Pending {
    version: u64,
    kind: String,
    data: Option<String>,
    again: bool,
    done: Waiter,
}

/// Who hears an event's end: an extendable event's, or a fetch's answer.
enum Waiter {
    Extend(Done<bool>),
    Fetch(Done<Responded>),
}

impl Waiter {
    /// The event did not run: it failed, or the fetch goes to the network.
    fn fail(self) {
        match self {
            Waiter::Extend(done) => done(false),
            Waiter::Fetch(done) => done(Responded::Network),
        }
    }
}

#[derive(Default)]
struct State {
    sites: HashMap<Lock, Site>,
    versions: HashMap<u64, Placed>,
    evaluating: HashMap<u64, Done<Option<Evaluated>>>,
    events: HashMap<u64, Pending>,
    next_event: u64,
    next_frame: u32,
}

/// The browser's host of a context's versions, in worker renderers.
#[derive(Clone)]
pub struct WorkerRenderers {
    inner: Arc<Inner>,
}

struct Inner {
    launcher: Arc<dyn Launcher>,
    number: Box<dyn Fn() -> u32 + Send + Sync>,
    grants: Arc<Grants>,
    services: Arc<dyn Serve>,
    workers: WeakServiceWorkers,
    state: Mutex<State>,
    events: Mutex<mpsc::Sender<(u32, Event)>>,
    /// The running versions by their renderer and frame, which the
    /// storage service asks as them.
    frames: super::storage::Versions,
}

/// Has `context`'s versions run in worker renderers `launcher` starts,
/// from now on; once for a context, the host made then.
pub(crate) fn install(
    context: &crate::Context,
    launcher: Arc<dyn Launcher>,
) -> Option<WorkerRenderers> {
    if !context.claim_worker_renderers() {
        return None;
    }
    let grants = Grants::new();
    let network = NetworkService::new(
        net::Client::default()
            .with_transport(context.transport().clone())
            .with_cookies(context.jar().clone())
            .with_cache(context.cache().clone())
            .with_hsts(context.hsts().clone())
            .with_local_network_gate(context.permissions().clone())
            .with_languages(context.accept_language().clone()),
        grants.clone(),
    );
    let frames: super::storage::Versions = Arc::default();
    let storage = StorageService::of_versions(
        context.shared_storage().clone(),
        grants.clone(),
        frames.clone(),
    );
    let services: Arc<dyn Serve> = Arc::new(Services(vec![network, storage]));
    let (sender, receiver) = mpsc::channel();
    let numbers = context.renderer_numbers();
    let inner = Arc::new(Inner {
        launcher,
        number: Box::new(move || numbers.fetch_add(1, Ordering::Relaxed)),
        grants,
        services,
        workers: context.service_workers().downgrade(),
        state: Mutex::new(State::default()),
        events: Mutex::new(sender),
        frames,
    });
    let weak = Arc::downgrade(&inner);
    let _ = thread::Builder::new()
        .name("worker renderers".to_owned())
        .spawn(move || run(&weak, &receiver));
    let host = WorkerRenderers { inner };
    context.service_workers().set_host(Arc::new(host.clone()));
    Some(host)
}

/// The worker renderers' thread: what they send, and their idleness,
/// until the context goes.
fn run(weak: &Weak<Inner>, receiver: &Receiver<(u32, Event)>) {
    loop {
        let received = receiver.recv_timeout(Duration::from_millis(500));
        let Some(inner) = weak.upgrade() else {
            return;
        };
        match received {
            Ok((number, Event::Message(message))) => inner.heard(number, *message),
            Ok((number, Event::Gone(_))) => inner.gone(number),
            Ok((_, Event::Ready)) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        inner.shut_idle();
    }
}

impl Inner {
    /// The worker renderer of `lock`, started if it is not.
    fn site(&self, state: &mut State, lock: &Lock) -> Option<u32> {
        if let Some(site) = state.sites.get(lock) {
            return Some(site.number);
        }
        let number = (self.number)();
        if !self.grants.set_lock(number, lock.clone()) {
            return None;
        }
        let events = lock_sender(&self.events);
        let renderer = Renderer::start(
            &*self.launcher,
            number,
            events,
            Arc::new(|| {}),
            Some(self.services.clone()),
        )
        .ok()?;
        state.sites.insert(
            lock.clone(),
            Site {
                renderer,
                number,
                empty_since: None,
            },
        );
        Some(number)
    }

    /// Starts `start`'s version in its site's worker renderer; false if
    /// it could not.
    fn start(&self, start: &Start, stored: &[(net::Url, Arc<[u8]>)], evaluate: bool) -> bool {
        let Some(own) = self
            .workers
            .upgrade()
            .and_then(|driver| driver.query(&start.key, |registry| own_json(registry, start)))
        else {
            return false;
        };
        let lock = Lock::of(&start.script);
        let mut state = lock_state(&self.state);
        let Some(number) = self.site(&mut state, &lock) else {
            return false;
        };
        state.next_frame += 1;
        let frame = state.next_frame;
        let grant = match &start.partition {
            Some(partition) => FrameGrant {
                top: partition.top.clone(),
                cross_site: partition.cross_site,
            },
            None => FrameGrant {
                top: start.script.origin(),
                cross_site: false,
            },
        };
        self.grants.grant_frame(number, frame, grant);
        self::lock(&self.frames).insert((number, frame), start.version);
        if let Some(old) = state.versions.insert(
            start.version,
            Placed {
                renderer: number,
                frame,
                key: start.key.clone(),
            },
        ) {
            self.grants.revoke_frame(old.renderer, old.frame);
            self::lock(&self.frames).remove(&(old.renderer, old.frame));
        }
        if let Some(site) = state.sites.get_mut(&lock) {
            site.empty_since = None;
        }
        let message = ToRenderer::StartServiceWorker {
            version: start.version,
            frame,
            registration: start.registration,
            key: start.key.clone(),
            scope: Box::new(start.scope.clone()),
            script: Box::new(start.script.clone()),
            source: Bytes(start.source.to_vec()),
            stored: stored
                .iter()
                .map(|(url, body)| (url.clone(), Bytes(body.to_vec())))
                .collect(),
            own,
            partition: start.partition.clone(),
            evaluate,
        };
        state
            .sites
            .get(&lock)
            .is_some_and(|site| site.renderer.send(&message).is_ok())
    }

    /// Sends version `version` the event `kind`, starting it again from
    /// its scripts if it runs nowhere.
    fn dispatch(&self, version: u64, kind: &str, data: Option<String>, again: bool, done: Waiter) {
        let placed = lock_state(&self.state).versions.get(&version).cloned();
        let placed = match placed {
            Some(placed) => placed,
            None => {
                let restart = self
                    .workers
                    .upgrade()
                    .and_then(|driver| driver.query_any(|registry| restart_of(registry, version)));
                let Some((start, stored)) = restart else {
                    return done.fail();
                };
                if !self.start(&start, &stored, false) {
                    return done.fail();
                }
                match lock_state(&self.state).versions.get(&version).cloned() {
                    Some(placed) => placed,
                    None => return done.fail(),
                }
            }
        };
        let mut state = lock_state(&self.state);
        state.next_event += 1;
        let event = state.next_event;
        state.events.insert(
            event,
            Pending {
                version,
                kind: kind.to_owned(),
                data: data.clone(),
                again,
                done,
            },
        );
        let sent = state
            .sites
            .values()
            .find(|site| site.number == placed.renderer)
            .is_some_and(|site| {
                let message = match kind {
                    "fetch" => ToRenderer::ServiceWorkerFetch {
                        version,
                        event,
                        data: data.unwrap_or_default(),
                    },
                    _ => ToRenderer::ServiceWorkerEvent {
                        version,
                        event,
                        kind: kind.to_owned(),
                        data,
                    },
                };
                site.renderer.send(&message).is_ok()
            });
        if !sent && let Some(pending) = state.events.remove(&event) {
            drop(state);
            pending.done.fail();
        }
    }

    /// What worker renderer `number` sent.
    fn heard(&self, number: u32, message: ToBrowser) {
        let owns = |state: &State, version: u64| {
            state
                .versions
                .get(&version)
                .is_some_and(|placed| placed.renderer == number)
        };
        match message {
            ToBrowser::ServiceWorkerEvaluated {
                version,
                listened,
                imports,
            } => {
                let mut state = lock_state(&self.state);
                if !owns(&state, version) {
                    return self.misbehaved(
                        number,
                        &mut state,
                        "an evaluation of no version of its",
                    );
                }
                let Some(done) = state.evaluating.remove(&version) else {
                    return;
                };
                drop(state);
                done(listened.map(|events| {
                    Evaluated {
                        events,
                        imports: imports
                            .into_iter()
                            .map(|(url, body)| (url, Arc::from(body.0)))
                            .collect(),
                    }
                }));
            }
            ToBrowser::ServiceWorkerEventDone {
                version,
                event,
                ok,
                ran,
            } => {
                let mut state = lock_state(&self.state);
                if !owns(&state, version) {
                    return;
                }
                let Some(pending) = state
                    .events
                    .remove(&event)
                    .filter(|pending| pending.version == version)
                else {
                    return;
                };
                if !ran {
                    // It stopped as the event came: started again, it is
                    // sent again, once.
                    if let Some(placed) = state.versions.remove(&version) {
                        self.grants.revoke_frame(placed.renderer, placed.frame);
                    }
                    drop(state);
                    if pending.again {
                        return pending.done.fail();
                    }
                    return self.dispatch(version, &pending.kind, pending.data, true, pending.done);
                }
                drop(state);
                match pending.done {
                    Waiter::Extend(done) => done(ok),
                    Waiter::Fetch(done) => done(Responded::Network),
                }
            }
            ToBrowser::ServiceWorkerResponded {
                version,
                event,
                answer,
                response,
                ran,
            } => {
                let mut state = lock_state(&self.state);
                if !owns(&state, version) {
                    return;
                }
                let Some(pending) = state
                    .events
                    .remove(&event)
                    .filter(|pending| pending.version == version)
                else {
                    return;
                };
                if !ran {
                    if let Some(placed) = state.versions.remove(&version) {
                        self.grants.revoke_frame(placed.renderer, placed.frame);
                    }
                    drop(state);
                    if pending.again {
                        return pending.done.fail();
                    }
                    return self.dispatch(version, &pending.kind, pending.data, true, pending.done);
                }
                drop(state);
                let responded = match (answer, response) {
                    (2, Some(response)) => Responded::Given(response),
                    (1, _) => Responded::Failed,
                    _ => Responded::Network,
                };
                match pending.done {
                    Waiter::Fetch(done) => done(responded),
                    Waiter::Extend(done) => done(false),
                }
            }
            ToBrowser::ServiceWorkerSkipWaiting { version } => {
                if owns(&lock_state(&self.state), version)
                    && let Some(driver) = self.workers.upgrade()
                {
                    driver.skip_waiting(version);
                }
            }
            ToBrowser::ServiceWorkerStopped { version } => {
                let mut state = lock_state(&self.state);
                if owns(&state, version) {
                    self.stopped(&mut state, version);
                }
            }
            // A worker renderer shows nothing: what else it says is
            // dropped.
            _ => {}
        }
    }

    /// Version `version` runs no more: its evaluation and its events
    /// fail, its frame goes.
    fn stopped(&self, state: &mut State, version: u64) {
        let Some(placed) = state.versions.remove(&version) else {
            return;
        };
        self.grants.revoke_frame(placed.renderer, placed.frame);
        lock(&self.frames).remove(&(placed.renderer, placed.frame));
        let evaluation = state.evaluating.remove(&version);
        let events: Vec<u64> = state
            .events
            .iter()
            .filter(|(_, pending)| pending.version == version)
            .map(|(event, _)| *event)
            .collect();
        let failed: Vec<Pending> = events
            .into_iter()
            .filter_map(|event| state.events.remove(&event))
            .collect();
        if !state
            .versions
            .values()
            .any(|p| p.renderer == placed.renderer)
            && let Some(site) = state
                .sites
                .values_mut()
                .find(|site| site.number == placed.renderer)
        {
            site.empty_since = Some(Instant::now());
        }
        if let Some(done) = evaluation {
            done(None);
        }
        for pending in failed {
            pending.done.fail();
        }
    }

    /// Worker renderer `number` is gone: its versions with it.
    fn gone(&self, number: u32) {
        let mut state = lock_state(&self.state);
        let versions: Vec<u64> = state
            .versions
            .iter()
            .filter(|(_, placed)| placed.renderer == number)
            .map(|(version, _)| *version)
            .collect();
        for version in versions {
            self.stopped(&mut state, version);
        }
        state.sites.retain(|_, site| site.number != number);
        self.grants.forget(number);
    }

    /// Worker renderer `number` sent what it may not: it is killed.
    fn misbehaved(&self, number: u32, state: &mut State, what: &str) {
        if let Some(site) = state.sites.values().find(|site| site.number == number) {
            site.renderer.bad_message(what);
        }
    }

    /// Shuts the worker renderers that ran nothing for a while.
    fn shut_idle(&self) {
        let mut state = lock_state(&self.state);
        let idle: Vec<Lock> = state
            .sites
            .iter()
            .filter(|(_, site)| {
                site.empty_since
                    .is_some_and(|since| since.elapsed() >= WORKER_RENDERER_IDLE)
            })
            .map(|(lock, _)| lock.clone())
            .collect();
        for lock in idle {
            if let Some(site) = state.sites.remove(&lock) {
                site.renderer.shutdown();
                self.grants.forget(site.number);
            }
        }
    }
}

fn lock_state(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    lock(state)
}

fn lock_sender(events: &Mutex<mpsc::Sender<(u32, Event)>>) -> mpsc::Sender<(u32, Event)> {
    lock(events).clone()
}

impl WorkerRenderers {
    /// The number of worker renderers running, for the tests.
    #[cfg(test)]
    pub(crate) fn renderers(&self) -> usize {
        lock_state(&self.inner.state).sites.len()
    }
}

impl ServiceWorkerHost for WorkerRenderers {
    fn evaluate(&self, start: Start, done: Done<Option<Evaluated>>) {
        let version = start.version;
        lock_state(&self.inner.state)
            .evaluating
            .insert(version, done);
        if !self.inner.start(&start, &[], true)
            && let Some(done) = lock_state(&self.inner.state).evaluating.remove(&version)
        {
            done(None);
        }
    }

    fn install(&self, version: u64, done: Done<bool>) {
        self.inner
            .dispatch(version, "install", None, false, Waiter::Extend(done));
    }

    fn activate(&self, version: u64, done: Done<()>) {
        self.inner.dispatch(
            version,
            "activate",
            None,
            false,
            Waiter::Extend(Box::new(move |_| done(()))),
        );
    }

    fn message(&self, version: u64, data: String, done: Done<bool>) {
        self.inner
            .dispatch(version, "message", Some(data), false, Waiter::Extend(done));
    }

    fn fetch(&self, version: u64, data: String, done: Done<Responded>) {
        self.inner
            .dispatch(version, "fetch", Some(data), false, Waiter::Fetch(done));
    }

    fn terminate(&self, version: u64) {
        let mut state = lock_state(&self.inner.state);
        let Some(placed) = state.versions.get(&version).cloned() else {
            return;
        };
        if let Some(site) = state
            .sites
            .values()
            .find(|site| site.number == placed.renderer)
        {
            let _ = site
                .renderer
                .send(&ToRenderer::StopServiceWorker { version });
        }
        self.inner.stopped(&mut state, version);
    }

    fn notice(&self, key: &str, notice: &Notice, view: &View) {
        let task = crate::service_worker::page::notice_json(notice, view);
        let state = lock_state(&self.inner.state);
        let mut told = Vec::new();
        for placed in state.versions.values().filter(|placed| placed.key == key) {
            if told.contains(&placed.renderer) {
                continue;
            }
            told.push(placed.renderer);
            if let Some(site) = state
                .sites
                .values()
                .find(|site| site.number == placed.renderer)
            {
                let _ = site.renderer.send(&ToRenderer::ServiceWorkerNotice {
                    key: key.to_owned(),
                    task: task.clone(),
                });
            }
        }
    }
}

/// Whether a context's versions run in worker renderers yet.
#[derive(Default)]
pub(crate) struct Claimed(AtomicBool);

impl Claimed {
    /// True the first time.
    pub(crate) fn claim(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }
}
