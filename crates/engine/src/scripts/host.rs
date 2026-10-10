//! The host a document's scripts reach it through (M24.4, M25,
//! `docs/design/dom-scripting.md`): the document, lent for each run, and
//! what must last between runs: the nodes scripts hold objects for, and
//! the subtrees they took out of the tree, freed once nothing holds them.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use dom::{Document, MutationError, NodeData, NodeId, forms};
use js::{Answer, Call, ConsoleLevel, DomError, Handle, NestedScript, ScriptError, ScriptingHost};
use net::cookies::{Api, Context};
use style::animation::Animations;
use style::selector_matching::{Matcher, PseudoElement, SelectorList};
use style::{CascadeContext, FontMeasure, Styles, Stylesheets};

use super::cssom;
use super::{
    ConsoleEntry, How, Lent, MAX_SCRIPT_NESTING, MAX_TIMERS, Place, Prepared, ReadyState,
    ScriptNavigation, Source, TimeOrigin, Wanted, WindowEnv, cut, document_title, parser_place,
    prepare_element, refusal_text, report_text,
};
use crate::loader::Page;
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
    /// A validation message reported, for the notice (M26.9b).
    pub(crate) notice: Option<String>,
    /// The fetches scripts asked for (M27.1), by their numbers.
    pub(crate) fetches: Vec<(u32, net::Request)>,
    /// The fetches their signals aborted.
    pub(crate) aborted: Vec<u32>,
}

/// The document's styles as a run's `getComputedStyle` sees them: the
/// page's sheets, prepared once a run, and the styles, as of `version`,
/// and of the journal's first `mark` changes.
struct ComputedStyles {
    sheets: Stylesheets,
    styles: Styles,
    version: u64,
    mark: usize,
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
    fn set(&mut self, id: u32, delay: i32, level: u32, now: Duration) -> Result<(), DomError> {
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
    fn clear(&mut self, id: u32) {
        if let Some(key) = self.by_id.remove(&id) {
            self.queue.remove(&key);
        }
    }

    /// When the next timer is due.
    pub(crate) fn next_due(&self) -> Option<Duration> {
        self.queue.keys().next().map(|(due, _)| *due)
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

/// The window's fonts, measuring for the cascade (`ex` and `ch`).
struct Measure<'a>(&'a text::FontCollection);

impl FontMeasure for Measure<'_> {
    fn x_height(&self, font_size_px: f32) -> f32 {
        self.0.x_height(font_size_px)
    }

    fn zero_advance(&self, font_size_px: f32) -> f32 {
        self.0.zero_advance(font_size_px)
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
    /// How many fetches the document has in flight (M27.1).
    pub(crate) in_flight: usize,
    /// The engine's client, lent for a run, for a synchronous XHR
    /// (M27.6).
    client: Option<Arc<net::Client>>,
    /// The engine's cookie jar, lent for a run (M27.4).
    cookies: Option<Arc<Mutex<net::cookies::Jar>>>,
    /// The document's page and parser, lent for a run; and where its
    /// loading stands.
    page: Option<Page>,
    parser: Option<html::Load>,
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
    /// The document's timers (M26).
    pub(crate) timers: Timers,
    /// The timer nesting level of the task the run is (HTML section
    /// 8.6): 0 but for a timer's.
    nesting: u32,
    /// The document's time origin, once it has one.
    pub(crate) origin: Option<TimeOrigin>,
    /// An animation frame callback waits for the next frame (M26).
    pub(crate) frame_wanted: bool,
}

impl ScriptingHost for DocumentHost {
    fn console(&mut self, level: ConsoleLevel, message: String) {
        self.say(level, message);
    }

    fn title(&mut self) -> String {
        document_title(&self.document)
    }

    fn set_title(&mut self, title: String) {
        self.set_document_title(&title);
    }

    fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        self.work += 1;
        let inserts = matches!(
            call,
            Call::Insert { .. }
                | Call::Replace { .. }
                | Call::SetTextContent(..)
                | Call::SetAttribute(..)
                | Call::SetInnerHtml(..)
                | Call::SetOuterHtml(..)
                | Call::InsertAdjacentHtml { .. }
        );
        let answer = self.answer(call)?;
        if inserts && !self.pending.is_empty() {
            self.prepare_made();
        }
        Ok(answer)
    }

    fn take_work(&mut self) -> u64 {
        std::mem::take(&mut self.work)
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
                return Some(NestedScript {
                    source: source.text,
                    url: source.url,
                    line: source.line,
                });
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
        if !page.origin().is_opaque() && self.window.storage.is_some() {
            return String::new();
        }
        if page.url().is_some_and(|url| url.scheme() == "data") {
            return "Storage is disabled inside 'data:' URLs.".to_owned();
        }
        "Access is denied for this document.".to_owned()
    }

    /// `f` of the document's `localStorage` area if `local`, else its
    /// `sessionStorage` area; a `SecurityError` when its scripts may not
    /// reach them.
    fn with_area<T>(
        &self,
        local: bool,
        f: impl FnOnce(&mut storage::Area) -> T,
    ) -> Result<T, DomError> {
        let refused = self.storage_refused();
        if !refused.is_empty() {
            return Err(DomError::new("SecurityError", refused));
        }
        let (Some(page), Some(storage)) = (&self.page, &self.window.storage) else {
            return Err(DomError::new("SecurityError", refused));
        };
        let origin = page.origin().serialization();
        let mut storage = storage.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(f(storage.area(local, &origin)))
    }

    /// Notes a change of an area (M28): for the profile, and, when
    /// another document alive shares the origin, for its `storage` event.
    fn storage_changed(
        &self,
        local: bool,
        key: Option<String>,
        old: Option<String>,
        new: Option<String>,
    ) {
        let (Some(page), Some(storage)) = (&self.page, &self.window.storage) else {
            return;
        };
        let origin = page.origin().serialization();
        let mut storage = storage.lock().unwrap_or_else(PoisonError::into_inner);
        storage.changed(local, &origin, key.as_deref());
        if self.window.storage_peer {
            storage.broadcast(storage::Change {
                local,
                origin,
                key,
                old,
                new,
                url: page
                    .url()
                    .map_or_else(String::new, |url| url.as_str().to_owned()),
                page: page.id(),
            });
        }
    }

    /// Answers `call`.
    fn answer(&mut self, call: Call) -> Result<Answer, DomError> {
        match call {
            Call::Document => Ok(self.give(Some(self.document.root()))),
            Call::Wrap(node) => {
                let id = self.node(node)?;
                self.exposed.insert(id);
                let name = match self.document.data(id) {
                    Some(NodeData::Element { tag, .. }) => tag.clone(),
                    Some(NodeData::Text(_)) => "#text".to_owned(),
                    _ => "#document".to_owned(),
                };
                let connected = if self.connected(id) { "connected" } else { "" };
                let mut answer = vec![name, connected.to_owned()];
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
                let kind = match self.document.data(self.node(node)?) {
                    Some(NodeData::Element { .. }) => 1.0,
                    Some(NodeData::Text(_)) => 3.0,
                    _ => 9.0,
                };
                Ok(Answer::Number(kind))
            }
            Call::Tag(node) => {
                let tag = self.document.tag(self.node(node)?).unwrap_or_default();
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
                let data = self.document.text(self.node(node)?).unwrap_or_default();
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
                if node == self.document.root() {
                    return Ok(Answer::MaybeString(None));
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
                let value = self.document.attr(self.node(node)?, &name);
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
                let element = self.document.create_element(&name).map_err(refused)?;
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
                    .filter(|&(from, _)| self.document.tag(from) == Some("script"))
                    .collect();
                for (from, to) in pairs {
                    let started = self.started.contains(&from);
                    self.made_by_script(to, started);
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
                let name = name.to_ascii_lowercase();
                let found = self
                    .elements_under(root)
                    .into_iter()
                    .filter(|&e| name == "*" || self.document.tag(e) == Some(name.as_str()))
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
            Call::InnerHtml(node) => {
                let node = self.node(node)?;
                let markup = html::serialize_children(&self.document, node, true);
                self.work += markup.len() as u64 / 64;
                Ok(Answer::String(markup))
            }
            Call::OuterHtml(node) => {
                let node = self.node(node)?;
                let markup = html::serialize_node(&self.document, node, true);
                self.work += markup.len() as u64 / 64;
                Ok(Answer::String(markup))
            }
            Call::SetInnerHtml(node, markup) => {
                let node = self.node(node)?;
                let context = self.document.tag(node).unwrap_or("body").to_owned();
                let container = self.parse_fragment(&context, node, &markup)?;
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
                let context = self.document.tag(parent).unwrap_or("body").to_owned();
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
                    Some("html") | None => "body".to_owned(),
                    Some(tag) => tag.to_owned(),
                };
                let container = self.parse_fragment(&context, parent, &markup)?;
                self.adopt(container, parent, before)?;
                Ok(Answer::Nothing)
            }
            Call::Write(text) => {
                self.write(&text);
                Ok(Answer::Nothing)
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
            Call::Origin => Ok(Answer::String(
                self.page
                    .as_ref()
                    .map_or_else(|| "null".to_owned(), |page| page.origin().serialization()),
            )),
            Call::SecureContext => Ok(Answer::Bool(
                self.page.as_ref().is_none_or(Page::is_secure_context),
            )),
            Call::Fetch(call) => {
                let page = self
                    .page
                    .as_ref()
                    .ok_or_else(|| DomError::new("TypeError", "Failed to fetch"))?;
                let request = super::fetch::request(page, &call)?;
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
                let request = super::fetch::request(page, &call)?;
                let Some(client) = &self.client else {
                    return Ok(Answer::Fetched(None));
                };
                // The run, and the page, wait for it, as the reference
                // browser's does.
                let result = client.fetch(&request);
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
            Call::UrlHasCredentials(url) => Ok(Answer::Bool(
                net::Url::parse(&url).is_ok_and(|url| url.includes_credentials()),
            )),
            Call::SameOriginUrl(url) => Ok(Answer::Bool(net::Url::parse(&url).is_ok_and(|url| {
                self.page
                    .as_ref()
                    .is_some_and(|page| *page.origin() == url.origin())
            }))),
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
                let cookies = self.cookie_url().and_then(|url| {
                    let jar = self.cookies.as_ref()?;
                    let mut jar = jar.lock().expect("never poisoned");
                    jar.header(&url, Api::Script, Context::SAME_SITE, SystemTime::now())
                });
                Ok(Answer::String(cookies.map_or_else(String::new, |bytes| {
                    String::from_utf8_lossy(&bytes).into_owned()
                })))
            }
            Call::SetCookie(value) => {
                if let (Some(url), Some(jar)) = (self.cookie_url(), &self.cookies) {
                    let mut jar = jar.lock().expect("never poisoned");
                    jar.store(
                        value.as_bytes(),
                        &url,
                        Api::Script,
                        Context::SAME_SITE,
                        SystemTime::now(),
                    );
                }
                Ok(Answer::Nothing)
            }
            Call::DocumentDomain => {
                Ok(Answer::String(match self.page.as_ref().map(Page::origin) {
                    Some(net::Origin::Tuple { host, .. }) => host.to_string(),
                    _ => String::new(),
                }))
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
                self.with_area(local, |area| Answer::Number(area.len() as f64))
            }
            Call::StorageKey(local, index) => self.with_area(local, |area| {
                Answer::MaybeString(usize::try_from(index).ok().and_then(|i| area.key(i)))
            }),
            Call::StorageKeys(local) => {
                self.with_area(local, |area| Answer::Strings(area.keys().to_vec()))
            }
            Call::StorageGet(local, key) => self.with_area(local, |area| {
                Answer::MaybeString(area.get(&key).map(str::to_owned))
            }),
            Call::StorageSet(local, key, value) => {
                let changed = self.with_area(local, |area| match area.set(&key, &value) {
                    Ok(old) => Ok(old),
                    Err(storage::OverQuota) => Err(()),
                })?;
                match changed {
                    Err(()) => Ok(Answer::Bool(false)),
                    Ok(None) => Ok(Answer::Bool(true)),
                    Ok(Some(old)) => {
                        self.storage_changed(local, Some(key), old, Some(value));
                        Ok(Answer::Bool(true))
                    }
                }
            }
            Call::StorageRemove(local, key) => {
                if let Some(old) = self.with_area(local, |area| area.remove(&key))? {
                    self.storage_changed(local, Some(key), Some(old), None);
                }
                Ok(Answer::Nothing)
            }
            Call::StorageClear(local) => {
                if self.with_area(local, |area| area.clear())? {
                    self.storage_changed(local, None, None, None);
                }
                Ok(Answer::Nothing)
            }
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
                });
                Ok(Answer::Nothing)
            }
            Call::NavigatorInfo => {
                let (agent, mode) = if self.window.compat {
                    (net::COMPAT_USER_AGENT, "chrome")
                } else {
                    (net::USER_AGENT, "gecko")
                };
                Ok(Answer::Strings(vec![
                    agent.to_owned(),
                    PLATFORM.to_owned(),
                    mode.to_owned(),
                ]))
            }
            Call::Viewport => {
                let (width, height) = self.window.viewport;
                Ok(Answer::Numbers(vec![
                    f64::from(width),
                    f64::from(height),
                    1.0,
                ]))
            }
            Call::DocumentInfo => {
                let page = self.page.as_ref();
                let url = page
                    .and_then(|page| page.url())
                    .map_or_else(|| "about:blank".to_owned(), ToString::to_string);
                let encoding = page.map_or(encoding::UTF_8, |page| page.encoding());
                Ok(Answer::Strings(vec![
                    url,
                    encoding.name().to_owned(),
                    "text/html".to_owned(),
                    "CSS1Compat".to_owned(),
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
                let styles = self.styles_now();
                let style = match pseudo {
                    None => styles.and_then(|styles| styles.get(node)),
                    Some(pseudo) => styles.and_then(|styles| styles.pseudo(node, pseudo)),
                };
                let value =
                    style.and_then(|style| style::serialize::computed_value(style, &property));
                Ok(Answer::MaybeString(value))
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
                    });
                }
                Ok(Answer::Nothing)
            }
            Call::ResetForm(form) => {
                let form = self.node(form)?;
                forms::reset_form(&mut self.document, form);
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
            Call::Dialog { kind, message } => {
                self.say(ConsoleLevel::Info, format!("{kind}: {message}"));
                Ok(Answer::Nothing)
            }
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

    /// Lends the host what a run needs, `how` the script runs, and
    /// whether a parsing-blocking script is pending.
    pub(crate) fn lend(&mut self, lent: &mut Lent<'_>, how: How, blocked: bool) {
        self.document = std::mem::take(lent.document);
        let placeholder = Page::given(lent.page.id());
        self.page = Some(std::mem::replace(lent.page, placeholder));
        self.cookies = Some(lent.network.cookies());
        self.client = Some(lent.network.client());
        self.parser = lent
            .parser
            .as_deref_mut()
            .map(|load| std::mem::replace(load, html::Load::new(None, false)));
        self.ready = lent.ready;
        self.blocked = blocked;
        self.too_deep = false;
        self.window = std::mem::take(&mut lent.window);
        self.computed = None;
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
        self.frames.clear();
        self.now.clear();
        self.computed = None;
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
        self.outputs.console.push(ConsoleEntry {
            level,
            text: cut(message),
        });
    }

    /// `document.write` (HTML section 8.4.3): the text put in before the
    /// insertion point, to be parsed when the call has made it in. With
    /// none, a script from a file writes nothing, and anything else would
    /// open the document again, which this version does not do (G48).
    fn write(&mut self, text: &str) {
        self.work += text.len() as u64 / 64;
        let defined = self
            .parser
            .as_mut()
            .and_then(html::Load::parser)
            .is_some_and(|parser| parser.has_insertion_point());
        if !defined {
            if !self.frames.iter().any(|frame| frame.external) {
                self.say(
                    ConsoleLevel::Warn,
                    "document.write() outside the parse would replace the document, which is \
                     not supported yet: nothing was written"
                        .to_owned(),
                );
            }
            return;
        }
        if let Some(parser) = self.parser.as_mut().and_then(html::Load::parser) {
            parser.write(text);
        }
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
        match prepare_element(page, &self.document, element, line) {
            Prepared::NotYet | Prepared::Nothing => {}
            Prepared::Failed => self.outputs.events.push((element, "error")),
            Prepared::Refused(what, violation) => {
                self.say(ConsoleLevel::Error, refusal_text(what, &violation));
            }
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
        let mut ready: Vec<NodeId> = self
            .pending
            .iter()
            .copied()
            .filter(|&script| {
                self.document.contains(script)
                    && self.document.ancestors(script).last() == Some(root)
            })
            .collect();
        if ready.is_empty() {
            return;
        }
        if ready.len() > 1 {
            let order: Vec<NodeId> = self
                .document
                .subtree(root)
                .filter(|node| ready.contains(node))
                .collect();
            ready = order;
        }
        for script in ready {
            let Some(page) = self.page.as_mut() else {
                return;
            };
            let prepared = prepare_element(page, &self.document, script, 1);
            if matches!(prepared, Prepared::NotYet) {
                continue;
            }
            self.pending.retain(|&pending| pending != script);
            self.started.insert(script);
            match prepared {
                Prepared::NotYet | Prepared::Nothing => {}
                Prepared::Failed => self.outputs.events.push((script, "error")),
                Prepared::Refused(what, violation) => {
                    self.say(ConsoleLevel::Error, refusal_text(what, &violation));
                }
                Prepared::Inline(source) => {
                    if self.may_nest() {
                        self.now.push_back(source);
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

    /// Notes that `element`, just made by a script, is a `script`: it
    /// runs once connected, unless already started.
    fn made_by_script(&mut self, element: NodeId, started: bool) {
        if self.document.tag(element) != Some("script") {
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

    /// `node`'s handle, given to a script. The script holds the node once
    /// it makes an object for it ([`Call::Wrap`]).
    fn give(&mut self, node: Option<NodeId>) -> Answer {
        Answer::Node(node.map(handle))
    }

    /// `markup` parsed as a fragment in the context of an element with
    /// local name `context` (HTML section 13.4) into a new detached
    /// container, nested no deeper than lets it go under `under`.
    fn parse_fragment(
        &mut self,
        context: &str,
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
        html::parse_fragment(
            &mut self.document,
            container,
            context,
            true,
            (dom::MAX_DEPTH + 1).saturating_sub(depth),
            markup,
        );
        self.work += self.document.len().saturating_sub(before) as u64;
        // A fragment's scripts are already started: they never run.
        let scripts: Vec<NodeId> = self
            .document
            .subtree(container)
            .filter(|&node| self.document.tag(node) == Some("script"))
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
        let document = &self.document;
        let hierarchy = |reason: &str| Err(DomError::new("HierarchyRequestError", reason));
        if document.is_text(parent) {
            return hierarchy("This node type does not support this method.");
        }
        if document.is_inclusive_ancestor(node, parent) {
            return hierarchy("The new child element contains the parent.");
        }
        if let Some(before) = before
            && document.parent(before) != Some(parent)
        {
            return Err(DomError::new(
                "NotFoundError",
                "The node before which the new node is to be inserted is not a child of this \
                 node.",
            ));
        }
        if node == document.root() {
            return hierarchy(
                "Nodes of type '#document' may not be inserted inside nodes of type '#document'.",
            );
        }
        if parent == document.root() {
            if document.is_text(node) {
                return hierarchy(
                    "Nodes of type '#text' may not be inserted inside nodes of type '#document'.",
                );
            }
            if document.element_children(parent).next().is_some() {
                return hierarchy("Only one element on document allowed.");
            }
        }
        if document.first_child(node).is_some() {
            self.work += document.subtree(node).count() as u64;
        }
        self.document
            .insert_before(parent, node, before)
            .map_err(refused)?;
        self.put_in(node);
        Ok(())
    }

    /// The DOM's "replace a child": its checks, in its order, then `child`
    /// taken out and `node` put in its place.
    fn replace(&mut self, parent: NodeId, node: NodeId, child: NodeId) -> Result<(), DomError> {
        let document = &self.document;
        let hierarchy = |reason: &str| Err(DomError::new("HierarchyRequestError", reason));
        if document.is_text(parent) {
            return hierarchy("This node type does not support this method.");
        }
        if document.is_inclusive_ancestor(node, parent) {
            return hierarchy("The new child element contains the parent.");
        }
        if document.parent(child) != Some(parent) {
            return Err(DomError::new(
                "NotFoundError",
                "The node to be replaced is not a child of this node.",
            ));
        }
        if parent == document.root() {
            if document.is_text(node) {
                return hierarchy(
                    "Nodes of type '#text' may not be inserted inside nodes of type '#document'.",
                );
            }
            if document.is_element(node)
                && document
                    .element_children(parent)
                    .any(|other| other != child)
            {
                return hierarchy("Only one element on document allowed.");
            }
        }
        // `dom` takes a reference that is `node` itself for the place after
        // it, as the DOM's step does by hand.
        let before = document.next_sibling(child);
        if document.first_child(node).is_some() {
            self.work += document.subtree(node).count() as u64;
        }
        self.document.remove_child(parent, child).map_err(refused)?;
        self.taken_out(child);
        self.document
            .insert_before(parent, node, before)
            .map_err(refused)?;
        self.put_in(node);
        Ok(())
    }

    /// The document's styles as they stand, for `getComputedStyle`
    /// (M25.4): cascaded whole at a run's first ask, from the page's
    /// sheets as they were when the run began, and at later asks
    /// restyled by the journal's changes since (M13.1), or cascaded whole
    /// again when it was cut short. A copy of the engine's: transitions
    /// and animations neither start nor show (G46).
    fn styles_now(&mut self) -> Option<&Styles> {
        let page = self.page.as_ref()?;
        let measure = self.window.fonts.as_ref().map(Measure);
        let animations = RefCell::new(Animations::new());
        let context = CascadeContext {
            viewport_width: self.window.viewport.0 as f32,
            viewport_height: self.window.viewport.1 as f32,
            fonts: measure.as_ref().map(|measure| measure as &dyn FontMeasure),
            language: page.language(),
            media: self.window.media,
            animations: Some(&animations),
        };
        let version = self.document.version();
        let changes = self.document.changes();
        let whole = changes.first() == Some(&dom::Change::Everything);
        match &mut self.computed {
            Some(cached) if cached.version == version => {}
            Some(cached) if !whole && cached.mark <= changes.len() => {
                let since = changes[cached.mark..].to_vec();
                self.work += since.len() as u64;
                cached.sheets.restyle_animated(
                    &self.document,
                    &context,
                    &mut cached.styles,
                    &since,
                    &[],
                );
                cached.mark = changes.len();
                cached.version = version;
            }
            _ => {
                self.work += self.document.len() as u64;
                let sheets = Stylesheets::new(page.sheets());
                let styles = sheets.cascade(&self.document, &context);
                self.computed = Some(ComputedStyles {
                    sheets,
                    styles,
                    version,
                    mark: changes.len(),
                });
            }
        }
        self.computed.as_ref().map(|computed| &computed.styles)
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
        if self.document.is_text(node) {
            return self.document.set_text(node, text).map_err(refused);
        }
        if node == self.document.root() {
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
                Some(NodeData::Element { tag, attrs }) => {
                    let (tag, attrs) = (tag.clone(), attrs.clone());
                    let made = document.create_element_with(&tag, attrs)?;
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
                _ => Err(MutationError::WrongKind),
            }
        };
        let top = copy(&mut self.document, node).map_err(refused)?;
        self.detached.push(top);
        if !deep {
            return Ok(top);
        }
        // Each entry: a node whose children are still to copy, and its copy.
        let mut pending = vec![(node, top)];
        while let Some((from, to)) = pending.pop() {
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
        self.detached.retain(|&root| {
            if !document.contains(root)
                || root == document.root()
                || document.parent(root).is_some()
            {
                return false;
            }
            if document.subtree(root).any(|node| exposed.contains(&node)) {
                return true;
            }
            let _ = document.free(root);
            false
        });
    }

    /// The realm is gone, and with it every object a script held.
    pub(crate) fn forget_objects(&mut self) {
        self.exposed.clear();
    }
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

/// `navigator.platform` (HTML section 8.10.1.1): the value every browser
/// on this system gives.
#[cfg(target_os = "windows")]
const PLATFORM: &str = "Win32";
#[cfg(target_os = "macos")]
const PLATFORM: &str = "MacIntel";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const PLATFORM: &str = "Linux x86_64";

/// The parts of `url` as `Location` gives them; `about:blank`'s for a
/// document with none.
fn location_parts(url: Option<&net::Url>) -> Vec<String> {
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
