//! The renderers the browser starts (M34.2, docs/design/processes.md,
//! "IPC"): a [`Launcher`] starts one (a process of the shell's
//! executable, or a thread in the tests), and a [`Renderer`] is the
//! browser's hold on it: its channel, its process, and what it says,
//! handed to the browser's thread as [`Event`]s. A renderer's first
//! message must be its hello, of this schema's version, within
//! [`HELLO_TIMEOUT`]; one that says anything else first, or nothing, is
//! killed.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

use crate::ipc::{
    self, Closed, IPC_VERSION, PipeBreaker, ReadError, Sender, ToBrowser, ToRenderer,
};
use crate::loader::Waker;

/// How long a renderer has to answer the hello before it is killed.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// What starts renderers.
pub trait Launcher: Send + Sync {
    /// Starts renderer number `number`.
    fn launch(&self, number: u32) -> io::Result<Launched>;
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

/// What a renderer did, for the browser's thread.
#[derive(Debug)]
pub enum Event {
    /// It answered the hello.
    Ready,
    /// A message after its hello.
    Message(ToBrowser),
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
    pub fn start(
        launcher: &dyn Launcher,
        number: u32,
        events: mpsc::Sender<(u32, Event)>,
        wake: Waker,
    ) -> io::Result<Renderer> {
        start_within(launcher, number, events, wake, HELLO_TIMEOUT)
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

    /// Kills it for having sent `what`, which the protocol does not
    /// allow.
    pub fn bad_message(&self, what: impl Into<String>) {
        self.shared.end(Gone::BadMessage(what.into()), true);
    }

    /// Asks it to end: it ends on its own, or when the browser lets go of
    /// it.
    pub fn shutdown(&self) {
        let _ = self.sender.send(&ToRenderer::Shutdown {});
        self.sender.close();
        self.shared.end(Gone::Shutdown, false);
    }

    /// Its exit code, once its process has ended.
    pub fn exit_code(&self) -> Option<i64> {
        self.shared
            .process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .exit_code()
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
    let (deliver, delivered, wake_too) = (events.clone(), wake.clone(), wake.clone());
    ipc::spawn_reader::<ToBrowser, _>(
        &format!("renderer-{number}-reader"),
        launched.from_renderer,
        move |message| {
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
                    #[allow(unreachable_patterns)]
                    (State::Ready, message) => Some(Event::Message(message)),
                    (State::Gone(_), _) => None,
                }
            };
            if let Some(event) = event {
                let _ = deliver.send((number, event));
                delivered();
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
#[derive(Debug, Default, Clone, Copy)]
pub struct ThreadLauncher;

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
        let thread = thread::Builder::new()
            .name(format!("renderer-{number}"))
            .stack_size(32 << 20)
            .spawn(move || crate::renderer::run(renderer_in, renderer_out))?;
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
