//! Scripts (M24, `docs/design/scripting.md`): a JavaScript engine behind
//! traits of the engine's own, the boundary scripts reach the document
//! through, and the limits a hostile script cannot get past. A leaf
//! crate: it depends on no crate of the engine's, so a script can touch
//! nothing but what its [`ScriptingHost`] is given, and it writes no
//! `unsafe`; QuickJS-ng is reached through `rquickjs`'s safe interface.

#![forbid(unsafe_code)]

mod dom;
mod quickjs;

#[cfg(test)]
mod dom_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod test_tree;
#[cfg(test)]
mod tests;

pub use dom::{Answer, Call, DomError, Handle, MAX_HANDLE, NestedScript, WORK_PER_STEP};
pub use quickjs::QuickJs;

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
pub const MAX_SCRIPT_STACK: usize = 256 << 10;

/// What a realm may use; each field defaults to its constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// [`MAX_SCRIPT_HEAP`].
    pub heap: usize,
    /// [`MAX_SCRIPT_STEPS`].
    pub steps: u32,
    /// [`MAX_SCRIPT_STACK`].
    pub stack: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            heap: MAX_SCRIPT_HEAP,
            steps: MAX_SCRIPT_STEPS,
            stack: MAX_SCRIPT_STACK,
        }
    }
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
}

/// Why a realm can run nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Death {
    /// Its heap stayed full after a run failed.
    Heap,
}

/// A console method's level (Console Standard section 1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLevel {
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
    /// `document.title`'s getter (HTML section 3.1.7).
    fn title(&mut self) -> String;
    /// `document.title`'s setter.
    fn set_title(&mut self, title: String);
    /// What the bindings ask of the document's nodes (M25): the call's
    /// answer, or the exception it throws. A host without a document
    /// answers none.
    fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        let _ = call;
        Err(DomError::not_supported())
    }
    /// The units of work done since this was last asked, which the
    /// realm charges to the run's budget ([`WORK_PER_STEP`]).
    fn take_work(&mut self) -> u64 {
        0
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
    /// A new realm, a global object with its heap, under `limits`.
    fn realm<H: ScriptingHost>(&self, limits: Limits) -> Result<Box<dyn Realm<H>>, ScriptError>;
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
/// (`bubbles`, `cancelable`, `clientX`, ...).
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
