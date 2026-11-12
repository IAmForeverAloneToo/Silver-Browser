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
mod stack;

#[cfg(test)]
mod chunk_tests;
#[cfg(test)]
mod clone_tests;
#[cfg(test)]
mod dom_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod intl_tests;
#[cfg(test)]
mod module_tests;
#[cfg(test)]
mod test_tree;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wasm_tests;
#[cfg(test)]
mod worker_tests;

pub use dom::{
    Answer, Call, CanvasCall, CustomCall, DomError, FetchCall, Handle, MAX_HANDLE, NestedScript,
    WORK_PER_STEP,
};
pub use pattern::pattern_matches;
pub use quickjs::{QuickJs, set_windows_time_zone, warm_bindings};
pub use stack::remaining_stack;

/// The most heap a realm may hold, in bytes. An allocation past it
/// throws; one that escapes a run with the heap still full leaves the
/// realm dead ([`Death::Heap`]).
pub const MAX_SCRIPT_HEAP: usize = 256 << 20;
/// The interrupt checks a run may take, each about 10,000 of
/// QuickJS-ng's operations: 2 to 5 seconds of running code. Past them
/// the script is stopped ([`ErrorKind::Budget`]).
pub const MAX_SCRIPT_STEPS: u32 = 10_000;
/// The native stack a realm's scripts may use, in bytes, measured from
/// where its runtime began (G165): a call of a script's function takes
/// some 3.5 KiB of it, QuickJS's frame in C, in either build (G253,
/// measured: 2,360 frames in 8 MiB), so that this gives a page some
/// 4,700 frames, where it was 256 KiB, fewer than a hundred. A realm is
/// made only on a thread with this much left and [`NATIVE_STACK`] under
/// it ([`remaining_stack`]).
///
/// No more than 16 MiB: rquickjs takes a larger limit for none at all
/// (its `set_max_stack_size`, so that QuickJS's subtraction from the
/// stack's top cannot underflow), and a script's recursion would then
/// end the process. Browsers give some ten thousand frames; if a page
/// needs them, QuickJS's own limit set past rquickjs is the lever (G253).
pub const MAX_SCRIPT_STACK: usize = 16 << 20;

const _: () = assert!(
    MAX_SCRIPT_STACK <= 16 << 20,
    "rquickjs takes more for no limit"
);

/// The stack a thread that runs scripts keeps under their limit, in
/// bytes: the deepest native work a script's call can start when its
/// stack is all but spent, a layout of a page as deep as the boxes go
/// (`layout::MAX_BOX_DEPTH`; tables' boxes about cells, shadow trees'
/// slots), its style and its boxes, in a debug build. Measured (G253):
/// a script recursing to its limit, then laying out each of those pages
/// at the bottom, needs a thread of 27 MiB, 11 MiB of it under the
/// scripts' 16; this keeps half as much again.
pub const NATIVE_STACK: usize = 16 << 20;

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
    /// The base URL its `import()`s resolve against (HTML's script's base
    /// URL): an inline script's document's when it was prepared; empty
    /// for `url`.
    pub base: &'a str,
    /// Its fetch options by the host's number for them (HTML's script
    /// fetch options), which its `import()`s fetch with: 0 for the
    /// defaults.
    pub options: u32,
}

/// The longest module specifier a realm resolves, in bytes (D61): the
/// reference browser's longest URL. A longer one throws a `TypeError`.
pub const MAX_SPECIFIER_BYTES: usize = 2 << 20;

/// The deepest module graph a realm links (D61), its root at depth 0:
/// the engine fetches none deeper. Its modules given leaves first, a
/// realm links a graph twice as deep within [`MAX_SCRIPT_STACK`] in a
/// release build, measured; QuickJS's linking is recursive.
pub const MAX_MODULE_DEPTH: usize = 256;

/// What a module is (HTML's module type, an import's `type` attribute):
/// JavaScript, JSON, a CSS style sheet or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ModuleKind {
    #[default]
    Js,
    Json,
    Css,
    Text,
}

impl ModuleKind {
    /// The kind a `type` attribute names: none for one the reference
    /// browser does not take (`javascript` among them, which the
    /// standard leaves out).
    pub fn of_type(kind: &str) -> Option<ModuleKind> {
        match kind {
            "json" => Some(ModuleKind::Json),
            "css" => Some(ModuleKind::Css),
            "text" => Some(ModuleKind::Text),
            _ => None,
        }
    }

    /// Its `type` attribute; empty for JavaScript, which has none.
    pub fn as_str(self) -> &'static str {
        match self {
            ModuleKind::Js => "",
            ModuleKind::Json => "json",
            ModuleKind::Css => "css",
            ModuleKind::Text => "text",
        }
    }
}

/// What separates a module's URL from its kind in its name, and an
/// inline module script's base URL from its number: a character no
/// serialized URL holds.
pub const MODULE_MARK: char = '\u{1}';

/// The name a realm and the engine know the module at `url` of `kind` by
/// (HTML's module map, keyed by URL and type): its URL for JavaScript,
/// else its URL, the mark and its type.
pub fn module_name(url: &str, kind: ModuleKind) -> String {
    match kind {
        ModuleKind::Js => url.to_owned(),
        _ => format!("{url}{MODULE_MARK}{}", kind.as_str()),
    }
}

/// A module's name split into its URL and its kind.
pub fn module_of_name(name: &str) -> (&str, ModuleKind) {
    match name.split_once(MODULE_MARK) {
        Some((url, kind)) => (url, ModuleKind::of_type(kind).unwrap_or_default()),
        None => (name, ModuleKind::Js),
    }
}

/// A static import a module names (D61): its specifier and the kind its
/// attributes ask for; `None` for attributes the reference browser does
/// not take (a key other than `type`, or a type unknown), which fail
/// its module when it is linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRequest {
    pub specifier: String,
    pub kind: Option<ModuleKind>,
}

/// A module the engine fetched for a realm (D61): what its loader
/// serves, and serves nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleSource {
    /// Its name ([`module_name`]): the URL it was asked for and its
    /// kind, by which the realm's module map knows it, so that it is
    /// evaluated once a realm.
    pub url: String,
    /// Its base URL, its response's: what its imports resolve against
    /// and what `import.meta.url` is.
    pub base: String,
    pub text: String,
}

/// A module script to run (HTML section 8.1.3.2, D61): its graph
/// fetched whole.
#[derive(Debug, Clone, Copy)]
pub struct ModuleScript<'a> {
    /// Its root's URL: an external one's, which is among `modules` or
    /// in the realm already; an inline one's base URL, the document's,
    /// the realm naming the module apart.
    pub url: &'a str,
    /// An inline script's text and the line of its document it starts
    /// on; `None` for an external one.
    pub inline: Option<(&'a str, u32)>,
    /// The modules of its graph the realm was not given yet.
    pub modules: &'a [ModuleSource],
    /// Its fetch options by the host's number, as [`Script::options`]:
    /// its graph's modules' `import()`s fetch with them.
    pub options: u32,
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
    /// The thread had too little stack for a realm (G253): none was
    /// made, and no script runs on it.
    Stack,
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
    /// The window's named properties that came (`true`) or went since
    /// this was last asked (gap batch A, D44), and whether those the
    /// realm has are to be dropped first (its first time, or more came
    /// and went than were kept); `None` for no change.
    fn take_named_changes(&mut self) -> Option<(bool, Vec<(String, bool)>)> {
        None
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
    /// Whether custom element reactions were queued since this was last
    /// asked (D70): the realm runs them before the call returns.
    fn take_reaction_wake(&mut self) -> bool {
        false
    }
    /// The custom element the parser stopped for in what a script wrote
    /// (D70): its local name and `is` value, which the realm constructs
    /// and gives the parser before it parses on.
    fn take_construct(&mut self) -> Option<(String, Option<String>)> {
        None
    }
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
    /// What a frame's update of the animations tells the animations a
    /// script holds (D75), as JSON: the notices their pending tasks owe
    /// their promises, or a playback event to dispatch at one.
    Animations(String),
    /// A `BroadcastChannel` message (gap batch A, D46): for the channels
    /// of `name` but the one numbered `sender`, its data as serialized,
    /// and the origin it came from.
    Broadcast {
        name: String,
        data: String,
        sender: u64,
        origin: String,
    },
    /// The entries a frame's layout made for resize and intersection
    /// observers (gap batch A), as JSON: their callbacks called.
    Observations(String),
    /// Records wait for mutation observers that no call of a script's
    /// queued (the parser's insertions): the realm tells them.
    Mutations,
    /// The parser stopped for a custom element its definition constructs
    /// (D70): the realm constructs the element of local name `name` and
    /// `is` value `is`, and gives the host what it made
    /// ([`CustomCall::ParserElement`]).
    Construct {
        name: String,
        is: Option<String>,
        /// The prefix the element's qualified name has, which an XML
        /// document's tag may give it, set once it is constructed.
        prefix: Option<String>,
    },
    /// Custom element reactions the parser's insertions queued (D70): the
    /// realm runs them.
    Reactions,
    /// The session history was traversed to another entry of the
    /// document's (HTML's "update document for history step
    /// application"): the entry's state, as serialized (`None` for
    /// null), becomes `history.state`, and `popstate` is fired at the
    /// window with it.
    PopState(Option<String>),
    /// The `matchMedia()` lists whose answer changed in a frame's update
    /// (gap batch A), by their numbers: a `change` at each.
    MediaChanged(Vec<u32>),
    /// The user's close request, an Escape no listener canceled (HTML
    /// section 6.13.2, gap batch H): the topmost close watcher asked to
    /// close, its `cancel` cancelable when `can_prevent` (the page has
    /// the user's activation). The task's answer is whether the closing
    /// was canceled.
    CloseRequest { can_prevent: bool },
    /// The light dismiss of popovers and dialogs (HTML sections 6.12
    /// and 4.11.4) after a trusted `pointerdown` (`up` false) or
    /// `pointerup` at `target`, at `(x, y)` in the viewport.
    LightDismiss {
        target: Handle,
        up: bool,
        x: f64,
        y: f64,
    },
    /// Interest in `target` gained (`gained`) or lost by the interest
    /// invoker `source` once its delay passed (HTML section 6.14):
    /// `interest` or `loseinterest` at the target, then its popover
    /// shown or hidden. The task's answer is whether it was canceled.
    Interest {
        source: Handle,
        target: Handle,
        gained: bool,
    },
    /// `import()` `id`'s graph came (D61, HTML's "fetch an import()
    /// module script graph"): its modules the realm was not given, and
    /// the promise settled as the module at `url` evaluates; or rejected
    /// with a `TypeError` of `error` when the graph could not be fetched.
    Imported {
        id: u32,
        url: String,
        modules: Vec<ModuleSource>,
        error: Option<String>,
    },
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
    /// The static imports of the module `source` at `url` (D61), found by
    /// declaring it apart, its imports none of the realm's; a syntax
    /// error's report if it does not parse.
    fn module_requests(
        &mut self,
        url: &str,
        source: &str,
    ) -> Result<Vec<ModuleRequest>, ScriptError>;
    /// Links and evaluates a module script (HTML's "run a module
    /// script", D61) with `host` and a step budget of its own, as
    /// [`Realm::run`] runs a classic one: an error linking it reported
    /// and returned, one evaluating it, a top-level await's too,
    /// reported as an uncaught exception when it rejects. The caller
    /// performs the microtask checkpoint that follows.
    fn run_module(&mut self, script: &ModuleScript<'_>, host: &mut H) -> Result<(), ScriptError>;
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
    /// Sweeps the groups of objects of trees out of the document held for
    /// custom elements (D70), when one is due: those nothing else reaches
    /// any more are collected and released. With `collect`, as the host
    /// asks when its document's nodes have grown, the garbage is collected
    /// whatever the bindings hold, the sweep with it, which frees the
    /// trees nothing reaches whose objects hold each other.
    fn sweep(&mut self, host: &mut H, collect: bool) {
        let _ = host;
        if collect {
            self.collect();
        }
    }
    /// Why the realm can run nothing more, if it cannot.
    fn dead(&self) -> Option<Death>;
}
