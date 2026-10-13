//! Scripts (M24, `docs/design/scripting.md`): a document's realm, the
//! host its scripts reach the document through, its console, and its
//! `script` elements prepared, fetched and run (HTML section 4.12.1.1).
//!
//! The host owns the document for the length of a run, moved in and
//! back: a script's changes go through `dom::Document`'s methods and
//! journal as any other change does. A realm is made at a document's
//! first script and kept with the document; one whose heap stays full
//! after a failed run is dropped, and the document runs no script again.
//! The host stays with the document between runs (M25), keeping what
//! scripts hold; after each run the subtrees they let go of are freed.

pub(crate) mod canvas;
#[cfg(test)]
mod canvas_tests;
mod cssom;
mod fetch;
mod host;
pub(crate) mod ports;
#[cfg(test)]
mod wasm_tests;
pub(crate) mod workers;

#[cfg(test)]
pub(crate) use fetch::MAX_SCRIPT_FETCHES;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The most scripts that run nested, one inside another's call, the
/// outermost counted (M25): past it no further one runs, and the console
/// says so. Each level takes some 30 KiB of a realm's 256 KiB of stack
/// (`js::MAX_SCRIPT_STACK`), which runs out at about eight, measured: the
/// cap stops a script that writes a script that writes one before that.
pub const MAX_SCRIPT_NESTING: usize = 6;

use dom::{Document, NodeId};
use encoding::Encoding;
use js::{
    Completion, ConsoleLevel, ErrorKind, Limits, QuickJs, Realm, Script, ScriptEngine, ScriptError,
};
use net::csp::{Csp, Violation};
use net::{Destination, Request, Response, ResponseType, Url};

use crate::loader::{FetchId, LoadError, Network, Page, Target};

/// The most bytes an external script may hold; past it its fetch fails,
/// and it is a script that failed (M24.5).
pub const MAX_SCRIPT_BYTES: usize = 32 << 20;

/// The most entries a document's console keeps, the oldest dropped.
pub const MAX_CONSOLE_ENTRIES: usize = 1_000;
/// The most bytes of an entry's text, cut at a character boundary.
pub const MAX_CONSOLE_MESSAGE: usize = 16 << 10;

/// A line of a document's console (M24.7): what a script wrote, or an
/// error it reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleEntry {
    pub level: ConsoleLevel,
    pub text: String,
}

/// Where the shown document's scripting stands (M24.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptState {
    /// Scripting is off, as it is unless asked for (risk R2).
    Off,
    /// Scripts run; `errors` have been reported.
    Running { errors: usize },
    /// The document's realm ran out of memory and was dropped: no
    /// further script runs in it.
    Stopped { errors: usize },
}

pub(crate) use host::DocumentHost;

/// `text`, cut to [`MAX_CONSOLE_MESSAGE`] bytes at a character boundary.
fn cut(mut text: String) -> String {
    if text.len() > MAX_CONSOLE_MESSAGE {
        let mut end = MAX_CONSOLE_MESSAGE;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// A script element's script on its way to being run (HTML section
/// 4.12.1.1): fetched, or inline and held by a style sheet.
enum Pending {
    /// Its fetch from `url` is on its way; `fallback` decodes it when
    /// neither a BOM nor its type names an encoding.
    Fetching {
        id: FetchId,
        url: Url,
        fallback: Encoding,
        element: NodeId,
    },
    /// Ready to run; `None` when its fetch failed (HTML: its result is
    /// null), which runs nothing.
    Ready(Option<Source>),
}

/// A classic script, ready to run.
pub(crate) struct Source {
    text: String,
    /// The URL its errors name: the document's for an inline script, the
    /// response's for a fetched one.
    url: String,
    line: u32,
    muted: bool,
    /// Its element: `document.currentScript` while it runs (M25).
    element: Option<NodeId>,
    /// Whether it came from a file: while it runs, a `document.write`
    /// with no insertion point does nothing (HTML's ignore-destructive-
    /// writes counter).
    external: bool,
    /// Whether its fetch failed (M26): it runs nothing, and its element
    /// gets an `error` event.
    failed: bool,
}

impl Source {
    /// The script of `element` whose fetch failed.
    fn failed(element: NodeId) -> Source {
        Source {
            text: String::new(),
            url: String::new(),
            line: 1,
            muted: false,
            element: Some(element),
            external: true,
            failed: true,
        }
    }
}

/// `document.readyState` (HTML section 3.1.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ReadyState {
    #[default]
    Loading,
    Interactive,
    Complete,
}

impl ReadyState {
    fn as_str(self) -> &'static str {
        match self {
            ReadyState::Loading => "loading",
            ReadyState::Interactive => "interactive",
            ReadyState::Complete => "complete",
        }
    }
}

/// What a document's scripts are lent for a run (M25): the document, its
/// page, its parser while it parses, the network for the scripts they
/// fetch, where its loading stands, and what the window reports.
pub(crate) struct Lent<'a> {
    pub(crate) document: &'a mut Document,
    pub(crate) page: &'a mut Page,
    pub(crate) parser: Option<&'a mut html::Load>,
    pub(crate) network: &'a mut Network,
    pub(crate) ready: ReadyState,
    pub(crate) window: WindowEnv,
}

/// What the window reports to a run's scripts (M25.4): the viewport, the
/// medium styles are computed for and the fonts they measure with,
/// whether the user asked for D12's compatibility user agent string, and
/// the engine's clock (M26).
#[derive(Clone, Default)]
pub(crate) struct WindowEnv {
    pub(crate) viewport: (u32, u32),
    pub(crate) media: style::Media,
    pub(crate) fonts: Option<text::FontCollection>,
    pub(crate) compat: bool,
    pub(crate) clock: Option<crate::Clock>,
    /// Whether the window is hidden: minimized or covered (M26).
    pub(crate) hidden: bool,
    /// Whether the window has the system's focus, and whether the last
    /// input was a key, so that a focus a script gives is shown (M26.6).
    pub(crate) focused: bool,
    pub(crate) keyboard: bool,
    /// What the shown document's `img`s give a canvas to draw (M30).
    pub(crate) drawables: Option<canvas::SharedDrawables>,
    /// The engine's storage, for `localStorage` and `sessionStorage`
    /// (M28), and whether another document alive shares this one's
    /// origin, to hear of its changes.
    pub(crate) storage: Option<Arc<Mutex<crate::storage::Storage>>>,
    pub(crate) storage_peer: bool,
    /// The tab's frames and the window's own (M33.2).
    pub(crate) frames: Option<crate::frames::FramesEnv>,
    /// Where the window's frame is in its tab (M33.2), its document
    /// apart: none at the top.
    pub(crate) partition: Option<net::Partition>,
}

/// A document's time origin (High Resolution Time): the engine's clock
/// when its navigation started, and that time in milliseconds since the
/// Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TimeOrigin {
    pub(crate) at: Duration,
    pub(crate) epoch_ms: f64,
}

/// The most timers a document may have set at once (M26): past them
/// `setTimeout` throws a `RangeError`.
pub(crate) const MAX_TIMERS: usize = 100_000;

/// A navigation a script asked for (`location`), which the engine starts
/// after the run: to `url`, in the current entry's place if `replace`;
/// or the document fetched again if `reload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptNavigation {
    pub(crate) url: Option<Url>,
    pub(crate) replace: bool,
    pub(crate) reload: bool,
    /// A link was followed (M26): its activation behaviour, which
    /// navigates as a click on it does, rather than as `location` does.
    /// A form submitted does too (M26.9), with its body for a POST.
    pub(crate) link: bool,
    pub(crate) post: Option<crate::submit::Post>,
    /// The navigable a link or a form named (M33.2): its `target`, empty
    /// for the document's own.
    pub(crate) target: String,
}

/// How a script is run.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct How {
    /// Its element, `document.currentScript`.
    pub(crate) element: Option<NodeId>,
    /// Whether it came from a file.
    pub(crate) external: bool,
    /// Whether it is a parser-inserted script run where the parser
    /// stopped after its end tag: the insertion point is defined there
    /// while it runs (HTML section 13.2.6.4.8).
    pub(crate) at_insertion_point: bool,
    /// The timer nesting level of the task it is (M26): 0 but for a
    /// timer's.
    pub(crate) nesting: u32,
}

/// What preparing a script element comes to (HTML section 4.12.1.1,
/// "prepare the script element").
pub(crate) enum Prepared {
    /// Not started, and nothing runs: an inline script with no text, or
    /// an element whose type is not a script's. It may be prepared again.
    NotYet,
    /// Started, and nothing runs: a module, an import map or speculation
    /// rules (G41), `nomodule`, legacy `for` and `event` attributes for
    /// another event, a `src` that is empty or does not parse, or a page
    /// that fetches nothing.
    Nothing,
    /// Started, and nothing runs, its `src` empty or not a URL: the
    /// element gets an `error` event (M26).
    Failed,
    /// Started, and refused by the document's policies, whose violations
    /// its page has (M33.1).
    Refused,
    /// An inline classic script.
    Inline(Source),
    /// A classic script to fetch.
    Fetch {
        request: Box<Request>,
        url: Url,
        fallback: Encoding,
    },
}

/// Prepares `element` as HTML section 4.12.1.1 does, its text starting
/// on the document's `line`, whoever inserted it; what runs, and when, is
/// its caller's to decide.
/// Whether `node` is a script element: HTML's, or SVG's (M32), which
/// runs as HTML's does.
pub(crate) fn is_script(document: &Document, node: NodeId) -> bool {
    document.tag(node) == Some("script") || document.svg_tag(node) == Some("script")
}

pub(crate) fn prepare_element(
    page: &mut Page,
    document: &Document,
    element: NodeId,
    line: u32,
    parser_inserted: bool,
) -> Prepared {
    let Some(kind) = script_type(document, element) else {
        return Prepared::NotYet;
    };
    // An SVG script names its file by `href`, or XLink's (M32).
    let src = if document.svg_tag(element).is_some() {
        document
            .attr(element, "href")
            .or_else(|| document.attr(element, "xlink:href"))
    } else {
        document.attr(element, "src")
    };
    let text = document.text_content(element);
    if src.is_none() && text.is_empty() {
        return Prepared::NotYet;
    }
    if kind != ScriptType::Classic
        || document.attr(element, "nomodule").is_some()
        || !for_the_window(document, element)
    {
        return Prepared::Nothing;
    }
    let Some(src) = src else {
        // By nonce, hash or 'unsafe-inline' (M33.1).
        let nonce = page.checked_nonce(document, element);
        let csp = page.csp(document);
        let violations = csp.inline_violations(net::csp::Inline::Script, &nonce, &text);
        csp.record(&violations);
        if net::csp::enforced(violations).is_err() {
            return Prepared::Refused;
        }
        return Prepared::Inline(Source {
            text,
            url: page.url().map(Url::to_string).unwrap_or_default(),
            line,
            muted: false,
            element: Some(element),
            external: false,
            failed: false,
        });
    };
    // An empty `src`, or one that does not parse, fires `error` (M26) and
    // is nothing more.
    let Some(url) = Some(src)
        .filter(|src| !src.is_empty())
        .and_then(|src| page.resolve(document, src))
    else {
        return Prepared::Failed;
    };
    let csp = page.csp(document).clone();
    let Some(mut request) = script_request(page, url.clone(), csp) else {
        return Prepared::Nothing;
    };
    // Its nonce and who inserted it, for its directive (M33.1).
    request.nonce = page.checked_nonce(document, element);
    request.parser_inserted = parser_inserted;
    let request = crate::loader::with_cors_settings(request, document.attr(element, "crossorigin"));
    let fallback = document
        .attr(element, "charset")
        .and_then(Encoding::for_label)
        .unwrap_or(page.encoding());
    Prepared::Fetch {
        request: Box::new(request),
        url,
        fallback,
    }
}

/// Where a script fetched for an element waits to run (HTML section
/// 4.12.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// The pending parsing-blocking script: a parser-inserted one with
    /// neither `async` nor `defer`.
    Blocking,
    /// Run in order once the document is parsed: parser-inserted `defer`.
    Deferred,
    /// Run as soon as it arrives: `async`, or inserted by a script with
    /// its "force async" flag.
    Soon,
    /// Run in order as soon as each and those before it have arrived: one
    /// a script inserted with `async` false.
    InOrder,
}

/// A script's fetch, found during a run, which the engine starts after
/// it.
pub(crate) struct Wanted {
    pub(crate) request: Box<Request>,
    pub(crate) url: Url,
    pub(crate) fallback: Encoding,
    pub(crate) element: NodeId,
    pub(crate) place: Place,
}

/// A document's scripts: its realm, once it has one, its console, and
/// the scripts on their way.
pub(crate) struct Scripts {
    realm: Option<Box<dyn Realm<DocumentHost>>>,
    /// The host the realm's scripts reach the document through, lent
    /// the document for each run.
    host: DocumentHost,
    limits: Limits,
    console: VecDeque<ConsoleEntry>,
    errors: usize,
    /// Its realm died: it runs nothing more.
    stopped: bool,
    /// It stopped since this was last asked.
    just_stopped: bool,
    /// The pending parsing-blocking script: the parser waits until it is
    /// ready and no style sheet blocks scripts.
    blocking: Option<Pending>,
    /// The scripts run in order once the document is parsed (`defer`).
    deferred: VecDeque<Pending>,
    /// The scripts run each as soon as it is ready (`async`).
    soon: Vec<Pending>,
    /// The scripts run in order as soon as possible: those scripts
    /// inserted with `async` false (M25).
    in_order: VecDeque<Pending>,
    /// Its parser was stopped: the scripts on their way are dropped, and
    /// none it meets after runs.
    aborted: bool,
    /// The last navigation its scripts asked for, for the engine to start.
    navigation: Option<ScriptNavigation>,
    /// The element a script focused, to be scrolled into view (M26.6).
    scroll_to: Option<NodeId>,
    /// What a script's validation reported, for the notice (M26.9b).
    notice: Option<String>,
    /// The events to fire at elements when its scripts next run (M26):
    /// a script element's `error` for a `src` that cannot be fetched.
    events: VecDeque<(NodeId, &'static str)>,
    /// Whether its markup holds an event handler, as of a version of the
    /// document: one with none makes no realm for an event.
    handlers: Option<(u64, bool)>,
    /// Its scripts' fetches in flight (M27.1): the script's number of
    /// each, and its URL, by the engine's.
    fetching: HashMap<FetchId, (u32, Url)>,
    /// The answers that came, each a task from when it came (the
    /// networking task source), in that order.
    answers: VecDeque<(Duration, u32, Option<js::FetchAnswer>)>,
    /// The messages posted (M31), each a task from when it was (the
    /// posted message task source), in that order.
    posted: VecDeque<(Duration, js::Task)>,
    /// The names of the window's child frames' elements, in tree order
    /// (M33.2), which its realm's frames by index and name follow.
    frame_list: Vec<String>,
    /// Whether the realm's code from strings asks the host (M33.1).
    code_guarded: bool,
}

impl Default for Scripts {
    fn default() -> Scripts {
        Scripts::with_limits(Limits::default())
    }
}

impl Scripts {
    /// A document's scripts, its realm to be made with `limits`.
    pub(crate) fn with_limits(limits: Limits) -> Scripts {
        Scripts {
            realm: None,
            host: DocumentHost::default(),
            limits,
            console: VecDeque::new(),
            errors: 0,
            stopped: false,
            just_stopped: false,
            blocking: None,
            deferred: VecDeque::new(),
            soon: Vec::new(),
            in_order: VecDeque::new(),
            events: VecDeque::new(),
            handlers: None,
            scroll_to: None,
            notice: None,
            aborted: false,
            navigation: None,
            fetching: HashMap::new(),
            answers: VecDeque::new(),
            posted: VecDeque::new(),
            frame_list: Vec::new(),
            code_guarded: false,
        }
    }

    /// The navigation the document's scripts asked for since this was
    /// last asked, if any: the last of them.
    pub(crate) fn take_navigation(&mut self) -> Option<ScriptNavigation> {
        self.navigation.take()
    }

    /// The element a script focused since this was last asked, to be
    /// scrolled into view (M26.6).
    pub(crate) fn take_scroll(&mut self) -> Option<NodeId> {
        self.scroll_to.take()
    }

    /// What a script's `reportValidity()` or a form's interactive
    /// validation said since this was last asked (M26.9b).
    pub(crate) fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Prepares the `script` element the parser has just closed (HTML
    /// section 4.12.1.1, "prepare the script element"), its text starting
    /// on the document's `line`. An inline classic script runs at once,
    /// the insertion point after its end tag, or, while a style sheet
    /// blocks scripts, is the pending parsing-blocking script; one with a
    /// `src` is fetched, and is that script, or waits for the end of the
    /// parse, or runs as soon as it comes, as its `defer` and `async`
    /// say.
    pub(crate) fn prepare(&mut self, lent: &mut Lent<'_>, element: NodeId, line: u32) {
        if self.aborted || self.stopped {
            return;
        }
        let prepared = prepare_element(lent.page, lent.document, element, line, true);
        self.report_violations(lent.page.take_violations());
        match prepared {
            Prepared::NotYet | Prepared::Nothing | Prepared::Refused => {}
            Prepared::Failed => self.events.push_back((element, "error")),
            Prepared::Inline(source) => {
                if lent.page.blocks_scripts() {
                    self.blocking = Some(Pending::Ready(Some(source)));
                } else {
                    self.run_source(lent, &source, true);
                }
            }
            Prepared::Fetch {
                request,
                url,
                fallback,
            } => {
                let place = parser_place(lent.document, element);
                self.fetch(
                    lent,
                    Wanted {
                        request,
                        url,
                        fallback,
                        element,
                        place,
                    },
                );
            }
        }
    }

    /// Starts `wanted`'s fetch, and puts the script where it waits.
    fn fetch(&mut self, lent: &mut Lent<'_>, wanted: Wanted) {
        let id = lent
            .network
            .fetch(lent.page.id(), Target::Script, *wanted.request);
        let pending = Pending::Fetching {
            id,
            url: wanted.url,
            fallback: wanted.fallback,
            element: wanted.element,
        };
        match wanted.place {
            Place::Blocking => self.blocking = Some(pending),
            Place::Deferred => self.deferred.push_back(pending),
            Place::Soon => self.soon.push(pending),
            Place::InOrder => self.in_order.push_back(pending),
        }
    }

    /// Takes the result of fetch `id` for a script (HTML's "fetch a
    /// classic script"): the script, decoded, if it came with an ok status
    /// and a type a script may have; else a failure, which the console
    /// says. A result no script waits for changes nothing.
    pub(crate) fn arrived(&mut self, id: FetchId, result: Result<Response, LoadError>) {
        let Some(pending) = self
            .blocking
            .iter_mut()
            .chain(&mut self.deferred)
            .chain(&mut self.soon)
            .chain(&mut self.in_order)
            .find(|pending| matches!(pending, Pending::Fetching { id: fetch, .. } if *fetch == id))
        else {
            return;
        };
        let Pending::Fetching {
            url,
            fallback,
            element,
            ..
        } = &*pending
        else {
            unreachable!("found fetching");
        };
        let element = *element;
        // A refusal by the policies says so itself (M33.1).
        let refused = matches!(result, Err(LoadError::Network(net::NetworkError::Csp(_))));
        let failed = match result
            .map_err(|error| error.to_string())
            .and_then(|response| script_source(&response, *fallback))
        {
            Ok(mut source) => {
                source.element = Some(element);
                *pending = Pending::Ready(Some(source));
                None
            }
            Err(why) => {
                let text = format!("Failed to load script {url}: {why}");
                *pending = Pending::Ready(Some(Source::failed(element)));
                (!refused).then_some(text)
            }
        };
        if let Some(text) = failed {
            self.push(ConsoleEntry {
                level: ConsoleLevel::Error,
                text,
            });
        }
    }

    /// Runs what is ready, as HTML's parser and event loop would: each
    /// script to run as soon as possible that has come; those to run in
    /// order as soon as possible, as far as they have come; the pending
    /// parsing-blocking script, once it is ready and no style sheet
    /// blocks scripts, with the insertion point where the parser stopped;
    /// and, the document `parsed`, its deferred scripts in order, each
    /// once it is ready and no style sheet blocks scripts. Whether the
    /// parser may go on.
    pub(crate) fn run_ready(&mut self, lent: &mut Lent<'_>, parsed: bool) -> bool {
        while let Some((element, kind)) = self.events.pop_front() {
            self.dispatch(
                lent,
                js::Target::Node(handle(element)),
                js::EventInit::new("Event", kind),
            );
        }
        let mut ready = Vec::new();
        self.soon.retain_mut(|pending| match pending {
            Pending::Ready(source) => {
                ready.push(source.take());
                false
            }
            Pending::Fetching { .. } => true,
        });
        for source in ready.into_iter().flatten() {
            self.run_source(lent, &source, false);
        }
        while let Some(Pending::Ready(source)) = self.in_order.front_mut() {
            let ready = source.take();
            self.in_order.pop_front();
            if let Some(source) = ready {
                self.run_source(lent, &source, false);
            }
        }
        if lent.page.blocks_scripts() {
            return self.blocking.is_none();
        }
        if let Some(Pending::Ready(source)) = &mut self.blocking {
            let ready = source.take();
            self.blocking = None;
            if let Some(source) = ready {
                self.run_source(lent, &source, true);
            }
        }
        while parsed && let Some(Pending::Ready(source)) = self.deferred.front_mut() {
            let ready = source.take();
            self.deferred.pop_front();
            if let Some(source) = ready {
                self.run_source(lent, &source, false);
            }
        }
        self.blocking.is_none()
    }

    /// Whether the scripts the parse's end waits for have run: the
    /// parsing-blocking one and the deferred ones, which
    /// `DOMContentLoaded` follows (M26).
    pub(crate) fn deferred_settled(&self) -> bool {
        self.blocking.is_none() && self.deferred.is_empty()
    }

    /// Whether no script is on its way: the load waits for them.
    pub(crate) fn settled(&self) -> bool {
        self.blocking.is_none()
            && self.deferred.is_empty()
            && self.soon.is_empty()
            && self.in_order.is_empty()
    }

    /// The document's parser was stopped (HTML's "abort a parser"): the
    /// scripts on their way, whose fetches were aborted, are dropped, and
    /// none the parser meets from now on runs.
    pub(crate) fn abort(&mut self) {
        self.blocking = None;
        self.deferred.clear();
        self.soon.clear();
        self.in_order.clear();
        self.aborted = true;
    }

    /// Says on the console what the document's policies refused, or would
    /// have (M24.6, M33.1).
    pub(crate) fn report_violations(&mut self, violations: Vec<Violation>) {
        for violation in violations {
            self.push(ConsoleEntry {
                level: violation_level(&violation),
                text: violation_text(&violation),
            });
        }
    }

    /// Runs `source`, a script element's; `at_insertion_point` for a
    /// parser-inserted one run where the parser stopped after it. An
    /// external script's element gets a `load` event after it runs, or an
    /// `error` event for a fetch that failed, which runs nothing (M26).
    fn run_source(&mut self, lent: &mut Lent<'_>, source: &Source, at_insertion_point: bool) {
        if source.failed {
            if let Some(element) = source.element {
                self.dispatch(
                    lent,
                    js::Target::Node(handle(element)),
                    js::EventInit::new("Event", "error"),
                );
            }
            return;
        }
        let script = Script {
            source: &source.text,
            url: &source.url,
            line: source.line,
            muted: source.muted,
        };
        let how = How {
            element: source.element,
            external: source.external,
            at_insertion_point,
            nesting: 0,
        };
        self.run(lent, &script, how);
        if source.external
            && let Some(element) = source.element
        {
            self.dispatch(
                lent,
                js::Target::Node(handle(element)),
                js::EventInit::new("Event", "load"),
            );
        }
    }

    /// Makes the realm if there is none: whether there is one now. A
    /// realm that cannot be made stops the document's scripts.
    fn ensure_realm(&mut self) -> bool {
        if self.stopped {
            return false;
        }
        if self.realm.is_none() {
            match QuickJs.realm::<DocumentHost>(self.limits) {
                Ok(realm) => {
                    self.realm = Some(realm);
                    self.code_guarded = false;
                }
                Err(error) => {
                    self.report(&error);
                    self.stop();
                    return false;
                }
            }
            // The window's frames, made before it runs (M33.2).
            if !self.frame_list.is_empty() {
                self.apply_frame_list();
            }
        }
        true
    }

    /// Guards the realm's code from strings once the document's policies
    /// would refuse or report one (M33.1), before what runs next.
    fn guard_code(&mut self, lent: &mut Lent<'_>) {
        if self.code_guarded {
            return;
        }
        let Some(realm) = &mut self.realm else {
            return;
        };
        if lent.page.csp(lent.document).eval_violations("").is_empty() {
            return;
        }
        let _ = realm.enter(&js::Task::GuardCode, &mut self.host);
        self.code_guarded = true;
    }

    /// The window's child frames are now named `names`, their elements'
    /// (M33.2): its realm's frames by index and name follow at once.
    pub(crate) fn set_frame_list(&mut self, names: Vec<String>) {
        if names != self.frame_list {
            self.frame_list = names;
            self.apply_frame_list();
        }
    }

    /// Makes the realm's frames by index and name, if it has one: none of
    /// it asks the host anything.
    fn apply_frame_list(&mut self) {
        if let Some(realm) = &mut self.realm {
            let task = js::Task::FrameList(self.frame_list.clone());
            let _ = realm.enter(&task, &mut self.host);
        }
    }

    /// Dispatches `event` at `target` as a task (M26): whether a listener
    /// canceled it, or `None` when nothing was dispatched, the document
    /// having no realm: one makes a realm only if its markup holds an
    /// event handler, there being no other listener. The caller does the
    /// event's default actions then, as a click's activation.
    pub(crate) fn dispatch(
        &mut self,
        lent: &mut Lent<'_>,
        target: js::Target,
        event: js::EventInit,
    ) -> Option<bool> {
        if self.realm.is_none()
            && (!has_handlers(lent.document, &mut self.handlers) || !self.ensure_realm())
        {
            return None;
        }
        Some(self.enter(lent, &js::Task::Dispatch { target, event }, 0))
    }

    /// Whether an animation frame callback waits for the next frame.
    pub(crate) fn wants_frame(&self) -> bool {
        self.host.frame_wanted && !self.stopped
    }

    /// The bitmap of the canvas `node` and its generation (M30.3), if a
    /// script made its context.
    pub(crate) fn canvas_pixels(
        &mut self,
        node: NodeId,
    ) -> Option<(u64, std::sync::Arc<images::Image>)> {
        self.host.canvases.pixels(canvas::Owner::Element(node))
    }

    /// The bitmap of the canvas `node` as it is now, for a print.
    pub(crate) fn canvas_image(&self, node: NodeId) -> Option<std::sync::Arc<images::Image>> {
        self.host.canvases.image(node)
    }

    /// Whether a canvas's bitmap changed since the last frame.
    pub(crate) fn canvases_changed(&self) -> bool {
        self.host.canvases.changed()
    }

    pub(crate) fn take_canvases_changed(&mut self) -> bool {
        self.host.canvases.take_changed()
    }

    /// Runs the animation frame callbacks (M26) for a frame at `now`, in
    /// milliseconds on the document's clock.
    pub(crate) fn run_frames(&mut self, lent: &mut Lent<'_>, now: f64) {
        if !self.wants_frame() {
            return;
        }
        self.host.frame_wanted = false;
        self.enter(lent, &js::Task::AnimationFrames(now), 0);
    }

    /// The document's time origin, if it has one.
    pub(crate) fn origin(&self) -> Option<TimeOrigin> {
        self.host.origin
    }

    /// Runs `script` in the document's realm, made if it has none, with
    /// what the run needs lent to the host, then the microtask checkpoint
    /// that follows a script (HTML section 8.1.4.4); its errors and those
    /// of the jobs the checkpoint runs go to the console, and the scripts
    /// it inserted or wrote that are to be fetched are fetched. What it
    /// ended with, unless it failed or the document runs no scripts.
    pub(crate) fn run(
        &mut self,
        lent: &mut Lent<'_>,
        script: &Script<'_>,
        how: How,
    ) -> Option<Completion> {
        if !self.ensure_realm() {
            return None;
        }
        self.guard_code(lent);
        let realm = self.realm.as_mut()?;
        let host = &mut self.host;
        let insertion_point = how.at_insertion_point
            && lent
                .parser
                .as_deref_mut()
                .and_then(html::Load::parser)
                .map(html::Parser::push_insertion_point)
                .is_some();
        host.in_flight = self.fetching.len();
        host.lend(lent, how, self.blocking.is_some());
        let result = realm.run(script, host);
        let errors = realm.checkpoint(host);
        let dead = realm.dead().is_some();
        host.sweep();
        let outputs = host.give_back(lent);
        if insertion_point
            && let Some(parser) = lent.parser.as_deref_mut().and_then(html::Load::parser)
        {
            parser.pop_insertion_point();
        }
        for entry in outputs.console {
            self.push(entry);
        }
        self.errors += outputs.errors;
        let completion = match result {
            Ok(completion) => Some(completion),
            Err(error) => {
                self.report(&error);
                None
            }
        };
        for error in &errors {
            self.report(error);
        }
        for wanted in outputs.wanted {
            self.fetch(lent, wanted);
        }
        self.start_fetches(lent, outputs.fetches, outputs.aborted);
        self.posted.extend(outputs.messages);
        self.drop_tasks_of(&outputs.terminated);
        self.events.extend(outputs.events);
        if outputs.scroll_to.is_some() {
            self.scroll_to = outputs.scroll_to;
        }
        if outputs.notice.is_some() {
            self.notice = outputs.notice;
        }
        if outputs.navigation.is_some() {
            self.navigation = outputs.navigation;
        }
        if dead {
            self.stop();
        }
        completion
    }

    /// Runs `task` in the document's realm (M26) as [`Scripts::run`]
    /// runs a script, `nesting` the timer nesting level of the task it
    /// is, then the microtask checkpoint; nothing if the document has no
    /// realm or runs no scripts. Whether the event it dispatched was
    /// canceled.
    pub(crate) fn enter(&mut self, lent: &mut Lent<'_>, task: &js::Task, nesting: u32) -> bool {
        if self.stopped {
            return false;
        }
        self.guard_code(lent);
        let Some(realm) = self.realm.as_mut() else {
            return false;
        };
        let host = &mut self.host;
        let how = How {
            nesting,
            ..How::default()
        };
        host.in_flight = self.fetching.len();
        host.lend(lent, how, self.blocking.is_some());
        let result = realm.enter(task, host);
        let errors = realm.checkpoint(host);
        let dead = realm.dead().is_some();
        host.sweep();
        let outputs = host.give_back(lent);
        for entry in outputs.console {
            self.push(entry);
        }
        self.errors += outputs.errors;
        let canceled = match result {
            Ok(canceled) => canceled,
            Err(error) => {
                self.report(&error);
                false
            }
        };
        for error in &errors {
            self.report(error);
        }
        for wanted in outputs.wanted {
            self.fetch(lent, wanted);
        }
        self.start_fetches(lent, outputs.fetches, outputs.aborted);
        self.posted.extend(outputs.messages);
        self.drop_tasks_of(&outputs.terminated);
        self.events.extend(outputs.events);
        if outputs.scroll_to.is_some() {
            self.scroll_to = outputs.scroll_to;
        }
        if outputs.notice.is_some() {
            self.notice = outputs.notice;
        }
        if outputs.navigation.is_some() {
            self.navigation = outputs.navigation;
        }
        if dead {
            self.stop();
        }
        canceled
    }

    /// Takes what the document's workers sent (M31): their messages,
    /// errors and ends queued as tasks from `now`, their console lines
    /// put in the document's console.
    pub(crate) fn take_worker_messages(&mut self, now: Duration) {
        while let Some(message) = self.host.workers.mailbox.try_take() {
            let (id, event) = match message {
                workers::WorkerMessage::FromWorker(id, event) => (id, event),
                workers::WorkerMessage::Frame((number, size, image)) => {
                    self.host.canvases.placeholder_frame(number, size, image);
                    continue;
                }
                workers::WorkerMessage::Port(port, data) => {
                    let task = js::Task::Message {
                        target: js::MessageTarget::Port(port),
                        data,
                        origin: String::new(),
                    };
                    self.posted.push_back((now, task));
                    continue;
                }
                _ => continue,
            };
            match self.host.workers.workers.receive(id, event) {
                workers::Received::Task(task) => self.posted.push_back((now, task)),
                workers::Received::Console(level, text) => self.push(ConsoleEntry { level, text }),
                workers::Received::Nothing => {}
            }
        }
    }

    /// Whether the document has a worker that has not ended (M31).
    pub(crate) fn has_workers(&self) -> bool {
        self.host.workers.workers.any()
    }

    /// The sizes the placeholder canvases are to take (M31.4), taken.
    pub(crate) fn take_placeholder_resizes(&mut self) -> Vec<(NodeId, (u32, u32))> {
        self.host.canvases.take_resizes()
    }

    /// The document's live workers, for the tests.
    #[cfg(test)]
    pub(crate) fn live_workers(&self) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
        self.host.workers.workers.live()
    }

    /// Drops the tasks not yet run of the workers `terminated`: their
    /// objects hear nothing more.
    fn drop_tasks_of(&mut self, terminated: &[u32]) {
        if terminated.is_empty() {
            return;
        }
        self.posted.retain(|(_, task)| {
            let worker = match task {
                js::Task::Message {
                    target: js::MessageTarget::Worker(worker),
                    ..
                }
                | js::Task::WorkerError { worker, .. }
                | js::Task::WorkerEnded(worker) => *worker,
                _ => return true,
            };
            !terminated.contains(&worker)
        });
    }

    /// Gives the document its time origin, unless it has one.
    pub(crate) fn set_time_origin(&mut self, origin: TimeOrigin) {
        self.host.origin.get_or_insert(origin);
    }

    /// When the document's next task is due (M26): its next timer, or a
    /// fetch's answer (M27.1), which is due from when it came.
    pub(crate) fn next_task_at(&self) -> Option<Duration> {
        if self.stopped {
            return None;
        }
        let answer = self.answers.front().map(|(came, ..)| *came);
        let posted = self.posted.front().map(|(at, _)| *at);
        [self.host.timers.next_due(), answer, posted]
            .into_iter()
            .flatten()
            .min()
    }

    /// Starts the fetches a run asked for, on the document's page, and
    /// aborts those whose signals did (M27.1).
    fn start_fetches(
        &mut self,
        lent: &mut Lent<'_>,
        fetches: Vec<(u32, Request)>,
        aborted: Vec<u32>,
    ) {
        for (id, mut request) in fetches {
            let url = request.url.clone();
            // Held to the document's policies as they stand (M33.1).
            request.csp = Some(lent.page.csp(lent.document).clone());
            let fetch = lent.network.fetch(lent.page.id(), Target::Fetch, request);
            self.fetching.insert(fetch, (id, url));
        }
        for id in aborted {
            if let Some(fetch) = self
                .fetching
                .iter()
                .find(|(_, (of, _))| *of == id)
                .map(|(fetch, _)| *fetch)
            {
                self.fetching.remove(&fetch);
                lent.network.abort(fetch);
            }
        }
    }

    /// Takes the result of fetch `id` for a script's `fetch()`, which
    /// came at `now`: what the script may see of it, or a network error,
    /// queued as a task; why it failed goes to the console.
    pub(crate) fn fetch_arrived(
        &mut self,
        id: FetchId,
        result: Result<Response, LoadError>,
        now: Duration,
    ) {
        if let Some((script, url)) = self.fetching.remove(&id) {
            let answer = match result {
                Ok(response) => Some(fetch::answer(&response)),
                Err(error) => {
                    self.push(ConsoleEntry {
                        level: ConsoleLevel::Error,
                        text: cut(fetch::failure_text(&url, error)),
                    });
                    None
                }
            };
            // In the order they came; those that came at one time, as
            // a hand's clock gives, in the order they were asked for.
            let at = self
                .answers
                .partition_point(|(came, of, _)| (*came, *of) <= (now, script));
            self.answers.insert(at, (now, script, answer));
        }
    }

    /// The task due first, taken: a timer's, with the nesting level of
    /// the task that runs it, or a fetch's answer, at level 0.
    pub(crate) fn take_next_task(&mut self) -> Option<(js::Task, u32)> {
        let timer = self.host.timers.next_due();
        let answer = self.answers.front().map(|(came, ..)| *came);
        // A message posted no later than the next timer and answer runs
        // first (M31).
        if let Some(&(at, _)) = self.posted.front()
            && timer.is_none_or(|due| at <= due)
            && answer.is_none_or(|came| at <= came)
        {
            return self.posted.pop_front().map(|(_, task)| (task, 0));
        }
        if answer.is_some_and(|came| timer.is_none_or(|due| came <= due)) {
            let (_, id, answer) = self.answers.pop_front()?;
            return Some((
                js::Task::Fetched {
                    id,
                    answer: answer.map(Box::new),
                },
                0,
            ));
        }
        let (id, nesting) = self.host.timers.take_next()?;
        Some((js::Task::Timer(id), nesting))
    }

    /// Whether the back-forward cache may keep the document: not when
    /// its realm died, nor while it has workers (M31), which end with it
    /// as it goes.
    pub(crate) fn keepable(&self) -> bool {
        !self.stopped && !self.host.workers.workers.any()
    }

    /// Collects the realm's garbage, as when the document is kept.
    pub(crate) fn collect(&mut self) {
        if let Some(realm) = &mut self.realm {
            realm.collect();
        }
    }

    /// The bytes of heap the realm holds; none without one.
    #[cfg(test)]
    pub(crate) fn heap(&self) -> usize {
        self.realm.as_ref().map_or(0, |realm| realm.heap())
    }

    /// The document started again (M24.8): its realm goes, and the next
    /// script runs in a new one; its console stays, since what was run
    /// was run.
    pub(crate) fn restart(&mut self) {
        *self = Scripts {
            console: std::mem::take(&mut self.console),
            ..Scripts::with_limits(self.limits)
        };
    }

    fn stop(&mut self) {
        self.realm = None;
        self.host.forget_objects();
        self.host.timers.clear_all();
        self.host.workers.workers.terminate_all();
        self.stopped = true;
        self.just_stopped = true;
        self.push(ConsoleEntry {
            level: ConsoleLevel::Error,
            text: "Scripts stopped on this page: out of memory".to_owned(),
        });
    }

    /// Reports `error` as HTML section 8.1.4.6 does, to the console: an
    /// uncaught exception and where it was thrown, or a rejection with no
    /// handler; a dead realm's refusal is not an error of the page's.
    pub(crate) fn report(&mut self, error: &ScriptError) {
        let Some(text) = report_text(error) else {
            return;
        };
        self.errors += 1;
        self.push(ConsoleEntry {
            level: ConsoleLevel::Error,
            text,
        });
    }

    /// Queues `task`, which another frame posted (M33.2), from `at`.
    pub(crate) fn post(&mut self, at: Duration, task: js::Task) {
        self.posted.push_back((at, task));
    }

    /// Makes the document's ports and workers' known to the tab's
    /// `router` (M33.2), so that a port may go to another frame: before
    /// its realm is made, while it has none.
    pub(crate) fn use_router(&mut self, router: ports::SharedRouter) {
        if self.realm.is_none() && !self.host.workers.workers.any() {
            self.host.workers = workers::DocumentWorkers::with_router(router);
        }
    }

    /// Writes `text` to the console at `level`: what the engine says of
    /// the document (M33.2: an iframe left empty).
    pub(crate) fn say(&mut self, level: ConsoleLevel, text: String) {
        self.push(ConsoleEntry { level, text });
    }

    fn push(&mut self, entry: ConsoleEntry) {
        if self.console.len() == MAX_CONSOLE_ENTRIES {
            self.console.pop_front();
        }
        self.console.push_back(entry);
    }

    /// The console's entries since it was last taken.
    pub(crate) fn take_console(&mut self) -> Vec<ConsoleEntry> {
        self.console.drain(..).collect()
    }

    /// Where the document's scripting stands, scripting being on.
    pub(crate) fn state(&self) -> ScriptState {
        if self.stopped {
            ScriptState::Stopped {
                errors: self.errors,
            }
        } else {
            ScriptState::Running {
                errors: self.errors,
            }
        }
    }

    /// Whether the realm died since this was last asked.
    pub(crate) fn take_just_stopped(&mut self) -> bool {
        std::mem::take(&mut self.just_stopped)
    }
}

/// The handle a script holds `node` by (M25).
pub(crate) fn handle(node: NodeId) -> js::Handle {
    js::Handle(node.to_bits())
}

/// Whether an element of `document` has an attribute that may be an
/// event handler, an `on...` one, as of the version `seen` was taken at.
fn has_handlers(document: &Document, seen: &mut Option<(u64, bool)>) -> bool {
    let version = document.version();
    if let Some((at, has)) = *seen
        && at == version
    {
        return has;
    }
    let has = document.subtree(document.root()).any(|node| {
        document
            .attrs(node)
            .is_some_and(|attrs| attrs.iter().any(|(name, _)| name.starts_with("on")))
    });
    *seen = Some((version, has));
    has
}

/// Where a parser-inserted script fetched for `element` waits: by its
/// `async` and `defer` attributes.
pub(crate) fn parser_place(document: &Document, element: NodeId) -> Place {
    if document.attr(element, "async").is_some() {
        Place::Soon
    } else if document.attr(element, "defer").is_some() {
        Place::Deferred
    } else {
        Place::Blocking
    }
}

/// The console's text for `error`, as HTML section 8.1.4.6 reports it;
/// none for a dead realm's refusal, which is no error of the page's, nor
/// for an exception a listener of the `error` event canceled (M26).
pub(crate) fn report_text(error: &ScriptError) -> Option<String> {
    let text = match error.kind {
        ErrorKind::Dead | ErrorKind::Handled => return None,
        ErrorKind::Rejection => format!("Uncaught (in promise) {}", error.message),
        ErrorKind::Budget => error.message.clone(),
        ErrorKind::Exception | ErrorKind::Heap => format!("Uncaught {}", error.message),
    };
    Some(if error.url.is_empty() {
        text
    } else {
        format!("{text} at {}:{}:{}", error.url, error.line, error.column)
    })
}

/// The console's text for a policy's refusal of `what`.
pub(crate) fn refusal_text(what: &str, violation: &Violation) -> String {
    format!("Content-Security-Policy refused {what}: {violation}")
}

/// What the console says of `violation` (M33.1): a refusal, or what a
/// report-only policy would have refused.
pub(crate) fn violation_text(violation: &Violation) -> String {
    match violation.disposition {
        net::csp::Disposition::Enforce => refusal_text(&violation.what, violation),
        net::csp::Disposition::Report => format!(
            "Content-Security-Policy (report-only) would refuse {}: {violation}",
            violation.what
        ),
    }
}

/// The console's level for `violation`: an error for a refusal, a
/// warning for a report.
pub(crate) fn violation_level(violation: &Violation) -> ConsoleLevel {
    match violation.disposition {
        net::csp::Disposition::Enforce => ConsoleLevel::Error,
        net::csp::Disposition::Report => ConsoleLevel::Warn,
    }
}

/// `document.title`'s getter (HTML section 3.1.7): the text of the
/// first `title` element in tree order, its ASCII white space stripped
/// and collapsed; empty without one.
pub(crate) fn document_title(document: &Document) -> String {
    let Some(title) = document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("title"))
    else {
        return String::new();
    };
    let text: String = document
        .children(title)
        .filter_map(|child| document.text(child))
        .collect();
    text.split(|c: char| c.is_ascii_whitespace())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `element`'s legacy `for` and `event` attributes, when it has
/// both, name the window's load event; a script with others is not run
/// ("prepare the script element", step 21).
fn for_the_window(document: &Document, element: NodeId) -> bool {
    let (Some(event), Some(target)) = (
        document.attr(element, "event"),
        document.attr(element, "for"),
    ) else {
        return true;
    };
    let trim = |s: &str| {
        s.trim_matches(|c: char| c.is_ascii_whitespace())
            .to_ascii_lowercase()
    };
    trim(target) == "window" && matches!(trim(event).as_str(), "onload" | "onload()")
}

/// The request for a classic script at `url`: no-cors, made by the
/// page's document, its body held to [`MAX_SCRIPT_BYTES`], checked
/// against the document's `csp` at each hop; none for a page with no
/// URL, which fetches nothing.
fn script_request(page: &Page, url: Url, csp: Csp) -> Option<Request> {
    Some(
        Request::get(url, Destination::Script)
            .initiated_by(page.initiator()?)
            .with_body_limit(MAX_SCRIPT_BYTES)
            .with_csp(csp),
    )
}

/// The script a response to a script's fetch is (HTML's "fetch a
/// classic script"), or why it is none: it needs an ok status and a type
/// Fetch lets a script have. It is decoded by its BOM, else its type's
/// `charset`, else `fallback`; its errors are muted when the response is
/// CORS-cross-origin (HTML): opaque, its request having left the
/// document's origin without CORS (M27).
fn script_source(response: &Response, fallback: Encoding) -> Result<Source, String> {
    if !response.ok() {
        return Err(format!("status {}", response.status));
    }
    if net::mime::blocked_as_script(response.mime_type.as_ref(), &response.headers) {
        return Err(match &response.mime_type {
            Some(mime_type) => format!("{} is not a script's type", mime_type.essence()),
            None => "no type, under nosniff".to_owned(),
        });
    }
    let encoding = net::mime::legacy_extract_encoding(response.mime_type.as_ref(), fallback);
    let (text, _) = encoding::decode(&response.body, encoding);
    let muted = matches!(
        response.response_type,
        ResponseType::Opaque | ResponseType::OpaqueRedirect
    );
    Ok(Source {
        text,
        url: response.url().to_string(),
        line: 1,
        muted,
        element: None,
        external: true,
        failed: false,
    })
}

/// What a `script` element is (HTML section 4.12.1.1, "prepare the
/// script element", step 9 on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptType {
    Classic,
    Module,
    ImportMap,
    SpeculationRules,
}

/// The type of the script `element` is, by its `type` and `language`
/// attributes; `None` when it is not a script at all.
pub(crate) fn script_type(document: &Document, element: NodeId) -> Option<ScriptType> {
    let type_attr = document.attr(element, "type");
    let language = document.attr(element, "language");
    let block_type = match (type_attr, language) {
        (Some(""), _) | (None, Some("")) | (None, None) => "text/javascript".to_owned(),
        (Some(value), _) => value.to_owned(),
        (None, Some(language)) => format!("text/{language}"),
    };
    let stripped = type_attr.map(|value| value.trim_matches(|c: char| c.is_ascii_whitespace()));
    if is_javascript_essence(&block_type) || stripped.is_some_and(is_javascript_essence) {
        return Some(ScriptType::Classic);
    }
    match block_type.to_ascii_lowercase().as_str() {
        "module" => Some(ScriptType::Module),
        "importmap" => Some(ScriptType::ImportMap),
        "speculationrules" => Some(ScriptType::SpeculationRules),
        _ => None,
    }
}

/// Whether `value` is a JavaScript MIME type essence match: one of the
/// essences, in any case, with no parameters (MIME Sniffing section 4.6).
fn is_javascript_essence(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    net::MimeType::parse(&lower).is_some_and(|mime| mime.essence() == lower && mime.is_javascript())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> Document {
        Document::from_node(&html::parse_document(source).expect("parsing never fails"))
    }

    /// Runs `script` as the engine runs one, `document` lent, of a page
    /// given to the engine and parsed whole.
    fn run(
        scripts: &mut Scripts,
        document: &mut Document,
        script: &Script<'_>,
    ) -> Option<Completion> {
        let mut page = Page::given(0);
        let mut network = Network::new();
        let mut lent = Lent {
            document,
            page: &mut page,
            parser: None,
            network: &mut network,
            ready: ReadyState::Complete,
            window: WindowEnv::default(),
        };
        scripts.run(&mut lent, script, How::default())
    }

    fn script(source: &str) -> Script<'_> {
        Script {
            source,
            url: "https://example.com/",
            line: 1,
            muted: false,
        }
    }

    /// `document.title` set to `value` in `document`, as a script's host
    /// sets it.
    fn set_document_title(document: &mut Document, value: &str) {
        let mut host = DocumentHost::default();
        host.document = std::mem::take(document);
        host.set_document_title(value);
        host.sweep();
        *document = std::mem::take(&mut host.document);
    }

    #[test]
    fn the_title_reads_and_writes_as_html_says() {
        let mut document = parsed("<title>  a \n b </title><p>x");
        assert_eq!(document_title(&document), "a b");
        set_document_title(&mut document, "new");
        assert_eq!(document_title(&document), "new");
        // None yet: one is made last in the head.
        let mut document = parsed("<meta charset=utf-8><p>x");
        assert_eq!(document_title(&document), "");
        set_document_title(&mut document, "  made  ");
        assert_eq!(document_title(&document), "made");
        let head = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("head"))
            .unwrap();
        let last = document.last_child(head).unwrap();
        assert_eq!(document.tag(last), Some("title"));
        // The empty string leaves the title element empty.
        set_document_title(&mut document, "");
        assert_eq!(document.children(last).count(), 0);
        // No document element: nothing.
        let mut empty = Document::new();
        set_document_title(&mut empty, "x");
        assert_eq!(empty.len(), 1);
        // The text replaced is freed, nothing holding it.
        let mut document = parsed("<title>old</title>");
        let before = document.len();
        set_document_title(&mut document, "new");
        assert_eq!(document.len(), before);
        document.validate().unwrap();
    }

    #[test]
    fn the_host_checks_what_the_bindings_check_and_replaces_the_document_element() {
        use js::{Call, Handle, ScriptingHost};
        let mut host = DocumentHost::default();
        host.document = parsed("<p>x");
        let html = host.document.document_element().unwrap();
        let handle = |node: NodeId| Handle(node.to_bits());
        let error = host
            .dom(Call::SetAttribute(handle(html), "a b".into(), "1".into()))
            .expect_err("a bad name");
        assert_eq!(error.name, "InvalidCharacterError");
        let error = host
            .dom(Call::CreateElement("1a".into()))
            .expect_err("a bad name");
        assert_eq!(error.name, "InvalidCharacterError");
        // The document's element may be replaced by another element.
        let made = match host.dom(Call::CreateElement("html".into())) {
            Ok(js::Answer::Node(Some(made))) => made,
            other => panic!("{other:?}"),
        };
        let root = handle(host.document.root());
        host.dom(Call::Replace {
            parent: root,
            node: made,
            child: handle(html),
        })
        .expect("replaced");
        assert_eq!(
            host.document.document_element().map(|e| e.to_bits()),
            Some(made.0)
        );
        host.sweep();
        assert!(!host.document.contains(html), "the old element is freed");
        host.document.validate().unwrap();
    }

    #[test]
    fn a_script_reads_and_writes_the_document_and_logs() {
        let mut document = parsed("<title>Before</title>");
        let mut scripts = Scripts::default();
        let done = run(
            &mut scripts,
            &mut document,
            &script(
                "console.log('hello', 1); document.title = document.title + ' and after'; 'ok'",
            ),
        );
        assert_eq!(done, Some(Completion::String("ok".to_owned())));
        assert_eq!(document_title(&document), "Before and after");
        let console = scripts.take_console();
        assert_eq!(console.len(), 1);
        assert_eq!(console[0].text, "hello 1");
        assert!(scripts.take_console().is_empty());
        assert_eq!(scripts.state(), ScriptState::Running { errors: 0 });
    }

    #[test]
    fn errors_go_to_the_console_and_the_next_script_runs() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::default();
        let inline = Script {
            source: "\nmissing()",
            url: "https://example.com/page.html",
            line: 4,
            muted: false,
        };
        assert_eq!(run(&mut scripts, &mut document, &inline), None);
        run(
            &mut scripts,
            &mut document,
            &script("Promise.reject(new Error('later'))"),
        );
        assert!(run(&mut scripts, &mut document, &script("1")).is_some());
        let console = scripts.take_console();
        assert_eq!(
            console[0].text,
            "Uncaught ReferenceError: missing is not defined at https://example.com/page.html:5:1"
        );
        assert_eq!(console[0].level, ConsoleLevel::Error);
        assert!(
            console[1]
                .text
                .starts_with("Uncaught (in promise) Error: later")
        );
        assert_eq!(scripts.state(), ScriptState::Running { errors: 2 });
    }

    #[test]
    fn a_realm_out_of_memory_stops_the_documents_scripts() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::with_limits(Limits {
            heap: 8 << 20,
            ..Limits::default()
        });
        run(
            &mut scripts,
            &mut document,
            &script("var kept = []; for (let i = 0; ; i++) kept.push('x'.repeat(1000) + i);"),
        );
        assert!(matches!(scripts.state(), ScriptState::Stopped { .. }));
        assert!(scripts.take_just_stopped());
        assert!(!scripts.take_just_stopped());
        assert_eq!(run(&mut scripts, &mut document, &script("1")), None);
        let console = scripts.take_console();
        assert_eq!(
            console.last().unwrap().text,
            "Scripts stopped on this page: out of memory"
        );
        // The document is the engine's again.
        assert_eq!(document.text_content(document.root()), "x");
    }

    #[test]
    fn the_console_keeps_its_last_entries_each_cut() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::default();
        run(
            &mut scripts,
            &mut document,
            &script(
                "for (let i = 0; i < 1004; i++) console.log(i); \
                 console.log('y'.repeat(20000)); console.log('y'.repeat(16383) + '\\u00e9');",
            ),
        );
        let console = scripts.take_console();
        assert_eq!(console.len(), MAX_CONSOLE_ENTRIES);
        assert_eq!(console[0].text, "6");
        assert_eq!(console[998].text.len(), MAX_CONSOLE_MESSAGE);
        // Cut short of the character that would cross the limit.
        assert_eq!(console[999].text.len(), MAX_CONSOLE_MESSAGE - 1);
    }

    #[test]
    fn a_script_that_runs_too_long_is_stopped_and_said_so() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::with_limits(Limits {
            steps: 10,
            ..Limits::default()
        });
        assert_eq!(
            run(&mut scripts, &mut document, &script("for (;;) {}")),
            None
        );
        let console = scripts.take_console();
        assert!(
            console[0]
                .text
                .starts_with("Script ran too long and was stopped at https://example.com/:1:"),
            "{}",
            console[0].text
        );
        // The realm lives on.
        assert!(run(&mut scripts, &mut document, &script("1")).is_some());
        assert_eq!(scripts.state(), ScriptState::Running { errors: 1 });
    }

    #[test]
    fn a_dead_realms_refusal_is_no_error_of_the_pages() {
        let mut scripts = Scripts::default();
        scripts.report(&ScriptError {
            kind: ErrorKind::Dead,
            message: "dead".to_owned(),
            url: String::new(),
            line: 0,
            column: 0,
        });
        assert!(scripts.take_console().is_empty());
        assert_eq!(scripts.state(), ScriptState::Running { errors: 0 });
    }

    #[test]
    fn a_restart_makes_a_new_realm_and_keeps_the_console() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::with_limits(Limits {
            heap: 8 << 20,
            ..Limits::default()
        });
        let source = "console.log(typeof seen); var seen = 1; missing()";
        run(&mut scripts, &mut document, &script(source));
        scripts.restart();
        assert_eq!(scripts.state(), ScriptState::Running { errors: 0 });
        run(&mut scripts, &mut document, &script(source));
        let console: Vec<String> = scripts.take_console().into_iter().map(|e| e.text).collect();
        assert_eq!(console[0], "undefined");
        assert_eq!(console[2], "undefined");
        // The limits stay.
        run(
            &mut scripts,
            &mut document,
            &script(
                "var kept = []; for (let i = 0; i < 20000; i++) kept.push('x'.repeat(1000) + i);",
            ),
        );
        assert!(matches!(scripts.state(), ScriptState::Stopped { .. }));
    }

    #[test]
    fn a_scripts_request_is_no_cors_from_the_document_and_bounded() {
        let url = Url::parse("https://example.com/page.html").unwrap();
        let page = Page::fetched(
            1,
            url.clone(),
            encoding::UTF_8,
            None,
            net::AddressSpace::Public,
        );
        let script = Url::parse("https://example.com/a.js").unwrap();
        let csp = Csp::none(url.origin());
        let request = script_request(&page, script.clone(), csp.clone()).expect("a request");
        assert_eq!(request.url, script);
        assert_eq!(request.destination, Destination::Script);
        assert_eq!(request.mode, net::Mode::NoCors);
        assert_eq!(request.body_limit, Some(MAX_SCRIPT_BYTES));
        assert!(request.csp.is_some());
        assert_eq!(request.origin, Some(url.origin()));
        assert_eq!(request.initiator.map(|i| i.url), Some(url));
        assert!(script_request(&Page::given(2), script, csp).is_none());
    }

    #[test]
    fn a_scripts_errors_are_muted_when_its_response_is_cors_cross_origin() {
        let response = |response_type| Response {
            url_list: vec![Url::parse("https://example.com/a.js").unwrap()],
            status: 200,
            status_message: Vec::new(),
            headers: net::HeaderList::new(),
            body: b"1".to_vec(),
            mime_type: None,
            address_space: None,
            response_type,
            cors_exposed: Vec::new(),
        };
        let muted = |response_type| {
            script_source(&response(response_type), encoding::UTF_8)
                .unwrap()
                .muted
        };
        assert!(!muted(ResponseType::Basic));
        assert!(!muted(ResponseType::Cors));
        assert!(muted(ResponseType::Opaque));
        assert!(muted(ResponseType::OpaqueRedirect));
    }

    #[test]
    fn a_script_elements_type_is_its_type_or_language() {
        let types = |source: &str| {
            let document = parsed(source);
            document
                .subtree(document.root())
                .filter(|&n| document.tag(n) == Some("script"))
                .map(|n| script_type(&document, n))
                .collect::<Vec<_>>()
        };
        use ScriptType::*;
        assert_eq!(
            types(
                "<script></script><script type=''></script><script type=' TEXT/JavaScript '></script>\
                 <script language=javascript></script><script language=''></script>\
                 <script type=application/x-ecmascript language=vbscript></script>"
            ),
            [Some(Classic); 6]
        );
        assert_eq!(
            types(
                "<script type='text/javascript;charset=utf-8'></script><script type=text/plain></script>\
                 <script language=vbscript></script><script type=module></script>\
                 <script type=ImportMap></script><script type=speculationrules></script>"
            ),
            [
                None,
                None,
                None,
                Some(Module),
                Some(ImportMap),
                Some(SpeculationRules)
            ]
        );
    }
}
