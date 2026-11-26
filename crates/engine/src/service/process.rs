//! The renderers the browser starts (M34.2, docs/design/processes.md,
//! "IPC"): a [`Launcher`] starts one (a process of the shell's
//! executable, or a thread in the tests), and a [`Renderer`] is the
//! browser's hold on it: its channel, its process, and what it says,
//! handed to the browser's thread as [`Event`]s. A renderer's first
//! message must be its hello, of this schema's version, within
//! [`HELLO_TIMEOUT`]; one that says anything else first, or nothing, is
//! killed.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AtomicOrdering, fence};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::ipc::{
    self, Closed, IPC_VERSION, PipeBreaker, ReadError, Sender, ToBrowser, ToRenderer,
};
use crate::loader::Waker;

/// How long a renderer has to answer the hello before it is killed.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// The memory a renderer's process may commit, where its sandbox can
/// limit it (M34.3, D2): past it, its allocations fail.
pub const MAX_RENDERER_MEMORY: u64 = 4 << 30;

/// What starts renderers, and the GPU process (M41, D38).
pub trait Launcher: Send + Sync {
    /// Starts renderer number `number`.
    fn launch(&self, number: u32) -> io::Result<Launched>;

    /// Starts the GPU process; its channel is the browser's as a
    /// renderer's is. Unsupported where the launcher has none.
    fn launch_gpu(&self) -> io::Result<Launched> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// A renderer started: its streams and its process.
pub struct Launched {
    /// What the renderer writes, which the browser reads.
    pub from_renderer: Box<dyn Read + Send>,
    /// What the renderer reads, which the browser writes.
    pub to_renderer: Box<dyn Write + Send>,
    /// The renderer's standard error, for the browser's own, where it has
    /// one of its own.
    pub log: Option<Box<dyn Read + Send>>,
    pub process: Box<dyn RendererProcess>,
}

/// A started renderer's process, as the browser holds it.
pub trait RendererProcess: Send {
    /// Ends it at once.
    fn kill(&mut self);
    /// The operating system's number for it; 0 for a thread.
    fn id(&self) -> u32;
    /// Its exit code, once it has ended.
    fn exit_code(&mut self) -> Option<i64>;
    /// The file the browser wrote a dump of it to when it crashed, if it
    /// did (M34.4): where the platform's launcher writes them.
    fn dump(&mut self) -> Option<std::path::PathBuf> {
        None
    }
}

/// What serves the messages a renderer sends the browser's services (the
/// network, the storage), on the renderer's reader thread, never waiting
/// on any renderer.
pub trait Serve: Send + Sync {
    /// Takes `message` of renderer `renderer` if it is a service's,
    /// answering through `reply`; gives it back otherwise. Err for what
    /// the protocol does not allow, for which the renderer is killed.
    fn serve(
        &self,
        renderer: u32,
        message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String>;
}

/// Services tried in turn: the first that takes a message serves it.
pub struct Services(pub Vec<Arc<dyn Serve>>);

impl Serve for Services {
    fn serve(
        &self,
        renderer: u32,
        mut message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        for service in &self.0 {
            match service.serve(renderer, message, reply)? {
                Some(left) => message = left,
                None => return Ok(None),
            }
        }
        Ok(Some(message))
    }
}

/// Why a renderer is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gone {
    /// Its channel ended: it exited, crashed or was killed from outside.
    Ended,
    /// It sent what the protocol does not allow, and was killed.
    BadMessage(String),
    /// It did not answer the hello in time, and was killed.
    NoHello,
    /// The browser shut it down.
    Shutdown,
}

/// The most messages of one renderer the browser may hold untaken, and
/// the most bytes of them (the lead's ruling, 2026-11-26): past either,
/// the renderer is a bad message source and ends, as Chrome ends a
/// renderer that sends what it may not. A renderer the browser keeps up
/// with holds a few; a compromised or broken one could have filled the
/// browser's memory.
pub const MOST_UNTAKEN_MESSAGES: usize = 4096;
pub const MOST_UNTAKEN_BYTES: usize = 64 << 20;

/// How long the browser may take nothing of a renderer before it counts
/// as stalled: its thread held elsewhere, in a modal loop (a window
/// dragged or resized, a menu held open, a native file or print dialog).
/// A renderer over its budget then is not ended: its messages past the
/// budget are parked, and its reader keeps reading, its sound, GPU and
/// frames messages served as ever (the lead's review, 2026-11-26: a page
/// logging each frame passed the budget in a minute of a Save dialog,
/// and a reader that stopped would have silenced its music). Frames
/// never pile up: a view draws its next only once the browser took its
/// last.
pub const STALLED: Duration = Duration::from_secs(1);

/// A renderer's messages the browser has not yet taken: their count and
/// their frames' bytes, when it last took one, and the messages parked
/// past the budget while it was stalled.
#[derive(Debug)]
pub struct Budget {
    messages: AtomicUsize,
    bytes: AtomicUsize,
    born: Instant,
    /// When the browser last took one of its messages, in milliseconds
    /// from `born` and one more; none yet, 0.
    last_taken: AtomicU64,
    /// A give-back could not take `parked` to unpark: whoever holds it
    /// unparks as it lets go. Set before trying, so that the holder, who
    /// looks after letting go, always sees it (no lost wakeup).
    owed: AtomicBool,
    parked: Mutex<Parked>,
    limits: Limits,
}

/// A budget's limits: [`MOST_UNTAKEN_MESSAGES`], [`MOST_UNTAKEN_BYTES`],
/// [`STALLED`], and how long a reader waits when even its parked messages
/// are at their limit; smaller in the tests.
#[derive(Debug, Clone, Copy)]
struct Limits {
    messages: usize,
    bytes: usize,
    stalled: Duration,
    wait: Duration,
}

/// Messages past a stalled browser's budget, in order, with their
/// frames' bytes, and where they go once there is room.
#[derive(Debug, Default)]
struct Parked {
    queue: VecDeque<(Box<ToBrowser>, usize)>,
    bytes: usize,
    to: Option<(mpsc::Sender<(u32, Event)>, u32)>,
}

/// What becomes of a message a reader takes.
enum Step {
    /// The browser's now.
    Now(Event),
    /// Parked, or merged into one parked.
    Parked,
    /// Its renderer outran a browser that takes, with what it held.
    Outran(usize, usize),
    /// Its parked messages at their limit too: given back, to try again.
    Wait(Box<ToBrowser>),
}

impl Default for Budget {
    fn default() -> Budget {
        Budget::with(Limits {
            messages: MOST_UNTAKEN_MESSAGES,
            bytes: MOST_UNTAKEN_BYTES,
            stalled: STALLED,
            wait: Duration::from_millis(20),
        })
    }
}

impl Budget {
    fn with(limits: Limits) -> Budget {
        Budget {
            messages: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            born: Instant::now(),
            last_taken: AtomicU64::new(0),
            owed: AtomicBool::new(false),
            parked: Mutex::default(),
            limits,
        }
    }

    /// A budget of `messages` and `bytes`, the browser stalled once it has
    /// taken nothing for `stalled`, a reader waiting `wait` at a time.
    #[cfg(test)]
    fn with_limits(messages: usize, bytes: usize, stalled: Duration, wait: Duration) -> Budget {
        Budget::with(Limits {
            messages,
            bytes,
            stalled,
            wait,
        })
    }

    /// Whether the browser has taken nothing of the renderer for the
    /// stall's time: it waits for the browser, the browser not outrun.
    fn stalled(&self) -> bool {
        let last = self.last_taken.load(AtomicOrdering::SeqCst);
        let now = self.born.elapsed().as_millis() as u64 + 1;
        last == 0 || now.saturating_sub(last) >= self.limits.stalled.as_millis() as u64
    }

    /// Charges a message of `bytes` to the budget: its share, given back
    /// when dropped; or, past the budget, what it would have held, and
    /// nothing charged.
    pub(crate) fn charge(self: &Arc<Self>, bytes: usize) -> Result<Held, (usize, usize)> {
        let messages = self.messages.fetch_add(1, AtomicOrdering::SeqCst) + 1;
        let held = self.bytes.fetch_add(bytes, AtomicOrdering::SeqCst) + bytes;
        if messages > self.limits.messages || held > self.limits.bytes {
            // Given back at once: a share never made, no message taken.
            self.messages.fetch_sub(1, AtomicOrdering::SeqCst);
            self.bytes.fetch_sub(bytes, AtomicOrdering::SeqCst);
            return Err((messages, held));
        }
        Ok(Held {
            budget: Arc::clone(self),
            bytes,
        })
    }

    /// Parked messages given to the browser, in order, while the budget
    /// has room.
    fn unpark(self: &Arc<Self>, parked: &mut Parked) {
        let Some((to, number)) = parked.to.clone() else {
            return;
        };
        while let Some(&(_, len)) = parked.queue.front() {
            let Ok(held) = self.charge(len) else { break };
            let Some((message, len)) = parked.queue.pop_front() else {
                break;
            };
            parked.bytes -= len;
            let _ = to.send((number, Event::Message(message, held)));
        }
    }

    /// The unparks owed, done while `parked` can be had: by the one that
    /// owes it, or by the holder as it lets go.
    fn settle(self: &Arc<Self>) {
        loop {
            // A StoreLoad pairing: the give-back stores `owed` then reads
            // the lock (its try_lock); the holder stores the lock (its
            // unlock) then reads `owed`. Acquire and release order neither
            // store before the load after it, and on a weakly ordered
            // processor (ARM64) each side could miss the other's store, the
            // wakeup lost again; a SeqCst fence between them on both sides
            // (this one: after a give-back's store, after a holder's
            // unlock) forbids it (the lead's review, 2026-11-26).
            fence(AtomicOrdering::SeqCst);
            if !self.owed.load(AtomicOrdering::SeqCst) {
                return;
            }
            let Ok(mut parked) = self.parked.try_lock() else {
                // Its holder settles as it lets go.
                return;
            };
            self.owed.store(false, AtomicOrdering::SeqCst);
            self.unpark(&mut parked);
        }
    }

    /// Takes renderer's `message` of `len` bytes, not served: its event
    /// now, or none, parked (the browser stalled) or the renderer ended
    /// (it outran a browser that takes).
    fn take(self: &Arc<Self>, message: ToBrowser, len: usize, reading: &Shared) -> Option<Event> {
        let mut message = Box::new(message);
        loop {
            let step = {
                let mut parked = self.parked.lock().unwrap_or_else(PoisonError::into_inner);
                // Held now: what is owed is done here.
                self.owed.store(false, AtomicOrdering::SeqCst);
                self.unpark(&mut parked);
                self.decide(&mut parked, message, len)
            };
            self.settle();
            match step {
                Step::Now(event) => return Some(event),
                Step::Parked => return None,
                Step::Outran(messages, bytes) => {
                    self.outran(reading, messages, bytes);
                    return None;
                }
                Step::Wait(back) => {
                    if matches!(*reading.state(), State::Gone(_)) {
                        return None;
                    }
                    message = back;
                    if self.limits.wait.is_zero() {
                        thread::yield_now();
                    } else {
                        thread::sleep(self.limits.wait);
                    }
                }
            }
        }
    }

    /// What becomes of `message`, `parked` held.
    fn decide(
        self: &Arc<Self>,
        parked: &mut Parked,
        mut message: Box<ToBrowser>,
        len: usize,
    ) -> Step {
        if parked.queue.is_empty() {
            match self.charge(len) {
                Ok(held) => return Step::Now(Event::Message(message, held)),
                Err((messages, bytes)) if !self.stalled() => return Step::Outran(messages, bytes),
                Err(_) => {}
            }
        }
        if parked.queue.len() >= self.limits.messages || parked.bytes >= self.limits.bytes {
            // Parked to its own limit: outrun, or else wait for the
            // browser, the renderer's pipe filling.
            if !self.stalled() {
                return Step::Outran(parked.queue.len(), parked.bytes);
            }
            return Step::Wait(message);
        }
        // Parked: a view's state replaces its last, a view's console joins
        // the console last parked for it.
        let joins = |last: Option<&(Box<ToBrowser>, usize)>, view: u32| {
            last.is_some_and(
                |(last, _)| matches!(&**last, ToBrowser::Console { view: v, .. } if *v == view),
            )
        };
        match &mut *message {
            ToBrowser::Console { view, entries } if joins(parked.queue.back(), *view) => {
                let mut more = std::mem::take(entries);
                if let Some((last, last_len)) = parked.queue.back_mut() {
                    if let ToBrowser::Console {
                        entries: before, ..
                    } = &mut **last
                    {
                        before.append(&mut more);
                    }
                    *last_len += len;
                }
            }
            ToBrowser::State { view, .. } => {
                let view = *view;
                let mut freed = 0;
                parked.queue.retain(|(parked, len)| {
                    let same = matches!(&**parked, ToBrowser::State { view: v, .. } if *v == view);
                    if same {
                        freed += len;
                    }
                    !same
                });
                parked.bytes -= freed;
                parked.queue.push_back((message, len));
            }
            _ => parked.queue.push_back((message, len)),
        }
        parked.bytes += len;
        Step::Parked
    }

    /// The renderer outran a browser that takes: a bad message source.
    fn outran(&self, reading: &Shared, messages: usize, bytes: usize) {
        let Limits {
            messages: most,
            bytes: most_bytes,
            ..
        } = self.limits;
        reading.end(
            Gone::BadMessage(format!(
                "past its budget of untaken messages: {messages} messages, {bytes} bytes \
                 (at most {most}, {most_bytes})"
            )),
            true,
        );
    }

    /// How many messages it holds untaken, for the tests.
    #[cfg(test)]
    pub(crate) fn untaken(&self) -> usize {
        self.messages.load(AtomicOrdering::SeqCst)
    }

    /// How many messages it holds parked, for the tests.
    #[cfg(test)]
    fn parked(&self) -> usize {
        self.parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .queue
            .len()
    }
}

/// A message's share of its renderer's [`Budget`], given back when it is
/// dropped: once the browser has taken the message, whoever took it; and
/// with the room, the messages parked meanwhile given to the browser.
#[derive(Debug)]
pub struct Held {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Drop for Held {
    fn drop(&mut self) {
        let now = self.budget.born.elapsed().as_millis() as u64 + 1;
        self.budget.last_taken.store(now, AtomicOrdering::SeqCst);
        self.budget.messages.fetch_sub(1, AtomicOrdering::SeqCst);
        self.budget
            .bytes
            .fetch_sub(self.bytes, AtomicOrdering::SeqCst);
        // An unpark is owed: done now, or by the parked queue's holder as
        // it lets go.
        self.budget.owed.store(true, AtomicOrdering::SeqCst);
        self.budget.settle();
    }
}

/// What a renderer did, for the browser's thread.
#[derive(Debug)]
pub enum Event {
    /// It answered the hello.
    Ready,
    /// A message after its hello, and its share of the renderer's
    /// budget, given back as the message is dropped.
    Message(Box<ToBrowser>, Held),
    /// It is gone, and why; nothing comes after.
    Gone(Gone),
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Starting,
    Ready,
    Gone(Gone),
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    process: Mutex<Box<dyn RendererProcess>>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Marks the renderer gone for `why`, unless it is gone already, and
    /// kills it then if `kill`; says whether it was not gone.
    fn end(&self, why: Gone, kill: bool) -> bool {
        let mut state = self.state();
        if matches!(*state, State::Gone(_)) {
            return false;
        }
        *state = State::Gone(why);
        drop(state);
        self.changed.notify_all();
        if kill {
            self.kill();
        }
        true
    }

    fn kill(&self) {
        self.process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kill();
    }
}

/// The browser's hold on a renderer. Dropping it kills the renderer.
pub struct Renderer {
    number: u32,
    sender: Sender<ToRenderer>,
    shared: Arc<Shared>,
}

impl Renderer {
    /// Starts renderer `number` with `launcher` and sends it the hello.
    /// What it does goes to `events` with its number, each followed by a
    /// call of `wake`; the last is its [`Event::Gone`].
    /// Messages after the hello go first to `serve`, if given.
    pub fn start(
        launcher: &dyn Launcher,
        number: u32,
        events: mpsc::Sender<(u32, Event)>,
        wake: Waker,
        serve: Option<Arc<dyn Serve>>,
    ) -> io::Result<Renderer> {
        start_within(launcher, number, events, wake, serve, HELLO_TIMEOUT)
    }

    /// The renderer's number.
    pub fn number(&self) -> u32 {
        self.number
    }

    /// The operating system's number for its process.
    pub fn process_id(&self) -> u32 {
        self.shared
            .process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .id()
    }

    /// Where messages for it are written.
    pub(crate) fn sender(&self) -> Sender<ToRenderer> {
        self.sender.clone()
    }

    /// Sends `message`.
    pub fn send(&self, message: &ToRenderer) -> Result<(), Closed> {
        self.sender.send(message)
    }

    /// Whether it answered the hello and is not gone.
    pub fn is_ready(&self) -> bool {
        *self.shared.state() == State::Ready
    }

    /// Why it is gone, if it is.
    pub fn gone(&self) -> Option<Gone> {
        match &*self.shared.state() {
            State::Gone(why) => Some(why.clone()),
            _ => None,
        }
    }

    /// Waits at most `timeout` for it to answer the hello or go; says
    /// whether it is ready.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let state = self.shared.state();
        let (state, _) = self
            .shared
            .changed
            .wait_timeout_while(state, timeout, |state| *state == State::Starting)
            .unwrap_or_else(PoisonError::into_inner);
        *state == State::Ready
    }

    /// Kills it, as from outside: its end comes as any end does.
    pub fn kill(&self) {
        self.shared.kill();
    }

    /// Kills it for having sent `what`, which the protocol does not
    /// allow.
    pub fn bad_message(&self, what: impl Into<String>) {
        self.shared.end(Gone::BadMessage(what.into()), true);
    }

    /// Asks it to end: it ends on its own, or when the browser lets go of
    /// it.
    pub fn shutdown(&self) {
        // Said first: the renderer may end at once, and its end is then
        // the shutdown's, not its channel's.
        self.shared.end(Gone::Shutdown, false);
        let _ = self.sender.send(&ToRenderer::Shutdown {});
        self.sender.close();
    }

    /// Its exit code, once its process has ended.
    pub fn exit_code(&self) -> Option<i64> {
        self.shared
            .process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .exit_code()
    }

    /// The file a dump of it was written to, if it crashed and one was.
    pub fn dump(&self) -> Option<std::path::PathBuf> {
        self.shared
            .process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dump()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.sender.close();
        self.shared.end(Gone::Shutdown, false);
        self.shared.kill();
    }
}

/// [`Renderer::start`], with `timeout` for the hello.
pub(super) fn start_within(
    launcher: &dyn Launcher,
    number: u32,
    events: mpsc::Sender<(u32, Event)>,
    wake: Waker,
    serve: Option<Arc<dyn Serve>>,
    timeout: Duration,
) -> io::Result<Renderer> {
    let launched = launcher.launch(number)?;
    let shared = Arc::new(Shared {
        state: Mutex::new(State::Starting),
        changed: Condvar::new(),
        process: Mutex::new(launched.process),
    });
    let sender = Sender::new(launched.to_renderer);
    if let Some(log) = launched.log {
        thread::Builder::new()
            .name(format!("renderer-{number}-log"))
            .spawn(move || {
                for line in BufReader::new(log).lines() {
                    let Ok(line) = line else { break };
                    eprintln!("renderer {number}: {line}");
                }
            })?;
    }
    let reading = shared.clone();
    let ending = shared.clone();
    let reply = sender.clone();
    let (deliver, delivered, wake_too) = (events.clone(), wake.clone(), wake.clone());
    let budget = Arc::new(Budget::default());
    budget
        .parked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .to = Some((events.clone(), number));
    ipc::spawn_reader_sized::<ToBrowser, _>(
        &format!("renderer-{number}-reader"),
        launched.from_renderer,
        move |message, len| {
            let event = {
                let mut state = reading.state();
                match (&*state, message) {
                    (State::Starting, ToBrowser::Hello { version }) if version == IPC_VERSION => {
                        *state = State::Ready;
                        reading.changed.notify_all();
                        Some(Event::Ready)
                    }
                    (State::Starting, message) => {
                        drop(state);
                        reading.end(
                            Gone::BadMessage(format!(
                                "{message:?} before a hello of version {IPC_VERSION}"
                            )),
                            true,
                        );
                        None
                    }
                    (State::Ready, ToBrowser::Hello { .. }) => {
                        drop(state);
                        reading.end(Gone::BadMessage("a second hello".into()), true);
                        None
                    }
                    // A sweep build's corpus (`dump-scripts`): the browser
                    // writes what the sandboxed renderer cannot, naming the
                    // file itself; a module flag not 0 or 1, or a kind not
                    // known, drops it.
                    #[cfg(feature = "dump-scripts")]
                    (
                        State::Ready,
                        ToBrowser::DumpScript {
                            url,
                            text,
                            module,
                            kind,
                        },
                    ) => {
                        drop(state);
                        if module <= 1 {
                            js::dump::write(&text, &url, module == 1, &kind);
                        }
                        None
                    }
                    (State::Ready, message) => {
                        drop(state);
                        let served = match &serve {
                            Some(serve) => serve.serve(number, message, &reply),
                            None => Ok(Some(message)),
                        };
                        match served {
                            Ok(None) => None,
                            Ok(Some(message)) => budget.take(message, len, &reading),
                            Err(why) => {
                                reading.end(Gone::BadMessage(why), true);
                                None
                            }
                        }
                    }
                    (State::Gone(_), _) => None,
                }
            };
            if let Some(event) = event {
                // A renderer's word that it took input wakes nothing: the
                // browser's next turn takes it, before it asks whether a
                // page is unresponsive. Woken by it, the browser drew a
                // frame, the frame moved the pointer, and the move was
                // input again: a core each in the browser and the
                // renderer on a page left alone (the idle CPU test).
                let quiet = matches!(&event, Event::Message(message, _)
                    if matches!(**message, ToBrowser::InputTaken { .. }));
                let _ = deliver.send((number, event));
                if !quiet {
                    delivered();
                }
            }
        },
        move |reason| {
            let why = match reason {
                Some(ReadError::Malformed(malformed)) => Gone::BadMessage(malformed.to_string()),
                Some(ReadError::TooLong(len)) => {
                    Gone::BadMessage(format!("a message of {len} bytes"))
                }
                None | Some(ReadError::Io(_)) => Gone::Ended,
            };
            let bad = matches!(why, Gone::BadMessage(_));
            ending.end(why, bad);
            let why = match &*ending.state() {
                State::Gone(why) => why.clone(),
                _ => Gone::Ended,
            };
            let _ = events.send((number, Event::Gone(why)));
            wake_too();
        },
    )?;
    let watching = shared.clone();
    thread::Builder::new()
        .name(format!("renderer-{number}-hello"))
        .spawn(move || {
            let state = watching.state();
            let (state, _) = watching
                .changed
                .wait_timeout_while(state, timeout, |state| *state == State::Starting)
                .unwrap_or_else(PoisonError::into_inner);
            if *state == State::Starting {
                drop(state);
                watching.end(Gone::NoHello, true);
            }
        })?;
    let renderer = Renderer {
        number,
        sender,
        shared,
    };
    // A renderer that cannot be written to is gone already, and its
    // reader says so.
    let _ = renderer.sender.send(&ToRenderer::Hello {
        version: IPC_VERSION,
        process: number,
    });
    Ok(renderer)
}

/// Starts renderers as threads of this process, over pipes in memory:
/// the whole of the protocol without a process, for the tests (M34.6).
#[derive(Default, Clone)]
pub struct ThreadLauncher {
    fonts: Option<text::FontCollection>,
}

impl ThreadLauncher {
    /// A launcher whose renderers draw with `fonts`, not the system's.
    pub fn with_fonts(fonts: text::FontCollection) -> ThreadLauncher {
        ThreadLauncher { fonts: Some(fonts) }
    }
}

struct ThreadRenderer {
    thread: Option<thread::JoinHandle<Result<(), String>>>,
    /// Its exit code once its thread was joined.
    code: Option<i64>,
    breakers: [PipeBreaker; 2],
}

impl RendererProcess for ThreadRenderer {
    fn kill(&mut self) {
        // A thread cannot be killed: its pipes are broken, so that it
        // reads their end and stops.
        for breaker in &self.breakers {
            breaker.break_pipe();
        }
    }

    fn id(&self) -> u32 {
        0
    }

    fn exit_code(&mut self) -> Option<i64> {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
        {
            self.code = self.thread.take().map(|thread| match thread.join() {
                Ok(Ok(())) => 0,
                Ok(Err(_)) => 1,
                Err(_) => 101,
            });
        }
        self.code
    }
}

impl Launcher for ThreadLauncher {
    fn launch(&self, number: u32) -> io::Result<Launched> {
        let ((browser_in, browser_out), (renderer_in, renderer_out)) = ipc::duplex();
        let breakers = [browser_out.breaker(), renderer_out.breaker()];
        let fonts = self.fonts.clone();
        let thread = thread::Builder::new()
            .name(format!("renderer-{number}"))
            .stack_size(crate::STACK_SIZE)
            .spawn(move || crate::renderer::run_with(renderer_in, renderer_out, fonts))?;
        Ok(Launched {
            from_renderer: Box::new(browser_in),
            to_renderer: Box::new(browser_out),
            log: None,
            process: Box::new(ThreadRenderer {
                thread: Some(thread),
                code: None,
                breakers,
            }),
        })
    }

    /// The GPU process as a thread, over pipes in memory (M41).
    #[cfg(feature = "gpu")]
    fn launch_gpu(&self) -> io::Result<Launched> {
        let ((browser_in, browser_out), (gpu_in, gpu_out)) = ipc::duplex();
        let breakers = [browser_out.breaker(), gpu_out.breaker()];
        let thread = thread::Builder::new()
            .name("gpu-process".to_owned())
            .spawn(move || crate::gpu::process::run(gpu_in, gpu_out))?;
        Ok(Launched {
            from_renderer: Box::new(browser_in),
            to_renderer: Box::new(browser_out),
            log: None,
            process: Box::new(ThreadRenderer {
                thread: Some(thread),
                code: None,
                breakers,
            }),
        })
    }
}

#[cfg(test)]
mod budget_tests {
    //! The budget's parking under every interleaving (the lead's review,
    //! 2026-11-26): a reader and a browser taking at random, no message
    //! lost, none out of order, none left parked. `SILVER_BUDGET_STRESS=
    //! long` runs many more (the extended check).

    use super::*;

    struct Nothing;

    impl RendererProcess for Nothing {
        fn kill(&mut self) {}
        fn id(&self) -> u32 {
            0
        }
        fn exit_code(&mut self) -> Option<i64> {
            None
        }
    }

    fn shared() -> Shared {
        Shared {
            state: Mutex::new(State::Ready),
            changed: Condvar::new(),
            process: Mutex::new(Box::new(Nothing)),
        }
    }

    /// xorshift64: the same interleavings for the same seed.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    fn stress(seed: u64, n: u32) {
        let (to, from) = mpsc::channel::<(u32, Event)>();
        // Always stalled: it parks, never ends; a budget of 8 and parked
        // messages of 8, so that parking and unparking happen at every turn.
        let budget = Arc::new(Budget::with_limits(
            8,
            1 << 20,
            Duration::ZERO,
            Duration::ZERO,
        ));
        budget
            .parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .to = Some((to.clone(), 1));
        let reading = Arc::new(shared());
        let producer = {
            let (budget, reading) = (Arc::clone(&budget), Arc::clone(&reading));
            thread::spawn(move || {
                let mut rng = Rng(seed);
                for seq in 0..n {
                    let message = ToBrowser::FocusLeft { view: seq };
                    if let Some(event) = budget.take(message, 16, &reading) {
                        to.send((1, event)).expect("the browser listens");
                    }
                    if rng.next().is_multiple_of(8) {
                        thread::yield_now();
                    }
                }
            })
        };
        let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
        let mut held: VecDeque<Event> = VecDeque::new();
        let mut seen = 0;
        let began = Instant::now();
        while seen < n {
            assert!(
                began.elapsed() < Duration::from_secs(60),
                "seed {seed}: {seen} of {n} after a minute: one lost or stuck parked"
            );
            match from.try_recv() {
                Ok((_, event)) => {
                    let Event::Message(message, _) = &event else {
                        panic!("seed {seed}: {event:?}");
                    };
                    let ToBrowser::FocusLeft { view } = **message else {
                        panic!("seed {seed}: {message:?}");
                    };
                    assert_eq!(view, seen, "seed {seed}: out of order");
                    seen += 1;
                    held.push_back(event);
                }
                // Nothing to take: the browser takes what it held, as a
                // browser does in the end.
                Err(_) => {
                    if held.pop_front().is_none() {
                        thread::yield_now();
                    }
                }
            }
            // Taken at random, some kept a while.
            while held.len() > (rng.next() % 12) as usize {
                held.pop_front();
            }
            if rng.next().is_multiple_of(5) {
                thread::yield_now();
            }
        }
        held.clear();
        producer.join().expect("the reader ran");
        assert_eq!(budget.untaken(), 0, "seed {seed}: all given back");
        assert_eq!(budget.parked(), 0, "seed {seed}: none left parked");
        assert!(from.try_recv().is_err(), "seed {seed}: none more");
    }

    #[test]
    fn parking_loses_no_message_keeps_their_order_and_leaves_none_parked() {
        let long = std::env::var("SILVER_BUDGET_STRESS").is_ok_and(|v| v == "long");
        let (seeds, n) = if long {
            (1..=16, 200_000)
        } else {
            (1..=4, 20_000)
        };
        for seed in seeds {
            stress(seed, n);
        }
    }
}
