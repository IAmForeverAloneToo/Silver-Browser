//! The host a document's scripts reach it through (M24.4, M25,
//! `docs/design/dom-scripting.md`): the document, lent for each run, and
//! what must last between runs: the nodes scripts hold objects for, and
//! the subtrees they took out of the tree, freed once nothing holds them.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use dom::{Document, MutationError, Namespace, NodeData, NodeId, forms};
use js::{Answer, Call, ConsoleLevel, DomError, Handle, NestedScript, ScriptError, ScriptingHost};
use net::cookies::Context;
use style::animation::Animations;
use style::selector_matching::{Matcher, PseudoElement, SelectorList};
use style::{CascadeContext, FontMeasure, Styles, Stylesheets};

use super::canvas::{Canvases, Drawn, Element, Owner, names_context, needs_source, needs_text};
use super::cssom;
use super::import_map::MapError;
use super::modules::{Modules, Registered};
use super::workers::{DocumentWorkers, Start, credentials_mode, worker_url};
use super::{
    ConsoleEntry, How, Lent, MAX_SCRIPT_NESTING, MAX_TIMERS, ModuleWanted, Place, Prepared,
    ReadyState, ScriptNavigation, Source, TimeOrigin, Wanted, WindowEnv, cut, document_title,
    is_script, module_place, parser_place, prepare_element, report_text,
};
use crate::loader::Page;
use crate::sandbox::{NO_SAME_ORIGIN, Sandbox};
use crate::storage;

/// A script running, innermost last.
struct Frame {
    /// Its element: `document.currentScript`.
    element: Option<NodeId>,
    /// Whether it came from a file (HTML's ignore-destructive-writes).
    external: bool,
    /// Whether it is a script `document.write` wrote, run with the
    /// insertion point after its end tag, which goes when it ends.
    written: bool,
}

/// A script's scroll (D65): of the viewport (`None`) or of a scroll
/// container, across to `x` and down to `y`, each left where it is when
/// `None`; smooth (CSSOM View's `smooth` behavior) or at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScriptScroll {
    pub(crate) node: Option<NodeId>,
    pub(crate) x: Option<f32>,
    pub(crate) y: Option<f32>,
    pub(crate) smooth: bool,
}

/// What a run leaves the engine to do: the console's lines and the count
/// of errors among them, and the scripts to fetch.
#[derive(Default)]
pub(crate) struct Outputs {
    pub(crate) console: Vec<ConsoleEntry>,
    pub(crate) errors: usize,
    pub(crate) wanted: Vec<Wanted>,
    pub(crate) navigation: Option<ScriptNavigation>,
    /// The events to fire at elements after the run (M26): a script
    /// element's `error` for a `src` that cannot be fetched.
    pub(crate) events: Vec<(NodeId, &'static str)>,
    /// The element a script focused, to be scrolled into view (M26.6).
    pub(crate) scroll_to: Option<NodeId>,
    /// The scroll offsets scripts set (D65), in order.
    pub(crate) scrolls: Vec<ScriptScroll>,
    /// A validation message reported, for the notice (M26.9b).
    pub(crate) notice: Option<String>,
    /// The fetches scripts asked for (M27.1), by their numbers.
    pub(crate) fetches: Vec<(u32, net::Request)>,
    /// The fetches their signals aborted.
    pub(crate) aborted: Vec<u32>,
    /// The messages posted (M31), each with when it was, delivered in
    /// tasks.
    pub(crate) messages: Vec<(Duration, js::Task)>,
    /// The workers terminated, whose tasks not yet run are dropped.
    pub(crate) terminated: Vec<u32>,
    /// What the document's policies refused, or would have, in the run
    /// (M33.1), each with the element to blame.
    /// With the element they are of and the line and column of the
    /// code that asked, 0 0 for none.
    pub(crate) violations: Vec<(net::csp::Violation, Option<NodeId>, (u32, u32))>,
    /// The faces scripts added to the document's fonts or took away
    /// (D54).
    pub(crate) fonts: Vec<crate::web_fonts::FontChange>,
    /// The module scripts scripts inserted or wrote (D61), whose graphs
    /// the engine fetches after the run.
    pub(crate) modules: Vec<ModuleWanted>,
    /// Whether each script wanted, classic or module, in the order found,
    /// is a module: those to run in order keep it across both kinds.
    pub(crate) order: Vec<bool>,
    /// The import maps scripts inserted that were none (D61): each
    /// exception, to report, and its element's line.
    pub(crate) thrown: Vec<(MapError, u32)>,
    /// `document.open()` aborted the load's parser: its fetch is to be
    /// aborted too, and its deferred scripts dropped.
    pub(crate) parser_aborted: bool,
    /// `document.open()` stops a navigation under way.
    pub(crate) stop_navigation: bool,
}

/// The document's styles as a run's `getComputedStyle` sees them: the
/// page's sheets, prepared once a run, and the styles, as of `version`,
/// and of the journal's first `mark` changes.
struct ComputedStyles {
    sheets: Stylesheets,
    /// The sheets as parsed, to tell what a script's change of them
    /// changed.
    raw: Vec<css::Stylesheet>,
    styles: Styles,
    version: u64,
    mark: usize,
    /// The fonts' faces they were computed with ([`fonts_key`]).
    fonts: (usize, usize, usize),
}

/// What tells one state of `fonts`' document faces from another: the
/// faces, how many, and how many came or failed. The styles of the
/// font-relative units are computed again when it changes.
fn fonts_key(fonts: Option<&text::FontCollection>) -> (usize, usize, usize) {
    match fonts.and_then(text::FontCollection::document) {
        Some(faces) => {
            let (len, settled) = faces.settled();
            (Arc::as_ptr(faces) as usize, len, settled)
        }
        None => (0, 0, 0),
    }
}

/// A document's timers (M26, HTML section 8.6): each set timer by when
/// it is due, then by the order it was set in, with the nesting level
/// the task that runs it has. The callbacks are the realm's.
#[derive(Default)]
pub(crate) struct Timers {
    queue: BTreeMap<(Duration, u64), (u32, u32)>,
    /// Each timer's place in `queue`, by its id.
    by_id: HashMap<u32, (Duration, u64)>,
    /// The order of the next timer set.
    next: u64,
}

impl Timers {
    /// Sets timer `id` (or sets it again) to be due `delay` milliseconds
    /// after `now`, from a task at nesting level `level`: a delay below
    /// 0 is 0, and below 4 is 4 past the fifth level, as HTML clamps it.
    /// A new timer past [`MAX_TIMERS`] is refused.
    pub(crate) fn set(
        &mut self,
        id: u32,
        delay: i32,
        level: u32,
        now: Duration,
    ) -> Result<(), DomError> {
        let mut delay = u64::try_from(delay).unwrap_or(0);
        if level > 5 && delay < 4 {
            delay = 4;
        }
        match self.by_id.remove(&id) {
            Some(key) => {
                self.queue.remove(&key);
            }
            None if self.by_id.len() >= MAX_TIMERS => {
                return Err(DomError::new(
                    "RangeError",
                    format!("No more than {MAX_TIMERS} timers may be set."),
                ));
            }
            None => {}
        }
        let key = (now + Duration::from_millis(delay), self.next);
        self.next += 1;
        self.queue.insert(key, (id, level.saturating_add(1)));
        self.by_id.insert(id, key);
        Ok(())
    }

    /// Clears timer `id`, if it is set.
    pub(crate) fn clear(&mut self, id: u32) {
        if let Some(key) = self.by_id.remove(&id) {
            self.queue.remove(&key);
        }
    }

    /// When the next timer is due.
    pub(crate) fn next_due(&self) -> Option<Duration> {
        self.queue.keys().next().map(|(due, _)| *due)
    }

    /// When the next timer is due, and its order among those set.
    pub(crate) fn next_key(&self) -> Option<(Duration, u64)> {
        self.queue.keys().next().copied()
    }

    /// The order the next timer set will take: a task queued now runs
    /// after the timers due with it that were set before.
    pub(crate) fn mark(&self) -> u64 {
        self.next
    }

    /// The timer due first, taken: its id and the nesting level of the
    /// task that runs it.
    pub(crate) fn take_next(&mut self) -> Option<(u32, u32)> {
        let (_, (id, level)) = self.queue.pop_first()?;
        self.by_id.remove(&id);
        Some((id, level))
    }

    /// Clears every timer.
    pub(crate) fn clear_all(&mut self) {
        *self = Timers::default();
    }
}

/// The window's fonts, measuring for the cascade's font-relative units.
struct Measure<'a>(&'a text::FontCollection);

impl FontMeasure for Measure<'_> {
    fn units(&self, font: &style::computed::FontProps, font_size_px: f32) -> style::FontUnits {
        crate::font_units(self.0, font, font_size_px)
    }
}

/// The host a document's scripts reach it through.
#[derive(Default)]
pub(crate) struct DocumentHost {
    /// The document, lent for a run.
    pub(crate) document: Document,
    /// The nodes a script holds an object for: handles given out and not
    /// released (M25). None of them is freed.
    exposed: HashSet<NodeId>,
    /// The roots of the subtrees scripts made or took out of the tree,
    /// freed once out of it with nothing in them exposed.
    detached: Vec<NodeId>,
    /// The work done since the realm last asked.
    work: u64,
    /// The handlers the parser set, by element and text, whose refusal
    /// by the document's policies was told as they were set.
    pub(crate) parsed_handlers: HashSet<(NodeId, String)>,
    /// How many fetches the document has in flight (M27.1).
    pub(crate) in_flight: usize,
    /// The engine's client, lent for a run, for a synchronous XHR
    /// (M27.6).
    client: Option<Arc<dyn crate::loader::Fetcher>>,
    /// The engine's cookies, lent for a run (M27.4): its jar, or the
    /// browser's in a renderer (M34.1).
    cookies: Option<Arc<dyn crate::loader::Cookies>>,
    /// The document's page and parser, lent for a run; and where its
    /// loading stands.
    page: Option<Page>,
    parser: Option<html::Load>,
    /// The parser `document.open()` made (HTML section 8.4.2.1): what
    /// `write` gives it is parsed at once, `close` ends it.
    opened: Option<html::Parser>,
    /// The document's readiness when `document.open()` made it loading.
    before_open: ReadyState,
    /// That parser is parsing, its scripts running; and what they wrote,
    /// for its insertion point.
    writing: bool,
    nested_writes: String,
    /// The load's parser was aborted (HTML's "active parser was
    /// aborted"): `document.open()` does nothing.
    parser_aborted: bool,
    ready: ReadyState,
    /// The scripts running, innermost last.
    frames: Vec<Frame>,
    /// Scripts to run now, nested, in order.
    now: VecDeque<Source>,
    /// A parsing-blocking script is pending: what is written is not
    /// parsed until it has run.
    blocked: bool,
    /// What the run leaves the engine to do.
    outputs: Outputs,
    /// A script was not run for nesting too deep, and the console said
    /// so, in this run.
    too_deep: bool,
    /// The `script` elements made by scripts (`createElement`,
    /// `cloneNode`), which run when connected (HTML section 4.12.1.1),
    /// and those of them not started yet.
    made: HashSet<NodeId>,
    pending: Vec<NodeId>,
    /// The scripts an insertion made ready, in the order it found them,
    /// waiting while the one before them runs (DOM's post-connection
    /// steps, one at a time): it may remove or move them.
    batch: VecDeque<NodeId>,
    /// Script elements already started: prepared, or parsed in a
    /// fragment.
    started: HashSet<NodeId>,
    /// Made scripts whose "force async" flag was cleared by `async`.
    not_forced: HashSet<NodeId>,
    /// The nodes scripts hold that came into the document or left it,
    /// in order, for the bindings to hold their objects or let them go.
    pins: Vec<(NodeId, bool)>,
    /// What the window reports, lent for a run.
    window: WindowEnv,
    /// The styles a run's `getComputedStyle` computed, kept for the run.
    computed: Option<ComputedStyles>,
    /// The tree the last SVG geometry question was answered from (M32):
    /// its outer `<svg>`, the document's version it was built at, kept
    /// for the run, so that a script asking of many elements builds it
    /// once.
    svg_questions: Option<(NodeId, u64, Arc<svg::Tree>)>,
    /// The shown document's images, lent for a run (D65).
    images: crate::image_store::ImageStore,
    /// The layout scripts asked of (D65), by the document's version it
    /// is of; at most one a version.
    laid_out: Option<(u64, layout::DocumentLayout)>,
    /// The document's version the page's `<style>` sheets were last
    /// made again at in the run ([`Page::rescan_inline`]).
    inline_scanned: Option<u64>,
    /// The document's timers (M26).
    pub(crate) timers: Timers,
    /// The timer nesting level of the task the run is (HTML section
    /// 8.6): 0 but for a timer's.
    nesting: u32,
    /// The document's time origin, once it has one.
    pub(crate) origin: Option<TimeOrigin>,
    /// An animation frame callback waits for the next frame (M26).
    pub(crate) frame_wanted: bool,
    /// The document's canvases and the objects scripts made for them
    /// (M30).
    pub(crate) canvases: Canvases,
    /// The document's workers and its inbox (M31).
    pub(crate) workers: DocumentWorkers,
    /// The document's Web Audio (M40).
    pub(crate) audio: crate::webaudio::WebAudio,
    /// The document's WebGPU objects (M41).
    pub(crate) webgpu: crate::webgpu::WebGpu,
    pub(crate) webgl: crate::webgl::WebGl,
    /// The document's media elements (M40).
    pub(crate) media: crate::playback::Players,
    /// Whether the document is the DevTools panel's page (M36), and what
    /// it sent the browser since the engine last took it.
    pub(crate) panel: bool,
    pub(crate) devtools_out: Vec<String>,
    /// Whether the DevTools console's evaluation runs (M36), and its
    /// answer once it has one.
    pub(crate) evaluating: bool,
    pub(crate) evaluated: Option<String>,
    /// The `blob:` URLs the document's scripts made (gap batch A).
    blob_urls: crate::blob_urls::BlobOwner,
    /// The text its scripts' `TextDecoder`s are decoding (gap batch A).
    text_decoders: super::decoding::TextDecoders,
    /// Its `matchMedia()` lists (gap batch A).
    pub(crate) media_lists: super::media_lists::MediaLists,
    /// Its resize and intersection observers (gap batch A, D45).
    pub(crate) layout_observers: super::layout_observers::LayoutObservers,
    /// Whether the realm has the window's named properties as the
    /// document's names were when last taken (gap batch A).
    named_synced: bool,
    /// Its mutation observers (gap batch A, D42), and how deep the
    /// operations of several calls the bindings began are nested.
    mutations: super::mutations::MutationObservers,
    scopes: u32,
    /// Its module map, module scripts' graphs and import map (D61).
    pub(crate) modules: Modules,
    /// The fetch options its scripts ran with, each once, by their
    /// numbers less one: what their `import()`s fetch with.
    script_options: Vec<super::ScriptOptions>,
}

/// The most sets of script fetch options a document numbers: past them
/// a script's `import()`s fetch with the defaults.
const MAX_SCRIPT_OPTIONS: usize = 1024;

impl DocumentHost {
    /// The number the realm knows `options` by: 0 for the defaults.
    pub(crate) fn options_number(&mut self, options: super::ScriptOptions) -> u32 {
        if options == super::ScriptOptions::default() {
            return 0;
        }
        let at = match self.script_options.iter().position(|o| *o == options) {
            Some(at) => at,
            None if self.script_options.len() < MAX_SCRIPT_OPTIONS => {
                self.script_options.push(options);
                self.script_options.len() - 1
            }
            None => return 0,
        };
        u32::try_from(at + 1).unwrap_or(0)
    }

    /// The options numbered `number`: the defaults for 0 or one unknown.
    pub(crate) fn script_options(&self, number: u32) -> super::ScriptOptions {
        (number as usize)
            .checked_sub(1)
            .and_then(|at| self.script_options.get(at))
            .cloned()
            .unwrap_or_default()
    }
}

impl ScriptingHost for DocumentHost {
    fn console(&mut self, level: ConsoleLevel, message: String) {
        self.say(level, message);
    }

    fn console_record(&mut self, record: js::ConsoleRecord) {
        let entry = ConsoleEntry::recorded(super::ConsoleSource::Console, record);
        self.outputs.console.push(entry);
    }

    fn title(&mut self) -> String {
        document_title(&self.document)
    }

    fn set_title(&mut self, title: String) {
        self.set_document_title(&title);
    }

    fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        self.work += 1;
        // What changed before the call (the parser's) is a record a
        // change; the call's own changes are one operation, unless the
        // bindings began one of several calls.
        if self.scopes == 0 {
            self.observe_changes(false);
        }
        let inserts = matches!(
            call,
            Call::Insert { .. }
                | Call::Replace { .. }
                | Call::SetTextContent(..)
                | Call::SetAttribute(..)
                | Call::SetInnerHtml(..)
                | Call::SetOuterHtml(..)
                | Call::InsertAdjacentHtml { .. }
                | Call::Write(..)
                | Call::DocumentClose
        );
        let answer = self.answer(call);
        if self.scopes == 0 {
            self.observe_changes(true);
        }
        let answer = answer?;
        if inserts && !self.pending.is_empty() {
            self.prepare_made();
        }
        Ok(answer)
    }

    fn take_work(&mut self) -> u64 {
        std::mem::take(&mut self.work)
    }

    fn take_mutation_wake(&mut self) -> bool {
        self.mutations.take_wake()
    }

    fn take_named_changes(&mut self) -> Option<(bool, Vec<(String, bool)>)> {
        // The realm's first time, or more than were kept: all of them.
        let changes = match std::mem::take(&mut self.named_synced) {
            true => self.document.take_named_changes(),
            false => None,
        };
        self.named_synced = true;
        match changes {
            Some(changes) if changes.is_empty() => None,
            Some(changes) => Some((false, changes)),
            None => {
                let names = self.document.named();
                Some((true, names.into_iter().map(|name| (name, true)).collect()))
            }
        }
    }

    fn take_script(&mut self) -> Option<NestedScript> {
        loop {
            if let Some(source) = self.now.pop_front() {
                let written = source.element.is_some_and(|e| !self.made.contains(&e));
                self.frames.push(Frame {
                    element: source.element,
                    external: false,
                    written,
                });
                let options = self.options_number(source.options);
                return Some(NestedScript {
                    source: source.text,
                    url: source.url,
                    line: source.line,
                    base: source.base,
                    options,
                });
            }
            // The next script an insertion made ready, if still there.
            if !self.batch.is_empty() {
                self.prepare_made();
                continue;
            }
            if self.blocked {
                return None;
            }
            // What a write put in before the insertion point, parsed.
            let parser = self.parser.as_mut().and_then(html::Load::parser)?;
            let html::Stop::Script { element, line } =
                parser.parse_written(&mut self.document, usize::MAX)
            else {
                return None;
            };
            self.prepare_written(element, line);
        }
    }

    fn script_ran(&mut self) {
        if let Some(frame) = self.frames.pop()
            && frame.written
            && let Some(parser) = self.parser.as_mut().and_then(html::Load::parser)
        {
            parser.pop_insertion_point();
        }
    }

    fn report(&mut self, error: ScriptError) {
        if let Some(text) = report_text(&error) {
            self.outputs.errors += 1;
            self.say(ConsoleLevel::Error, text);
        }
    }
}

impl DocumentHost {
    /// The URL `document.cookie` reads and writes the cookies of: the
    /// document's, unless the document is cookie-averse (HTML 3.1.4),
    /// its URL not `http:` or `https:`.
    fn cookie_url(&self) -> Option<net::Url> {
        self.page
            .as_ref()?
            .url()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
            .cloned()
    }

    /// Why the document's scripts may not reach its Web Storage areas
    /// (HTML 12.2.2 and 12.2.3, M28), in the reference browser's words:
    /// the empty string when they may. An opaque origin has none.
    fn storage_refused(&self) -> String {
        let Some(page) = &self.page else {
            return "Access is denied for this document.".to_owned();
        };
        if page.sandbox().has(Sandbox::ORIGIN) {
            return NO_SAME_ORIGIN.to_owned();
        }
        if !page.origin().is_opaque() && self.window.storage.is_some() {
            return String::new();
        }
        if page.url().is_some_and(|url| url.scheme() == "data") {
            return "Storage is disabled inside 'data:' URLs.".to_owned();
        }
        "Access is denied for this document.".to_owned()
    }

    /// `f` of the Web Storage areas and the document's key to them; a
    /// `SecurityError` when its scripts may not reach them.
    fn with_areas<T>(
        &self,
        f: impl FnOnce(&dyn storage::Areas, &storage::AreaKey) -> T,
    ) -> Result<T, DomError> {
        let refused = self.storage_refused();
        if !refused.is_empty() {
            return Err(DomError::new("SecurityError", refused));
        }
        let (Some(page), Some(areas)) = (&self.page, &self.window.areas) else {
            return Err(DomError::new("SecurityError", refused));
        };
        Ok(f(areas.as_ref(), &self.storage_key(page)))
    }

    /// Where the document is in its tab (M33.2): its frame's place, its
    /// own origin making its context a cross-site one when it is of
    /// another site than the top; none at the top.
    fn partition(&self) -> Option<net::Partition> {
        let frame = self.window.partition.as_ref()?;
        let cross = self
            .page
            .as_ref()
            .is_none_or(|page| !frame.top.same_site(page.origin()));
        Some(net::Partition {
            top: frame.top.clone(),
            cross_site: frame.cross_site || cross,
        })
    }

    /// The document's storage key (the Storage Standard's, M33.2): its
    /// origin, and for a frame in a cross-site context the top-level
    /// site too, so that a third party's areas are kept apart by the
    /// site it is embedded under, as the reference browser keeps them.
    fn storage_key(&self, page: &Page) -> storage::AreaKey {
        storage::AreaKey {
            origin: page.origin().clone(),
            partition: self.partition(),
        }
    }

    /// The context `document.cookie` reads and writes in (M33.2): a
    /// top-level document's same-site, its partition its own site; a
    /// frame's same-site unless it is in a cross-site context, its
    /// partition the top-level site.
    fn cookie_context(&self) -> Context {
        match &self.partition() {
            Some(partition) => Context {
                same_site: !partition.cross_site,
                top_level: false,
                safe_method: true,
                partition: Some(partition.cookie_key()),
            },
            None => Context {
                partition: self
                    .page
                    .as_ref()
                    .map(|page| page.origin().site().serialization()),
                ..Context::SAME_SITE
            },
        }
    }

    /// Says `violations` on the console as they come and hands them to
    /// the run's outputs (M33.1), `element` the one to blame: their
    /// events and reports are made after the run.
    fn say_violations(&mut self, violations: Vec<net::csp::Violation>, element: Option<NodeId>) {
        self.say_violations_at(violations, element, (0, 0));
    }

    /// [`Host::say_violations`] of code at `position`, its line and
    /// column.
    fn say_violations_at(
        &mut self,
        violations: Vec<net::csp::Violation>,
        element: Option<NodeId>,
        position: (u32, u32),
    ) {
        for violation in violations {
            let level = super::violation_level(&violation);
            self.say(level, super::violation_text(&violation));
            self.outputs.violations.push((violation, element, position));
        }
    }

    /// Whether the document's flags, all of `flags`, are set (M33.2).
    fn sandboxed(&self, flags: Sandbox) -> bool {
        self.page
            .as_ref()
            .is_some_and(|page| page.sandbox().has(flags))
    }

    /// The `SecurityError` a sandboxed document's `document.cookie`
    /// throws without `allow-same-origin` (M33.2), `doing` what the
    /// script tried.
    fn cookie_refused(&self, doing: &str) -> Result<(), DomError> {
        match self.sandboxed(Sandbox::ORIGIN) {
            true => Err(DomError::new(
                "SecurityError",
                format!("Failed to {doing} 'Document': {NO_SAME_ORIGIN}"),
            )),
            false => Ok(()),
        }
    }

    /// Tells a change of an area (M28) to another document alive of the
    /// origin, if there is one, in a `storage` event.
    fn broadcast(
        &self,
        local: bool,
        key: Option<String>,
        old: Option<String>,
        new: Option<String>,
    ) {
        let (Some(page), Some(storage)) = (&self.page, &self.window.storage) else {
            return;
        };
        let area = self.storage_key(page);
        let origin = area.serialization();
        let (listener, tab) = self.window.listener;
        let mut storage = storage.lock().unwrap_or_else(PoisonError::into_inner);
        storage.broadcast(storage::Change {
            scope: storage::Scope::of(local, tab),
            origin,
            key,
            old,
            new,
            url: page
                .url()
                .map_or_else(String::new, |url| url.as_str().to_owned()),
            page: page.id(),
            source: storage::Source::Engine(listener),
            owner: area.origin,
            partition: area.partition,
        });
    }

    /// Answers `call`.
    fn answer(&mut self, call: Call) -> Result<Answer, DomError> {
        if let Some(answer) = self.workers.port_call(&call) {
            return answer;
        }
        match call {
            Call::Document => Ok(self.give(Some(self.document.root()))),
            Call::Wrap(node) => {
                let id = self.node(node)?;
                self.exposed.insert(id);
                let data = self
                    .document
                    .data(id)
                    .ok_or(MutationError::NoSuchNode)
                    .map_err(refused)?;
                let kind = data.node_type().to_string();
                let (name, ns) = match data {
                    NodeData::Element { tag, ns, .. } => {
                        (tag.clone(), ns.uri().unwrap_or_default().to_owned())
                    }
                    NodeData::Text(_) => ("#text".to_owned(), String::new()),
                    NodeData::Comment(_) => ("#comment".to_owned(), String::new()),
                    NodeData::ProcessingInstruction { target, .. } => {
                        (target.to_string(), String::new())
                    }
                    NodeData::DocumentType { name, .. } => (name.to_string(), String::new()),
                    NodeData::DocumentFragment => ("#document-fragment".to_owned(), String::new()),
                    NodeData::Document => ("#document".to_owned(), String::new()),
                };
                let connected = if self.connected(id) { "connected" } else { "" };
                let mut answer = vec![name, connected.to_owned(), ns, kind];
                // The attributes that may be event handlers (M26).
                answer.extend(
                    self.document
                        .attrs(id)
                        .unwrap_or_default()
                        .iter()
                        .filter(|(name, _)| name.starts_with("on"))
                        .map(|(name, _)| name.clone()),
                );
                Ok(Answer::Strings(answer))
            }
            Call::TakePins => {
                let pins = std::mem::take(&mut self.pins);
                Ok(Answer::Pins(
                    pins.into_iter()
                        .map(|(node, connected)| (handle(node), connected))
                        .collect(),
                ))
            }
            Call::Kind(node) => {
                let kind = self
                    .document
                    .data(self.node(node)?)
                    .map_or(9, NodeData::node_type);
                Ok(Answer::Number(f64::from(kind)))
            }
            Call::Tag(node) => {
                let tag = self
                    .document
                    .element_name(self.node(node)?)
                    .map_or("", |(_, tag)| tag);
                Ok(Answer::String(tag.to_owned()))
            }
            Call::Parent(node) => {
                let parent = self.document.parent(self.node(node)?);
                Ok(self.give(parent))
            }
            Call::FirstChild(node) => {
                let child = self.document.first_child(self.node(node)?);
                Ok(self.give(child))
            }
            Call::LastChild(node) => {
                let child = self.document.last_child(self.node(node)?);
                Ok(self.give(child))
            }
            Call::PreviousSibling(node) => {
                let sibling = self.document.prev_sibling(self.node(node)?);
                Ok(self.give(sibling))
            }
            Call::NextSibling(node) => {
                let sibling = self.document.next_sibling(self.node(node)?);
                Ok(self.give(sibling))
            }
            Call::Children(node) => {
                let node = self.node(node)?;
                let children: Vec<Handle> = self.document.children(node).map(handle).collect();
                self.work += children.len() as u64;
                Ok(Answer::Nodes(children))
            }
            Call::ElementChildren(node) => {
                let node = self.node(node)?;
                let mut children = Vec::new();
                for child in self.document.children(node) {
                    self.work += 1;
                    if self.document.is_element(child) {
                        children.push(handle(child));
                    }
                }
                Ok(Answer::Nodes(children))
            }
            Call::Version => Ok(Answer::Number(self.document.version() as f64)),
            Call::Connected(node) => {
                let node = self.node(node)?;
                let root = self.document.root();
                let top = self.document.ancestors(node).last().unwrap_or(node);
                Ok(Answer::Bool(top == root))
            }
            Call::Contains { node, other } => {
                let (node, other) = (self.node(node)?, self.node(other)?);
                Ok(Answer::Bool(
                    self.document.is_inclusive_ancestor(node, other),
                ))
            }
            Call::Data(node) => {
                let data = self
                    .document
                    .character_data(self.node(node)?)
                    .unwrap_or_default();
                self.work += data.len() as u64 / 64;
                Ok(Answer::String(data.to_owned()))
            }
            Call::SetData(node, data) => {
                let node = self.node(node)?;
                self.work += data.len() as u64 / 64;
                self.document.set_text(node, &data).map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::TextContent(node) => {
                let node = self.node(node)?;
                if matches!(
                    self.document.data(node),
                    Some(NodeData::Document | NodeData::DocumentType { .. })
                ) {
                    return Ok(Answer::MaybeString(None));
                }
                if let Some(data) = self.document.character_data(node) {
                    self.work += data.len() as u64 / 64;
                    return Ok(Answer::MaybeString(Some(data.to_owned())));
                }
                let mut text = String::new();
                for n in self.document.subtree(node) {
                    self.work += 1;
                    if let Some(data) = self.document.text(n) {
                        text.push_str(data);
                    }
                }
                self.work += text.len() as u64 / 64;
                Ok(Answer::MaybeString(Some(text)))
            }
            Call::SetTextContent(node, text) => {
                let node = self.node(node)?;
                self.work += text.len() as u64 / 64;
                self.set_text_content(node, &text)?;
                Ok(Answer::Nothing)
            }
            Call::Attribute(node, name) => {
                let node = self.node(node)?;
                let value = self.document.attr(node, &name);
                // A `style` the policies refused is empty on an element
                // of another namespace than HTML's, as the reference
                // browser keeps such an element's from its declarations.
                if name == "style"
                    && !self.document.is_html(node)
                    && let (Some(value), Some(page)) = (value, &self.page)
                    && !style::InlineStyles::allowed(page, node, value)
                {
                    return Ok(Answer::MaybeString(Some(String::new())));
                }
                Ok(Answer::MaybeString(value.map(str::to_owned)))
            }
            Call::SetAttribute(node, name, value) => {
                let node = self.node(node)?;
                if !valid_attribute_local_name(&name) {
                    return Err(DomError::new(
                        "InvalidCharacterError",
                        format!("'{name}' is not a valid attribute name."),
                    ));
                }
                self.work += (name.len() + value.len()) as u64 / 64;
                self.document
                    .set_attribute(node, &name, &value)
                    .map_err(refused)?;
                self.canvas_attribute(node, &name);
                // Its policies look at a `style` set so (M33.1).
                if name.eq_ignore_ascii_case("style")
                    && let Some(page) = self.page.as_mut()
                {
                    page.set_cssom_style(node, false);
                }
                Ok(Answer::Nothing)
            }
            Call::RemoveAttribute(node, name) => {
                let node = self.node(node)?;
                // Only an element has attributes; another node has none
                // to remove.
                if self.document.is_element(node) {
                    self.document
                        .remove_attribute(node, &name)
                        .map_err(refused)?;
                    self.canvas_attribute(node, &name);
                }
                Ok(Answer::Nothing)
            }
            Call::AttributeNames(node) => {
                let attrs = self.document.attrs(self.node(node)?).unwrap_or_default();
                Ok(Answer::Strings(
                    attrs.iter().map(|(name, _)| name.clone()).collect(),
                ))
            }
            Call::CreateElement(name) => {
                if !valid_element_local_name(&name) {
                    return Err(DomError::new(
                        "InvalidCharacterError",
                        format!("The tag name provided ('{name}') is not a valid name."),
                    ));
                }
                // An XML document's element is HTML's only in XHTML (DOM
                // section 4.5, `createElement`), its name as given.
                let element = match self.page.as_ref().and_then(Page::content_type) {
                    Some(kind) if kind != "application/xhtml+xml" => self
                        .document
                        .create_element_ns(Namespace::Other(None), &name, Vec::new()),
                    _ => self.document.create_element(&name),
                }
                .map_err(refused)?;
                self.detached.push(element);
                self.made_by_script(element, false);
                Ok(self.give(Some(element)))
            }
            Call::CreateElementNs(ns, qualified) => {
                let ns = Namespace::from_uri(ns.as_deref());
                let element = self
                    .document
                    .create_element_ns(ns, &qualified, Vec::new())
                    .map_err(refused)?;
                self.detached.push(element);
                self.made_by_script(element, false);
                Ok(self.give(Some(element)))
            }
            Call::CreateText(data) => {
                self.work += data.len() as u64 / 64;
                let text = self.document.create_text(&data).map_err(refused)?;
                self.detached.push(text);
                Ok(self.give(Some(text)))
            }
            Call::CreateComment(data) => {
                self.work += data.len() as u64 / 64;
                let made = self.document.create_comment(&data).map_err(refused)?;
                self.detached.push(made);
                Ok(self.give(Some(made)))
            }
            Call::CreateProcessingInstruction(target, data) => {
                self.work += (target.len() + data.len()) as u64 / 64;
                let made = self
                    .document
                    .create_processing_instruction(&target, &data)
                    .map_err(refused)?;
                self.detached.push(made);
                Ok(self.give(Some(made)))
            }
            Call::CreateFragment => {
                let made = self.document.create_fragment().map_err(refused)?;
                self.detached.push(made);
                Ok(self.give(Some(made)))
            }
            Call::ContextualFragment(node, markup) => {
                let node = self.node(node)?;
                // The start's element, or its parent's; a body for none,
                // and for an `html` element.
                let element = Some(node)
                    .filter(|&node| self.document.is_element(node))
                    .or_else(|| {
                        self.document
                            .parent(node)
                            .filter(|&parent| self.document.is_element(parent))
                    });
                let context = match element {
                    Some(element) if self.document.tag(element) != Some("html") => {
                        self.fragment_context(element)
                    }
                    _ => (Namespace::Html, "body".to_owned()),
                };
                let under = element.unwrap_or(node);
                let container = self.parse_fragment(&context, under, &markup)?;
                // Its scripts are not started: they run once inserted.
                let scripts: Vec<NodeId> = self
                    .document
                    .subtree(container)
                    .filter(|&node| is_script(&self.document, node))
                    .collect();
                for script in scripts {
                    self.started.remove(&script);
                    self.made_by_script(script, false);
                }
                let made = self.document.create_fragment().map_err(refused)?;
                self.detached.push(made);
                self.adopt(container, made, None)?;
                Ok(self.give(Some(made)))
            }
            Call::CreateDoctype(name, public_id, system_id) => {
                self.work += (name.len() + public_id.len() + system_id.len()) as u64 / 64;
                let made = self
                    .document
                    .create_doctype(&name, &public_id, &system_id)
                    .map_err(refused)?;
                self.detached.push(made);
                Ok(self.give(Some(made)))
            }
            Call::DoctypeIds(node) => match self.document.data(self.node(node)?) {
                Some(NodeData::DocumentType {
                    name,
                    public_id,
                    system_id,
                }) => Ok(Answer::Strings(vec![
                    name.to_string(),
                    public_id.to_string(),
                    system_id.to_string(),
                ])),
                _ => Ok(Answer::Strings(vec![String::new(); 3])),
            },
            Call::TemplateContent(node) => {
                let content = self.document.template_content(self.node(node)?);
                Ok(self.give(content))
            }
            Call::Doctype => {
                let doctype = self.document.doctype();
                Ok(self.give(doctype))
            }
            Call::ComparePosition(node, other) => {
                let (node, other) = (self.node(node)?, self.node(other)?);
                self.work += 1 + self.document.ancestors(node).count() as u64;
                let bits = self.document.compare_position(node, other);
                Ok(Answer::Number(f64::from(bits)))
            }
            Call::IsEqualNode(a, b) => {
                let (a, b) = (self.node(a)?, self.node(b)?);
                self.work += self.document.subtree(a).count() as u64;
                Ok(Answer::Bool(self.document.nodes_equal(a, b)))
            }
            Call::Normalize(node) => {
                let node = self.node(node)?;
                let texts: Vec<NodeId> = self
                    .document
                    .subtree(node)
                    .filter(|&n| self.document.is_text(n))
                    .collect();
                self.work += texts.len() as u64;
                self.document.normalize(node).map_err(refused)?;
                // The text nodes it took out may be freed.
                for text in texts {
                    if self.document.parent(text).is_none() {
                        self.taken_out(text);
                    }
                }
                Ok(Answer::Nothing)
            }
            Call::ReplaceData(node, offset, count, data) => {
                let node = self.node(node)?;
                self.work += data.len() as u64 / 64;
                self.document
                    .replace_data(node, offset, count, &data)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::SplitText(node, offset) => {
                let node = self.node(node)?;
                let made = self.document.split_text(node, offset).map_err(refused)?;
                if self.document.parent(made).is_none() {
                    self.detached.push(made);
                }
                Ok(self.give(Some(made)))
            }
            Call::NodeLength(node) => {
                let length = self.document.node_length(self.node(node)?);
                Ok(Answer::Number(f64::from(length)))
            }
            Call::IndexOf(node) => {
                let index = self.document.index_of(self.node(node)?);
                self.work += u64::from(index) / 64;
                Ok(Answer::Number(f64::from(index)))
            }
            Call::CreateRange(node) => {
                let id = self
                    .document
                    .create_range(self.node(node)?)
                    .map_err(refused)?;
                Ok(Answer::Number(f64::from(id)))
            }
            Call::DropRange(id) => {
                self.document.drop_range(id);
                Ok(Answer::Nothing)
            }
            Call::RangeOf(id) => {
                let range = self
                    .document
                    .range(id)
                    .ok_or_else(|| refused(MutationError::NoSuchNode))?;
                Ok(Answer::Numbers(vec![
                    handle(range.start.node).0 as f64,
                    f64::from(range.start.offset),
                    handle(range.end.node).0 as f64,
                    f64::from(range.end.offset),
                ]))
            }
            Call::SetRange(id, start, start_offset, end, end_offset) => {
                let range = dom::LiveRange {
                    start: dom::Boundary {
                        node: self.node(start)?,
                        offset: start_offset,
                    },
                    end: dom::Boundary {
                        node: self.node(end)?,
                        offset: end_offset,
                    },
                };
                self.document.set_range(id, range).map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::CompareBoundaries(a, a_offset, b, b_offset) => {
                let a = dom::Boundary {
                    node: self.node(a)?,
                    offset: a_offset,
                };
                let b = dom::Boundary {
                    node: self.node(b)?,
                    offset: b_offset,
                };
                self.work += 1 + self.document.ancestors(a.node).count() as u64;
                let order = match self.document.compare_boundaries(a, b) {
                    Some(std::cmp::Ordering::Less) => -1.0,
                    Some(std::cmp::Ordering::Equal) => 0.0,
                    Some(std::cmp::Ordering::Greater) => 1.0,
                    None => 2.0,
                };
                Ok(Answer::Number(order))
            }
            Call::CreateIterator(root) => {
                let id = self
                    .document
                    .create_iterator(self.node(root)?)
                    .map_err(refused)?;
                Ok(Answer::Number(f64::from(id)))
            }
            Call::DropIterator(id) => {
                self.document.drop_iterator(id);
                Ok(Answer::Nothing)
            }
            Call::IteratorOf(id) => {
                let state = self
                    .document
                    .iterator(id)
                    .ok_or_else(|| refused(MutationError::NoSuchNode))?;
                Ok(Answer::Numbers(vec![
                    handle(state.reference).0 as f64,
                    f64::from(u8::from(state.before)),
                ]))
            }
            Call::SetIterator(id, reference, before) => {
                let reference = self.node(reference)?;
                self.document.set_iterator(id, reference, before);
                Ok(Answer::Nothing)
            }
            Call::MoveBefore {
                parent,
                node,
                before,
            } => {
                let (parent, node) = (self.node(parent)?, self.node(node)?);
                let before = before.map(|before| self.node(before)).transpose()?;
                self.move_before(parent, node, before)?;
                Ok(Answer::Nothing)
            }
            Call::Geometry { node, scrolled } => {
                let node = self.node(node)?;
                Ok(Answer::Numbers(self.geometry(node, scrolled)))
            }
            Call::OffsetParent(node) => {
                let node = self.node(node)?;
                let parent = self.offset_parent(node);
                Ok(self.give(parent))
            }
            Call::ScrollOffset(node) => {
                let node = node.map(|node| self.node(node)).transpose()?;
                Ok(Answer::Numbers(self.scroll_offset(node)))
            }
            Call::ScrollTo(node, x, y, smooth) => {
                let node = node.map(|node| self.node(node)).transpose()?;
                self.scroll_to(node, x, y, smooth);
                Ok(Answer::Nothing)
            }
            Call::ScrollIntoView(node, block, inline, behavior) => {
                let node = self.node(node)?;
                if self.connected(node) {
                    self.scroll_into_view(node, &block, &inline, &behavior);
                }
                Ok(Answer::Nothing)
            }
            Call::ElementsFromPoint { x, y, all } => {
                let found = self.elements_from_point(x as f32, y as f32, all);
                Ok(Answer::Nodes(found.into_iter().map(handle).collect()))
            }
            Call::Cssom {
                op,
                owner,
                path,
                text,
                index,
            } => {
                let owner = owner.map(|owner| self.node(owner)).transpose()?;
                self.work += 1 + text.len() as u64 / 64;
                self.cssom(&op, owner, &path, &text, index)
            }
            Call::InnerText(node) => {
                let node = self.node(node)?;
                Ok(Answer::String(self.inner_text(node)))
            }
            Call::SelectedText(start, start_offset, end, end_offset) => {
                let start = dom::Boundary {
                    node: self.node(start)?,
                    offset: start_offset,
                };
                let end = dom::Boundary {
                    node: self.node(end)?,
                    offset: end_offset,
                };
                Ok(Answer::MaybeString(self.selected_text(start, end)))
            }
            Call::DocumentOpen => {
                self.only_html("open")?;
                let erased = self.open_document();
                Ok(Answer::Nodes(erased.into_iter().map(handle).collect()))
            }
            Call::DocumentClose => {
                self.only_html("close")?;
                if let Some(mut parser) = self.opened.take() {
                    parser.end();
                    self.parse_opened(&mut parser);
                    // Its parse over, the document is as ready as before.
                    self.ready = self.before_open;
                }
                Ok(Answer::Nothing)
            }
            Call::Insert {
                parent,
                node,
                before,
            } => {
                let (parent, node) = (self.node(parent)?, self.node(node)?);
                let before = before.map(|before| self.node(before)).transpose()?;
                self.insert(parent, node, before)?;
                Ok(Answer::Nothing)
            }
            Call::Remove { parent, node } => {
                let (parent, node) = (self.node(parent)?, self.node(node)?);
                if self.document.parent(node) != Some(parent) {
                    return Err(DomError::new(
                        "NotFoundError",
                        "The node to be removed is not a child of this node.",
                    ));
                }
                self.document.remove_child(parent, node).map_err(refused)?;
                self.taken_out(node);
                Ok(Answer::Nothing)
            }
            Call::Replace {
                parent,
                node,
                child,
            } => {
                let (parent, node, child) =
                    (self.node(parent)?, self.node(node)?, self.node(child)?);
                self.replace(parent, node, child)?;
                Ok(Answer::Nothing)
            }
            Call::Clone { node, deep } => {
                let node = self.node(node)?;
                let copy = self.clone_node(node, deep)?;
                // A copy of a script is a script's made, started if the
                // original is (HTML's cloning steps); those under it too.
                let pairs: Vec<(NodeId, NodeId)> = self
                    .document
                    .subtree(node)
                    .zip(self.document.subtree(copy))
                    .filter(|&(from, _)| is_script(&self.document, from))
                    .collect();
                for (from, to) in pairs {
                    let started = self.started.contains(&from);
                    self.made_by_script(to, started);
                }
                // A copy's `style` is its original's declarations, which
                // its policies are not asked about again (M33.1), as the
                // reference browser copies them.
                let styled: Vec<(NodeId, NodeId)> = self
                    .document
                    .subtree(node)
                    .zip(self.document.subtree(copy))
                    .filter(|&(from, _)| self.document.attr(from, "style").is_some())
                    .collect();
                for (from, to) in styled {
                    if self.style_allowed(from)
                        && let Some(page) = self.page.as_mut()
                    {
                        page.set_cssom_style(to, true);
                    }
                }
                Ok(self.give(Some(copy)))
            }
            Call::ElementById(root, id) => {
                let root = self.node(root)?;
                let found = if id.is_empty() {
                    None
                } else {
                    self.elements_under(root)
                        .into_iter()
                        .find(|&e| self.document.attr(e, "id") == Some(id.as_str()))
                };
                Ok(self.give(found))
            }
            Call::Query {
                root,
                selectors,
                all,
            } => {
                let root = self.node(root)?;
                let list = selector_list(&selectors)?;
                let scope = Some(root).filter(|&root| self.document.is_element(root));
                let mut matcher = Matcher::new(&self.document).with_scope(scope);
                let mut found = Vec::new();
                for element in self.document.subtree(root).skip(1) {
                    self.work += 1;
                    if matcher.matches_list(element, &list) {
                        found.push(handle(element));
                        if !all {
                            break;
                        }
                    }
                }
                Ok(Answer::Nodes(found))
            }
            Call::Matches(element, selectors) => {
                let element = self.node(element)?;
                let list = selector_list(&selectors)?;
                let mut matcher = Matcher::new(&self.document).with_scope(Some(element));
                Ok(Answer::Bool(matcher.matches_list(element, &list)))
            }
            Call::Closest(element, selectors) => {
                let element = self.node(element)?;
                let list = selector_list(&selectors)?;
                let mut matcher = Matcher::new(&self.document).with_scope(Some(element));
                let found = std::iter::once(element)
                    .chain(self.document.ancestors(element))
                    .find(|&node| matcher.matches_list(node, &list));
                Ok(self.give(found))
            }
            Call::ElementsByTagName(root, name) => {
                let root = self.node(root)?;
                // An HTML element's name is matched lowercased in an HTML
                // document, another's as given (DOM section 4.2.6).
                let xml = self.document.is_xml_document();
                let lower = name.to_ascii_lowercase();
                let found = self
                    .elements_under(root)
                    .into_iter()
                    .filter(|&e| {
                        name == "*"
                            || (!xml && self.document.tag(e) == Some(lower.as_str()))
                            || (xml && self.document.tag(e) == Some(name.as_str()))
                            || self
                                .document
                                .element_name(e)
                                .is_some_and(|(ns, tag)| *ns != Namespace::Html && tag == name)
                    })
                    .map(handle)
                    .collect();
                Ok(Answer::Nodes(found))
            }
            Call::ElementsByTagNameNs(root, ns, local) => {
                let root = self.node(root)?;
                let any_ns = ns.as_deref() == Some("*");
                let found = self
                    .elements_under(root)
                    .into_iter()
                    .filter(|&e| {
                        self.document
                            .element_name(e)
                            .is_some_and(|(namespace, tag)| {
                                (any_ns || namespace.uri() == ns.as_deref())
                                    && (local == "*" || dom::local_part(tag) == local)
                            })
                    })
                    .map(handle)
                    .collect();
                Ok(Answer::Nodes(found))
            }
            Call::ElementsByClassName(root, names) => {
                let root = self.node(root)?;
                let wanted: Vec<&str> = names
                    .split(is_ascii_space)
                    .filter(|n| !n.is_empty())
                    .collect();
                if wanted.is_empty() {
                    return Ok(Answer::Nodes(Vec::new()));
                }
                let elements = self.elements_under(root);
                let document = &self.document;
                let found = elements
                    .into_iter()
                    .filter(|&e| {
                        let classes = document.attr(e, "class").unwrap_or_default();
                        wanted
                            .iter()
                            .all(|w| classes.split(is_ascii_space).any(|c| c == *w))
                    })
                    .map(handle)
                    .collect();
                Ok(Answer::Nodes(found))
            }
            // An XML document's markup is XML's (DOM Parsing section 3.2).
            Call::InnerHtml(node) => {
                let node = self.node(node)?;
                let markup = match self.document.is_xml_document() {
                    true => html::xml::serialize_xml_children(&self.document, node),
                    false => html::serialize_children(&self.document, node, true),
                };
                self.work += markup.len() as u64 / 64;
                Ok(Answer::String(markup))
            }
            Call::OuterHtml(node) => {
                let node = self.node(node)?;
                let markup = match self.document.is_xml_document() {
                    true => html::xml::serialize_xml_node(&self.document, node),
                    false => html::serialize_node(&self.document, node, true),
                };
                self.work += markup.len() as u64 / 64;
                Ok(Answer::String(markup))
            }
            Call::SetInnerHtml(node, markup) => {
                let node = self.node(node)?;
                let context = self.fragment_context(node);
                let container = self.parse_fragment(&context, node, &markup)?;
                // A template's markup is its contents'.
                let node = self.document.template_content(node).unwrap_or(node);
                while let Some(child) = self.document.first_child(node) {
                    self.work += 1;
                    if self.document.remove_child(node, child).is_err() {
                        break;
                    }
                    self.taken_out(child);
                }
                self.adopt(container, node, None)?;
                Ok(Answer::Nothing)
            }
            Call::SetOuterHtml(node, markup) => {
                let node = self.node(node)?;
                let Some(parent) = self.document.parent(node) else {
                    return Ok(Answer::Nothing);
                };
                if parent == self.document.root() {
                    return Err(DomError::new(
                        "NoModificationAllowedError",
                        "This element's parent is a Document, which may not be modified.",
                    ));
                }
                let context = self.fragment_context(parent);
                let container = self.parse_fragment(&context, parent, &markup)?;
                self.adopt(container, parent, Some(node))?;
                self.document.remove_child(parent, node).map_err(refused)?;
                self.taken_out(node);
                Ok(Answer::Nothing)
            }
            Call::InsertAdjacentHtml {
                node,
                position,
                markup,
            } => {
                let node = self.node(node)?;
                let (parent, before) = match position.as_str() {
                    "beforebegin" | "afterend" => {
                        let parent = self
                            .document
                            .parent(node)
                            .filter(|&parent| parent != self.document.root())
                            .ok_or_else(|| {
                                DomError::new(
                                    "NoModificationAllowedError",
                                    "The element has no parent.",
                                )
                            })?;
                        let before = if position == "beforebegin" {
                            Some(node)
                        } else {
                            self.document.next_sibling(node)
                        };
                        (parent, before)
                    }
                    "afterbegin" => (node, self.document.first_child(node)),
                    _ => (node, None),
                };
                // An `html` context is taken for a body (HTML section 8.5.6).
                let context = match self.document.tag(parent) {
                    Some("html") => (Namespace::Html, "body".to_owned()),
                    _ => self.fragment_context(parent),
                };
                let container = self.parse_fragment(&context, parent, &markup)?;
                self.adopt(container, parent, before)?;
                Ok(Answer::Nothing)
            }
            Call::Write(text) => {
                self.only_html("write")?;
                let erased = self.write(&text);
                Ok(Answer::Nodes(erased.into_iter().map(handle).collect()))
            }
            Call::CurrentScript => {
                let current = self.frames.last().and_then(|frame| frame.element);
                Ok(self.give(current))
            }
            Call::ReadyState => Ok(Answer::String(self.ready.as_str().to_owned())),
            Call::ChildText(node) => {
                let node = self.node(node)?;
                let text: String = self
                    .document
                    .children(node)
                    .filter_map(|child| self.document.text(child))
                    .collect();
                Ok(Answer::String(text))
            }
            Call::ForceAsync(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(
                    self.made.contains(&node) && !self.not_forced.contains(&node),
                ))
            }
            Call::SetForceAsync(node, on) => {
                let node = self.node(node)?;
                if on {
                    self.not_forced.remove(&node);
                } else {
                    self.not_forced.insert(node);
                }
                Ok(Answer::Nothing)
            }
            Call::ParseDeclarations(text) => {
                self.work += text.len() as u64 / 64 + 1;
                let mut flat = Vec::new();
                for declared in cssom::declarations(&text) {
                    flat.push(declared.name);
                    flat.push(declared.value);
                    flat.push(if declared.important { "important" } else { "" }.to_owned());
                }
                Ok(Answer::Strings(flat))
            }
            Call::CssCanonical(name) => {
                let name = cssom::canonical(&name);
                Ok(Answer::MaybeString(cssom::known(&name).then_some(name)))
            }
            Call::CssShorthand(name, flat) => Ok(Answer::String(cssom::shorthand_value(
                &name,
                &cssom::from_flat(&flat),
            ))),
            Call::CssLonghands(name) => Ok(Answer::Strings(
                cssom::longhands(&name)
                    .unwrap_or_default()
                    .iter()
                    .map(|l| (*l).to_owned())
                    .collect(),
            )),
            Call::CssBlockText(flat) => {
                Ok(Answer::String(cssom::block_text(&cssom::from_flat(&flat))))
            }
            Call::CssValid(name, value) => {
                Ok(Answer::Bool(cssom::valid(&cssom::canonical(&name), &value)))
            }
            Call::ResolveUrl(text) => {
                let url = self
                    .page
                    .as_ref()
                    .and_then(|page| page.resolve(&self.document, &text));
                Ok(Answer::MaybeString(url.map(|url| url.to_string())))
            }
            Call::ResolveUrlUtf8(text) => {
                let url = self
                    .page
                    .as_ref()
                    .and_then(|page| page.resolve_utf8(&self.document, &text));
                Ok(Answer::MaybeString(url.map(|url| url.to_string())))
            }
            Call::ResolveUrlFromDocument(text) => {
                let url = self
                    .page
                    .as_ref()
                    .and_then(|page| page.resolve_from_document(&text));
                Ok(Answer::MaybeString(url.map(|url| url.to_string())))
            }
            Call::DocumentUrl => {
                let url = self.page.as_ref().and_then(|page| page.url());
                Ok(Answer::MaybeString(url.map(ToString::to_string)))
            }
            Call::Location => {
                let url = self.page.as_ref().and_then(|page| page.url());
                Ok(Answer::Strings(location_parts(url)))
            }
            Call::Origin => Ok(Answer::String(self.origin_text())),
            Call::WorkerStart {
                url,
                name,
                module,
                credentials,
            } => {
                let (Some(page), Some(client)) = (&self.page, &self.client) else {
                    return Err(DomError::not_supported());
                };
                // Parsed as the document's URLs are, its query in the
                // document's encoding.
                let parsed = page.resolve(&self.document, &url);
                let url = worker_url(&url, parsed, page.origin())?;
                let initiator = page.initiator().ok_or_else(DomError::not_supported)?;
                let start = Start {
                    url,
                    name,
                    module,
                    credentials: credentials_mode(&credentials),
                    client: client.clone(),
                    initiator,
                    secure: page.is_secure_context(),
                    viewport: self.window.viewport,
                    fonts: self.window.fonts.clone(),
                    compat: self.window.compat,
                    partition: self.partition(),
                    csp: page.policies().clone(),
                    referrer_policy: page.referrer_policy(&self.document),
                    local_network: page.local_network().cloned(),
                    // Its creator's storage key's caches (M39), unless
                    // its creator's storage is refused.
                    storage: match (self.storage_refused().is_empty(), &self.window.areas) {
                        (true, Some(areas)) => Some((areas.clone(), self.storage_key(page))),
                        _ => None,
                    },
                };
                Ok(Answer::Number(f64::from(self.workers.workers.start(start))))
            }
            Call::WorkerPost { worker, data } => {
                self.workers.workers.post(worker, data)?;
                Ok(Answer::Nothing)
            }
            Call::WorkerTerminate(worker) => {
                self.workers.workers.terminate(worker);
                self.outputs.terminated.push(worker);
                Ok(Answer::Nothing)
            }
            Call::PostToOwner(_)
            | Call::CloseWorker
            | Call::ImportScript(_)
            | Call::PortChannel
            | Call::PortPost { .. }
            | Call::PortStart(_)
            | Call::PortClose(_)
            | Call::PortAway(_)
            | Call::PortAdopt(_) => Err(DomError::not_supported()),
            Call::SecureContext => Ok(Answer::Bool(
                self.page.as_ref().is_none_or(Page::is_secure_context),
            )),
            Call::Fetch(call) => {
                let page = self
                    .page
                    .as_ref()
                    .ok_or_else(|| DomError::new("TypeError", "Failed to fetch"))?;
                let request = super::fetch::request(page.initiator(), &call)?;
                if self.in_flight >= super::fetch::MAX_SCRIPT_FETCHES {
                    return Err(DomError::new("TypeError", "Too many requests in flight."));
                }
                self.in_flight += 1;
                self.outputs.fetches.push((call.id, request));
                Ok(Answer::Nothing)
            }
            Call::FetchNow(call) => {
                let page = self
                    .page
                    .as_ref()
                    .ok_or_else(|| DomError::new("TypeError", "Failed to fetch"))?;
                let mut request = super::fetch::request(page.initiator(), &call)?;
                request.partition = self.partition();
                request.csp = Some(page.policies().clone());
                if request.referrer_policy.is_none() {
                    request.referrer_policy = Some(page.referrer_policy(&self.document));
                }
                let Some(client) = &self.client else {
                    return Ok(Answer::Fetched(None));
                };
                // The run, and the page, wait for it, as the reference
                // browser's does.
                let result = client.fetch(&request);
                let violations = page.take_violations();
                self.say_violations(violations, None);
                Ok(Answer::Fetched(match result {
                    Ok(response) => Some(Box::new(super::fetch::answer(&response))),
                    Err(error) => {
                        let text = super::fetch::failure_text(&request.url, error);
                        self.say(ConsoleLevel::Error, text);
                        None
                    }
                }))
            }
            Call::Decode(bytes, label) => Ok(Answer::MaybeString(
                encoding::Encoding::for_label(&label)
                    .map(|encoding| encoding::decode(&bytes, encoding).0),
            )),
            Call::AbortFetch(id) => {
                self.outputs.aborted.push(id);
                Ok(Answer::Nothing)
            }
            Call::MutationObserve {
                observer,
                node,
                flags,
                filter,
            } => {
                let node = self.node(node)?;
                let options = super::mutations::Options {
                    child_list: flags & 1 != 0,
                    attributes: flags & 2 != 0,
                    character_data: flags & 4 != 0,
                    subtree: flags & 8 != 0,
                    attribute_old_value: flags & 16 != 0,
                    character_data_old_value: flags & 32 != 0,
                    attribute_filter: filter,
                };
                self.mutations.observe(observer, node, options);
                self.watch();
                Ok(Answer::Nothing)
            }
            Call::NamedItems(name) => {
                if name.is_empty() {
                    return Ok(Answer::Nodes(Vec::new()));
                }
                let root = self.document.root();
                let found = self
                    .elements_under(root)
                    .into_iter()
                    .filter(|&e| {
                        let document = &self.document;
                        document.attr(e, "id") == Some(name.as_str())
                            || (document.is_html(e)
                                && matches!(
                                    document.local_name(e),
                                    Some("embed" | "form" | "img" | "object")
                                )
                                && document.attr(e, "name") == Some(name.as_str()))
                    })
                    .map(super::handle)
                    .collect();
                Ok(Answer::Nodes(found))
            }
            Call::BaseUrl => Ok(Answer::MaybeString(
                self.page
                    .as_ref()
                    .and_then(|page| page.base_url(&self.document))
                    .map(|url| url.to_string()),
            )),
            Call::BroadcastId => Ok(Answer::Number(super::broadcast_id() as f64)),
            Call::BroadcastPost { name, data, sender } => {
                // Of no storage key, an opaque origin's channel reaches no
                // other.
                let Some(page) = &self.page else {
                    return Ok(Answer::Nothing);
                };
                if page.origin().is_opaque() {
                    return Ok(Answer::Nothing);
                }
                let key = self.storage_key(page);
                if let Some(storage) = &self.window.storage {
                    storage
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .post(storage::Broadcast {
                            key,
                            name,
                            data,
                            sender,
                            source: storage::Source::Engine(self.window.listener.0),
                        });
                }
                Ok(Answer::Nothing)
            }
            Call::ResizeObserve {
                observer,
                node,
                observed,
            } => {
                let node = self.node(node)?;
                let observed = match observed {
                    1 => super::layout_observers::ObservedBox::Border,
                    2 => super::layout_observers::ObservedBox::DevicePixelContent,
                    _ => super::layout_observers::ObservedBox::Content,
                };
                self.layout_observers
                    .resize_observe(observer, node, observed);
                // A frame measures it.
                self.frame_wanted = true;
                Ok(Answer::Nothing)
            }
            Call::ResizeUnobserve { observer, node } => {
                match node {
                    Some(node) => {
                        let node = self.node(node)?;
                        self.layout_observers.resize_unobserve(observer, node);
                    }
                    None => self.layout_observers.resize_disconnect(observer),
                }
                Ok(Answer::Nothing)
            }
            Call::IntersectionObserve {
                observer,
                node,
                root,
                margin,
                thresholds,
            } => {
                let node = self.node(node)?;
                let root = root.map(|root| self.node(root)).transpose()?;
                let side = |i: usize| {
                    let value = margin.get(2 * i).copied().unwrap_or(0.0) as f32;
                    (value, margin.get(2 * i + 1).is_some_and(|&p| p != 0.0))
                };
                let margin = [side(0), side(1), side(2), side(3)];
                self.layout_observers
                    .intersection_observe(observer, node, root, margin, thresholds);
                self.frame_wanted = true;
                Ok(Answer::Nothing)
            }
            Call::IntersectionUnobserve { observer, node } => {
                match node {
                    Some(node) => {
                        let node = self.node(node)?;
                        self.layout_observers.intersection_unobserve(observer, node);
                    }
                    None => self.layout_observers.intersection_disconnect(observer),
                }
                Ok(Answer::Nothing)
            }
            Call::MutationScope(begin) => {
                self.scopes = match begin {
                    true => self.scopes.saturating_add(1),
                    false => self.scopes.saturating_sub(1),
                };
                Ok(Answer::Nothing)
            }
            Call::MutationDisconnect(observer) => {
                self.mutations.disconnect(observer);
                self.watch();
                Ok(Answer::Nothing)
            }
            Call::MutationTake(observer) => {
                let records = self.mutations.take(observer);
                Ok(Answer::String(super::mutations::json(&records)))
            }
            Call::MutationNotify => {
                let observers = self.mutations.notifying();
                self.watch();
                Ok(Answer::Numbers(
                    observers.into_iter().map(f64::from).collect(),
                ))
            }
            Call::MatchMedia(query) => {
                let (media, _, id) = self.media_lists.make(&query, &self.window.media);
                Ok(Answer::Strings(vec![media, id.to_string()]))
            }
            Call::MediaMatches(id) => Ok(Answer::Bool(
                self.media_lists.matches(id, &self.window.media),
            )),
            Call::MediaForget(id) => {
                self.media_lists.forget(id);
                Ok(Answer::Nothing)
            }
            Call::IdleDeadline => {
                // An idle period ends 50 ms on, at the next timer, or at
                // the next frame when one is wanted.
                let now = self.now();
                let mut end = now + std::time::Duration::from_millis(50);
                if let Some(due) = self.timers.next_due() {
                    end = end.min(due.max(now));
                }
                if self.frame_wanted {
                    end = end.min(now + std::time::Duration::from_micros(16_667));
                }
                // In tenths of a millisecond, as `performance.now()` is.
                let origin = self.origin.map(|origin| origin.at).unwrap_or_default();
                let tenths = end.saturating_sub(origin).as_micros() / 100;
                Ok(Answer::Number(tenths as f64 / 10.0))
            }
            Call::TextDecoderName(label) => Ok(super::decoding::name(&label)),
            Call::TextDecode {
                id,
                encoding,
                bytes,
                fatal,
                stream,
            } => Ok(self
                .text_decoders
                .decode(id, &encoding, &bytes, fatal, stream)),
            Call::MimeCharset(kind) => Ok(super::decoding::mime_charset(&kind)),
            Call::CssSupports(name, value) => Ok(Answer::Bool(super::cssom::supports(
                &name,
                value.as_deref(),
            ))),
            Call::BlobUrlCreate(bytes, kind) => Ok(Answer::String(self.blob_urls.create(
                bytes,
                kind,
                &self.blob_origin(),
            ))),
            Call::BlobUrlRevoke(url) => {
                self.blob_urls.revoke(&url, &self.blob_origin());
                Ok(Answer::Nothing)
            }
            Call::UrlParse(input, base) => Ok(super::url::parse(&input, base.as_deref())),
            Call::UrlSet { href, part, value } => Ok(super::url::set(&href, &part, &value)),
            Call::UrlHasCredentials(url) => Ok(Answer::Bool(
                net::Url::parse(&url).is_ok_and(|url| url.includes_credentials()),
            )),
            Call::SameOriginUrl(url) => Ok(Answer::Bool(net::Url::parse(&url).is_ok_and(|url| {
                self.page
                    .as_ref()
                    .is_some_and(|page| *page.origin() == url.origin())
            }))),
            Call::TargetOrigin(target) => match target.as_str() {
                "*" => Ok(Answer::String(target)),
                "/" => Ok(Answer::String(self.origin_text())),
                _ => net::Url::parse(&target)
                    .map(|url| Answer::String(url.origin().serialization()))
                    .map_err(|_| {
                        DomError::new(
                            "SyntaxError",
                            format!("Invalid target origin '{target}' in a call to 'postMessage'."),
                        )
                    }),
            },
            Call::PostMessage {
                target_origin,
                data,
            } => {
                // A message for another origin than the document's is
                // dropped, as when it would have been delivered.
                let origin = self.origin_text();
                if target_origin == "*" || target_origin == origin {
                    let now = self.now();
                    self.outputs.messages.push((
                        now,
                        js::Task::Message {
                            target: js::MessageTarget::Global,
                            data,
                            origin,
                        },
                    ));
                }
                Ok(Answer::Nothing)
            }
            Call::FormEntries { form, submitter } => {
                let form = self.node(form)?;
                if self.document.tag(form) != Some("form") {
                    return Err(DomError::new(
                        "TypeError",
                        "parameter 1 is not of type 'HTMLFormElement'.",
                    ));
                }
                let submitter = match submitter {
                    Some(submitter) => {
                        let submitter = self.node(submitter)?;
                        if !forms::is_submit_button(&self.document, submitter) {
                            return Err(DomError::new(
                                "TypeError",
                                "The specified element is not a submit button.",
                            ));
                        }
                        if forms::form_owner(&self.document, submitter) != Some(form) {
                            return Err(DomError::new(
                                "NotFoundError",
                                "The specified element is not owned by this form element.",
                            ));
                        }
                        Some(submitter)
                    }
                    None => None,
                };
                let mut out = Vec::new();
                for entry in forms::entry_list(&self.document, form, submitter, "UTF-8") {
                    out.push(entry.name);
                    match entry.value {
                        Some(value) => {
                            out.push("text".to_owned());
                            out.push(value);
                        }
                        None => {
                            out.push("file".to_owned());
                            out.push(String::new());
                        }
                    }
                }
                Ok(Answer::Strings(out))
            }
            Call::PlatformNewline => Ok(Answer::String(
                if cfg!(windows) { "\r\n" } else { "\n" }.to_owned(),
            )),
            Call::Cookie => {
                self.cookie_refused("read the 'cookie' property from")?;
                let cookies = self.cookie_url().and_then(|url| {
                    let cookies = self.cookies.as_ref()?;
                    cookies.script_cookies(&url, self.cookie_context())
                });
                Ok(Answer::String(cookies.map_or_else(String::new, |bytes| {
                    String::from_utf8_lossy(&bytes).into_owned()
                })))
            }
            Call::SetCookie(value) => {
                self.cookie_refused("set the 'cookie' property on")?;
                if let (Some(url), Some(cookies)) = (self.cookie_url(), &self.cookies) {
                    cookies.set_script_cookie(value.as_bytes(), &url, self.cookie_context());
                }
                Ok(Answer::Nothing)
            }
            call @ (Call::FrameSelf
            | Call::FrameParent(_)
            | Call::FrameTop(_)
            | Call::FrameLength(_)
            | Call::FrameChild(..)
            | Call::FrameNamed(..)
            | Call::FrameClosed(_)
            | Call::FrameOf(_)
            | Call::FramePost { .. }
            | Call::FrameNavigate { .. }
            | Call::WindowName
            | Call::SetWindowName(_)) => self.frame_call(call),
            Call::DocumentDomain => {
                Ok(Answer::String(match self.page.as_ref().map(Page::origin) {
                    Some(net::Origin::Tuple { host, .. }) => host.to_string(),
                    _ => String::new(),
                }))
            }
            Call::SetDocumentDomain(_) if self.sandboxed(Sandbox::DOCUMENT_DOMAIN) => {
                Err(DomError::new(
                    "SecurityError",
                    "Failed to set the 'domain' property on 'Document': Assignment is forbidden \
                     for sandboxed iframes.",
                ))
            }
            Call::SetDocumentDomain(value) => match self.page.as_ref().map(Page::origin) {
                Some(net::Origin::Tuple { host, .. }) if host.has_registrable_suffix(&value) => {
                    Ok(Answer::Nothing)
                }
                Some(net::Origin::Tuple { host, .. }) => Err(DomError::new(
                    "SecurityError",
                    format!("'{value}' is not a suffix of '{host}'."),
                )),
                _ => Err(DomError::new(
                    "SecurityError",
                    "The document's origin is opaque.",
                )),
            },
            Call::StorageOpen(_) => Ok(Answer::String(self.storage_refused())),
            Call::StorageLength(local) => {
                self.with_areas(|areas, area| Answer::Number(areas.length(local, area) as f64))
            }
            Call::StorageKey(local, index) => self.with_areas(|areas, area| {
                Answer::MaybeString(
                    usize::try_from(index)
                        .ok()
                        .and_then(|i| areas.key(local, area, i)),
                )
            }),
            Call::StorageKeys(local) => {
                self.with_areas(|areas, area| Answer::Strings(areas.keys(local, area)))
            }
            Call::StorageGet(local, key) => {
                self.with_areas(|areas, area| Answer::MaybeString(areas.get(local, area, &key)))
            }
            Call::StorageSet(local, key, value) => {
                match self.with_areas(|areas, area| areas.set(local, area, &key, &value))? {
                    Err(storage::OverQuota) => Ok(Answer::Bool(false)),
                    Ok(None) => Ok(Answer::Bool(true)),
                    Ok(Some(old)) => {
                        self.broadcast(local, Some(key), old, Some(value));
                        Ok(Answer::Bool(true))
                    }
                }
            }
            Call::StorageRemove(local, key) => {
                let old = self.with_areas(|areas, area| areas.remove(local, area, &key))?;
                if let Some(old) = old {
                    self.broadcast(local, Some(key), Some(old), None);
                }
                Ok(Answer::Nothing)
            }
            Call::Idb(request) => {
                self.with_areas(|areas, key| Answer::String(areas.idb(key, &request)))
            }
            Call::Caches(request, bytes) => {
                // The reference browser's words for a sandbox's (M39).
                if self
                    .page
                    .as_ref()
                    .is_some_and(|page| page.sandbox().has(Sandbox::ORIGIN))
                {
                    return Err(DomError::new(
                        "SecurityError",
                        "Cache storage is disabled because the context is sandboxed and lacks the 'allow-same-origin' flag.",
                    ));
                }
                self.with_areas(|areas, key| {
                    let (answer, bytes) = areas.caches(key, &request, &bytes);
                    Answer::Pair(answer, bytes)
                })
            }
            // A version's alone (M39).
            Call::ServiceWorkerRespond(..) => Err(DomError::not_supported()),
            Call::ServiceWorker(request) => {
                // The reference browser's words for a sandbox's (M39).
                if self
                    .page
                    .as_ref()
                    .is_some_and(|page| page.sandbox().has(Sandbox::ORIGIN))
                {
                    return Err(DomError::new(
                        "SecurityError",
                        "Service worker is disabled because the context is sandboxed and lacks the 'allow-same-origin' flag.",
                    ));
                }
                let page = self.page.as_ref().ok_or_else(DomError::not_supported)?;
                let url = page.url().cloned().ok_or_else(DomError::not_supported)?;
                let base = page.base_url(&self.document).unwrap_or_else(|| url.clone());
                let client = self.workers.service_client(
                    &self.storage_key(page),
                    page.id(),
                    url,
                    base,
                    self.window.partition.is_some(),
                );
                self.with_areas(|areas, _| Answer::String(areas.service_worker(&client, &request)))
            }
            Call::StorageUsage => self.with_areas(|areas, key| {
                // The caches' share too (M39), for `usageDetails`.
                let (caches, _) = areas.caches(key, r#"{"op":"usage"}"#, &[]);
                let caches = devtools::Json::parse(&caches)
                    .ok()
                    .and_then(|answer| answer.get("result").and_then(devtools::Json::as_u64))
                    .unwrap_or(0);
                Answer::Strings(vec![
                    areas.usage(key).to_string(),
                    crate::storage::devtools::SITE_QUOTA.to_string(),
                    caches.to_string(),
                ])
            }),
            Call::Audio(request, bytes) => {
                let activated = self
                    .window
                    .frames
                    .as_ref()
                    .is_some_and(|env| crate::frames::lock(&env.tree).may_start_sound());
                // A `MediaElementAudioSourceNode`'s element, by its handle.
                let element = devtools::Json::parse(&request)
                    .ok()
                    .filter(|op| {
                        op.get("kind").and_then(devtools::Json::as_str)
                            == Some("mediaElementSource")
                    })
                    .and_then(|op| op.get("element").and_then(devtools::Json::as_f64))
                    .and_then(|handle| self.node(Handle(handle as u64)).ok())
                    .and_then(|node| self.media.sound(node, &self.document));
                let env = crate::webaudio::Env {
                    inbox: self.workers.inbox(),
                    activated,
                    element,
                };
                let (json, bytes) = self.audio.call(&request, bytes, &env);
                Ok(Answer::Pair(json, bytes))
            }
            Call::Gpu(request, bytes) => {
                let (json, bytes) = self.webgpu.call(
                    &request,
                    bytes,
                    self.window.gpu.as_ref(),
                    &self.workers.inbox(),
                );
                Ok(Answer::Pair(json, bytes))
            }
            Call::WebGl(call) => {
                let document = &self.document;
                let canvases = &self.canvases;
                let size = |handle: f64| -> (u32, u32) {
                    if handle < 0.0 {
                        canvases
                            .offscreen_size((-handle) as u32)
                            .unwrap_or((300, 150))
                    } else {
                        dom::NodeId::from_bits(handle as u64)
                            .filter(|&node| document.contains(node))
                            .map_or((300, 150), |node| layout::canvas_size(document, node))
                    }
                };
                let inbox = self.workers.inbox();
                let env = crate::webgl::Env {
                    client: self.window.gpu.as_ref(),
                    inbox: &inbox,
                    canvas_size: &size,
                };
                self.webgl.call(*call, &env)
            }
            Call::Media(request) => Ok(Answer::String(self.media_call(&request))),
            Call::IdbWake => {
                let now = self.now();
                self.outputs.messages.push((now, js::Task::Idb));
                Ok(Answer::Nothing)
            }
            Call::DevToolsAnswer(answer) => {
                if !self.evaluating {
                    return Err(DomError::not_supported());
                }
                self.evaluated = Some(answer);
                Ok(Answer::Nothing)
            }
            Call::DevToolsSend(message) => {
                if !self.panel {
                    return Err(DomError::not_supported());
                }
                if message.len() > devtools::MAX_MESSAGE_BYTES
                    || self.devtools_out.len() >= MAX_PANEL_MESSAGES
                {
                    return Err(DomError::new(
                        "QuotaExceededError",
                        "The browser has not taken the panel's messages.",
                    ));
                }
                self.devtools_out.push(message);
                Ok(Answer::Nothing)
            }
            Call::StorageClear(local) => {
                if self.with_areas(|areas, area| areas.clear(local, area))? {
                    self.broadcast(local, None, None, None);
                }
                Ok(Answer::Nothing)
            }
            Call::OpenWindow { url, target } => self.open_window(&url, &target),
            Call::CloseWindow => self.close_window(),
            Call::Navigate { url, replace } => {
                let parsed = self
                    .page
                    .as_ref()
                    .and_then(|page| page.resolve(&self.document, &url))
                    .ok_or_else(|| {
                        DomError::new("SyntaxError", format!("'{url}' is not a valid URL."))
                    })?;
                self.outputs.navigation = Some(ScriptNavigation {
                    url: Some(parsed),
                    replace,
                    reload: false,
                    link: false,
                    post: None,
                    target: String::new(),
                    form: false,
                    element: None,
                });
                Ok(Answer::Nothing)
            }
            Call::Reload => {
                self.outputs.navigation = Some(ScriptNavigation {
                    url: None,
                    replace: true,
                    reload: true,
                    link: false,
                    post: None,
                    target: String::new(),
                    form: false,
                    element: None,
                });
                Ok(Answer::Nothing)
            }
            Call::NavigatorInfo => {
                let (agent, mode) = if self.window.compat {
                    (net::COMPAT_USER_AGENT, "chrome")
                } else {
                    (net::USER_AGENT, "gecko")
                };
                let mut info = vec![agent.to_owned(), PLATFORM.to_owned(), mode.to_owned()];
                info.extend(self.window.languages.iter().cloned());
                Ok(Answer::Strings(info))
            }
            Call::Viewport => {
                let (width, height) = self.window.viewport;
                // `devicePixelRatio`: the display's scale (M32.4).
                Ok(Answer::Numbers(vec![
                    f64::from(width),
                    f64::from(height),
                    f64::from(self.window.media.resolution),
                ]))
            }
            Call::DocumentInfo => {
                let page = self.page.as_ref();
                let url = page
                    .and_then(|page| page.url())
                    .map_or_else(|| "about:blank".to_owned(), ToString::to_string);
                let encoding = page.map_or(encoding::UTF_8, |page| page.encoding());
                let content_type = page
                    .and_then(|page| page.content_type())
                    .unwrap_or("text/html");
                Ok(Answer::Strings(vec![
                    url,
                    encoding.name().to_owned(),
                    content_type.to_owned(),
                    "CSS1Compat".to_owned(),
                    page.map(Page::referrer).unwrap_or_default().to_owned(),
                ]))
            }
            Call::ComputedValue {
                node,
                pseudo,
                property,
            } => {
                let node = self.node(node)?;
                // Out of the document there is no style; the cascade
                // would say so too, but need not be run to.
                if !self.connected(node) {
                    return Ok(Answer::MaybeString(None));
                }
                let pseudo = match pseudo.as_str() {
                    "" => None,
                    "before" => Some(PseudoElement::Before),
                    "after" => Some(PseudoElement::After),
                    "first-letter" => Some(PseudoElement::FirstLetter),
                    "first-line" => Some(PseudoElement::FirstLine),
                    "marker" => Some(PseudoElement::Marker),
                    _ => return Ok(Answer::MaybeString(None)),
                };
                // The properties CSSOM names resolve to their used values
                // (D65), from the layout, and the shorthands of them (D66).
                let used = pseudo.is_none()
                    && (USED_PROPERTIES.contains(&property.as_str())
                        || property == "margin"
                        || property == "padding"
                        || property.starts_with("margin-")
                        || property.starts_with("padding-")
                        || matches!(property.as_str(), "inline-size" | "block-size"));
                let geometry = if used {
                    self.geometry(node, false)
                } else {
                    Vec::new()
                };
                let insets = if pseudo.is_none()
                    && matches!(
                        property.as_str(),
                        "top"
                            | "right"
                            | "bottom"
                            | "left"
                            | "inset"
                            | "inset-block"
                            | "inset-inline"
                            | "inset-block-start"
                            | "inset-block-end"
                            | "inset-inline-start"
                            | "inset-inline-end"
                    ) {
                    self.used_insets(node)
                } else {
                    [None; 4]
                };
                let styles = self.styles_now();
                let style = match pseudo {
                    None => styles.and_then(|styles| styles.get(node)),
                    Some(pseudo) => styles.and_then(|styles| styles.pseudo(node, pseudo)),
                };
                let Some(style) = style.cloned() else {
                    return Ok(Answer::MaybeString(None));
                };
                let longhand = |name: &str| {
                    let side = ["top", "right", "bottom", "left"]
                        .iter()
                        .position(|side| *side == name);
                    match side.and_then(|i| insets[i]) {
                        Some(value) => Some(style::serialize::px(value)),
                        None => used_value(&style, name, &geometry),
                    }
                };
                // URLs resolved against the document's base, which the
                // style crate has not (gap batch A, G105).
                let url = |url: &str| {
                    let page = self.page.as_ref()?;
                    Some(page.resolve(&self.document, url)?.as_str().to_owned())
                };
                let resolver = style::serialize::Resolver {
                    longhand: &longhand,
                    url: &url,
                };
                let value = style::serialize::resolved_value(&style, &property, &resolver);
                Ok(Answer::MaybeString(value))
            }
            Call::SvgQuery { node, query, args } => {
                let node = self.node(node)?;
                let (a, b) = (args.first().copied(), args.get(1).copied());
                let question = match query.as_str() {
                    "bbox" => svg::Query::BBox,
                    "ctm" => svg::Query::Ctm,
                    "textLength" => svg::Query::TextLength,
                    "totalLength" => svg::Query::TotalLength,
                    "pointAtLength" => svg::Query::PointAtLength(a.unwrap_or(0.0)),
                    "inFill" => svg::Query::InFill(a.unwrap_or(0.0), b.unwrap_or(0.0)),
                    "inStroke" => svg::Query::InStroke(a.unwrap_or(0.0), b.unwrap_or(0.0)),
                    _ => return Ok(Answer::Numbers(Vec::new())),
                };
                if !self.connected(node) {
                    return Ok(Answer::Numbers(Vec::new()));
                }
                Ok(Answer::Numbers(
                    self.svg_query(node, question).unwrap_or_default(),
                ))
            }
            Call::ComputedProperties => Ok(Answer::Strings(
                style::serialize::COMPUTED_PROPERTIES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
            )),
            Call::Released(node) => {
                if let Some(id) = dom::NodeId::from_bits(node.0) {
                    self.exposed.remove(&id);
                }
                Ok(Answer::Nothing)
            }
            Call::SetTimer { id, delay } => {
                let now = self.now();
                self.timers.set(id, delay, self.nesting, now)?;
                Ok(Answer::Nothing)
            }
            Call::ClearTimer(id) => {
                self.timers.clear(id);
                Ok(Answer::Nothing)
            }
            Call::Now => {
                let since = self
                    .now()
                    .saturating_sub(self.origin.map(|origin| origin.at).unwrap_or_default());
                // A tenth of a millisecond, as the reference browser
                // coarsens a page that is not cross-origin isolated.
                let tenths = since.as_micros() / 100;
                Ok(Answer::Number(tenths as f64 / 10.0))
            }
            Call::TimeOrigin => Ok(Answer::Number(
                self.origin
                    .map(|origin| origin.epoch_ms)
                    .unwrap_or_default(),
            )),
            Call::Ancestors(node) => {
                let node = self.node(node)?;
                let ancestors: Vec<Handle> = self.document.ancestors(node).map(handle).collect();
                self.work += ancestors.len() as u64;
                Ok(Answer::Nodes(ancestors))
            }
            Call::Activator(node) => {
                let node = self.node(node)?;
                let found = std::iter::once(node)
                    .chain(self.document.ancestors(node))
                    .find(|&n| has_activation_behaviour(&self.document, n));
                Ok(self.give(found))
            }
            Call::Activate(node) => {
                let node = self.node(node)?;
                self.activate(node);
                Ok(Answer::Nothing)
            }
            Call::Disabled(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(forms::actually_disabled(&self.document, node)))
            }
            Call::FormOwner(node) => {
                let node = self.node(node)?;
                let owner = forms::form_associated(&self.document, node)
                    .then(|| forms::form_owner(&self.document, node))
                    .flatten();
                Ok(self.give(owner))
            }
            Call::WantFrame => {
                self.frame_wanted = true;
                Ok(Answer::Nothing)
            }
            Call::Hidden => Ok(Answer::Bool(self.window.hidden)),
            Call::Fonts(op, data) => {
                // The `@font-face` rules of the sheets as they stand now,
                // which the engine's faces are made from.
                let rules = match (op.as_str(), &self.page) {
                    ("rules", Some(page)) => {
                        self.work += 1;
                        let sheets = page.sheets_now(&self.document, &self.window.media);
                        style::Stylesheets::new(sheets).font_faces(&self.window.media)
                    }
                    _ => Vec::new(),
                };
                // Laid out first when a layout is to come, so that the
                // faces it needs are asked for (D54): `ready` and `status`
                // say loading until one asks for nothing new. A window of
                // no size lays nothing out, and waits for nothing.
                if op == "status" && self.window.fonts_unsettled && !self.window.fonts_loading {
                    self.layout_now();
                }
                // Loading too while a face a canvas or that layout asked
                // for in this run is wanted or coming.
                let asked = self
                    .window
                    .layout_fonts
                    .as_ref()
                    .and_then(text::FontCollection::document)
                    .is_some_and(|faces| faces.is_loading());
                let env = crate::web_fonts::FontsEnv {
                    loading: self.window.fonts_loading || asked,
                    rules: &rules,
                    fonts: self.window.layout_fonts.as_ref(),
                };
                let before = self.outputs.fonts.len();
                let answer = crate::web_fonts::answer(&op, data, &env, &mut self.outputs.fonts);
                // A face added is laid out with in this run already, as
                // its `load()` has settled.
                if let Some(fonts) = &mut self.window.layout_fonts
                    && crate::web_fonts::lay_over(fonts, &self.outputs.fonts[before..])
                {
                    self.laid_out = None;
                }
                Ok(answer)
            }
            Call::Focusable(node) => {
                let node = self.node(node)?;
                self.styles_now();
                let styles = self.computed.as_ref().map(|computed| &computed.styles);
                Ok(Answer::Bool(crate::focus::focusable(
                    &self.document,
                    styles,
                    node,
                )))
            }
            Call::Tabbable(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(crate::focus::natively_focusable(
                    &self.document,
                    node,
                )))
            }
            Call::Focused => Ok(self.give(self.document.focused())),
            Call::SetFocus {
                node,
                prevent_scroll,
            } => {
                let node = node.map(|node| self.node(node)).transpose()?;
                // A focus a script gives is shown after a key, and in a
                // text control always, as the reference browser decides.
                let visible = self.window.keyboard
                    || node.is_some_and(|node| forms::is_text_control(&self.document, node));
                self.document.set_focus(node, visible);
                if !prevent_scroll && node.is_some() {
                    self.outputs.scroll_to = node;
                }
                Ok(Answer::Nothing)
            }
            Call::BlurToward(next) => {
                let next = self.node(next)?;
                self.document.blur_toward(next);
                Ok(Answer::Nothing)
            }
            Call::HasFocus => Ok(Answer::Bool(self.window.focused)),
            Call::ControlValue(node) => {
                let node = self.node(node)?;
                Ok(Answer::String(forms::value(&self.document, node)))
            }
            Call::ValueMode(node) => {
                let node = self.node(node)?;
                let mode = match self.document.tag(node) {
                    Some("input") => match forms::value_mode(&self.document, node) {
                        forms::ValueMode::Value => "value",
                        forms::ValueMode::Default => "default",
                        forms::ValueMode::DefaultOn => "default/on",
                        forms::ValueMode::Filename => "filename",
                    },
                    Some("textarea") => "value",
                    _ => "",
                };
                Ok(Answer::String(mode.to_owned()))
            }
            Call::SetControlValue { node, value } => {
                let node = self.node(node)?;
                forms::set_value(&mut self.document, node, &value).map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::InputType(node) => {
                let node = self.node(node)?;
                Ok(Answer::String(
                    forms::input_type(&self.document, node).to_owned(),
                ))
            }
            Call::Checkedness(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(forms::checkedness(&self.document, node)))
            }
            Call::SetCheckedness { node, checked } => {
                let node = self.node(node)?;
                forms::set_checkedness(&mut self.document, node, checked, true).map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::Indeterminate(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(forms::indeterminate(&self.document, node)))
            }
            Call::SetIndeterminate {
                node,
                indeterminate,
            } => {
                let node = self.node(node)?;
                forms::set_indeterminate(&mut self.document, node, indeterminate)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::Selectedness(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(forms::selectedness(&self.document, node)))
            }
            Call::SetSelectedness { node, selected } => {
                let node = self.node(node)?;
                forms::set_selectedness(&mut self.document, node, selected, true)
                    .map_err(refused)?;
                if let Some(select) = forms::owner_select(&self.document, node) {
                    html::tree_builder::update_selectedcontent(&mut self.document, select);
                }
                Ok(Answer::Nothing)
            }
            Call::Options(node) => {
                let node = self.node(node)?;
                let options = forms::list_of_options(&self.document, node);
                Ok(Answer::Nodes(options.into_iter().map(handle).collect()))
            }
            Call::OwnerSelect(node) => {
                let node = self.node(node)?;
                let select = forms::owner_select(&self.document, node);
                Ok(self.give(select))
            }
            Call::SelectedIndex(node) => {
                let node = self.node(node)?;
                let index = forms::selected_index(&self.document, node);
                Ok(Answer::Number(index.map_or(-1.0, |i| i as f64)))
            }
            Call::OptionText(node) => {
                let node = self.node(node)?;
                Ok(Answer::String(forms::option_text(&self.document, node)))
            }
            Call::TextSelection(node) => {
                let node = self.node(node)?;
                let value = forms::value(&self.document, node);
                let selection = forms::selection(&self.document, node);
                let direction = match selection.direction {
                    forms::Direction::None => 0.0,
                    forms::Direction::Forward => 1.0,
                    forms::Direction::Backward => 2.0,
                };
                Ok(Answer::Numbers(vec![
                    utf16_units(&value, selection.start) as f64,
                    utf16_units(&value, selection.end) as f64,
                    direction,
                ]))
            }
            Call::SetTextSelection {
                node,
                start,
                end,
                direction,
            } => {
                let node = self.node(node)?;
                let value = forms::value(&self.document, node);
                let selection = forms::Selection {
                    start: byte_at_units(&value, start as usize),
                    end: byte_at_units(&value, end as usize),
                    direction: match direction {
                        1 => forms::Direction::Forward,
                        2 => forms::Direction::Backward,
                        _ => forms::Direction::None,
                    },
                };
                forms::set_selection(&mut self.document, node, selection).map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::FormControls(node) => {
                let form = self.node(node)?;
                let document = &self.document;
                let root = document.ancestors(form).last().unwrap_or(form);
                let controls = document
                    .subtree(root)
                    .filter(|&node| forms::is_listed(document, node))
                    .filter(|&node| forms::form_owner(document, node) == Some(form))
                    .map(handle)
                    .collect();
                Ok(Answer::Nodes(controls))
            }
            Call::LabeledControl(node) => {
                let node = self.node(node)?;
                let control = forms::labeled_control(&self.document, node);
                Ok(self.give(control))
            }
            Call::Labels(node) => {
                let node = self.node(node)?;
                let labels = forms::labels(&self.document, node);
                Ok(Answer::Nodes(labels.into_iter().map(handle).collect()))
            }
            Call::SubmitForm { form, submitter } => {
                let form = self.node(form)?;
                let submitter = submitter.map(|node| self.node(node)).transpose()?;
                if !self.connected(form) {
                    return Ok(Answer::Nothing);
                }
                // A sandboxed document submits none (M33.2).
                let blocked = self
                    .page
                    .as_ref()
                    .and_then(|page| crate::submit::blocked(&self.document, page, form));
                if let Some(line) = blocked {
                    self.say(ConsoleLevel::Error, line);
                    return Ok(Answer::Nothing);
                }
                let seed = self.document.version() ^ (self.work << 20);
                let planned = self.page.as_ref().and_then(|page| {
                    crate::submit::plan(&self.document, page, form, submitter, seed)
                });
                if let Some(submission) = planned {
                    self.outputs.navigation = Some(ScriptNavigation {
                        url: Some(submission.url),
                        replace: false,
                        reload: false,
                        link: true,
                        post: submission.post,
                        target: submission.target,
                        form: true,
                        element: Some(form),
                    });
                }
                Ok(Answer::Nothing)
            }
            Call::ResetForm(form) => {
                let form = self.node(form)?;
                forms::reset_form(&mut self.document, form);
                let root = self.document.root();
                html::tree_builder::update_selectedcontents_under(&mut self.document, root);
                Ok(Answer::Nothing)
            }
            Call::Validity(node) => {
                let node = self.node(node)?;
                let bits = dom::validity::validity(&self.document, node).bits();
                Ok(Answer::Number(f64::from(bits)))
            }
            Call::WillValidate(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(dom::validity::will_validate(
                    &self.document,
                    node,
                )))
            }
            Call::ValidationMessage(node) => {
                let node = self.node(node)?;
                Ok(Answer::String(dom::validity::validation_message(
                    &self.document,
                    node,
                )))
            }
            Call::SetCustomValidity { node, message } => {
                let node = self.node(node)?;
                dom::validity::set_custom_validity(&mut self.document, node, &message)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::InvalidControls(form) => {
                let form = self.node(form)?;
                let invalid = dom::validity::invalid_controls(&self.document, form);
                Ok(Answer::Nodes(invalid.into_iter().map(handle).collect()))
            }
            Call::MarkSubmitted(form) => {
                let form = self.node(form)?;
                dom::validity::mark_submitted(&mut self.document, form);
                Ok(Answer::Nothing)
            }
            Call::SetUserValidity(node) => {
                let node = self.node(node)?;
                dom::validity::set_user_validity(&mut self.document, node, true)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::ReportInvalid(node) => {
                let node = self.node(node)?;
                self.outputs.notice = Some(dom::validity::validation_message(&self.document, node));
                Ok(Answer::Nothing)
            }
            Call::CommitEdit(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(forms::commit_edit(&mut self.document, node)))
            }
            Call::RadioGroup(node) => {
                let node = self.node(node)?;
                let group = forms::radio_group(&self.document, node);
                Ok(Answer::Nodes(group.into_iter().map(handle).collect()))
            }
            Call::CodeAllowed {
                kind,
                source,
                element,
                position,
            } => {
                let element = element.and_then(|element| self.node(element).ok());
                let Some(page) = self.page.as_mut() else {
                    return Ok(Answer::String(String::new()));
                };
                let csp = page.csp(&self.document);
                let violations = match kind.as_str() {
                    "wasm" => csp.wasm_violations(),
                    "handler" => {
                        csp.inline_violations(net::csp::Inline::ScriptAttribute, "", &source)
                    }
                    _ => csp.eval_violations(&source),
                };
                let refused = net::csp::enforced(violations.clone()).err();
                // One the parser set was told of as it was set.
                let told = kind == "handler"
                    && position == (0, 0)
                    && element.is_some_and(|element| {
                        self.parsed_handlers.remove(&(element, source.clone()))
                    });
                if !told {
                    self.say_violations_at(violations, element, position);
                }
                Ok(Answer::String(
                    refused.map_or_else(String::new, |violation| code_refusal(&kind, &violation)),
                ))
            }
            Call::CssomStyled(node) => {
                let node = self.node(node)?;
                if let Some(page) = self.page.as_mut() {
                    page.set_cssom_style(node, true);
                }
                Ok(Answer::Nothing)
            }
            Call::StyleAllowed(node) => {
                let node = self.node(node)?;
                Ok(Answer::Bool(self.style_allowed(node)))
            }
            // Module scripts (D61): a specifier resolved against its
            // script's base URL, or the document's for code with none (a
            // string `eval` ran); an `import()` that waits for a fetch.
            Call::ResolveModule { base, specifier } => {
                let base = net::Url::parse(&base).ok().or_else(|| {
                    self.page
                        .as_ref()
                        .and_then(|page| page.base_url(&self.document))
                });
                let Some(base) = base else {
                    return Err(DomError::new(
                        "TypeError",
                        format!("Failed to resolve module specifier \"{specifier}\"."),
                    ));
                };
                self.modules
                    .resolve(&specifier, &base)
                    .map(|url| Answer::String(url.to_string()))
                    .map_err(|message| DomError::new("TypeError", message))
            }
            // A `blob:` URL's blob is the one it stands for now, though
            // revoked before its fetch.
            Call::ImportModule { id, url, options } => {
                if let Ok(parsed) = net::Url::parse(js::module_of_name(&url).0)
                    && parsed.scheme() == "blob"
                    && let Some(blob) = crate::blob_urls::entry_of(&parsed, &self.blob_origin())
                {
                    self.modules.pin(&parsed, blob);
                }
                self.modules
                    .want(id, url, options)
                    .map(|()| Answer::Nothing)
                    .map_err(|message| DomError::new("TypeError", message))
            }
            Call::Nonce(node) => {
                let node = self.node(node)?;
                let nonce = self
                    .page
                    .as_ref()
                    .map(|page| page.nonce(&self.document, node));
                Ok(Answer::String(nonce.unwrap_or_default()))
            }
            Call::SetNonce(node, nonce) => {
                let node = self.node(node)?;
                if let Some(page) = self.page.as_mut() {
                    page.set_nonce(node, nonce);
                }
                Ok(Answer::Nothing)
            }
            Call::Dialog { kind, .. } if self.sandboxed(Sandbox::MODALS) => {
                self.say(
                    ConsoleLevel::Error,
                    format!(
                        "Ignored call to '{kind}()'. The document is sandboxed, and the \
                         'allow-modals' keyword is not set."
                    ),
                );
                Ok(Answer::MaybeString(None))
            }
            Call::Dialog {
                kind,
                message,
                default,
            } => {
                let asked = crate::prompts::DialogKind::from_name(&kind)
                    .zip(self.window.asker.clone())
                    .zip(self.page.as_ref().map(|page| page.origin().clone()));
                let Some(((kind, asker), origin)) = asked else {
                    // No one to ask: answered at once, as a browser that
                    // shows no dialogs answers.
                    self.say(ConsoleLevel::Info, format!("{kind}: {message}"));
                    return Ok(Answer::MaybeString(None));
                };
                let (message, default) = (
                    crate::prompts::clean(&message),
                    crate::prompts::clean(&default),
                );
                let answer = asker.ask(&origin, kind, &message, &default);
                Ok(Answer::MaybeString(answer))
            }
            Call::Canvas(call) => self.canvas(*call),
        }
    }

    /// Sends what the document's WebGPU calls queued (M41).
    pub(crate) fn flush_gpu(&self) {
        if let Some(gpu) = &self.window.gpu {
            gpu.flush();
        }
    }

    /// A media element's call (M40), JSON in and out: `{"result"}` or
    /// `{"error": {name, message}}`. A pipeline's report, heard in its
    /// task, answers the actions to carry out now.
    fn media_call(&mut self, request: &str) -> String {
        let answer = devtools::Json::parse(request)
            .map_err(|_| ("TypeError", "a malformed media call".to_owned()))
            .and_then(|op| {
                let node = op
                    .get("node")
                    .and_then(devtools::Json::as_f64)
                    .and_then(|n| self.node(Handle(n as u64)).ok());
                let activated = self
                    .window
                    .frames
                    .as_ref()
                    .is_some_and(|env| crate::frames::lock(&env.tree).may_start_sound());
                let env = crate::playback::Env {
                    document: &self.document,
                    page: self.page.as_ref(),
                    inbox: Some(self.workers.inbox()),
                    activated,
                };
                if op.get("op").and_then(devtools::Json::as_str) == Some("report") {
                    let generation = op
                        .get("generation")
                        .and_then(devtools::Json::as_f64)
                        .unwrap_or(0.0) as u64;
                    let report = op.get("report").cloned().unwrap_or(devtools::Json::Null);
                    return Ok(match node {
                        Some(node) => self.media.report(node, generation, &report, &env),
                        None => devtools::Json::Array(Vec::new()),
                    });
                }
                self.media.call(&op, &env, node)
            });
        match answer {
            Ok(result) => devtools::Json::object([("result", result)]).write(),
            Err((name, message)) => devtools::Json::object([(
                "error",
                devtools::Json::object([
                    ("name", devtools::Json::from(name)),
                    ("message", devtools::Json::from(message)),
                ]),
            )])
            .write(),
        }
    }

    /// A canvas's call (M30): a context's names its `canvas` element,
    /// whose size its attributes give and whose computed `color` is
    /// `currentcolor`, read only when a colour says it.
    fn canvas(&mut self, call: js::CanvasCall) -> Result<Answer, DomError> {
        // An `img`'s state, which `complete` and `naturalWidth` read
        // (M30): the image a canvas would draw.
        if call.op == "imageState" {
            let which = call.numbers.get(1).copied().unwrap_or(-1.0);
            if !(0.0..=js::MAX_HANDLE as f64).contains(&which) || which.fract() != 0.0 {
                return Err(DomError::new("TypeError", "Illegal invocation"));
            }
            let node = self.node(Handle(which as u64))?;
            let page = self.page.as_ref().map(Page::id);
            let now = crate::image_store::scanned(&self.document, node);
            let state = self.window.drawables.as_ref().map_or((0, 0, 0), |shared| {
                let drawables = shared.lock().unwrap_or_else(PoisonError::into_inner);
                if drawables.page.is_some() && drawables.page == page {
                    drawables.state(node, &now)
                } else {
                    (0, 0, 0)
                }
            });
            return Ok(Answer::Numbers(vec![
                f64::from(state.0),
                f64::from(state.1),
                f64::from(state.2),
            ]));
        }
        if call.op == "imageBitmap" {
            let answer = self.image_bitmap(&call);
            self.work += self.canvases.take_work();
            return answer;
        }
        let element = if names_context(&call.op) && call.numbers.first() == Some(&0.0) {
            let handle = call.numbers.get(1).copied().unwrap_or(f64::NAN);
            // An `OffscreenCanvas`'s context, by the negative of its
            // number.
            if (-f64::from(u32::MAX)..0.0).contains(&handle) && handle.fract() == 0.0 {
                let source = if needs_source(&call.op) {
                    let kind = call.numbers.get(2).copied().unwrap_or(-1.0);
                    let which = call.numbers.get(3).copied().unwrap_or(-1.0);
                    self.canvas_source(kind, which, &call.op)?
                } else {
                    None
                };
                let element =
                    offscreen_element(handle, self.window.viewport, self.canvas_fonts(), source);
                let answer = self.canvases.call(call, Some(element));
                self.work += self.canvases.take_work();
                return answer;
            }
            if !(0.0..=js::MAX_HANDLE as f64).contains(&handle) || handle.fract() != 0.0 {
                return Err(DomError::new("TypeError", "Illegal invocation"));
            }
            let node = self.node(Handle(handle as u64))?;
            if self.document.tag(node) != Some("canvas") {
                return Err(DomError::new("TypeError", "Illegal invocation"));
            }
            let current = call
                .strings
                .iter()
                .any(|text| text.trim().eq_ignore_ascii_case("currentcolor"));
            let color = if current && self.connected(node) {
                self.styles_now()
                    .and_then(|styles| styles.get(node))
                    .map_or(canvas::Rgba::BLACK, |style| {
                        let channel = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                        let c = style.color;
                        canvas::Rgba::new(channel(c.r), channel(c.g), channel(c.b), channel(c.a))
                    })
            } else {
                canvas::Rgba::BLACK
            };
            let (font_base, rtl, fonts) = if needs_text(&call.op) {
                self.canvas_text(node)
            } else {
                (canvas::FontBase::default(), false, None)
            };
            let lang = if needs_text(&call.op) {
                self.language_of(node)
            } else {
                String::new()
            };
            let source = if needs_source(&call.op) {
                let kind = call.numbers.get(2).copied().unwrap_or(-1.0);
                let which = call.numbers.get(3).copied().unwrap_or(-1.0);
                self.canvas_source(kind, which, &call.op)?
            } else {
                None
            };
            Some(Element {
                owner: Owner::Element(node),
                size: layout::canvas_size(&self.document, node),
                color,
                font_base,
                rtl,
                lang,
                fonts,
                source,
            })
        } else {
            None
        };
        let answer = self.canvases.call(call, element);
        self.work += self.canvases.take_work();
        // A placeholder takes its offscreen's new size (M31.4).
        for (node, (width, height)) in self.canvases.take_resizes() {
            let _ = self
                .document
                .set_attribute(node, "width", &width.to_string());
            let _ = self
                .document
                .set_attribute(node, "height", &height.to_string());
        }
        answer
    }

    /// The pixels of the image a `drawImage()` or a `createPattern()`
    /// (`op`) names (M30): an `img` (kind 1) once decoded, a `canvas`
    /// (kind 2) as its bitmap is, a `video` (kind 5) never; `None` for
    /// one with nothing to draw yet. A broken image and a canvas of no
    /// area are refused, in the reference browser's words.
    fn canvas_source(
        &mut self,
        kind: f64,
        which: f64,
        op: &str,
    ) -> Result<Option<canvas::Source>, DomError> {
        if let Some(found) = offscreen_source(&mut self.canvases, kind, which) {
            return found;
        }
        if !(0.0..=js::MAX_HANDLE as f64).contains(&which) || which.fract() != 0.0 {
            return Err(DomError::new("TypeError", "Illegal invocation"));
        }
        let node = self.node(Handle(which as u64))?;
        let tag = self.document.tag(node);
        match (kind as u8, tag) {
            (1, Some("img")) => {
                let Some(shared) = self.window.drawables.clone() else {
                    return Ok(None);
                };
                let page = self.page.as_ref().map(Page::id);
                let mut drawables = shared.lock().unwrap_or_else(PoisonError::into_inner);
                if drawables.page.is_none() || drawables.page != page {
                    return Ok(None);
                }
                let now = crate::image_store::scanned(&self.document, node);
                match drawables.get(node, &now) {
                    Drawn::Pixels(image, clean) => {
                        let Some(frame) = image.frames.first() else {
                            return Ok(None);
                        };
                        Ok(Some(canvas::Source {
                            pixels: Arc::clone(&frame.pixels),
                            width: image.width,
                            height: image.height,
                            origin_clean: clean,
                        }))
                    }
                    Drawn::Broken => Err(DomError::new(
                        "InvalidStateError",
                        if op == "createPattern" {
                            "Source image is in the 'broken' state."
                        } else {
                            "The HTMLImageElement provided is in the 'broken' state."
                        },
                    )),
                    Drawn::Coming => Ok(None),
                }
            }
            (2, Some("canvas")) => {
                let (width, height) = layout::canvas_size(&self.document, node);
                if width == 0 || height == 0 {
                    let error = canvas::empty_canvas();
                    return Err(DomError::new(error.name, error.message));
                }
                let clean = self.canvases.origin_clean(Owner::Element(node));
                Ok(self
                    .canvases
                    .pixels(Owner::Element(node))
                    .and_then(|(_, image)| {
                        let frame = image.frames.first()?;
                        Some(canvas::Source {
                            pixels: Arc::clone(&frame.pixels),
                            width: image.width,
                            height: image.height,
                            origin_clean: clean,
                        })
                    })
                    .or_else(|| transparent((width, height))))
            }
            // A video's picture now (M40), none before it has one.
            (5, Some("video")) => Ok(self.media.picture(node).and_then(|(image, clean)| {
                let frame = image.frames.first()?;
                Some(canvas::Source {
                    pixels: Arc::clone(&frame.pixels),
                    width: image.width,
                    height: image.height,
                    origin_clean: clean,
                })
            })),
            _ => Err(DomError::new("TypeError", "Illegal invocation")),
        }
    }

    /// `createImageBitmap()` (M30.2): the source (kind 1 an img, 2 a
    /// canvas, 3 an OffscreenCanvas, 4 a bitmap, 6 an ImageData's
    /// pixels, `width` and `height` after, 7 a Blob's bytes to decode),
    /// cut to the crop rectangle, sized to the resize, flipped, drawn
    /// into a new bitmap: its number and size. Refused in the reference
    /// browser's words.
    fn image_bitmap(&mut self, call: &js::CanvasCall) -> Result<Answer, DomError> {
        let n = &call.numbers;
        let at = |i: usize| n.get(i).copied().unwrap_or(f64::NAN);
        let state = |message: &str| DomError::new("InvalidStateError", message);
        let unusable = || state("The image source is not usable.");
        let (kind, which) = (at(0), at(1));
        let image_of = |image: &images::Image, clean: bool| {
            let frame = image.frames.first()?;
            Some(canvas::Source {
                pixels: Arc::clone(&frame.pixels),
                width: image.width,
                height: image.height,
                origin_clean: clean,
            })
        };
        let empty_side = |size: (u32, u32)| -> Result<(), DomError> {
            if size.0 == 0 {
                return Err(state("The image source's width is 0."));
            }
            if size.1 == 0 {
                return Err(state("The image source's height is 0."));
            }
            Ok(())
        };
        let transparent = |size: (u32, u32)| -> Result<canvas::Source, DomError> {
            let pixels = canvas::blank_image_data(u64::from(size.0), u64::from(size.1))
                .map_err(|_| state("The ImageBitmap could not be allocated."))?;
            Ok(canvas::Source {
                pixels: Arc::from(pixels),
                width: size.0,
                height: size.1,
                origin_clean: true,
            })
        };
        let given = match kind as u8 {
            1 => match self.canvas_source(1.0, which, "createImageBitmap") {
                Ok(Some(source)) => Some(source),
                _ => return Err(unusable()),
            },
            2 => {
                if !(0.0..=js::MAX_HANDLE as f64).contains(&which) || which.fract() != 0.0 {
                    return Err(DomError::new("TypeError", "Illegal invocation"));
                }
                let node = self.node(Handle(which as u64))?;
                let size = layout::canvas_size(&self.document, node);
                empty_side(size)?;
                let owner = Owner::Element(node);
                let clean = self.canvases.origin_clean(owner);
                Some(match self.canvases.pixels(owner) {
                    Some((_, image)) => image_of(&image, clean).ok_or_else(unusable)?,
                    None => transparent(size)?,
                })
            }
            // A video's picture now (M40); none before it has one.
            5 => match self.canvas_source(5.0, which, "createImageBitmap") {
                Ok(Some(source)) => Some(source),
                Ok(None) => return Err(state("The provided element has not retrieved data.")),
                Err(error) => return Err(error),
            },
            _ => None,
        };
        image_bitmap(&mut self.canvases, call, given, &mut self.work)
    }

    /// What a canvas's text reads of its element `node` (M30): the sizes
    /// a font's relative units are of, the element's computed
    /// `font-size` and the root's (10 pixels each out of the document,
    /// as the reference browser has them) and the viewport's; whether
    /// its direction is right to left; and the document's fonts.
    /// The language of `node`: its `lang`, or its nearest ancestor's;
    /// empty for none.
    fn language_of(&self, node: NodeId) -> String {
        std::iter::once(node)
            .chain(self.document.ancestors(node))
            .find_map(|n| self.document.attr(n, "lang"))
            .map(|lang| lang.trim().to_owned())
            .unwrap_or_default()
    }

    fn canvas_text(
        &mut self,
        node: NodeId,
    ) -> (canvas::FontBase, bool, Option<text::FontCollection>) {
        let viewport = (
            f64::from(self.window.viewport.0),
            f64::from(self.window.viewport.1),
        );
        let fonts = self.canvas_fonts();
        let mut base = canvas::FontBase {
            viewport,
            ..canvas::FontBase::default()
        };
        let mut rtl = self
            .document
            .attr(node, "dir")
            .is_some_and(|dir| dir.trim().eq_ignore_ascii_case("rtl"));
        if self.connected(node) {
            let root = self.document.document_element();
            let measure = self
                .window
                .layout_fonts
                .clone()
                .or(self.window.fonts.clone());
            // A style's line in pixels, a normal one its font's.
            let line = |style: &style::ComputedStyle| {
                let size = style.font_size_px;
                f64::from(match style.font().line_height {
                    style::computed::LineHeight::Px(px) => px,
                    style::computed::LineHeight::Number(n) => n * size,
                    style::computed::LineHeight::Normal => {
                        measure.as_ref().map_or(size * 1.2, |fonts| {
                            crate::font_units(fonts, style.font(), size).normal_line_height
                        })
                    }
                })
            };
            if let Some(styles) = self.styles_now() {
                if let Some(style) = styles.get(node) {
                    base.em = f64::from(style.font_size_px);
                    base.lh = line(style);
                    rtl = style.direction == style::computed::Direction::Rtl;
                }
                if let Some(style) = root.and_then(|root| styles.get(root)) {
                    base.rem = f64::from(style.font_size_px);
                    base.rlh = line(style);
                }
            }
        }
        (base, rtl, fonts)
    }

    /// Whether the policies let element `node`'s `style` attribute be its
    /// declarations (M33.1): one the CSSOM wrote, or one they allow.
    fn style_allowed(&self, node: NodeId) -> bool {
        let value = self.document.attr(node, "style").unwrap_or_default();
        self.page
            .as_ref()
            .is_none_or(|page| style::InlineStyles::allowed(page, node, value))
    }

    /// The fonts a canvas's text is drawn in: the document's, its web
    /// fonts over the window's (D54).
    fn canvas_fonts(&self) -> Option<text::FontCollection> {
        self.window
            .layout_fonts
            .clone()
            .or_else(|| self.window.fonts.clone())
    }

    /// A `canvas` element's `width` or `height` was set or removed: its
    /// context's bitmap made again at its size.
    fn canvas_attribute(&mut self, node: NodeId, name: &str) {
        if (name.eq_ignore_ascii_case("width") || name.eq_ignore_ascii_case("height"))
            && self.document.tag(node) == Some("canvas")
        {
            let size = layout::canvas_size(&self.document, node);
            self.canvases.resized(Owner::Element(node), size);
        }
    }

    /// Runs the activation behaviour of `element` (HTML section 6.5), a
    /// click at it not canceled: a link is followed.
    fn activate(&mut self, element: NodeId) {
        if matches!(self.document.tag(element), Some("a" | "area")) {
            self.follow_hyperlink(element);
        }
    }

    /// HTML's "follow the hyperlink" (section 4.6.4), as a link's
    /// activation behaviour: its `href` parsed now against the document's
    /// base URL, and the navigation a followed link's, after the run;
    /// nothing for a link without one, or out of the document.
    fn follow_hyperlink(&mut self, element: NodeId) {
        if !self.connected(element) {
            return;
        }
        let Some(href) = self.document.attr(element, "href") else {
            return;
        };
        let Some(url) = self
            .page
            .as_ref()
            .and_then(|page| page.resolve(&self.document, href))
        else {
            return;
        };
        self.outputs.navigation = Some(ScriptNavigation {
            url: Some(url),
            replace: false,
            reload: false,
            link: true,
            post: None,
            target: crate::frames::element_target(&self.document, element),
            form: false,
            element: Some(element),
        });
    }

    /// The window's clock now.
    fn now(&self) -> Duration {
        self.window
            .clock
            .as_ref()
            .map(|clock| clock())
            .unwrap_or_default()
    }

    /// The serialization of the document's origin, `null` without a
    /// page.
    /// `window.open(url, target)` (M37): the navigable `target` names,
    /// navigated as a link to `url` is; or else a new tab, opened when the
    /// user activated the tab within [`crate::frames::ACTIVATION_SPAN`]
    /// and the document's sandbox allows popups, its activation
    /// consumed. No window is given back: every tab opens as with
    /// `noopener`.
    fn open_window(&mut self, url: &str, target: &str) -> Result<Answer, DomError> {
        let parsed = match url {
            "" => net::Url::parse("about:blank").ok(),
            _ => self
                .page
                .as_ref()
                .and_then(|page| page.resolve(&self.document, url)),
        };
        let Some(url) = parsed else {
            return Err(DomError::new(
                "SyntaxError",
                format!(
                    "Failed to execute 'open' on 'Window': Unable to open a window with invalid URL '{url}'."
                ),
            ));
        };
        let Some(env) = self.window.frames.clone() else {
            return Ok(Answer::Nothing);
        };
        let lower = target.to_ascii_lowercase();
        let named = match lower.as_str() {
            "_self" | "_parent" | "_top" => true,
            "_blank" => false,
            _ => crate::frames::lock(&env.tree).find_named(target).is_some(),
        };
        if named {
            self.outputs.navigation = Some(ScriptNavigation {
                url: Some(url),
                replace: false,
                reload: false,
                link: true,
                post: None,
                target: target.to_owned(),
                form: false,
                element: None,
            });
            return Ok(Answer::Nothing);
        }
        let sandboxed = self
            .page
            .as_ref()
            .is_some_and(|page| page.sandbox().has(Sandbox::AUXILIARY_NAVIGATION));
        if sandboxed {
            self.say(
                ConsoleLevel::Error,
                format!(
                    "Blocked opening '{url}' in a new window because the request was made in a \
                     sandboxed frame whose 'allow-popups' permission is not set."
                ),
            );
            return Ok(Answer::Nothing);
        }
        if !crate::frames::opens_in_tab(&url) {
            return Ok(Answer::Nothing);
        }
        let now = self
            .window
            .clock
            .as_ref()
            .map(|clock| clock())
            .unwrap_or_default();
        let mut tree = crate::frames::lock(&env.tree);
        if !tree.consume_activation(now) {
            drop(tree);
            self.outputs.notice = Some(crate::POPUP_BLOCKED.to_owned());
            return Ok(Answer::Nothing);
        }
        tree.open(crate::frames::OpenTab {
            url,
            show: true,
            by_script: true,
        });
        Ok(Answer::Nothing)
    }

    /// `window.close()` (M37): the top-level document asks to close its
    /// tab, which the browser does if a script opened it.
    fn close_window(&mut self) -> Result<Answer, DomError> {
        if let Some(env) = &self.window.frames {
            let mut tree = crate::frames::lock(&env.tree);
            if tree.parent(env.id).is_none() {
                tree.ask_close();
            }
        }
        Ok(Answer::Nothing)
    }

    /// Answers a call about the tab's frames (M33.2): from the tree, a
    /// message or a navigation put in another frame's inbox. An engine
    /// that has no frames answers as a top-level window with none.
    fn frame_call(&mut self, call: Call) -> Result<Answer, DomError> {
        let number =
            |id: Option<crate::frames::FrameId>| Answer::Number(id.map_or(-1.0, f64::from));
        let Some(env) = self.window.frames.clone() else {
            return Ok(match call {
                Call::FrameSelf | Call::FrameTop(_) => Answer::Number(0.0),
                Call::FrameLength(_) => Answer::Number(0.0),
                Call::FrameClosed(_) => Answer::Bool(false),
                Call::WindowName => Answer::String(String::new()),
                Call::FramePost { .. } | Call::FrameNavigate { .. } | Call::SetWindowName(_) => {
                    Answer::Nothing
                }
                _ => Answer::Number(-1.0),
            });
        };
        let me = env.id;
        match call {
            Call::FrameSelf => Ok(Answer::Number(f64::from(me))),
            Call::FrameParent(id) => Ok(number(crate::frames::lock(&env.tree).parent(id))),
            Call::FrameTop(id) => Ok(number(Some(crate::frames::lock(&env.tree).top(id)))),
            Call::FrameLength(id) => Ok(Answer::Number(
                crate::frames::lock(&env.tree).children(id).len() as f64,
            )),
            Call::FrameChild(id, index) => {
                let tree = crate::frames::lock(&env.tree);
                Ok(number(tree.children(id).get(index as usize).copied()))
            }
            Call::FrameNamed(id, name) => Ok(number(
                crate::frames::lock(&env.tree).child_named(id, &name),
            )),
            Call::FrameClosed(id) => Ok(Answer::Bool(!crate::frames::lock(&env.tree).contains(id))),
            Call::FrameOf(element) => {
                // An iframe in the document has its frame now, made later
                // by the engine under this number (M33.2).
                let element = self.node(element)?;
                let iframe =
                    self.document.tag(element) == Some("iframe") && self.connected(element);
                let mut tree = crate::frames::lock(&env.tree);
                let id = match tree.of_element(me, element) {
                    Some(id) => Some(id),
                    None if iframe => {
                        let name = self.document.attr(element, "name").unwrap_or_default();
                        tree.reserve(me, element, name)
                    }
                    None => None,
                };
                Ok(number(id))
            }
            Call::FramePost {
                frame,
                target_origin,
                data,
            } => {
                if data.len() > super::workers::MAX_MESSAGE_BYTES {
                    return Err(DomError::new(
                        "DataCloneError",
                        "Data cannot be cloned, out of memory.",
                    ));
                }
                let task = crate::frames::FrameTask::Message {
                    data,
                    origin: self.origin_text(),
                    source: me,
                    target_origin,
                };
                if crate::frames::lock(&env.tree).post(frame, task)
                    && let Some(wake) = &env.wake
                {
                    wake();
                }
                Ok(Answer::Nothing)
            }
            Call::FrameNavigate {
                frame,
                url,
                replace,
            } => {
                let parsed = self
                    .page
                    .as_ref()
                    .and_then(|page| page.resolve(&self.document, &url))
                    .ok_or_else(|| {
                        DomError::new("SyntaxError", format!("'{url}' is not a valid URL."))
                    })?;
                let origin = self
                    .page
                    .as_ref()
                    .map_or_else(net::Origin::new_opaque, |page| page.origin().clone());
                let mut tree = crate::frames::lock(&env.tree);
                let sandbox = self.page.as_ref().map_or(Sandbox::NONE, Page::sandbox);
                if !tree.may_navigate(me, frame, (&origin, sandbox), false) {
                    return Err(DomError::new(
                        "SecurityError",
                        format!(
                            "The current window does not have permission to navigate the \
                             target frame to '{parsed}'."
                        ),
                    ));
                }
                // Another frame's document runs no `javascript:` URL of
                // this one's: every frame is another origin's (D24).
                if parsed.scheme() == "javascript" {
                    return Ok(Answer::Nothing);
                }
                let task =
                    crate::frames::FrameTask::Navigate(Box::new(crate::frames::FrameNavigation {
                        url: parsed,
                        replace,
                        post: None,
                        origin,
                    }));
                if tree.post(frame, task)
                    && let Some(wake) = &env.wake
                {
                    wake();
                }
                Ok(Answer::Nothing)
            }
            Call::WindowName => Ok(Answer::String(
                crate::frames::lock(&env.tree).window_name(me),
            )),
            Call::SetWindowName(name) => {
                crate::frames::lock(&env.tree).set_window_name(me, name);
                Ok(Answer::Nothing)
            }
            _ => Err(DomError::not_supported()),
        }
    }

    /// The document's changes since they were last taken, as records for
    /// its mutation observers: one operation's, grouped, when
    /// `operation`.
    fn observe_changes(&mut self, operation: bool) {
        if self.mutations.observing() {
            let changes = self.document.take_observed();
            if !changes.is_empty() {
                self.mutations.record(changes, operation);
            }
        }
    }

    /// Tells the document which nodes are observed, and whether to keep
    /// its changes at all.
    fn watch(&mut self) {
        self.document.observe(self.mutations.observing());
        self.document.set_watched(self.mutations.watched());
    }

    /// Whether changes wait for mutation observers that no script's call
    /// took (the parser's).
    pub(crate) fn mutations_waiting(&self, document: &dom::Document) -> bool {
        self.mutations.observing() && document.has_observed()
    }

    /// The origin the document's `blob:` URLs are of.
    fn blob_origin(&self) -> net::Origin {
        self.page
            .as_ref()
            .map_or_else(net::Origin::new_opaque, |page| page.origin().clone())
    }

    fn origin_text(&self) -> String {
        self.page
            .as_ref()
            .map_or_else(|| "null".to_owned(), |page| page.origin().serialization())
    }

    /// Lends the host what a run needs, `how` the script runs, and
    /// whether a parsing-blocking script is pending.
    pub(crate) fn lend(&mut self, lent: &mut Lent<'_>, how: How, blocked: bool) {
        self.document = std::mem::take(lent.document);
        let placeholder = Page::given(lent.page.id());
        self.page = Some(std::mem::replace(lent.page, placeholder));
        self.cookies = Some(lent.network.cookie_access());
        self.client = Some(lent.network.client());
        self.workers.set_wake(lent.network.waker());
        self.parser = lent
            .parser
            .as_deref_mut()
            .map(|load| std::mem::replace(load, html::Load::new(None, false)));
        self.ready = lent.ready;
        self.blocked = blocked;
        self.too_deep = false;
        self.window = std::mem::take(&mut lent.window);
        self.computed = None;
        self.svg_questions = None;
        self.laid_out = None;
        self.inline_scanned = None;
        if let Some(images) = lent.images.as_deref_mut() {
            self.images = std::mem::take(images);
        }
        self.nesting = how.nesting;
        self.frames = vec![Frame {
            element: how.element,
            external: how.external,
            written: false,
        }];
    }

    /// Gives back what was lent, and what the run leaves the engine to do.
    pub(crate) fn give_back(&mut self, lent: &mut Lent<'_>) -> Outputs {
        *lent.document = std::mem::take(&mut self.document);
        if let Some(page) = self.page.take() {
            *lent.page = page;
        }
        if let (Some(load), Some(lent)) = (self.parser.take(), lent.parser.as_deref_mut()) {
            *lent = load;
        }
        if let Some(images) = lent.images.as_deref_mut() {
            *images = std::mem::take(&mut self.images);
        }
        self.frames.clear();
        self.now.clear();
        self.computed = None;
        self.svg_questions = None;
        self.laid_out = None;
        std::mem::take(&mut self.outputs)
    }

    /// Whether a script may run nested in those running: not past
    /// [`MAX_SCRIPT_NESTING`], where the console says, once a run, that
    /// it was not run.
    fn may_nest(&mut self) -> bool {
        if self.frames.len() < MAX_SCRIPT_NESTING {
            return true;
        }
        if !self.too_deep {
            self.too_deep = true;
            self.say(
                ConsoleLevel::Error,
                format!("A script nested more than {MAX_SCRIPT_NESTING} deep was not run"),
            );
        }
        false
    }

    /// Writes `message` to the console.
    fn say(&mut self, level: ConsoleLevel, message: String) {
        self.outputs.console.push(ConsoleEntry::of(
            super::ConsoleSource::Engine,
            level,
            cut(message),
        ));
    }

    /// `document.write` (HTML section 8.4.3): the text put in before the
    /// insertion point, to be parsed when the call has made it in. With
    /// none, a script from a file writes nothing, and anything else would
    /// open the document again, which this version does not do (G48).
    /// Gives the nodes whose listeners the bindings erase when the write
    /// opened the document.
    fn write(&mut self, text: &str) -> Vec<NodeId> {
        self.work += text.len() as u64 / 64;
        let defined = self
            .parser
            .as_mut()
            .and_then(html::Load::parser)
            .is_some_and(|parser| parser.has_insertion_point());
        if defined {
            if let Some(parser) = self.parser.as_mut().and_then(html::Load::parser) {
                parser.write(text);
            }
            return Vec::new();
        }
        // A script `document.open()`'s parser runs writes at its
        // insertion point, just after the script's end tag.
        if self.writing {
            self.nested_writes.push_str(text);
            return Vec::new();
        }
        // No insertion point: a script from a file writes nothing (the
        // ignore-destructive-writes counter); another opens the document
        // first, which replaces it.
        if self.frames.iter().any(|frame| frame.external) {
            return Vec::new();
        }
        let mut erased = Vec::new();
        if self.opened.is_none() {
            erased = self.open_document();
        }
        if let Some(mut parser) = self.opened.take() {
            parser.feed(text);
            self.parse_opened(&mut parser);
            self.opened = Some(parser);
        }
        erased
    }

    /// An `InvalidStateError` for `method` (`write`, `open`, `close`)
    /// in an XML document, which has no HTML parser to write to (HTML
    /// section 8.4.2).
    fn only_html(&self, method: &str) -> Result<(), DomError> {
        if self.document.is_xml_document() {
            return Err(DomError::new(
                "InvalidStateError",
                format!("Only HTML documents support {method}()."),
            ));
        }
        Ok(())
    }

    /// `document.open()`, HTML section 8.4.2.1's "document open steps":
    /// nothing while the parser runs a script (a write goes to its
    /// insertion point), nor while `document.open()`'s parser runs one,
    /// nor once the load's parser was aborted (`window.stop()`). Else a
    /// navigation under way stops; the load's parser, if it still runs,
    /// is aborted with its fetch, so that it builds no more; the
    /// document's children are removed; and a new parser is made for
    /// what is written, the document loading again. The URL becomes the
    /// entry document's, which with one realm a document is this one: it
    /// stays. Gives the nodes whose event listeners and handlers the
    /// bindings erase, the document and the descendants scripts hold
    /// (the window's with them); none when it opened nothing.
    fn open_document(&mut self) -> Vec<NodeId> {
        let nested = self
            .parser
            .as_mut()
            .and_then(html::Load::parser)
            .is_some_and(|parser| parser.has_insertion_point());
        if nested || self.writing || self.parser_aborted {
            return Vec::new();
        }
        self.outputs.stop_navigation = true;
        self.outputs.navigation = None;
        if let Some(load) = self.parser.as_mut()
            && !load.is_done()
        {
            load.abort();
            self.outputs.parser_aborted = true;
        }
        let root = self.document.root();
        let mut erased = vec![root];
        erased.extend(
            self.document
                .subtree(root)
                .skip(1)
                .filter(|node| self.exposed.contains(node)),
        );
        self.work += erased.len() as u64;
        while let Some(child) = self.document.first_child(root) {
            self.work += 1;
            if self.document.remove_child(root, child).is_err() {
                break;
            }
            self.taken_out(child);
        }
        self.opened = Some(html::Parser::new(true));
        self.before_open = self.ready;
        self.ready = ReadyState::Loading;
        erased
    }

    /// Whether `document.open()` left the document open: its parser has
    /// not been closed and ended yet, and the document is loading.
    pub(crate) fn is_open(&self) -> bool {
        self.opened.is_some()
    }

    /// The document's parser was aborted (`window.stop()`): a later
    /// `document.open()` does nothing.
    pub(crate) fn parser_was_aborted(&mut self) {
        self.parser_aborted = true;
    }

    /// Parses what `parser`, `document.open()`'s, has been given: its
    /// scripts are the parser's, run as scripts' scripts are.
    fn parse_opened(&mut self, parser: &mut html::Parser) {
        let before = self.document.len();
        self.writing = true;
        loop {
            match parser.parse(&mut self.document, usize::MAX) {
                html::Stop::Script { element, .. } => {
                    self.made_by_script(element, false);
                    let written = std::mem::take(&mut self.nested_writes);
                    if !written.is_empty() {
                        parser.push_insertion_point();
                        parser.write(&written);
                        parser.pop_insertion_point();
                    }
                }
                html::Stop::Slice => {}
                html::Stop::Input | html::Stop::Done => break,
            }
        }
        self.writing = false;
        self.work += self.document.len().saturating_sub(before) as u64;
    }

    /// The CSSOM's sheets and rules (D66), `op` on `owner`'s sheet:
    /// `sheets`, the document's list as owners and serial numbers;
    /// `sheet`, an element's sheet's serial number; `info`, a sheet's URL,
    /// title and media; `rules`, a rule list's rules as
    /// [`super::rules::rules`] gives them; `insert`, `delete`, and `set`
    /// a rule's declarations (`index` 0) or selector (1).
    fn cssom(
        &mut self,
        op: &str,
        owner: Option<NodeId>,
        path: &[f64],
        text: &str,
        index: f64,
    ) -> Result<Answer, DomError> {
        let gone = || DomError::new("NotFoundError", "The style sheet is gone.");
        // A media query list's queries, each serialized (a `MediaList`'s).
        if op == "media" {
            self.work += text.len() as u64 / 64;
            return Ok(Answer::Strings(super::rules::media_items(&css::tokenize(
                text,
            ))));
        }
        if owner.is_none() && op != "sheets" {
            return self.constructed_cssom(op, path, text, index);
        }
        self.inline_sheets();
        let Some(page) = self.page.as_mut() else {
            return Err(gone());
        };
        let indices: Vec<usize> = path
            .iter()
            .take(1 << 10)
            .map(|&n| {
                if n.is_finite() && n >= 0.0 {
                    n as usize
                } else {
                    usize::MAX
                }
            })
            .collect();
        let count = indices
            .first()
            .copied()
            .unwrap_or(0)
            .min(indices.len().saturating_sub(1));
        let imports = indices.get(1..=count).unwrap_or_default();
        let rule_path = indices.get(count + 1..).unwrap_or_default();
        let index = if index.is_finite() && index >= 0.0 {
            index.min(u32::MAX as f64) as usize
        } else {
            usize::MAX
        };
        if op == "sheets" {
            let sheets = page.cssom_sheets();
            self.work += sheets.len() as u64;
            return Ok(Answer::Numbers(
                sheets
                    .into_iter()
                    .flat_map(|(node, serial)| [handle(node).0 as f64, serial as f64])
                    .collect(),
            ));
        }
        let owner = owner.ok_or_else(gone)?;
        match op {
            // An element out of the document has no sheet, though the
            // sheet it had is kept for the objects scripts hold.
            "sheet" => Ok(Answer::Numbers(
                page.cssom_sheet(owner, imports)
                    .filter(|_| {
                        !imports.is_empty()
                            || page
                                .cssom_owner(owner)
                                .is_some_and(|(_, _, attached)| attached)
                    })
                    .map(|view| vec![view.serial as f64])
                    .unwrap_or_default(),
            )),
            "info" => {
                let view = page.cssom_sheet(owner, imports).ok_or_else(gone)?;
                let href = view
                    .location
                    .map_or_else(String::new, |url| url.to_string());
                let (title, media, attached) = page.cssom_owner(owner).unwrap_or(("", None, false));
                let (title, media) = if imports.is_empty() {
                    (title.to_owned(), media.unwrap_or_default().to_owned())
                } else {
                    (String::new(), String::new())
                };
                let attached = if attached { "1" } else { "" }.to_owned();
                Ok(Answer::Strings(vec![href, title, media, attached]))
            }
            "rules" => {
                let view = page.cssom_sheet(owner, imports).ok_or_else(gone)?;
                let flat = super::rules::rules(view.rules, rule_path);
                self.work += flat.len() as u64;
                Ok(Answer::Strings(flat))
            }
            "insert" | "delete" | "set" => {
                let mut failure = None;
                let done = page.cssom_edit(owner, imports, |sheet| {
                    let same: Vec<Option<usize>> = (0..sheet.rules.len()).map(Some).collect();
                    let result = match op {
                        "insert" => super::rules::insert(sheet, rule_path, text, index),
                        "delete" => {
                            super::rules::delete(sheet, rule_path, index).map(|kept| (0, kept))
                        }
                        _ if index == 2 => Ok((
                            usize::from(super::rules::set_media(sheet, rule_path, text)),
                            same,
                        )),
                        _ => Ok((
                            usize::from(super::rules::set(sheet, rule_path, text, index == 1)),
                            same,
                        )),
                    };
                    match result {
                        Ok(done) => Some(done),
                        Err(error) => {
                            failure = Some(error);
                            None
                        }
                    }
                });
                if let Some(error) = failure {
                    return Err(error);
                }
                let done = done.ok_or_else(gone)?;
                // The cascade and layout of the run are of the old rules.
                self.computed = None;
                self.laid_out = None;
                Ok(Answer::Number(done as f64))
            }
            _ => Err(gone()),
        }
    }

    /// The CSSOM's ops on constructed sheets (`new CSSStyleSheet()`, CSS
    /// module scripts): `construct`, a new sheet whose URLs resolve
    /// against `text` (the document's base URL when empty), its serial
    /// number; `adopt`, `document.adoptedStyleSheets` set to the sheets
    /// `path` numbers; and on the sheet `path[0]` (its rule path after an
    /// empty import count), `replace` its rules by `text`, and `info`,
    /// `sheet`, `rules`, `insert`, `delete` and `set` as an element's
    /// sheet has them.
    fn constructed_cssom(
        &mut self,
        op: &str,
        path: &[f64],
        text: &str,
        index: f64,
    ) -> Result<Answer, DomError> {
        let gone = || DomError::new("NotFoundError", "The style sheet is gone.");
        let base = (op == "construct").then(|| {
            net::Url::parse(text).ok().or_else(|| {
                self.page
                    .as_ref()
                    .and_then(|page| page.base_url(&self.document))
            })
        });
        let Some(page) = self.page.as_mut() else {
            return Err(gone());
        };
        let number = |n: f64| {
            if n.is_finite() && n >= 0.0 {
                n as u64
            } else {
                u64::MAX
            }
        };
        match op {
            "construct" => {
                let serial = page.construct_sheet(base.flatten()).ok_or_else(|| {
                    DomError::new(
                        "NotSupportedError",
                        "Too many style sheets were constructed.",
                    )
                })?;
                return Ok(Answer::Numbers(vec![serial as f64]));
            }
            "adopt" => {
                self.work += path.len() as u64;
                page.adopt_sheets(path.iter().take(1 << 12).map(|&n| number(n)).collect());
                self.computed = None;
                self.laid_out = None;
                return Ok(Answer::Nothing);
            }
            _ => {}
        }
        let serial = number(path.first().copied().unwrap_or(-1.0));
        let rule_path: Vec<usize> = path
            .iter()
            .skip(2)
            .take(1 << 10)
            .map(|&n| number(n).min(usize::MAX as u64) as usize)
            .collect();
        let index = if index.is_finite() && index >= 0.0 {
            index.min(u32::MAX as f64) as usize
        } else {
            usize::MAX
        };
        match op {
            "replace" => {
                self.work += text.len() as u64 / 64;
                if !page.replace_sheet(serial, text) {
                    return Err(gone());
                }
                self.computed = None;
                self.laid_out = None;
                Ok(Answer::Nothing)
            }
            "sheet" => Ok(Answer::Numbers(
                page.constructed_sheet(serial)
                    .map(|view| vec![view.serial as f64])
                    .unwrap_or_default(),
            )),
            "info" => {
                page.constructed_sheet(serial).ok_or_else(gone)?;
                Ok(Answer::Strings(vec![
                    String::new(),
                    String::new(),
                    String::new(),
                ]))
            }
            "rules" => {
                let view = page.constructed_sheet(serial).ok_or_else(gone)?;
                let flat = super::rules::rules(view.rules, &rule_path);
                self.work += flat.len() as u64;
                Ok(Answer::Strings(flat))
            }
            "insert" | "delete" | "set" => {
                let mut failure = None;
                let done = page.constructed_edit(serial, |sheet| {
                    let same: Vec<Option<usize>> = (0..sheet.rules.len()).map(Some).collect();
                    let result = match op {
                        "insert" => super::rules::insert(sheet, &rule_path, text, index),
                        "delete" => {
                            super::rules::delete(sheet, &rule_path, index).map(|kept| (0, kept))
                        }
                        _ if index == 2 => Ok((
                            usize::from(super::rules::set_media(sheet, &rule_path, text)),
                            same,
                        )),
                        _ => Ok((
                            usize::from(super::rules::set(sheet, &rule_path, text, index == 1)),
                            same,
                        )),
                    };
                    match result {
                        Ok(done) => Some(done),
                        Err(error) => {
                            failure = Some(error);
                            None
                        }
                    }
                });
                if let Some(error) = failure {
                    return Err(error);
                }
                let done = done.ok_or_else(gone)?;
                self.computed = None;
                self.laid_out = None;
                Ok(Answer::Number(done as f64))
            }
            _ => Err(gone()),
        }
    }

    /// The document laid out as it now stands, for what scripts ask of
    /// it (D65): its styles brought up to date, and at most one layout a
    /// version of the document; none for a document without a window.
    fn layout_now(&mut self) -> Option<&layout::DocumentLayout> {
        let version = self.document.version();
        if self.laid_out.as_ref().is_some_and(|(at, _)| *at == version) {
            return self.laid_out.as_ref().map(|(_, layout)| layout);
        }
        let (width, height) = self.window.viewport;
        if width == 0 {
            return None;
        }
        self.styles_now()?;
        let page = self.page.as_ref()?;
        let computed = self.computed.as_ref()?;
        let fonts = self
            .window
            .layout_fonts
            .as_ref()
            .or(self.window.fonts.as_ref())?;
        let measure = Measure(fonts);
        let animations = RefCell::new(Animations::new());
        let context = CascadeContext {
            viewport_width: width as f32,
            viewport_height: height as f32,
            fonts: Some(&measure as &dyn FontMeasure),
            language: page.language(),
            media: self.window.media,
            animations: Some(&animations),
            inline_styles: Some(page),
            frame_margins: self.window.frame_margins,
        };
        let recascade = computed
            .sheets
            .recascader(&self.document, &computed.styles, &context);
        let boxes = layout::build_box_tree_showing(
            &self.document,
            &computed.styles,
            &recascade,
            &self.images,
        );
        let laid_out = layout::layout_document_in(&boxes, width as f32, Some(height as f32), fonts);
        self.work += self.document.len() as u64;
        self.laid_out = Some((version, laid_out));
        self.laid_out.as_ref().map(|(_, layout)| layout)
    }

    /// The viewport's scroll offset and each scroll container's, down
    /// and across, as the frame drew them and scripts since set them at
    /// once (a smooth scroll moves them over the frames to come).
    fn scroll_state(&self) -> (f32, paint::Offsets) {
        let (mut y, mut offsets) = (*self.window.scroll).clone();
        for scroll in self.outputs.scrolls.iter().filter(|s| !s.smooth) {
            match scroll.node {
                None => {
                    y = scroll.y.unwrap_or(y);
                    offsets.viewport_x = scroll.x.unwrap_or(offsets.viewport_x);
                }
                Some(node) => {
                    if let Some(to) = scroll.y {
                        offsets.y.insert(node, to);
                    }
                    if let Some(to) = scroll.x {
                        offsets.x.insert(node, to);
                    }
                }
            }
        }
        (y, offsets)
    }

    /// `node` measured in the layout (CSSOM View): where its border box
    /// is, in viewport coordinates when `scrolled`, else in the
    /// document's, unscrolled; then its border widths, its padding, its
    /// margins (each top, right, bottom, left), its scroll height, and
    /// each of its boxes' rectangles. Empty for an element with no box.
    fn geometry(&mut self, node: NodeId, scrolled: bool) -> Vec<f64> {
        let (scroll_y, offsets) = if scrolled {
            self.scroll_state()
        } else {
            (0.0, paint::Offsets::default())
        };
        let (width, height) = self.window.viewport;
        let Some(layout) = self.layout_now() else {
            return Vec::new();
        };
        let viewport = paint::Viewport {
            width: width as f32,
            height: height as f32,
            scroll_y,
        };
        let targets = std::iter::once(node).collect();
        let measured = crate::geometry::measure(layout, viewport, &offsets, &targets);
        let scroll_height = layout
            .iter()
            .filter(|f| f.node == node && f.generated.is_none())
            .map(|f| f.scroll_height)
            .fold(0.0f32, f32::max);
        let Some(m) = measured.get(&node) else {
            return Vec::new();
        };
        let mut out: Vec<f64> = m
            .bounds
            .iter()
            .chain(&m.border)
            .chain(&m.padding)
            .chain(&m.margin)
            .map(|&v| f64::from(v))
            .collect();
        out.push(f64::from(scroll_height));
        out.extend(m.rects.iter().flatten().map(|&v| f64::from(v)));
        self.work += layout.iter().count() as u64 / 16;
        out
    }

    /// The resolved values of `node`'s `top`, `right`, `bottom` and
    /// `left` where they are used values (CSSOM section 9, as the
    /// reference browser has them, D65); `None` for one that is its
    /// computed value. An absolutely positioned box's are its margin
    /// box's distances from its containing block's padding box, a side
    /// given keeping its value, a percentage of that box; a relatively
    /// positioned box's are a percentage of its containing block's
    /// content box, and `auto` the other side's opposite; a sticky box's
    /// a percentage of its scroll container's content box, `auto` kept.
    fn used_insets(&mut self, node: NodeId) -> [Option<f32>; 4] {
        use style::computed::Position;
        let none = [None; 4];
        let Some(style) = self
            .styles_now()
            .and_then(|styles| styles.get(node))
            .cloned()
        else {
            return none;
        };
        if style.position == Position::Static {
            return none;
        }
        let (width, height) = self.window.viewport;
        let viewport = [width as f32, height as f32];
        let Some(layout) = self.layout_now() else {
            return none;
        };
        let Some(((x, y, own), parent)) = own_box(layout, node) else {
            return none;
        };
        let (w, h) = (own.w, own.h);
        // The box it hangs in: its padding box's origin and size, and
        // its node.
        let parent = parent.map(|(px, py, p)| {
            let [top, right, bottom, left] = p.style.used_border_width();
            (
                [px + left, py + top],
                [(p.w - left - right).max(0.0), (p.h - top - bottom).max(0.0)],
                p.node,
            )
        });
        let inset = style.inset.clone();
        // The containing block's size along each side's axis, by side.
        let basis = |size: [f32; 2]| [size[1], size[0], size[1], size[0]];
        match style.position {
            Position::Absolute | Position::Fixed => {
                let (origin, size) = parent.map_or(([0.0, 0.0], viewport), |(o, s, _)| (o, s));
                let cb = basis(size);
                let margin = style.margin.clone().map(|m| m.resolve(size[0], 0.0));
                let from_start = [y - margin[0] - origin[1], x - margin[3] - origin[0]];
                let to_end = [
                    origin[1] + size[1] - (y + h + margin[2]),
                    origin[0] + size[0] - (x + w + margin[1]),
                ];
                let laid = [from_start[0], to_end[1], to_end[0], from_start[1]];
                std::array::from_fn(|i| Some(inset[i].resolve(cb[i], laid[i])))
            }
            Position::Relative => {
                let size = match parent {
                    Some((_, _, p)) if p != node => self.content_size(p),
                    _ => None,
                }
                .unwrap_or(viewport);
                let cb = basis(size);
                let given: [Option<f32>; 4] = std::array::from_fn(|i| {
                    (!inset[i].is_auto()).then(|| inset[i].resolve(cb[i], 0.0))
                });
                std::array::from_fn(|i| {
                    Some(given[i].unwrap_or_else(|| given[(i + 2) % 4].map_or(0.0, |v| -v)))
                })
            }
            Position::Sticky => {
                let scroller = self.document.ancestors(node).find(|&a| {
                    self.computed
                        .as_ref()
                        .and_then(|c| c.styles.get(a))
                        .is_some_and(|s| s.overflow.clips())
                });
                let size = scroller
                    .and_then(|a| self.content_size(a))
                    .unwrap_or(viewport);
                let cb = basis(size);
                std::array::from_fn(|i| (!inset[i].is_auto()).then(|| inset[i].resolve(cb[i], 0.0)))
            }
            Position::Static => none,
        }
    }

    /// The width and height of `node`'s content box, from the layout;
    /// `None` for an element with no box.
    fn content_size(&mut self, node: NodeId) -> Option<[f32; 2]> {
        let g = self.geometry(node, false);
        if g.len() < 12 {
            return None;
        }
        Some([
            (g[2] - g[5] - g[7] - g[9] - g[11]).max(0.0) as f32,
            (g[3] - g[4] - g[6] - g[8] - g[10]).max(0.0) as f32,
        ])
    }

    /// CSSOM View's `offsetParent`: none for the root, the body, a fixed
    /// box or one not rendered; else the nearest ancestor that is
    /// positioned, a cell or a table, or the body.
    fn offset_parent(&mut self, node: NodeId) -> Option<NodeId> {
        if !self.connected(node) || !self.document.is_element(node) {
            return None;
        }
        let ancestors: Vec<NodeId> = self.document.ancestors(node).collect();
        self.styles_now()?;
        let styles = &self.computed.as_ref()?.styles;
        let shown = |n: NodeId| {
            styles
                .get(n)
                .is_some_and(|s| s.display != style::computed::Display::None)
        };
        let position = |n: NodeId| styles.get(n).map(|s| s.position);
        let document = &self.document;
        if matches!(document.tag(node), Some("body" | "html"))
            || document.parent(node) == Some(document.root())
            || !shown(node)
            || ancestors
                .iter()
                .any(|&a| document.is_element(a) && !shown(a))
            || position(node) == Some(style::computed::Position::Fixed)
        {
            return None;
        }
        let static_position = position(node) == Some(style::computed::Position::Static);
        ancestors.into_iter().find(|&a| {
            document.is_element(a)
                && (position(a).is_some_and(|p| p != style::computed::Position::Static)
                    || document.tag(a) == Some("body")
                    || (static_position && matches!(document.tag(a), Some("td" | "th" | "table"))))
        })
    }

    /// A scroll offset and how far it may go, down then across: the
    /// viewport's for `None`, else the scroll container `node`'s (none
    /// for another element); then how far the content reaches past the
    /// box, down and across, whether it scrolls or not.
    fn scroll_offset(&mut self, node: Option<NodeId>) -> Vec<f64> {
        let (scroll_y, offsets) = self.scroll_state();
        let (width, height) = self.window.viewport;
        let Some(layout) = self.layout_now() else {
            return vec![0.0; 6];
        };
        let (at, max, at_x, max_x) = match node {
            None => {
                let bottom = crate::geometry::document_height(layout);
                let right = crate::geometry::document_width(layout);
                (
                    scroll_y,
                    (bottom - height as f32).max(0.0),
                    offsets.viewport_x,
                    (right - width as f32).max(0.0),
                )
            }
            Some(node) => {
                let own = || {
                    layout
                        .iter()
                        .filter(move |f| f.node == node && f.generated.is_none() && f.clips())
                };
                let max = own().map(|f| f.max_scroll()).fold(0.0f32, f32::max);
                let max_x = own()
                    .map(crate::geometry::max_scroll_x)
                    .fold(0.0f32, f32::max);
                (
                    offsets.y.get(&node).copied().unwrap_or(0.0),
                    max,
                    offsets.x.get(&node).copied().unwrap_or(0.0),
                    max_x,
                )
            }
        };
        // How far the content reaches past the box whether it scrolls or
        // not, down and across: what `scrollHeight` and `scrollWidth`
        // add to the client size.
        let [over_x, over_y] = match node {
            None => [max_x, max],
            Some(node) => layout
                .iter()
                .find(|f| {
                    f.node == node
                        && f.generated.is_none()
                        && matches!(f.kind, layout::FragmentKind::Box)
                })
                .map_or([0.0, 0.0], crate::geometry::overflow),
        };
        [at.min(max), max, at_x.min(max_x), max_x, over_y, over_x]
            .map(f64::from)
            .to_vec()
    }

    /// A script's scroll of the viewport (`None`) or of `node` across to
    /// `x` and down to `y`, each clamped and left where it is when not
    /// a number: kept for what the run asks next when at once, and for
    /// the engine, which moves a smooth one over the frames to come.
    fn scroll_to(&mut self, node: Option<NodeId>, x: f64, y: f64, smooth: bool) {
        let now = self.scroll_offset(node);
        let (max, max_x) = (now[1], now[3]);
        if node.is_some() && max <= 0.0 && max_x <= 0.0 {
            return;
        }
        let y = y.is_finite().then(|| y.clamp(0.0, max) as f32);
        let x = x.is_finite().then(|| x.clamp(0.0, max_x) as f32);
        if x.is_none() && y.is_none() {
            return;
        }
        self.outputs
            .scrolls
            .push(ScriptScroll { node, x, y, smooth });
    }

    /// CSSOM View's `scrollIntoView` (section 7.1, "scroll a target into
    /// view"): each scroll container around `node`, innermost first, then
    /// the viewport, scrolled so that the element's border box is where
    /// `block` and `inline` (`start`, `center`, `end`, `nearest`) say in
    /// its scrollport, down and across; smoothly for `behavior` `smooth`,
    /// or `auto` and a container whose `scroll-behavior` is (the root's
    /// for the viewport).
    fn scroll_into_view(&mut self, node: NodeId, block: &str, inline: &str, behavior: &str) {
        let smooth_for = |host: &mut Self, container: Option<NodeId>| match behavior {
            "smooth" => true,
            "auto" => {
                let element = container.or_else(|| host.document.document_element());
                element.is_some_and(|e| {
                    host.styles_now()
                        .and_then(|styles| styles.get(e))
                        .is_some_and(|s| s.effects().scroll_behavior_smooth)
                })
            }
            _ => false,
        };
        let g = self.geometry(node, true);
        if g.len() < 4 {
            return;
        }
        // The target's box in viewport coordinates, moved as each scroll
        // moves it.
        let mut target = [g[0], g[1], g[2], g[3]];
        let containers: Vec<NodeId> = self
            .document
            .ancestors(node)
            .filter(|&a| self.document.is_element(a))
            .collect();
        let laid_out = self.layout_now().is_some();
        if !laid_out {
            return;
        }
        let clips = |host: &mut Self, a: NodeId| {
            host.layout_now().is_some_and(|layout| {
                layout
                    .iter()
                    .any(|f| f.node == a && f.generated.is_none() && f.clips())
            })
        };
        // The new offset along one axis: the box `[start, length]` in a
        // scrollport `[start, length]` scrolled to `at`.
        let wanted = |how: &str, at: f64, item: (f64, f64), port: (f64, f64)| -> f64 {
            let (start, end) = (item.0 - port.0, item.0 + item.1 - port.0 - port.1);
            let delta = match how {
                "start" => start,
                "end" => end,
                "center" => (start + end) / 2.0,
                // Nearest: no move when it is in, else the edge nearer.
                _ if start >= 0.0 && end <= 0.0 => 0.0,
                _ if start < 0.0 && item.1 <= port.1 => start,
                _ if start < 0.0 => end.max(start),
                _ if item.1 <= port.1 => end,
                _ => start,
            };
            at + delta
        };
        let mut ports: Vec<(Option<NodeId>, [f64; 4])> = Vec::new();
        for a in containers {
            if !clips(self, a) {
                continue;
            }
            let c = self.geometry(a, true);
            if c.len() < 8 {
                continue;
            }
            // The padding box: the border box less its borders.
            ports.push((
                Some(a),
                [
                    c[0] + c[7],
                    c[1] + c[4],
                    (c[2] - c[5] - c[7]).max(0.0),
                    (c[3] - c[4] - c[6]).max(0.0),
                ],
            ));
        }
        let (width, height) = self.window.viewport;
        ports.push((None, [0.0, 0.0, f64::from(width), f64::from(height)]));
        for (container, port) in ports {
            let now = self.scroll_offset(container);
            let y = wanted(block, now[0], (target[1], target[3]), (port[1], port[3]))
                .clamp(0.0, now[1]);
            let x = wanted(inline, now[2], (target[0], target[2]), (port[0], port[2]))
                .clamp(0.0, now[3]);
            target[0] -= x - now[2];
            target[1] -= y - now[0];
            if x != now[2] || y != now[0] {
                let smooth = smooth_for(self, container);
                self.outputs.scrolls.push(ScriptScroll {
                    node: container,
                    x: Some(x as f32),
                    y: Some(y as f32),
                    smooth,
                });
            }
        }
    }

    /// CSSOM View's `elementsFromPoint` at the viewport point `(x, y)`:
    /// the elements under it, topmost first, the root last; or only the
    /// topmost, for `elementFromPoint`.
    fn elements_from_point(&mut self, x: f32, y: f32, all: bool) -> Vec<NodeId> {
        let (scroll_y, offsets) = self.scroll_state();
        let (width, height) = self.window.viewport;
        if x < 0.0 || y < 0.0 || x > width as f32 || y > height as f32 {
            return Vec::new();
        }
        let Some(layout) = self.layout_now() else {
            return Vec::new();
        };
        let viewport = paint::Viewport {
            width: width as f32,
            height: height as f32,
            scroll_y,
        };
        let (_, _, hits) = crate::geometry::hit(layout, viewport, &offsets, x, y);
        let document = &self.document;
        let mut found: Vec<NodeId> = Vec::new();
        for node in hits.into_iter().rev() {
            let element = std::iter::once(node)
                .chain(document.ancestors(node))
                .find(|&n| document.is_element(n));
            if let Some(element) = element
                && !found.contains(&element)
            {
                found.push(element);
            }
        }
        if let Some(root) = document.document_element()
            && !found.contains(&root)
        {
            found.push(root);
        }
        if !all {
            found.truncate(1);
        }
        found
    }

    /// HTML's `innerText` getter (section 3.2.7, "rendered text
    /// collection steps") over the styles: the text of what is rendered,
    /// white space collapsed as its `white-space` says, a line break for
    /// each `br` and around each block, two around a `p`, a tab between
    /// cells and a line break after a row; the text content for an
    /// element not rendered.
    fn inner_text(&mut self, node: NodeId) -> String {
        self.rendered_text(node, None)
            .unwrap_or_else(|| self.document.text_content(node))
    }

    /// The rendered text between `start` and `end`, as
    /// [`Self::inner_text`] collects it, which the reference browser's
    /// `Selection.toString()` gives (text-transform applied, white space
    /// collapsed, hidden content left out); `None` where the range is in
    /// nothing rendered.
    fn selected_text(&mut self, start: dom::Boundary, end: dom::Boundary) -> Option<String> {
        let mut root = start.node;
        while !self.document.is_inclusive_ancestor(root, end.node) {
            root = self.document.parent(root)?;
        }
        // The walk starts at an element: a text node's parent.
        if self.document.text(root).is_some() {
            root = self.document.parent(root)?;
        }
        self.rendered_text(root, Some((start, end)))
    }

    /// [`Self::inner_text`] of `node`, only the text within `range` where
    /// one is given; `None` where styles are not had or `node` is not
    /// rendered.
    fn rendered_text(
        &mut self,
        node: NodeId,
        range: Option<(dom::Boundary, dom::Boundary)>,
    ) -> Option<String> {
        use style::computed::{Display, Visibility, WhiteSpaceCollapse};
        self.styles_now()?;
        let computed = self.computed.as_ref()?;
        let styles = &computed.styles;
        let document = &self.document;
        let mut work = 0u64;
        let rendered = |n: NodeId| styles.get(n).is_some_and(|s| s.display != Display::None);
        let connected =
            node == document.root() || document.ancestors(node).last() == Some(document.root());
        if !connected || !rendered(node) {
            return None;
        }
        // The part of a text node within the range, in UTF-16 units.
        let within = |n: NodeId, length: u32| -> Option<(u32, u32)> {
            let Some((start, end)) = range else {
                return Some((0, length));
            };
            let at = |offset| dom::Boundary { node: n, offset };
            let from = match document.compare_boundaries(start, at(0))? {
                std::cmp::Ordering::Greater if start.node == n => start.offset.min(length),
                std::cmp::Ordering::Greater => return None,
                _ => 0,
            };
            let to = match document.compare_boundaries(end, at(length))? {
                std::cmp::Ordering::Less if end.node == n => end.offset.min(length),
                std::cmp::Ordering::Less => return None,
                _ => length,
            };
            (from < to).then_some((from, to))
        };
        // The text, and line breaks still to come (a count, or none).
        let mut out = String::new();
        let mut pending: usize = 0;
        let mut started = false;
        let push = |out: &mut String, pending: &mut usize, started: &mut bool, text: &str| {
            if text.is_empty() {
                return;
            }
            if *started {
                out.extend(std::iter::repeat_n('\n', *pending));
            }
            *pending = 0;
            *started = true;
            out.push_str(text);
        };
        // A walk with each element's end visited: (node, leaving).
        let mut stack = vec![(node, false)];
        let mut space = false;
        // Whether the next text starts a word, for `capitalize`: as the
        // text before the element, in its block, leaves it.
        let mut word_start =
            text_before(document, styles, node).is_none_or(|text| text::ends_word(&text, true));
        while let Some((n, leaving)) = stack.pop() {
            work += 1;
            if let Some(text) = document.text(n) {
                let Some(style) = document.parent(n).and_then(|p| styles.get(p)) else {
                    continue;
                };
                if style.visibility != Visibility::Visible {
                    continue;
                }
                let part;
                let text = if range.is_none() {
                    text
                } else {
                    let units: Vec<u16> = text.encode_utf16().collect();
                    let Some((from, to)) = within(n, units.len() as u32) else {
                        continue;
                    };
                    part = String::from_utf16_lossy(&units[from as usize..to as usize]);
                    part.as_str()
                };
                let collapse = style.white_space.collapse;
                let piece = match collapse {
                    WhiteSpaceCollapse::Collapse => {
                        let mut piece = String::new();
                        for c in text.chars() {
                            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
                                space = true;
                            } else {
                                if space && !piece.is_empty() || space && started && pending == 0 {
                                    piece.push(' ');
                                }
                                space = false;
                                piece.push(c);
                            }
                        }
                        piece
                    }
                    _ => {
                        space = false;
                        text.to_owned()
                    }
                };
                // As `text-transform` renders it (CSS Text 3 section 2.1).
                let how = text_transform_of(style);
                let piece = if how.is_none() {
                    word_start = text::ends_word(&piece, word_start);
                    piece
                } else {
                    text::transform(&piece, how, &mut word_start)
                };
                push(&mut out, &mut pending, &mut started, &piece);
                continue;
            }
            if !document.is_element(n) {
                continue;
            }
            let Some(style) = styles.get(n) else {
                continue;
            };
            if style.display == Display::None {
                continue;
            }
            let tag = document.tag(n);
            let block = !matches!(style.display, Display::Inline | Display::None)
                && !matches!(
                    style.display,
                    Display::InlineBlock
                        | Display::InlineFlex
                        | Display::InlineGrid
                        | Display::InlineTable
                );
            if leaving {
                match tag {
                    Some("p") => pending = pending.max(2),
                    _ if style.display == Display::TableCell
                        && document.next_element_sibling(n).is_some() =>
                    {
                        push(&mut out, &mut pending, &mut started, "\t");
                    }
                    _ if style.display == Display::TableRow => pending = pending.max(1),
                    _ if block => pending = pending.max(1),
                    _ => {}
                }
                if block || tag == Some("p") {
                    space = false;
                    word_start = true;
                }
                continue;
            }
            if n != node {
                if tag == Some("br") {
                    push(&mut out, &mut 0, &mut started, "\n");
                    space = false;
                    continue;
                }
                match tag {
                    Some("p") => pending = pending.max(2),
                    _ if block && style.display != Display::TableCell => {
                        pending = pending.max(1);
                    }
                    _ => {}
                }
                if block {
                    space = false;
                    word_start = true;
                }
            }
            stack.push((n, true));
            let children: Vec<NodeId> = document.children(n).collect();
            for child in children.into_iter().rev() {
                stack.push((child, false));
            }
        }
        self.work += work;
        Some(out)
    }

    /// `moveBefore` (DOM section 4.2.3's "move"): its checks with the
    /// reference browser's messages, then `node` moved keeping its state.
    fn move_before(
        &mut self,
        parent: NodeId,
        node: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), DomError> {
        let hierarchy = |reason: &str| Err(DomError::new("HierarchyRequestError", reason));
        if self.document.root_of(parent) != self.document.root_of(node) {
            return hierarchy(
                "State-preserving atomic move cannot be performed on nodes participating in \
                 different trees.",
            );
        }
        self.check_insert(
            parent,
            node,
            before,
            None,
            "The node before which the new node is to be moved is not a child of this node.",
        )
        .or_else(|error| {
            // The node is already the document's element when moved
            // within the document.
            if self.document.parent(node) == Some(parent) {
                Ok(())
            } else {
                Err(error)
            }
        })?;
        if !matches!(
            self.document.data(node),
            Some(
                NodeData::Element { .. }
                    | NodeData::Text(_)
                    | NodeData::Comment(_)
                    | NodeData::ProcessingInstruction { .. }
            )
        ) {
            return hierarchy("The node to be moved must be an Element or CharacterData node.");
        }
        self.document
            .move_before(parent, node, before)
            .map_err(refused)?;
        Ok(())
    }

    /// The parser stopped at the end tag of a script that a write put in
    /// (HTML section 13.2.6.4.8, the nesting level above zero): prepared
    /// as the parser's own are; an inline one run now, nested, the
    /// insertion point after its end tag; an external one fetched, the
    /// pending parsing-blocking script, which stops what was written from
    /// being parsed further, or one to run later. No style sheet blocks
    /// a written script: a script runs only when none blocks scripts, and
    /// the sheets a write brings are found after the run.
    fn prepare_written(&mut self, element: NodeId, line: u32) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        // `document.write`'s scripts are the parser's.
        let prepared = prepare_element(page, &self.document, element, line, true);
        let violations = page.take_violations();
        self.say_violations(violations, Some(element));
        self.parser_prepared(element, !matches!(prepared, Prepared::NotYet));
        match prepared {
            Prepared::NotYet | Prepared::Nothing | Prepared::Refused => {}
            Prepared::Failed => self.outputs.events.push((element, "error")),
            Prepared::Module { request, inline } => {
                let place = module_place(&self.document, element, true);
                self.outputs.order.push(true);
                self.outputs.modules.push(ModuleWanted {
                    element,
                    request,
                    inline,
                    place,
                });
            }
            Prepared::ImportMap(text) => self.register_map(&text, line),
            Prepared::Inline(source) => {
                if self.may_nest()
                    && let Some(parser) = self.parser.as_mut().and_then(html::Load::parser)
                {
                    parser.push_insertion_point();
                    self.now.push_back(source);
                }
            }
            Prepared::Fetch {
                request,
                url,
                fallback,
            } => {
                let place = parser_place(&self.document, element);
                self.blocked |= place == Place::Blocking;
                self.outputs.order.push(false);
                self.outputs.wanted.push(Wanted {
                    request,
                    url,
                    fallback,
                    element,
                    place,
                });
            }
        }
    }

    /// Prepares the scripts scripts made that are now connected and not
    /// started (HTML section 4.12.1.1: when one becomes connected, or is
    /// given children, or a `src`), in tree order: an inline one runs
    /// now, nested; an external one is fetched to run as soon as it comes,
    /// or in order with the others whose `async` was set false.
    fn prepare_made(&mut self) {
        let root = self.document.root();
        // Only those that would start: a script's type, and a `src` or
        // text (a page's templates in `script` elements wait unasked);
        // those an earlier insertion found wait in its order.
        let mut ready: Vec<NodeId> = self
            .pending
            .iter()
            .copied()
            .filter(|&script| {
                self.document.contains(script)
                    && !self.batch.contains(&script)
                    && super::script_type(&self.document, script).is_some()
                    && (self.document.attr(script, "src").is_some()
                        || self.document.attr(script, "href").is_some()
                        || self.document.attr(script, "xlink:href").is_some()
                        || !super::child_text(&self.document, script).is_empty())
                    && self.document.ancestors(script).last() == Some(root)
            })
            .collect();
        if ready.len() > 1 {
            let order: Vec<NodeId> = self
                .document
                .subtree(root)
                .filter(|node| ready.contains(node))
                .collect();
            ready = order;
        }
        // The new ones first: they run within the insertion that made
        // them ready, which a script of the batch made.
        let mut queue: VecDeque<NodeId> = ready.into();
        queue.extend(std::mem::take(&mut self.batch));
        while let Some(script) = queue.pop_front() {
            let connected = self.document.contains(script)
                && self.document.ancestors(script).last() == Some(root);
            if !connected || !self.pending.contains(&script) {
                continue;
            }
            let Some(page) = self.page.as_mut() else {
                return;
            };
            let prepared = prepare_element(page, &self.document, script, 1, false);
            let violations = page.take_violations();
            self.say_violations(violations, Some(script));
            if matches!(prepared, Prepared::NotYet) {
                continue;
            }
            self.pending.retain(|&pending| pending != script);
            self.started.insert(script);
            match prepared {
                Prepared::NotYet | Prepared::Nothing | Prepared::Refused => {}
                Prepared::Failed => self.outputs.events.push((script, "error")),
                Prepared::Module { request, inline } => {
                    let forced = !self.not_forced.contains(&script);
                    let place = match forced {
                        true => Place::Soon,
                        false => module_place(&self.document, script, false),
                    };
                    self.outputs.order.push(true);
                    self.outputs.modules.push(ModuleWanted {
                        element: script,
                        request,
                        inline,
                        place,
                    });
                }
                Prepared::ImportMap(text) => self.register_map(&text, 1),
                Prepared::Inline(source) => {
                    if self.may_nest() {
                        self.now.push_back(source);
                        // It runs before the others are prepared.
                        self.batch = queue;
                        return;
                    }
                }
                Prepared::Fetch {
                    request,
                    url,
                    fallback,
                } => {
                    let forced = !self.not_forced.contains(&script);
                    let place = if forced || self.document.attr(script, "async").is_some() {
                        Place::Soon
                    } else {
                        Place::InOrder
                    };
                    self.outputs.order.push(false);
                    self.outputs.wanted.push(Wanted {
                        request,
                        url,
                        fallback,
                        element: script,
                        place,
                    });
                }
            }
        }
    }

    /// HTML's "register an import map" for a script's element or one a
    /// write put in, of `text` (D61): its warnings on the console; one
    /// that is none an exception reported after the run, at `line`.
    fn register_map(&mut self, text: &str, line: u32) {
        let Some(base) = self
            .page
            .as_ref()
            .and_then(|page| page.base_url(&self.document))
        else {
            return;
        };
        match self.modules.register(text, &base) {
            Registered::Map(warnings) => {
                for warning in warnings {
                    self.say(ConsoleLevel::Warn, warning);
                }
            }
            Registered::Error(error) => self.outputs.thrown.push((error, line)),
        }
    }

    /// Notes that the parser prepared the script `element` (HTML's
    /// "prepare the script element"): it started, or, its type not a
    /// script's or it with neither text nor a `src`, it is left to run
    /// when a script gives it them, as one a script made, its "force
    /// async" flag set (its parser document is null now).
    pub(crate) fn parser_prepared(&mut self, element: NodeId, started: bool) {
        if started {
            self.started.insert(element);
        } else {
            self.made.insert(element);
            self.pending.push(element);
        }
    }

    /// Notes that `element`, just made by a script, is a `script`: it
    /// runs once connected, unless already started.
    fn made_by_script(&mut self, element: NodeId, started: bool) {
        if !is_script(&self.document, element) {
            return;
        }
        self.made.insert(element);
        if started {
            self.started.insert(element);
        } else {
            self.pending.push(element);
        }
    }

    /// The node `handle` names, which a script holds and so is live.
    fn node(&self, handle: Handle) -> Result<NodeId, DomError> {
        NodeId::from_bits(handle.0)
            .filter(|&id| self.document.contains(id))
            .ok_or_else(|| DomError::new("NotFoundError", "The node is no longer there."))
    }

    /// The context a fragment parsed for `element` has: its namespace
    /// and local name, a `body` for what is no element.
    fn fragment_context(&self, element: NodeId) -> (Namespace, String) {
        match self.document.element_name(element) {
            Some((ns, tag)) => (ns.clone(), dom::local_part(tag).to_owned()),
            None => (Namespace::Html, "body".to_owned()),
        }
    }

    /// `node`'s handle, given to a script. The script holds the node once
    /// it makes an object for it ([`Call::Wrap`]).
    fn give(&mut self, node: Option<NodeId>) -> Answer {
        Answer::Node(node.map(handle))
    }

    /// `markup` parsed as a fragment in the context of an element of
    /// namespace and local name `context` (HTML section 13.4) into a new
    /// detached container, nested no deeper than lets it go under
    /// `under`.
    fn parse_fragment(
        &mut self,
        context: &(Namespace, String),
        under: NodeId,
        markup: &str,
    ) -> Result<NodeId, DomError> {
        self.work += markup.len() as u64 / 64;
        let container = self.document.create_element("html").map_err(refused)?;
        let depth = std::iter::once(under)
            .chain(self.document.ancestors(under))
            .filter(|&node| self.document.is_element(node))
            .count();
        let before = self.document.len();
        let max_depth = (dom::MAX_DEPTH + 1).saturating_sub(depth);
        // An XML document's fragment is the XML parser's, in the context
        // of the element it goes under (DOM Parsing section 2.4).
        if self.document.is_xml_document() {
            let parsed = html::xml::parse_xml_fragment(
                &mut self.document,
                container,
                under,
                markup,
                max_depth,
            );
            if parsed.is_err() {
                self.taken_out(container);
                return Err(DomError::new(
                    "SyntaxError",
                    "The provided markup is invalid XML, and therefore cannot be inserted into \
                     an XML document.",
                ));
            }
        } else {
            html::parse_fragment(
                &mut self.document,
                container,
                (&context.0, &context.1),
                true,
                max_depth,
                markup,
            );
        }
        self.work += self.document.len().saturating_sub(before) as u64;
        // A fragment's scripts are already started: they never run.
        let scripts: Vec<NodeId> = self
            .document
            .subtree(container)
            .filter(|&node| is_script(&self.document, node))
            .collect();
        self.started.extend(scripts);
        Ok(container)
    }

    /// Moves the children of `container`, a fragment's, under `parent`
    /// before `before`, and frees the container.
    fn adopt(
        &mut self,
        container: NodeId,
        parent: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), DomError> {
        let mut result = Ok(());
        while let Some(child) = self.document.first_child(container) {
            self.work += 1;
            if let Err(error) = self.document.insert_before(parent, child, before) {
                result = Err(refused(error));
                break;
            }
        }
        let _ = self.document.free(container);
        result
    }

    /// The elements under `root`, in tree order, its descendants alone;
    /// each visited counts as work.
    fn elements_under(&mut self, root: NodeId) -> Vec<NodeId> {
        let mut found = Vec::new();
        for node in self.document.subtree(root).skip(1) {
            self.work += 1;
            if self.document.is_element(node) {
                found.push(node);
            }
        }
        found
    }

    /// The DOM's "pre-insert": its checks, in its order, then `node`
    /// taken from where it was and put under `parent` before `before`.
    fn insert(
        &mut self,
        parent: NodeId,
        node: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), DomError> {
        self.check_insert(
            parent,
            node,
            before,
            None,
            "The node before which the new node is to be inserted is not a child of this node.",
        )?;
        let document = &self.document;
        if document.first_child(node).is_some() {
            self.work += document.subtree(node).count() as u64;
        }
        // A fragment's children go in its place.
        let moved: Vec<NodeId> = match document.data(node) {
            Some(NodeData::DocumentFragment) => document.children(node).collect(),
            _ => vec![node],
        };
        self.document
            .insert_before(parent, node, before)
            .map_err(refused)?;
        for node in moved {
            self.put_in(node);
        }
        Ok(())
    }

    /// The DOM's checks of putting `node` under `parent` before `before`,
    /// or in place of `replacing`, in its order, with the reference
    /// browser's messages; `missing` is the one for a reference that is
    /// not `parent`'s child.
    fn check_insert(
        &self,
        parent: NodeId,
        node: NodeId,
        before: Option<NodeId>,
        replacing: Option<NodeId>,
        missing: &str,
    ) -> Result<(), DomError> {
        let document = &self.document;
        let hierarchy = |reason: String| Err(DomError::new("HierarchyRequestError", reason));
        // The `nodeName`s the reference browser's messages give.
        let name = |n: NodeId| match document.data(n) {
            Some(NodeData::Element { tag, ns, .. }) if *ns == Namespace::Html => {
                tag.to_ascii_uppercase()
            }
            Some(NodeData::Element { tag, .. }) => tag.clone(),
            Some(NodeData::Text(_)) => "#text".to_owned(),
            Some(NodeData::Comment(_)) => "#comment".to_owned(),
            Some(NodeData::ProcessingInstruction { target, .. }) => target.to_string(),
            Some(NodeData::DocumentType { name, .. }) => name.to_string(),
            Some(NodeData::DocumentFragment) => "#document-fragment".to_owned(),
            Some(NodeData::Document) | None => "#document".to_owned(),
        };
        if !matches!(
            document.data(parent),
            Some(NodeData::Element { .. } | NodeData::Document | NodeData::DocumentFragment)
        ) {
            return hierarchy("This node type does not support this method.".to_owned());
        }
        let mut up = Some(parent);
        while let Some(at) = up {
            if at == node {
                return hierarchy("The new child element contains the parent.".to_owned());
            }
            up = document.parent(at).or_else(|| document.template_host(at));
        }
        if let Some(reference) = before
            && document.parent(reference) != Some(parent)
        {
            return Err(DomError::new("NotFoundError", missing));
        }
        let Err(_) = document.check_insert(parent, node, before, replacing) else {
            return Ok(());
        };
        let (kind, into) = (name(node), name(parent));
        if parent == document.root() {
            let element = || {
                document.is_element(node) || document.children(node).any(|n| document.is_element(n))
            };
            if !document.is_text(node)
                && node != document.root()
                && !matches!(document.data(node), Some(NodeData::DocumentType { .. }))
                && element()
            {
                return hierarchy("Only one element on document allowed.".to_owned());
            }
            if matches!(document.data(node), Some(NodeData::DocumentType { .. })) {
                return hierarchy("Only one doctype on document allowed.".to_owned());
            }
        }
        hierarchy(format!(
            "Nodes of type '{kind}' may not be inserted inside nodes of type '{into}'."
        ))
    }

    /// The DOM's "replace a child": its checks, in its order, then `child`
    /// taken out and `node` put in its place.
    fn replace(&mut self, parent: NodeId, node: NodeId, child: NodeId) -> Result<(), DomError> {
        self.check_insert(
            parent,
            node,
            Some(child),
            Some(child),
            "The node to be replaced is not a child of this node.",
        )?;
        let document = &self.document;
        // `dom` takes a reference that is `node` itself for the place after
        // it, as the DOM's step does by hand.
        let before = document.next_sibling(child);
        if document.first_child(node).is_some() {
            self.work += document.subtree(node).count() as u64;
        }
        let moved: Vec<NodeId> = match document.data(node) {
            Some(NodeData::DocumentFragment) => document.children(node).collect(),
            _ => vec![node],
        };
        if child == node {
            return Ok(());
        }
        self.document.remove_child(parent, child).map_err(refused)?;
        self.taken_out(child);
        self.document
            .insert_before(parent, node, before)
            .map_err(refused)?;
        for node in moved {
            self.put_in(node);
        }
        Ok(())
    }

    /// The answer to a script's question about the SVG element `node`'s
    /// geometry (M32), from the styles as they stand: its outer `<svg>`'s
    /// render tree, an instance's style recascaded from its `<use>`.
    fn svg_query(&mut self, node: NodeId, question: svg::Query) -> Option<Vec<f64>> {
        self.styles_now()?;
        let page = self.page.as_ref()?;
        let computed = self.computed.as_ref()?;
        let measure = self.window.fonts.as_ref().map(Measure);
        let animations = RefCell::new(Animations::new());
        let context = CascadeContext {
            viewport_width: self.window.viewport.0 as f32,
            viewport_height: self.window.viewport.1 as f32,
            fonts: measure.as_ref().map(|measure| measure as &dyn FontMeasure),
            language: page.language(),
            media: self.window.media,
            animations: Some(&animations),
            inline_styles: Some(page),
            frame_margins: self.window.frame_margins,
        };
        let recascade = computed
            .sheets
            .recascader(&self.document, &computed.styles, &context);
        let context = svg::Context {
            document: &self.document,
            styles: &computed.styles,
            recascade: Some(&recascade),
            fonts: self.window.fonts.as_ref(),
        };
        let outer = svg::outer_svg(context, node)?;
        let version = self.document.version();
        let tree = match &self.svg_questions {
            Some((svg, built, tree)) if *svg == outer && *built == version => Arc::clone(tree),
            _ => {
                let tree = Arc::new(svg::query_tree(context, outer)?);
                self.work += self.document.len() as u64 / 16;
                self.svg_questions = Some((outer, version, Arc::clone(&tree)));
                tree
            }
        };
        self.work += 1;
        svg::query_in(context, &tree, node, question)
    }

    /// The document's styles as they stand, for `getComputedStyle`
    /// (M25.4): cascaded whole at a run's first ask, from the page's
    /// sheets as they stand in the document (those a script put in or
    /// took out in the run too), and at later asks restyled by the
    /// journal's changes since (M13.1), or cascaded whole again when it
    /// was cut short or a change reached a sheet. A copy of the engine's:
    /// transitions and animations neither start nor show (G46).
    fn styles_now(&mut self) -> Option<&Styles> {
        self.inline_sheets();
        let page = self.page.as_ref()?;
        // The document's fonts, its web fonts among them, which its
        // font-relative units measure.
        let fonts = self
            .window
            .layout_fonts
            .as_ref()
            .or(self.window.fonts.as_ref());
        let fonts_now = fonts_key(fonts);
        if self
            .computed
            .as_ref()
            .is_some_and(|cached| cached.styles.uses_font_units && cached.fonts != fonts_now)
        {
            self.computed = None;
        }
        let measure = fonts.map(Measure);
        let animations = RefCell::new(Animations::new());
        let context = CascadeContext {
            viewport_width: self.window.viewport.0 as f32,
            viewport_height: self.window.viewport.1 as f32,
            fonts: measure.as_ref().map(|measure| measure as &dyn FontMeasure),
            language: page.language(),
            media: self.window.media,
            animations: Some(&animations),
            inline_styles: Some(page),
            frame_margins: self.window.frame_margins,
        };
        let version = self.document.version();
        let changes = self.document.changes();
        let whole = changes.first() == Some(&dom::Change::Everything);
        let sheets_changed = self.computed.as_ref().is_some_and(|cached| {
            cached.version != version
                && changes
                    .get(cached.mark..)
                    .is_some_and(|since| touches_sheets(&self.document, since))
        });
        match &mut self.computed {
            Some(cached) if cached.version == version => {}
            Some(cached) if !whole && cached.mark <= changes.len() => {
                let since = changes[cached.mark..].to_vec();
                self.work += since.len() as u64;
                // New sheets: the elements the rules that changed match
                // are restyled with the rest, as a change of their own,
                // so that what changed starts its transitions.
                let mut matched = Vec::new();
                if sheets_changed {
                    let raw = page.sheets_now(&self.document, &self.window.media);
                    if raw != cached.raw {
                        self.work += self.document.len() as u64;
                        matched = matched_by_changed_rules(&self.document, &cached.raw, &raw);
                        cached.sheets = Stylesheets::new(raw.clone());
                        cached.raw = raw;
                    }
                }
                cached.sheets.restyle_animated(
                    &self.document,
                    &context,
                    &mut cached.styles,
                    &since,
                    &matched,
                );
                cached.mark = changes.len();
                cached.version = version;
            }
            _ => {
                self.work += self.document.len() as u64;
                let raw = page.sheets_now(&self.document, &self.window.media);
                let sheets = Stylesheets::new(raw.clone());
                let styles = sheets.cascade(&self.document, &context);
                self.computed = Some(ComputedStyles {
                    sheets,
                    raw,
                    styles,
                    version,
                    mark: changes.len(),
                    fonts: fonts_now,
                });
            }
        }
        self.computed.as_ref().map(|computed| &computed.styles)
    }

    /// Brings the page's `<style>` sheets up to the document as it now
    /// stands ([`Page::rescan_inline`]), once a version of it.
    fn inline_sheets(&mut self) {
        let version = self.document.version();
        if self.inline_scanned == Some(version) {
            return;
        }
        self.inline_scanned = Some(version);
        if let Some(page) = self.page.as_mut() {
            self.work += self.document.len() as u64 / 16;
            page.rescan_inline(&self.document);
        }
    }

    /// Whether `node` is in the document.
    fn connected(&self, node: NodeId) -> bool {
        let root = self.document.root();
        node == root || self.document.ancestors(node).last() == Some(root)
    }

    /// `node` was taken out of its parent by a script: its subtree may be
    /// freed once let go of, and the bindings let go of the objects of the
    /// nodes in it they hold for the document.
    fn taken_out(&mut self, node: NodeId) {
        self.detached.push(node);
        self.pin(node, false);
    }

    /// `node` was put somewhere by a script: the bindings hold the objects
    /// of the nodes in its subtree if it is now in the document, and let
    /// them go if it is not.
    fn put_in(&mut self, node: NodeId) {
        let connected = self.connected(node);
        self.pin(node, connected);
    }

    /// Notes that the nodes scripts hold in `root`'s subtree are, or are
    /// not, in the document.
    fn pin(&mut self, root: NodeId, connected: bool) {
        if self.exposed.is_empty() {
            return;
        }
        for node in self.document.subtree(root) {
            self.work += 1;
            if self.exposed.contains(&node) {
                self.pins.push((node, connected));
            }
        }
    }

    /// The DOM's `textContent` setter: an element's children replaced by
    /// one text node holding `text`, or by none for the empty string; a
    /// text node's data set; nothing for the document.
    fn set_text_content(&mut self, node: NodeId, text: &str) -> Result<(), DomError> {
        if self.document.character_data(node).is_some() {
            return self.document.set_text(node, text).map_err(refused);
        }
        if matches!(
            self.document.data(node),
            Some(NodeData::Document | NodeData::DocumentType { .. })
        ) {
            return Ok(());
        }
        let made = if text.is_empty() {
            None
        } else {
            Some(self.document.create_text(text).map_err(refused)?)
        };
        while let Some(child) = self.document.first_child(node) {
            self.work += 1;
            if self.document.remove_child(node, child).is_err() {
                break;
            }
            self.taken_out(child);
        }
        if let Some(made) = made
            && let Err(error) = self.document.append_child(node, made)
        {
            let _ = self.document.free(made);
            return Err(refused(error));
        }
        Ok(())
    }

    /// A detached copy of `node` (DOM section 4.5, "clone a node"): its
    /// tag and attributes or its data, and its descendants' if `deep`; a
    /// control's state too, by HTML's cloning steps (M26.7): its value,
    /// checkedness and selectedness, with their dirty flags.
    fn clone_node(&mut self, node: NodeId, deep: bool) -> Result<NodeId, DomError> {
        let copy = |document: &mut Document, from: NodeId| -> Result<NodeId, MutationError> {
            match document.data(from) {
                Some(NodeData::Element { tag, attrs, ns }) => {
                    let (tag, attrs, ns) = (tag.clone(), attrs.clone(), ns.clone());
                    let made = document.create_element_ns(ns, &tag, attrs)?;
                    if let Some(state) = document.control(from) {
                        let state = forms::ControlState {
                            selection: forms::Selection::default(),
                            indeterminate: false,
                            edited_from: None,
                            scroll: 0,
                            custom_error: None,
                            user_validity: false,
                            ..state.clone()
                        };
                        if let Err(error) = document.set_control(made, state) {
                            let _ = document.free(made);
                            return Err(error);
                        }
                    }
                    Ok(made)
                }
                Some(NodeData::Text(data)) => {
                    let data = data.clone();
                    document.create_text(&data)
                }
                Some(NodeData::Comment(data)) => {
                    let data = data.clone();
                    document.create_comment(&data)
                }
                Some(NodeData::ProcessingInstruction { target, data }) => {
                    let (target, data) = (target.clone(), data.clone());
                    document.create_processing_instruction(&target, &data)
                }
                Some(NodeData::DocumentType {
                    name,
                    public_id,
                    system_id,
                }) => {
                    let (name, public_id, system_id) =
                        (name.clone(), public_id.clone(), system_id.clone());
                    document.create_doctype(&name, &public_id, &system_id)
                }
                Some(NodeData::DocumentFragment) => document.create_fragment(),
                _ => Err(MutationError::WrongKind),
            }
        };
        let top = copy(&mut self.document, node).map_err(refused)?;
        self.detached.push(top);
        if !deep {
            return Ok(top);
        }
        // Each entry: a node whose children are still to copy, and its
        // copy; a template's contents are copied into its copy's.
        let mut pending = vec![(node, top)];
        while let Some((from, to)) = pending.pop() {
            if let (Some(from), Some(to)) = (
                self.document.template_content(from),
                self.document.template_content(to),
            ) {
                pending.push((from, to));
            }
            let children: Vec<NodeId> = self.document.children(from).collect();
            for child in children {
                self.work += 1;
                let made = copy(&mut self.document, child).map_err(refused)?;
                if let Err(error) = self.document.append_new(to, made) {
                    let _ = self.document.free(made);
                    return Err(refused(error));
                }
                pending.push((child, made));
            }
        }
        Ok(top)
    }

    /// `document.title`'s setter (HTML section 3.1.7): the first `title`
    /// element's children replaced by `value`'s text, a `title` made last
    /// in the head if there is none; nothing without a document element,
    /// or without a head to put a `title` in. The children replaced are
    /// detached, and freed once no script holds them.
    pub(crate) fn set_document_title(&mut self, value: &str) {
        let document = &mut self.document;
        let Some(html) = document.document_element() else {
            return;
        };
        let existing = document
            .subtree(document.root())
            .find(|&node| document.tag(node) == Some("title"));
        let title = match existing {
            Some(title) => title,
            None => {
                let Some(head) = document
                    .element_children(html)
                    .find(|&child| document.tag(child) == Some("head"))
                else {
                    return;
                };
                let Ok(title) = document.create_element("title") else {
                    return;
                };
                if document.append_child(head, title).is_err() {
                    let _ = document.free(title);
                    return;
                }
                title
            }
        };
        // String replace all: the children go, the text comes in their place.
        let children: Vec<NodeId> = document.children(title).collect();
        let mut removed = Vec::new();
        for child in children {
            if document.remove_child(title, child).is_ok() {
                removed.push(child);
            }
        }
        if !value.is_empty()
            && let Ok(text) = document.create_text(value)
            && document.append_child(title, text).is_err()
        {
            let _ = document.free(text);
        }
        for child in removed {
            self.taken_out(child);
        }
    }

    /// Frees the subtrees scripts took out of the tree that are still
    /// out of it and hold no node a script has an object for; keeps the
    /// rest to look at again after the next run (M25).
    pub(crate) fn sweep(&mut self) {
        let document = &mut self.document;
        let exposed = &self.exposed;
        // The nodes mutation records name are kept too, and those live
        // ranges and iterators hold.
        let mut held = self.mutations.held();
        held.extend(document.live_nodes());
        self.detached.retain(|&root| {
            if !document.contains(root)
                || root == document.root()
                || document.parent(root).is_some()
            {
                return false;
            }
            if document
                .subtree_with_contents(root)
                .into_iter()
                .any(|node| exposed.contains(&node) || held.contains(&node))
            {
                return true;
            }
            let _ = document.free(root);
            false
        });
        let document = &self.document;
        self.canvases.retain(|node| document.contains(node));
    }

    /// The realm is gone, and with it every object a script held.
    pub(crate) fn forget_objects(&mut self) {
        self.exposed.clear();
        // A new realm is given the window's names afresh.
        self.named_synced = false;
    }
}

/// `createImageBitmap()` (M30.2) of a source a document or a worker
/// may have: `given` (an element's, which the document found), or an
/// OffscreenCanvas (3), a bitmap (4), an ImageData's pixels (6) or a
/// Blob's bytes (7). See [`DocumentHost::image_bitmap`].
pub(crate) fn image_bitmap(
    canvases: &mut Canvases,
    call: &js::CanvasCall,
    given: Option<canvas::Source>,
    work: &mut u64,
) -> Result<Answer, DomError> {
    let n = &call.numbers;
    let at = |i: usize| n.get(i).copied().unwrap_or(f64::NAN);
    let state = |message: &str| DomError::new("InvalidStateError", message);
    let unusable = || state("The image source is not usable.");
    let (kind, which) = (at(0), at(1));
    let image_of = |image: &images::Image, clean: bool| {
        let frame = image.frames.first()?;
        Some(canvas::Source {
            pixels: Arc::clone(&frame.pixels),
            width: image.width,
            height: image.height,
            origin_clean: clean,
        })
    };
    let empty_side = |size: (u32, u32)| -> Result<(), DomError> {
        if size.0 == 0 {
            return Err(state("The image source's width is 0."));
        }
        if size.1 == 0 {
            return Err(state("The image source's height is 0."));
        }
        Ok(())
    };
    let source = match given {
        Some(source) => source,
        None => match kind as u8 {
            3 => {
                let id = which as u32;
                let size = canvases.offscreen_size(id).ok_or_else(unusable)?;
                empty_side(size)?;
                let owner = Owner::Offscreen(id);
                let clean = canvases.origin_clean(owner);
                match canvases.pixels(owner) {
                    Some((_, image)) => image_of(&image, clean).ok_or_else(unusable)?,
                    None => return Err(state("The ImageBitmap could not be allocated.")),
                }
            }
            4 => {
                let (image, clean) = canvases.bitmap(which as u32).map_err(|_| unusable())?;
                image_of(&image, clean).ok_or_else(unusable)?
            }
            6 => {
                let (width, height) = (which as u32, at(11) as u32);
                let bytes = call.bytes.as_deref().unwrap_or_default();
                if u64::from(width) * u64::from(height) * 4 != bytes.len() as u64 {
                    return Err(unusable());
                }
                canvas::Source {
                    pixels: Arc::from(canvas::premultiplied(bytes)),
                    width,
                    height,
                    origin_clean: true,
                }
            }
            7 => {
                let bytes = call.bytes.as_deref().unwrap_or_default();
                let image = images::decode(bytes)
                    .map_err(|_| state("The source image could not be decoded."))?;
                *work += bytes.len() as u64 / 64;
                image_of(&image, true).ok_or_else(unusable)?
            }
            _ => return Err(DomError::new("TypeError", "Illegal invocation")),
        },
    };
    // The crop rectangle, flipped to positive sizes; the source's whole
    // when none is given.
    let (w, h) = (f64::from(source.width), f64::from(source.height));
    let crop = if at(2) == 1.0 {
        let (mut sx, mut sy, mut sw, mut sh) = (at(3), at(4), at(5), at(6));
        if sw < 0.0 {
            sx += sw;
            sw = -sw;
        }
        if sh < 0.0 {
            sy += sh;
            sh = -sh;
        }
        [sx, sy, sw, sh]
    } else {
        [0.0, 0.0, w, h]
    };
    // The size: the resize's, one side of it keeping the crop's ratio.
    let (rw, rh) = (at(7), at(8));
    let (out_w, out_h) = match (rw > 0.0, rh > 0.0) {
        (true, true) => (rw, rh),
        (true, false) => (rw, (crop[3] * rw / crop[2]).ceil()),
        (false, true) => ((crop[2] * rh / crop[3]).ceil(), rh),
        (false, false) => (crop[2], crop[3]),
    };
    let pixels = out_w * out_h;
    if !(out_w >= 1.0 && out_h >= 1.0) || pixels > canvas::MAX_CANVAS_PIXELS as f64 {
        return Err(state("The ImageBitmap could not be allocated."));
    }
    let (out_w, out_h) = (out_w as u32, out_h as u32);
    let mut made = canvas::Canvas::new(out_w, out_h, false);
    made.state_mut().image_smoothing = at(9) != 1.0;
    if at(10) == 1.0 {
        made.set_transform(&canvas::Matrix::new(
            1.0,
            0.0,
            0.0,
            -1.0,
            0.0,
            f64::from(out_h),
        ));
    }
    made.draw_image(
        &source,
        Some(crop),
        [0.0, 0.0, f64::from(out_w), f64::from(out_h)],
    );
    *work += made.take_work();
    let image = made
        .snapshot()
        .ok_or_else(|| state("The ImageBitmap could not be allocated."))?;
    let id = canvases.make_bitmap(image, source.origin_clean)?;
    Ok(Answer::Numbers(vec![
        f64::from(id),
        f64::from(out_w),
        f64::from(out_h),
    ]))
}

/// The element an `OffscreenCanvas`'s context draws for, by the
/// negative of its number (M30): no element's size, black for
/// `currentcolor`, the viewport's units for its fonts.
pub(crate) fn offscreen_element(
    handle: f64,
    viewport: (u32, u32),
    fonts: Option<text::FontCollection>,
    source: Option<canvas::Source>,
) -> Element {
    Element {
        owner: Owner::Offscreen((-handle) as u32),
        size: (0, 0),
        color: canvas::Rgba::BLACK,
        font_base: canvas::FontBase {
            viewport: (f64::from(viewport.0), f64::from(viewport.1)),
            ..canvas::FontBase::default()
        },
        rtl: false,
        lang: String::new(),
        fonts,
        source,
    }
}

/// The pixels a drawing takes of an `OffscreenCanvas` (kind 3) or an
/// `ImageBitmap` (kind 4), by number (M30): `None` for another kind,
/// which is an element's.
pub(crate) fn offscreen_source(
    canvases: &mut Canvases,
    kind: f64,
    which: f64,
) -> Option<Result<Option<canvas::Source>, DomError>> {
    let source = |image: &images::Image, clean: bool| {
        let frame = image.frames.first()?;
        Some(canvas::Source {
            pixels: Arc::clone(&frame.pixels),
            width: image.width,
            height: image.height,
            origin_clean: clean,
        })
    };
    match kind as u8 {
        // An `OffscreenCanvas`, by its number.
        3 => {
            let id = which as u32;
            let Some(size) = canvases.offscreen_size(id) else {
                return Some(Err(DomError::new("TypeError", "Illegal invocation")));
            };
            if size.0 == 0 || size.1 == 0 {
                let error = canvas::empty_canvas();
                return Some(Err(DomError::new(error.name, error.message)));
            }
            let owner = Owner::Offscreen(id);
            let clean = canvases.origin_clean(owner);
            Some(Ok(canvases
                .pixels(owner)
                .and_then(|(_, image)| source(&image, clean))
                .or_else(|| transparent(size))))
        }
        // An `ImageBitmap`, by its number.
        4 => Some(
            canvases
                .bitmap(which as u32)
                .map(|(image, clean)| source(&image, clean)),
        ),
        _ => None,
    }
}

/// What a canvas with no context gives a draw (M30): its size of
/// transparent black, as its bitmap is; nothing past the bound of a
/// canvas's pixels.
fn transparent(size: (u32, u32)) -> Option<canvas::Source> {
    let pixels = canvas::blank_image_data(u64::from(size.0), u64::from(size.1)).ok()?;
    Some(canvas::Source {
        pixels: Arc::from(pixels),
        width: size.0,
        height: size.1,
        origin_clean: true,
    })
}

/// A node's handle.
fn handle(node: NodeId) -> Handle {
    Handle(node.to_bits())
}

/// Whether `node` is an element with an activation behaviour (HTML
/// section 6.5): a link, `a` or `area`.
fn has_activation_behaviour(document: &Document, node: NodeId) -> bool {
    // A submit or reset button not disabled, with a form (M26.9).
    if (forms::is_submit_button(document, node) || forms::is_reset_button(document, node))
        && !forms::actually_disabled(document, node)
        && forms::form_owner(document, node).is_some()
    {
        return true;
    }
    match document.tag(node) {
        Some("a" | "area") => true,
        // A checkbox or a radio button not disabled (M26.8), and a label
        // with a control.
        Some("input") => {
            matches!(forms::input_type(document, node), "checkbox" | "radio")
                && !forms::actually_disabled(document, node)
        }
        Some("label") => forms::labeled_control(document, node).is_some(),
        _ => false,
    }
}

/// The most messages the DevTools panel's page may have waiting for the
/// browser (M36): it takes them each turn.
pub(crate) const MAX_PANEL_MESSAGES: usize = 1024;

/// `navigator.platform` (HTML section 8.10.1.1): the value every browser
/// on this system gives.
#[cfg(target_os = "windows")]
pub(crate) const PLATFORM: &str = "Win32";
#[cfg(target_os = "macos")]
pub(crate) const PLATFORM: &str = "MacIntel";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(crate) const PLATFORM: &str = "Linux x86_64";

/// The parts of `url` as `Location` gives them; `about:blank`'s for a
/// document with none.
pub(crate) fn location_parts(url: Option<&net::Url>) -> Vec<String> {
    let Some(url) = url else {
        return ["about:blank", "null", "about:", "", "", "", "blank", "", ""]
            .map(str::to_owned)
            .to_vec();
    };
    let host = url.host().map(ToString::to_string).unwrap_or_default();
    let port = url.port().map(|port| port.to_string()).unwrap_or_default();
    let host_and_port = if port.is_empty() {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
    let search = url
        .query()
        .filter(|query| !query.is_empty())
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let hash = url
        .fragment()
        .filter(|fragment| !fragment.is_empty())
        .map(|fragment| format!("#{fragment}"))
        .unwrap_or_default();
    vec![
        url.to_string(),
        url.origin().serialization(),
        format!("{}:", url.scheme()),
        host_and_port,
        host,
        port,
        url.path_str(),
        search,
        hash,
    ]
}

/// HTML's ASCII white space, which separates classes.
fn is_ascii_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

/// `text` parsed as a selector list, as the style rules' are (M4): one
/// that does not parse is the `SyntaxError` the reference browser
/// throws. A selector ending in a pseudo-element matches no element, as
/// the matcher has it.
fn selector_list(text: &str) -> Result<SelectorList, DomError> {
    if text.chars().all(|c| c.is_ascii_whitespace()) {
        return Err(DomError::new(
            "SyntaxError",
            "The provided selector is empty.",
        ));
    }
    style::parse_selector_list(&css::tokenize(text))
        .map_err(|_| DomError::new("SyntaxError", format!("'{text}' is not a valid selector.")))
}

/// The DOM's "valid attribute local name": not empty, and no ASCII
/// white space, NUL, `/`, `=` or `>`.
pub(crate) fn valid_attribute_local_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_ascii_whitespace() || matches!(c, '\0' | '/' | '=' | '>'))
}

/// The properties whose resolved values are used values (CSSOM section
/// 9, D65).
const USED_PROPERTIES: [&str; 10] = [
    "width",
    "height",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
];

/// `property`'s used value from an element's `geometry` (as
/// [`DocumentHost::geometry`] gives it), for one of [`USED_PROPERTIES`];
/// `None` for an element with no box, or a width or height of an inline
/// box, which keep their computed values.
fn used_value(style: &style::ComputedStyle, property: &str, geometry: &[f64]) -> Option<String> {
    if geometry.len() < 16 {
        return None;
    }
    let g = |i: usize| geometry[i] as f32;
    let inline = style.display == style::computed::Display::Inline;
    let border_box = style.box_sizing == style::computed::BoxSizing::BorderBox;
    let value = match property {
        "width" if !inline => {
            let edges = if border_box {
                0.0
            } else {
                g(5) + g(7) + g(9) + g(11)
            };
            (g(2) - edges).max(0.0)
        }
        "height" if !inline => {
            let edges = if border_box {
                0.0
            } else {
                g(4) + g(6) + g(8) + g(10)
            };
            (g(3) - edges).max(0.0)
        }
        "padding-top" => g(8),
        "padding-right" => g(9),
        "padding-bottom" => g(10),
        "padding-left" => g(11),
        "margin-top" => g(12),
        "margin-right" => g(13),
        "margin-bottom" => g(14),
        "margin-left" => g(15),
        _ => return None,
    };
    Some(style::serialize::px(value))
}

/// A fragment and its place on the page.
type PlacedBox<'a> = (f32, f32, &'a layout::Fragment);

/// `node`'s first own box in `layout` and the box it hangs in (an
/// absolutely positioned box's containing block's), each with its place
/// on the page; `None` for an element with no box, and no parent for a
/// top-level box.
fn own_box(
    layout: &layout::DocumentLayout,
    node: NodeId,
) -> Option<(PlacedBox<'_>, Option<PlacedBox<'_>>)> {
    let mut stack: Vec<(f32, f32, &layout::Fragment, Option<PlacedBox<'_>>)> = layout
        .roots
        .iter()
        .rev()
        .map(|f| (0.0, 0.0, f, None))
        .collect();
    while let Some((dx, dy, f, parent)) = stack.pop() {
        let (x, y) = (dx + f.x, dy + f.y);
        if f.node == node && f.generated.is_none() && matches!(f.kind, layout::FragmentKind::Box) {
            return Some(((x, y, f), parent));
        }
        stack.extend(f.children.iter().rev().map(|c| (x, y, c, Some((x, y, f)))));
    }
    None
}

/// The style rules of `sheets`, those in grouping rules too, in order.
fn style_rules(sheets: &[css::Stylesheet]) -> Vec<&css::QualifiedRule> {
    let mut out = Vec::new();
    let mut stack: Vec<&[css::Rule]> = sheets.iter().rev().map(|s| s.rules.as_slice()).collect();
    while let Some(rules) = stack.pop() {
        for (i, rule) in rules.iter().enumerate() {
            match rule {
                css::Rule::Qualified(q) => out.push(q),
                css::Rule::At(at) => {
                    if let Some(css::Block::Rules(inner)) = &at.block {
                        // The rest of this list after the inner one.
                        stack.push(&rules[i + 1..]);
                        stack.push(inner);
                        break;
                    }
                }
            }
        }
    }
    out
}

/// The elements of `document` a style rule that differs between `old`
/// and `new` matches: the rules between the two lists' common start
/// and common end (a script adds or edits rules in one place).
fn matched_by_changed_rules(
    document: &dom::Document,
    old: &[css::Stylesheet],
    new: &[css::Stylesheet],
) -> Vec<NodeId> {
    let (a, b) = (style_rules(old), style_rules(new));
    let start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let end = a[start..]
        .iter()
        .rev()
        .zip(b[start..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let lists: Vec<style::selector_matching::SelectorList> = a[start..a.len() - end]
        .iter()
        .chain(&b[start..b.len() - end])
        .filter_map(|rule| style::parse_selector_list(&rule.prelude).ok())
        .collect();
    if lists.is_empty() {
        return Vec::new();
    }
    document
        .subtree(document.root())
        .filter(|&n| document.is_element(n))
        .filter(|&n| {
            lists
                .iter()
                .any(|list| style::matches_selector_list(document, n, list))
        })
        .collect()
}

/// Whether one of `changes` reaches a style sheet: a `<style>` or
/// `<link>` put in or taken out (or a subtree holding one), its text, or
/// one of the attributes that choose its sheet.
fn touches_sheets(document: &dom::Document, changes: &[dom::Change]) -> bool {
    let owner = |node: NodeId| matches!(document.tag(node), Some("style" | "link"));
    let holds = |node: NodeId| document.subtree(node).any(owner);
    changes.iter().any(|change| match change {
        dom::Change::Inserted { parent, child } | dom::Change::Removed { parent, child, .. } => {
            owner(*parent) || holds(*child)
        }
        dom::Change::Moved { child, .. } => holds(*child),
        dom::Change::TextChanged { node } => document.parent(*node).is_some_and(owner),
        dom::Change::AttributeChanged { element, name, .. } => {
            owner(*element)
                && matches!(
                    name.as_str(),
                    "media" | "type" | "title" | "rel" | "href" | "disabled"
                )
        }
        dom::Change::Everything => true,
        _ => false,
    })
}

/// A refusal of `dom`'s as the exception a script gets.
fn refused(error: MutationError) -> DomError {
    match error {
        MutationError::TooManyNodes => {
            DomError::new("RangeError", "The document holds too many nodes.")
        }
        MutationError::TooMuchData => {
            DomError::new("RangeError", "The document holds too much data.")
        }
        MutationError::TooDeep => DomError::new(
            "HierarchyRequestError",
            "The node would be nested too deeply.",
        ),
        MutationError::NotAChild => DomError::new(
            "NotFoundError",
            "The node before which the new node is to be inserted is not a child of this node.",
        ),
        MutationError::NoSuchNode => DomError::new("NotFoundError", "The node is no longer there."),
        MutationError::IndexSize => DomError::new(
            "IndexSizeError",
            "The offset is greater than the node's length.",
        ),
        MutationError::HierarchyRequest | MutationError::WrongKind | MutationError::Attached => {
            DomError::new(
                "HierarchyRequestError",
                "The operation would yield an incorrect node tree.",
            )
        }
    }
}

/// The DOM's "valid element local name": a name the HTML parser could
/// make (an ASCII letter first, then anything but ASCII white space,
/// NUL, `/` and `>`), or one of a few more beginning `:`, `_` or past
/// ASCII.
pub(crate) fn valid_element_local_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_alphabetic() {
        return !name
            .chars()
            .any(|c| c.is_ascii_whitespace() || matches!(c, '\0' | '/' | '>'));
    }
    if !(matches!(first, ':' | '_') || first >= '\u{80}') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_') || c >= '\u{80}')
}

/// The UTF-16 code units of `text` before its byte `at` (M26.7): a text
/// control's selection as scripts count it.
fn utf16_units(text: &str, at: usize) -> usize {
    text.get(..at).map_or(0, |head| head.encode_utf16().count())
}

/// The byte of `text` `units` UTF-16 code units in, at a character's
/// start; its end when it is shorter.
fn byte_at_units(text: &str, units: usize) -> usize {
    let mut count = 0;
    for (at, c) in text.char_indices() {
        if count >= units {
            return at;
        }
        count += c.len_utf16();
    }
    text.len()
}

/// What a string compiled as code (`eval`) or a WebAssembly module
/// (`wasm`) refused by `violation` throws, as the reference browser words
/// it (M33.1, p4): an `EvalError`'s message, or a `CompileError`'s after
/// its operation's name.
pub(crate) fn code_refusal(kind: &str, violation: &net::csp::Violation) -> String {
    match kind {
        "wasm" => format!(
            "Compiling or instantiating WebAssembly module violates the following Content \
             Security policy directive because 'unsafe-eval' is not an allowed source of script \
             in the following Content Security Policy directive: \"{violation}\"."
        ),
        _ => format!(
            "Evaluating a string as JavaScript violates the following Content Security Policy \
             directive because 'unsafe-eval' is not an allowed source of script: {violation}\".\n"
        ),
    }
}

/// `style`'s `text-transform` as `innerText` applies it, tailored by the
/// content language.
fn text_transform_of(style: &style::computed::ComputedStyle) -> text::Transform<'_> {
    use style::computed::TextTransform;
    let props = style.text();
    text::Transform {
        case: match style.text_transform {
            TextTransform::None => text::Case::None,
            TextTransform::Uppercase => text::Case::Upper,
            TextTransform::Lowercase => text::Case::Lower,
            TextTransform::Capitalize => text::Case::Capitalize,
        },
        full_width: props.transform_extra.full_width,
        full_size_kana: props.transform_extra.full_size_kana,
        language: props.lang.as_deref().unwrap_or_default(),
    }
}

/// The text before `node` in its block, inline elements and all: the
/// last text node before it whose parent is rendered inline.
fn text_before(document: &Document, styles: &Styles, node: NodeId) -> Option<String> {
    use style::computed::Display;
    let inline = |n: NodeId| styles.get(n).is_some_and(|s| s.display == Display::Inline);
    let mut at = node;
    loop {
        let Some(prev) = document.prev_sibling(at) else {
            let parent = document.parent(at)?;
            if !inline(parent) {
                return None;
            }
            at = parent;
            continue;
        };
        // The last text in it, its last child's first.
        let mut last = Some(prev);
        while let Some(n) = last {
            if let Some(text) = document.text(n) {
                return Some(text.to_owned());
            }
            if !inline(n) {
                return None;
            }
            last = document.children(n).last();
        }
        at = prev;
    }
}
