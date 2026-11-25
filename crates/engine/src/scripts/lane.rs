//! The crypto lane (G255 step 7, `docs/design/webcrypto.md`; the lead's
//! ruling): Chrome's split, probed on Chrome 158. PBKDF2's derivations
//! and RSA's key generation run off the calling realm's thread, on one
//! lane a renderer's realms share (its documents and their workers),
//! first come first served; every other call runs where it is made.
//!
//! A job runs against a store of its own ([`crypto::lend`]); what it
//! made is its realm's when the realm takes the result on its own
//! thread ([`crypto::adopt`]). Nothing stops a job once it runs: when
//! its realm goes (its document unloaded, its worker ended, its engine
//! dropped) the realm's queued jobs go unrun, and a running one's lane is
//! abandoned, the job running on to its end and its result dropped,
//! while a fresh lane takes the queue at once: no realm ever waits
//! behind dead work. At most [`MOST_ABANDONED`] abandoned lanes run at
//! once; the next abandonment marks the renderer for retirement
//! ([`Lane::take_retire`]), after which none is abandoned and the queue
//! waits for what runs. Renderers being a tab's, only that tab waits.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::loader::Waker;

/// How many abandoned lanes may run at once before the next abandonment
/// marks the renderer for retirement (the lead's ruling, 2026-11-25).
pub(crate) const MOST_ABANDONED: usize = 2;

/// How long a lane waits with nothing to run before it ends; the next
/// job starts another.
const IDLE: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(30)
};

/// A job's result: its call's number, the store it ran against (the
/// keys it made), its outcome, and the bytes it held.
pub(crate) struct Done {
    pub(crate) id: u32,
    pub(crate) keys: crypto::Keys,
    pub(crate) outcome: Result<crypto::Outcome, crypto::Error>,
    pub(crate) cost: usize,
}

/// A job's work, run on the lane.
pub(crate) type Work = Box<dyn FnOnce() -> Done + Send>;

struct Job {
    realm: Arc<Realm>,
    work: Work,
}

/// A realm's side of the lane: whether it is there, its jobs' results as
/// they finish, and what wakes its thread.
#[derive(Default)]
struct Realm {
    alive: AtomicBool,
    done: Mutex<VecDeque<Done>>,
    wake: Mutex<Option<Waker>>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    /// The number of the lane serving the queue; an abandoned one's is
    /// older.
    current: u64,
    /// Whether lane `current` is running.
    serving: bool,
    /// The realm of the job lane `current` runs.
    running: Option<Arc<Realm>>,
    /// Abandoned lanes still running their jobs.
    abandoned: usize,
    /// Marked for retirement: no lane is abandoned any more.
    retiring: bool,
    /// The retirement not yet told to the renderer's owner.
    retire_untold: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A renderer's lane, its engines' and their workers'; clones are it.
#[derive(Clone)]
pub(crate) struct Lane {
    shared: Arc<Shared>,
}

impl Default for Lane {
    fn default() -> Self {
        Lane {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                ready: Condvar::new(),
            }),
        }
    }
}

impl Lane {
    /// A realm's way onto the lane.
    pub(crate) fn client(&self) -> Client {
        Client {
            shared: Arc::clone(&self.shared),
            realm: Arc::new(Realm {
                alive: AtomicBool::new(true),
                ..Realm::default()
            }),
        }
    }

    /// Whether the renderer was marked for retirement since this was last
    /// asked: its owner gives its site's next documents a new renderer.
    pub(crate) fn take_retire(&self) -> bool {
        std::mem::take(&mut self.shared.lock().retire_untold)
    }

    /// Whether a job of its realms waits or runs (an abandoned one's is
    /// no realm's any more): its owner gives it real time, as it gives a
    /// worker's.
    pub(crate) fn busy(&self) -> bool {
        let state = self.shared.lock();
        state.running.is_some() || !state.queue.is_empty()
    }

    /// Abandoned lanes still running, for the tests.
    #[cfg(test)]
    pub(crate) fn abandoned(&self) -> usize {
        self.shared.lock().abandoned
    }

    /// Whether the renderer is marked for retirement.
    #[cfg(test)]
    pub(crate) fn retiring(&self) -> bool {
        self.shared.lock().retiring
    }

    /// Whether a lane is running, for the tests.
    #[cfg(test)]
    pub(crate) fn serving(&self) -> bool {
        self.shared.lock().serving
    }
}

/// A realm's way onto the lane. Dropped as the realm goes: its jobs go
/// with it.
pub(crate) struct Client {
    shared: Arc<Shared>,
    realm: Arc<Realm>,
}

impl Client {
    /// Queues `work`, behind every job of the renderer's realms queued
    /// before it; `wake` wakes the realm's thread when it is done.
    pub(crate) fn submit(&self, work: Work, wake: Option<Waker>) {
        if wake.is_some() {
            *self
                .realm
                .wake
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = wake;
        }
        let mut state = self.shared.lock();
        state.queue.push_back(Job {
            realm: Arc::clone(&self.realm),
            work,
        });
        if !state.serving {
            serve(&self.shared, &mut state);
        }
        drop(state);
        self.shared.ready.notify_all();
    }

    /// The results of its jobs done since this was last asked, in the
    /// order they were.
    pub(crate) fn take_done(&self) -> Vec<Done> {
        self.realm
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    /// Whether a result waits, for the tests.
    #[cfg(test)]
    pub(crate) fn has_done(&self) -> bool {
        !self
            .realm
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.realm.alive.store(false, Ordering::SeqCst);
        let mut state = self.shared.lock();
        state
            .queue
            .retain(|job| !Arc::ptr_eq(&job.realm, &self.realm));
        let runs_ours = state
            .running
            .as_ref()
            .is_some_and(|running| Arc::ptr_eq(running, &self.realm));
        if runs_ours && state.serving && !state.retiring {
            // Its lane is abandoned; a fresh one takes the queue.
            state.abandoned += 1;
            if state.abandoned > MOST_ABANDONED {
                state.retiring = true;
                state.retire_untold = true;
            }
            state.current += 1;
            state.serving = false;
            state.running = None;
            if !state.queue.is_empty() {
                serve(&self.shared, &mut state);
            }
        }
        drop(state);
        self.shared.ready.notify_all();
        self.realm
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

/// Starts lane `current`, its state locked by the caller.
fn serve(shared: &Arc<Shared>, state: &mut State) {
    let number = state.current;
    let shared = Arc::clone(shared);
    let started = std::thread::Builder::new()
        .name("crypto lane".to_owned())
        .spawn(move || run(&shared, number));
    // A lane that could not start leaves its jobs queued for the next
    // submission to try again.
    state.serving = started.is_ok();
}

/// Lane `number`: runs the queue's jobs in order until it is abandoned,
/// or has waited [`IDLE`] with nothing to run.
fn run(shared: &Shared, number: u64) {
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if state.current != number {
                    return;
                }
                if let Some(job) = state.queue.pop_front() {
                    if !job.realm.alive.load(Ordering::SeqCst) {
                        continue;
                    }
                    state.running = Some(Arc::clone(&job.realm));
                    break job;
                }
                let (waited, timeout) = shared
                    .ready
                    .wait_timeout(state, IDLE)
                    .unwrap_or_else(PoisonError::into_inner);
                state = waited;
                if timeout.timed_out() && state.queue.is_empty() && state.current == number {
                    state.serving = false;
                    return;
                }
            }
        };
        let done = (job.work)();
        let mut state = shared.lock();
        if state.current != number {
            // Abandoned while it ran: its result goes with it.
            state.abandoned -= 1;
            return;
        }
        state.running = None;
        drop(state);
        if job.realm.alive.load(Ordering::SeqCst) {
            job.realm
                .done
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(done);
            let wake = job
                .realm
                .wake
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(wake) = wake {
                wake();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    /// A job that says it began, waits for `go`, and says it ended.
    fn held(id: u32, log: &Arc<Mutex<Vec<String>>>, go: mpsc::Receiver<()>) -> Work {
        let log = Arc::clone(log);
        Box::new(move || {
            log.lock().unwrap().push(format!("began {id}"));
            let _ = go.recv_timeout(DEADLINE);
            log.lock().unwrap().push(format!("ended {id}"));
            done(id)
        })
    }

    /// A job that ends at once.
    fn quick(id: u32, log: &Arc<Mutex<Vec<String>>>) -> Work {
        let log = Arc::clone(log);
        Box::new(move || {
            log.lock().unwrap().push(format!("ran {id}"));
            done(id)
        })
    }

    fn done(id: u32) -> Done {
        Done {
            id,
            keys: crypto::Keys::default(),
            outcome: Ok(crypto::Outcome::Bool(true)),
            cost: 0,
        }
    }

    /// How long a test waits for anything; a hang is no result.
    const DEADLINE: Duration = Duration::from_secs(30);

    /// Until `holds`, or a failure naming `what` past the deadline.
    fn until(what: &str, holds: impl Fn() -> bool) {
        let start = Instant::now();
        while !holds() {
            assert!(start.elapsed() < DEADLINE, "waited {DEADLINE:?} for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn ids(client: &Client) -> Vec<u32> {
        client.take_done().into_iter().map(|done| done.id).collect()
    }

    #[test]
    fn jobs_run_one_at_a_time_in_the_order_they_came_across_realms() {
        let lane = Lane::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (a, b) = (lane.client(), lane.client());
        let (go, wait) = mpsc::channel();
        a.submit(held(1, &log, wait), None);
        b.submit(quick(2, &log), None);
        a.submit(quick(3, &log), None);
        until("job 1 to begin", || log.lock().unwrap().len() == 1);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(*log.lock().unwrap(), ["began 1"]);
        go.send(()).unwrap();
        until("jobs 1 to 3 to run", || log.lock().unwrap().len() == 4);
        assert_eq!(
            *log.lock().unwrap(),
            ["began 1", "ended 1", "ran 2", "ran 3"]
        );
        assert_eq!(ids(&a), [1, 3]);
        assert_eq!(ids(&b), [2]);
    }

    #[test]
    fn a_gone_realms_queued_jobs_never_run_and_its_results_are_dropped() {
        let lane = Lane::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (a, b) = (lane.client(), lane.client());
        let (go, wait) = mpsc::channel();
        a.submit(held(1, &log, wait), None);
        b.submit(quick(2, &log), None);
        drop(b);
        go.send(()).unwrap();
        a.submit(quick(3, &log), None);
        until("jobs 1 and 3 to run", || log.lock().unwrap().len() == 3);
        assert_eq!(*log.lock().unwrap(), ["began 1", "ended 1", "ran 3"]);
        assert_eq!(lane.abandoned(), 0);
    }

    #[test]
    fn a_gone_realms_running_job_is_abandoned_and_the_queue_goes_on() {
        let lane = Lane::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (a, b) = (lane.client(), lane.client());
        let (go, wait) = mpsc::channel();
        a.submit(held(1, &log, wait), None);
        b.submit(quick(2, &log), None);
        until("job 1 to begin", || log.lock().unwrap().len() == 1);
        drop(a);
        // The queue runs on a fresh lane while the dead job runs on.
        until("job 2 to run past the abandoned job 1", || b.has_done());
        assert_eq!(ids(&b), [2]);
        assert_eq!(lane.abandoned(), 1);
        go.send(()).unwrap();
        until("the abandoned lane to end", || lane.abandoned() == 0);
        assert!(!lane.take_retire());
    }

    #[test]
    fn the_third_abandoned_lane_marks_the_renderer_for_retirement() {
        let lane = Lane::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut held_by = Vec::new();
        for id in 1..=3 {
            let client = lane.client();
            let (go, wait) = mpsc::channel();
            client.submit(held(id, &log, wait), None);
            until(&format!("job {id} to begin"), || {
                log.lock()
                    .unwrap()
                    .iter()
                    .any(|l| *l == format!("began {id}"))
            });
            drop(client);
            held_by.push(go);
        }
        assert_eq!(lane.abandoned(), 3);
        assert!(lane.retiring());
        assert!(lane.take_retire());
        assert!(!lane.take_retire());
        // Retiring, a fourth realm's running job is not abandoned: the
        // queue waits for it.
        let (d, e) = (lane.client(), lane.client());
        let (go, wait) = mpsc::channel();
        d.submit(held(4, &log, wait), None);
        until("job 4 to begin", || {
            log.lock().unwrap().iter().any(|l| l == "began 4")
        });
        e.submit(quick(5, &log), None);
        drop(d);
        std::thread::sleep(Duration::from_millis(50));
        assert!(!e.has_done());
        assert_eq!(lane.abandoned(), 3);
        go.send(()).unwrap();
        until("job 5 to run after job 4", || e.has_done());
        assert_eq!(ids(&e), [5]);
        for go in held_by {
            go.send(()).unwrap();
        }
        until("the abandoned lanes to end", || lane.abandoned() == 0);
    }

    #[test]
    fn an_idle_lane_ends_and_its_realm_is_woken_for_each_result() {
        let lane = Lane::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let client = lane.client();
        let count = Arc::clone(&woken);
        let wake: Waker = Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        client.submit(quick(1, &log), Some(wake));
        client.submit(quick(2, &log), None);
        until("two wakes", || woken.load(Ordering::SeqCst) == 2);
        assert_eq!(ids(&client), [1, 2]);
        until("the idle lane to end", || !lane.serving());
        // The next job starts another.
        client.submit(quick(3, &log), None);
        until("job 3 on a new lane", || client.has_done());
        assert_eq!(ids(&client), [3]);
    }
}
