//! Threads to fetch on (docs/design/net.md, "Threads, not an
//! asynchronous runtime"): a thread per job, at most a fixed number at
//! once, and the jobs beyond them waiting in the order they came. Each
//! finished job's result goes back over a channel, and the waker is
//! called so that the engine's owner knows to take it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

/// What wakes the event loop from another thread.
pub(crate) type Waker = Arc<dyn Fn() + Send + Sync>;

/// A job's number, given by its submitter.
pub(crate) type JobId = u64;

/// A job that ended without a result: its thread could not be started,
/// or panicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lost;

type Job<T> = Box<dyn FnOnce(&Report<T>) -> T + Send>;

/// What a running job reports through before it finishes (M24.8): each
/// value goes back over the channel results take, the waker called, and
/// is taken as a result is, the job still running.
pub(crate) struct Report<T> {
    shared: Arc<Shared<T>>,
    id: JobId,
}

impl<T: Send + 'static> Report<T> {
    pub(crate) fn send(&self, value: T) {
        finish(&self.shared, self.id, Ok(value));
    }
}
type Finished<T> = (JobId, Result<T, Lost>);

/// Runs jobs on threads of its own.
pub(crate) struct Pool<T> {
    shared: Arc<Shared<T>>,
    results: Receiver<Finished<T>>,
}

struct Shared<T> {
    /// The most threads running at once.
    cap: usize,
    queue: Mutex<Queue<T>>,
    results: Sender<Finished<T>>,
    waker: Mutex<Option<Waker>>,
    /// The most threads that were running at once, for the tests.
    most_running: AtomicUsize,
}

struct Queue<T> {
    jobs: VecDeque<(JobId, Job<T>)>,
    /// How many threads are running: each takes jobs until none waits.
    running: usize,
}

impl<T: Send + 'static> Pool<T> {
    /// A pool of at most `cap` threads, none started yet.
    pub(crate) fn new(cap: usize) -> Pool<T> {
        let (sender, results) = channel();
        Pool {
            shared: Arc::new(Shared {
                cap: cap.max(1),
                queue: Mutex::new(Queue {
                    jobs: VecDeque::new(),
                    running: 0,
                }),
                results: sender,
                waker: Mutex::new(None),
                most_running: AtomicUsize::new(0),
            }),
            results,
        }
    }

    /// Calls `waker` from now on whenever a job finishes.
    pub(crate) fn set_waker(&self, waker: Waker) {
        *lock(&self.shared.waker) = Some(waker);
    }

    /// The waker, if one was set.
    pub(crate) fn waker(&self) -> Option<Waker> {
        lock(&self.shared.waker).clone()
    }

    /// Runs `job` as `id`: on a new thread if fewer than the cap are
    /// running, or else on the first thread free, after the jobs
    /// already waiting.
    pub(crate) fn run(&self, id: JobId, job: impl FnOnce(&Report<T>) -> T + Send + 'static) {
        let mut queue = lock(&self.shared.queue);
        queue.jobs.push_back((id, Box::new(job)));
        start_thread(&self.shared, &mut queue);
    }

    /// A finished job's number and result, if one has finished and not
    /// been taken.
    pub(crate) fn try_take(&self) -> Option<Finished<T>> {
        self.results.try_recv().ok()
    }

    /// Waits up to `timeout` for a finished job, for the tests.
    #[cfg(test)]
    pub(crate) fn take_within(&self, timeout: std::time::Duration) -> Option<Finished<T>> {
        self.results.recv_timeout(timeout).ok()
    }

    /// The most threads that were running at once.
    #[cfg(test)]
    pub(crate) fn most_running(&self) -> usize {
        self.shared.most_running.load(Ordering::SeqCst)
    }

    /// How many threads are running now.
    #[cfg(test)]
    pub(crate) fn running(&self) -> usize {
        lock(&self.shared.queue).running
    }

    /// Says how many threads are running, for as long as it is kept,
    /// whatever becomes of the pool.
    #[cfg(test)]
    pub(crate) fn running_probe(&self) -> impl Fn() -> usize + use<T> {
        let shared = self.shared.clone();
        move || lock(&shared.queue).running
    }
}

/// Starts a thread for the waiting jobs if any wait and the cap allows.
/// A thread that cannot be started leaves its jobs to the threads
/// running; with none running, they are lost.
fn start_thread<T: Send + 'static>(shared: &Arc<Shared<T>>, queue: &mut Queue<T>) {
    if queue.jobs.is_empty() || queue.running >= shared.cap {
        return;
    }
    queue.running += 1;
    shared
        .most_running
        .fetch_max(queue.running, Ordering::SeqCst);
    let worker = shared.clone();
    let started = thread::Builder::new()
        .name("silver-fetch".to_owned())
        .spawn(move || work(&worker));
    if started.is_err() {
        queue.running -= 1;
        if queue.running == 0 {
            for (id, _) in queue.jobs.drain(..) {
                finish(shared, id, Err(Lost));
            }
        }
    }
}

/// A thread's life: it takes the first waiting job until none waits.
fn work<T: Send + 'static>(shared: &Arc<Shared<T>>) {
    let mut guard = Running {
        shared,
        job: None,
        done: false,
    };
    loop {
        let (id, job) = {
            let mut queue = lock(&shared.queue);
            let Some(next) = queue.jobs.pop_front() else {
                // Under the lock that a new job is queued under, so a
                // job queued now finds this thread gone and starts one.
                queue.running -= 1;
                guard.done = true;
                return;
            };
            next
        };
        guard.job = Some(id);
        let report = Report {
            shared: shared.clone(),
            id,
        };
        let result = job(&report);
        guard.job = None;
        finish(shared, id, Ok(result));
    }
}

/// A running thread, which on a panic gives its place up, reports its
/// job lost, and starts another thread for the jobs still waiting.
struct Running<'a, T: Send + 'static> {
    shared: &'a Arc<Shared<T>>,
    job: Option<JobId>,
    done: bool,
}

impl<T: Send + 'static> Drop for Running<'_, T> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        if let Some(id) = self.job {
            finish(self.shared, id, Err(Lost));
        }
        let mut queue = lock(&self.shared.queue);
        queue.running -= 1;
        start_thread(self.shared, &mut queue);
    }
}

/// Sends a job's result and calls the waker. A pool dropped since has
/// no one to take it, which is no error.
fn finish<T>(shared: &Shared<T>, id: JobId, result: Result<T, Lost>) {
    let _ = shared.results.send((id, result));
    let waker = lock(&shared.waker).clone();
    if let Some(waker) = waker {
        waker();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Condvar;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(20);

    /// A gate jobs wait at until the test opens it.
    #[derive(Default)]
    struct Gate {
        open: Mutex<bool>,
        opened: Condvar,
    }

    impl Gate {
        fn wait(&self) {
            let mut open = lock(&self.open);
            while !*open {
                open = self
                    .opened
                    .wait(open)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }

        fn open(&self) {
            *lock(&self.open) = true;
            self.opened.notify_all();
        }
    }

    fn take_all(pool: &Pool<usize>, count: usize) -> Vec<(JobId, Result<usize, Lost>)> {
        (0..count)
            .map(|_| pool.take_within(WAIT).expect("a job finishes"))
            .collect()
    }

    #[test]
    fn no_more_threads_run_than_the_cap() {
        let pool = Pool::new(4);
        let gate = Arc::new(Gate::default());
        let started = Arc::new(AtomicUsize::new(0));
        for id in 0..10 {
            let gate = gate.clone();
            let started = started.clone();
            pool.run(id, move |_| {
                started.fetch_add(1, Ordering::SeqCst);
                gate.wait();
                id as usize
            });
        }
        // Four begin and wait; the other six wait their turn.
        while started.load(Ordering::SeqCst) < 4 {
            thread::yield_now();
        }
        thread::sleep(Duration::from_millis(50));
        assert_eq!(started.load(Ordering::SeqCst), 4);
        gate.open();
        let mut ids: Vec<JobId> = take_all(&pool, 10).into_iter().map(|(id, _)| id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..10).collect::<Vec<_>>());
        assert_eq!(pool.most_running(), 4);
    }

    #[test]
    fn waiting_jobs_run_in_the_order_they_came() {
        let pool = Pool::new(1);
        let order = Arc::new(Mutex::new(Vec::new()));
        for id in 0..20 {
            let order = order.clone();
            pool.run(id, move |_| {
                lock(&order).push(id);
                0
            });
        }
        take_all(&pool, 20);
        assert_eq!(*lock(&order), (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn a_finished_job_wakes_the_owner_and_its_thread_ends() {
        let pool = Pool::new(32);
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = woken.clone();
        pool.set_waker(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        pool.run(7, |_| 42);
        assert_eq!(pool.take_within(WAIT), Some((7, Ok(42))));
        // The waker is called once the result can be taken, so it comes
        // after.
        let deadline = std::time::Instant::now() + WAIT;
        while woken.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "the owner is woken");
            thread::yield_now();
        }
        // The thread gives its place up once no job waits.
        while pool.running() > 0 {
            assert!(std::time::Instant::now() < deadline, "the thread ends");
            thread::yield_now();
        }
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        assert_eq!(pool.try_take(), None);
    }

    #[test]
    fn a_job_reports_as_it_goes_before_it_finishes() {
        let pool = Pool::new(2);
        let (go, wait) = std::sync::mpsc::channel::<()>();
        pool.run(3, move |report| {
            report.send(1);
            report.send(2);
            // The reports can be taken while the job still runs.
            let _ = wait.recv();
            9
        });
        assert_eq!(pool.take_within(WAIT), Some((3, Ok(1))));
        assert_eq!(pool.take_within(WAIT), Some((3, Ok(2))));
        assert_eq!(pool.try_take(), None);
        go.send(()).unwrap();
        assert_eq!(pool.take_within(WAIT), Some((3, Ok(9))));
    }

    #[test]
    fn a_job_that_panics_is_lost_and_the_rest_still_run() {
        let pool = Pool::new(1);
        pool.run(0, |_| panic!("a job's bug"));
        for id in 1..4 {
            pool.run(id, move |_| id as usize);
        }
        let mut finished = take_all(&pool, 4);
        finished.sort_unstable_by_key(|(id, _)| *id);
        assert_eq!(
            finished,
            [(0, Err(Lost)), (1, Ok(1)), (2, Ok(2)), (3, Ok(3))]
        );
    }
}
