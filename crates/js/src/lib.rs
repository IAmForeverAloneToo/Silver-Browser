//! Scripts (M24, `docs/design/scripting.md`): a JavaScript engine behind
//! traits of the engine's own, the boundary scripts reach the document
//! through, and the limits a hostile script cannot get past. A leaf
//! crate: it depends on no crate of the engine's, so a script can touch
//! nothing but what its [`ScriptingHost`] is given, and it writes no
//! `unsafe` but one reviewed call, which reads the bindings' bytecode
//! this process wrote (M35.12, `quickjs/bytecode.rs`); QuickJS-ng is
//! otherwise reached through `rquickjs`'s safe interface.

// Denied rather than forbidden, so that the one module that reads the
// bindings' bytecode may allow it (M35.12); the shell's test of where
// `unsafe` is written admits that module and no other.
#![deny(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

mod dom;
mod pattern;
mod quickjs;

#[cfg(test)]
mod clone_tests;
#[cfg(test)]
mod dom_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod test_tree;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wasm_tests;
#[cfg(test)]
mod worker_tests;

pub use dom::{
    Answer, Call, CanvasCall, DomError, FetchCall, Handle, MAX_HANDLE, NestedScript, WORK_PER_STEP,
};
pub use pattern::pattern_matches;
pub use quickjs::{QuickJs, warm_bindings};

/// The most heap a realm may hold, in bytes. An allocation past it
/// throws; one that escapes a run with the heap still full leaves the
/// realm dead ([`Death::Heap`]).
pub const MAX_SCRIPT_HEAP: usize = 256 << 20;
/// The interrupt checks a run may take, each about 10,000 of
/// QuickJS-ng's operations: 2 to 5 seconds of running code. Past them
/// the script is stopped ([`ErrorKind::Budget`]).
pub const MAX_SCRIPT_STEPS: u32 = 10_000;
/// The native stack a realm's scripts may use, in bytes. QuickJS-ng's
/// default is a megabyte, the whole of a Windows main thread's stack.
/// Twice this in a debug build (M38): QuickJS measures from where a
/// realm's runtime began (G165), and the engine's own frames between
/// there and where its scripts run, a frame's inside its parent's, are
/// some five times larger unoptimized (QuickJS itself is optimized in
/// every build); Windows CI's timing ran out of the release bound there.
pub const MAX_SCRIPT_STACK: usize = if cfg!(debug_assertions) {
    512 << 10
} else {
    256 << 10
};

/// What a realm may use; each field defaults to its constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// [`MAX_SCRIPT_HEAP`].
    pub heap: usize,
    /// [`MAX_SCRIPT_STEPS`].
    pub steps: u32,
    /// [`MAX_SCRIPT_STACK`].
    pub stack: usize,
    /// Whether the realm is a secure context's (M39): the interfaces Web
    /// IDL marks `[SecureContext]` (the Cache API, service workers) are
    /// its alone. True by default.
    pub secure: bool,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            heap: MAX_SCRIPT_HEAP,
            steps: MAX_SCRIPT_STEPS,
            stack: MAX_SCRIPT_STACK,
            secure: true,
        }
    }
}

impl Limits {
    /// A worker's (M31): no step budget, since a worker is there to
    /// compute for long, and the page's heap and stack.
    pub fn worker() -> Limits {
        Limits {
            steps: u32::MAX,
            ..Limits::default()
        }
    }
}

/// The global object a realm is made with (M31).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RealmKind {
    /// A document's `Window`.
    #[default]
    Window,
    /// A dedicated worker's `DedicatedWorkerGlobalScope` (HTML section
    /// 10.2.1.2), with only the interfaces Web IDL exposes there, and
    /// the name its creator gave it.
    DedicatedWorker { name: String },
    /// A service worker's `ServiceWorkerGlobalScope` (M39, Service
    /// Workers 4.1), with the interfaces Web IDL exposes there.
    ServiceWorker,
    /// The browser's DevTools panel's page (M36, docs/design/devtools.md):
    /// a window with the global `devtools`, its bridge to the browser.
    DevTools,
}

/// A classic script to run (HTML section 8.1.4.1).
#[derive(Debug, Clone, Copy)]
pub struct Script<'a> {
    /// Its source text.
    pub source: &'a str,
    /// The URL its errors name: the document's for an inline script.
    pub url: &'a str,
    /// The line of its document its text starts on, from 1, so that an
    /// inline script's errors name the document's lines.
    pub line: u32,
    /// Whether its errors are muted (section 8.1.4.6): it came from
    /// another origin, whose details a page may not learn.
    pub muted: bool,
}

/// The value a script ends with, kept as far as a `javascript:` URL
/// needs it (HTML section 7.4.2.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    Undefined,
    String(String),
    Other,
}

/// What failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// A syntax error or an exception the script did not catch.
    Exception,
    /// The step budget ran out and the script was stopped.
    Budget,
    /// The heap ran out.
    Heap,
    /// A promise rejected with no handler by the end of a checkpoint
    /// (section 8.1.4.7).
    Rejection,
    /// The realm is dead and ran nothing.
    Dead,
    /// An uncaught exception a listener of the window's `error` event
    /// canceled (HTML section 8.1.4.6, M26): reported to the page, not to
    /// the console.
    Handled,
}

/// An error a script reported: what failed, its message as the
/// reference browser words it (`TypeError: x is not a function`), and
/// where, or no URL and no position when muted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub kind: ErrorKind,
    pub message: String,
    pub url: String,
    pub line: u32,
    pub column: u32,
    /// An `Error`'s stack, innermost first, at most [`MAX_STACK_FRAMES`]
    /// (M36): none when muted, or for a value that is no `Error`.
    pub stack: Vec<StackFrame>,
}

/// The most frames a stack keeps (M36).
pub const MAX_STACK_FRAMES: usize = 64;

/// The most bytes of a console call's previews (M36): past it, the call
/// has its text alone.
pub const MAX_PREVIEW_BYTES: usize = 64 << 10;

/// A frame of a stack (M36): the function called, and where it was.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackFrame {
    /// Its name, empty for an anonymous function or a script's body.
    pub function: String,
    pub url: String,
    pub line: u32,
    pub column: u32,
}

/// What a console call was (Console Standard, M36): a log of its
/// arguments, or one of the calls the DevTools panel shows its own way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConsoleKind {
    #[default]
    Log,
    /// `dir`: its one argument, open.
    Dir,
    /// `table`: its argument's rows and columns.
    Table,
    /// `trace`: with the stack where it was called.
    Trace,
    /// `group` and `groupCollapsed`: the entries until its end inside it.
    Group,
    GroupCollapsed,
    GroupEnd,
    /// `clear`: the entries before it dropped from view.
    Clear,
}

/// A console call (M36): its level, kind and text, as the shell prints
/// it; its arguments' previews, a JSON array (docs/design/devtools.md,
/// "The console"), empty past [`MAX_PREVIEW_BYTES`]; and the stack where
/// it was called, for an error, an assertion or a trace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsoleRecord {
    pub level: ConsoleLevel,
    pub kind: ConsoleKind,
    pub text: String,
    pub previews: String,
    pub stack: Vec<StackFrame>,
}

/// Why a realm can run nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Death {
    /// Its heap stayed full after a run failed.
    Heap,
}

/// A console method's level (Console Standard section 1.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConsoleLevel {
    #[default]
    Log,
    Info,
    Warn,
    Error,
    Debug,
}

/// Everything a script can do to the engine (M24.4): plain data in and
/// out, no reference to anything of the engine's. A run moves the host
/// into the realm and back, so it must have a value to leave behind
/// meanwhile: its default.
pub trait ScriptingHost: Default + 'static {
    /// A console method was called; `message` is formatted.
    fn console(&mut self, level: ConsoleLevel, message: String);
    /// A console method was called (M36): the record of it, its text
    /// formatted. A host that keeps no more than the text has it alone.
    fn console_record(&mut self, record: ConsoleRecord) {
        self.console(record.level, record.text);
    }
    /// `document.title`'s getter (HTML section 3.1.7).
    fn title(&mut self) -> String;
    /// `document.title`'s setter.
    fn set_title(&mut self, title: String);
    /// What the bindings ask of the document's nodes (M25): the call's
    /// answer, or the exception it throws. A host without a document
    /// answers none, but that code from strings and WebAssembly may be
    /// compiled, no document's policy saying otherwise (M33.1).
    fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        match call {
            Call::CodeAllowed { .. } => Ok(Answer::String(String::new())),
            _ => Err(DomError::not_supported()),
        }
    }
    /// The units of work done since this was last asked, which the
    /// realm charges to the run's budget ([`WORK_PER_STEP`]).
    fn take_work(&mut self) -> u64 {
        0
    }
    /// Whether records were queued for mutation observers since this was
    /// last asked (gap batch A): the realm queues its microtask.
    fn take_mutation_wake(&mut self) -> bool {
        false
    }
    /// The next script to run now, inside the call that made it ready
    /// (M25): one a script inserted, or one in what `document.write`
    /// wrote; `None` once there is none. The realm runs it, nested in the
    /// running script, and says when it has run.
    fn take_script(&mut self) -> Option<NestedScript> {
        None
    }
    /// The script [`ScriptingHost::take_script`] gave has run.
    fn script_ran(&mut self) {}
    /// A script run nested failed: an error of the page's, which the
    /// script that made it run does not see.
    fn report(&mut self, error: ScriptError) {
        let _ = error;
    }
}

/// A JavaScript engine (M24.1, roadmap D1): makes realms.
pub trait ScriptEngine {
    /// A new realm, a window with its heap, under `limits`.
    fn realm<H: ScriptingHost>(&self, limits: Limits) -> Result<Box<dyn Realm<H>>, ScriptError> {
        self.realm_of(RealmKind::Window, limits, None)
    }
    /// A new realm of `kind` under `limits` (M31), which `stop` stops:
    /// once it is set, what runs ends at its next interrupt check as a
    /// run whose budget ran out (a worker's `terminate()`).
    fn realm_of<H: ScriptingHost>(
        &self,
        kind: RealmKind,
        limits: Limits,
        stop: Option<Arc<AtomicBool>>,
    ) -> Result<Box<dyn Realm<H>>, ScriptError>;
}

/// What the engine asks a realm to do besides running a script (M26,
/// `docs/design/events.md`): run the callbacks a page gave, which stay
/// in the realm, named here by plain data.
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    /// Runs timer `id`'s callback (HTML section 8.6), which sets it
    /// again if it repeats.
    Timer(u32),
    /// Dispatches an event the engine makes, a trusted one, at `target`
    /// (DOM section 2.9), with a microtask checkpoint after each
    /// listener it calls.
    Dispatch { target: Target, event: EventInit },
    /// Runs the animation frame callbacks (HTML section 8.10) with the
    /// frame's time, in milliseconds on the document's clock, a
    /// microtask checkpoint after each.
    AnimationFrames(f64),
    /// Settles `fetch()` `id`'s promise (M27.1) with the response a
    /// script may see, or rejects it with a network error for `None`.
    Fetched {
        id: u32,
        answer: Option<Box<FetchAnswer>>,
    },
    /// Delivers a message posted to `target` (M31, HTML section 9): the
    /// serialized form of its data, which the realm makes again, and
    /// the origin of the realm that sent it; a `messageerror` when the
    /// data cannot be made again.
    Message {
        target: MessageTarget,
        data: String,
        origin: String,
    },
    /// Worker `worker`'s error (M31.2): an exception it did not handle,
    /// as the console words it, fired at its `Worker` object and then,
    /// unhandled, reported in this realm; `None` for a script that could
    /// not be fetched or run, a plain `error` event.
    WorkerError {
        worker: u32,
        error: Option<ScriptError>,
    },
    /// Worker `worker` ended: its object hears nothing more.
    WorkerEnded(u32),
    /// The window's child frames are now as many as these names, their
    /// elements' (M33.2): its frames by index and name, made again.
    FrameList(Vec<String>),
    /// The document's policies would refuse or report code from strings
    /// (M33.1): `eval` and the function constructors ask the host from
    /// now on.
    GuardCode,
    /// A message from the browser to the DevTools panel's page (M36):
    /// `devtools.onmessage` is called with it.
    DevTools(String),
    /// The DevTools console's `expression`, run in the realm's global
    /// scope as a script of its own, the page's policies aside, with `$0`
    /// the node `selected` names (M36): its value's preview, or its
    /// exception's, is given to the host by [`dom::Call::DevToolsAnswer`].
    Evaluate {
        expression: String,
        selected: Option<Handle>,
    },
    /// IndexedDB's next request completed (M36.7): its event is
    /// dispatched, a microtask checkpoint after each listener.
    Idb,
    /// What the service workers' host tells the realm (M39), as JSON: a
    /// job's promise settled, a registration's or a version's change,
    /// or an event for a service worker's global.
    ServiceWorker(String),
    /// What the GPU process tells a document's WebGPU (M41), as JSON,
    /// and the bytes it carries (a mapped range's).
    Gpu(String, Vec<u8>),
    /// What a document's Web Audio tells its realm (M40), as JSON, and
    /// the samples it carries: a source ended, a context's state, a
    /// buffer decoded or rendered.
    Audio(String, Vec<u8>),
    /// What a document's media elements do (M40), as JSON: actions to
    /// carry out, or a pipeline's report to hear.
    Media(String),
    /// Records wait for mutation observers that no call of a script's
    /// queued (the parser's insertions): the realm tells them.
    Mutations,
    /// The `matchMedia()` lists whose answer changed in a frame's update
    /// (gap batch A), by their numbers: a `change` at each.
    MediaChanged(Vec<u32>),
}

/// Where a message is delivered (M31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageTarget {
    /// The realm's global: the window, by `window.postMessage`, or a
    /// worker's, from its owner.
    Global,
    /// The `Worker` object of worker `n`, from its worker.
    Worker(u32),
    /// `MessagePort` `n` (M31.5), from its entangled port.
    Port(u32),
    /// The window, from frame `n`'s (M33.2): its `source`.
    Frame(u32),
}

/// What a script may see of a response (M27.1): `net`'s exposed view,
/// as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchAnswer {
    /// `basic`, `cors`, `opaque` or `opaqueredirect`.
    pub response_type: &'static str,
    pub url_list: Vec<String>,
    pub status: u16,
    pub status_text: String,
    /// Each field's name and value, as Latin-1 decodes their bytes.
    pub headers: Vec<(String, String)>,
    /// None for an opaque or opaque-redirect response.
    pub body: Option<Vec<u8>>,
}

/// Where an event the engine makes is dispatched: a node, the document
/// being one, or the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Node(Handle),
    Window,
}

/// An event the engine makes, as data: the interface it is an object of
/// (`Event`, `MouseEvent`, ...), its type, and the members of that
/// interface's dictionary it does not leave at their defaults, by name
/// (`bubbles`, `cancelable`, `clientX`, ...); a mouse event's `pageX`,
/// `pageY`, `offsetX` and `offsetY` too, which no dictionary has.
#[derive(Debug, Clone, PartialEq)]
pub struct EventInit {
    pub interface: &'static str,
    pub kind: String,
    pub fields: Vec<(&'static str, Field)>,
}

impl EventInit {
    /// An event of `interface` and type `kind` with its defaults.
    pub fn new(interface: &'static str, kind: impl Into<String>) -> EventInit {
        EventInit {
            interface,
            kind: kind.into(),
            fields: Vec::new(),
        }
    }

    /// The event with member `name` set to `value`.
    pub fn with(mut self, name: &'static str, value: Field) -> EventInit {
        self.fields.push((name, value));
        self
    }
}

/// A member's value in an [`EventInit`].
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Bool(bool),
    Number(f64),
    String(String),
    /// A node (a `relatedTarget`, a `submitter`), or `null`.
    Node(Option<Handle>),
    /// The window (a `view`).
    Window,
    /// `null` (a `key` a `clear()` leaves `null`, M28).
    Null,
    /// The window's `localStorage` (`true`) or `sessionStorage`: a
    /// `storageArea` (M28).
    Storage(bool),
}

/// A realm: a global object and the heap its scripts share.
pub trait Realm<H: ScriptingHost> {
    /// Parses and runs a classic script with `host` and a step budget
    /// of its own; what it ended with, or why it failed. The caller
    /// performs the microtask checkpoint that follows.
    fn run(&mut self, script: &Script<'_>, host: &mut H) -> Result<Completion, ScriptError>;
    /// Runs `task` with `host` and a step budget of its own, as
    /// [`Realm::run`] runs a script: whether the event it dispatched was
    /// canceled (false for a task with no event), or why it failed. The
    /// caller performs the microtask checkpoint that follows.
    fn enter(&mut self, task: &Task, host: &mut H) -> Result<bool, ScriptError>;
    /// Runs the promise jobs queued (HTML section 8.1.7.3), with a step
    /// budget of its own, and reports the rejections left unhandled.
    fn checkpoint(&mut self, host: &mut H) -> Vec<ScriptError>;
    /// The heap in use, in bytes.
    fn heap(&self) -> usize;
    /// Collects the garbage now.
    fn collect(&mut self);
    /// Why the realm can run nothing more, if it cannot.
    fn dead(&self) -> Option<Death>;
}
