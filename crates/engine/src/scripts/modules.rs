//! Module scripts' graphs (D61, HTML section 8.1.3.2): a document's
//! module map, which fetches each module once, by its URL, and the graphs
//! of its module scripts, `import()`s and module preloads, each fetched
//! whole before it runs. A module's static imports are found by the
//! realm declaring it apart ([`js::Realm::module_requests`]) and resolved
//! here by the document's import map; the realm is given each module's
//! text once, with the first graph to run that holds it, imports before
//! the modules that import them.
//!
//! The map is keyed by URL and module type, as a module's name has them
//! ([`js::module_name`]): a JSON, CSS or text module is fetched apart
//! from the JavaScript at its URL, held to its own MIME types, and
//! imports nothing.

use std::collections::{HashMap, HashSet, VecDeque};

use js::{MAX_MODULE_DEPTH, ModuleKind, ModuleRequest, ModuleSource, module_name, module_of_name};
use net::{AbortHandle, Destination, Request, Response, Url};

use super::import_map::{ImportMap, MapError, Resolved};
use crate::loader::{FetchId, LoadError};

/// The most modules a document's realm fetches: past them a graph that
/// needs another fails.
pub(crate) const MAX_MODULES: usize = 2048;

/// The most specifiers resolved a document keeps (HTML's resolved module
/// set), which a later import map may not change: past them, one may.
pub(crate) const MAX_RESOLVED: usize = 10_000;

/// The most bytes of modules a document's realm fetches, all together:
/// past them a module fails as its fetch would.
pub(crate) const MAX_MODULE_BYTES: usize = 128 << 20;

/// What finds a module's static imports: the realm declaring it apart,
/// given its URL and text; `None` when it does not parse.
pub(crate) type Scan<'a> = &'a mut dyn FnMut(&str, &str) -> Option<Vec<ModuleRequest>>;

/// A module the map knows, by its name: the URL it was asked for and
/// its kind.
enum Entry {
    Fetching,
    /// Its base URL, its response's; its text, until the realm is given
    /// it; and its imports' names, `None` when it does not parse or one
    /// does not resolve, which its graph's run reports.
    Fetched {
        base: String,
        text: Option<String>,
        imports: Option<Vec<String>>,
    },
    /// Its fetch failed: so does every graph that holds it.
    Failed,
}

/// A graph's root.
pub(crate) enum Root {
    /// An inline module script's text, the line of its document it
    /// starts on, and its base URL, the document's.
    Inline {
        text: String,
        line: u32,
        base: Box<Url>,
    },
    /// An external module script's, an `import()`'s or a module
    /// preload's module, by its name.
    Module(String),
}

/// What a graph is fetched for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// A `script` element's module script.
    Element,
    /// `import()` `id`'s.
    Import(u32),
    /// A `<link rel=modulepreload>`'s.
    Preload,
}

/// Where a graph stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Progress {
    Waiting,
    /// Fetched whole.
    Ready,
    /// A module of it could not be fetched, or it is too large or deep.
    Failed,
}

struct Graph {
    purpose: Purpose,
    /// Its root's name: an inline root's base URL.
    url: String,
    /// An inline root's text, line and imports.
    inline: Option<(String, u32, Option<Vec<String>>)>,
    /// The request its modules are fetched by (HTML's script fetch
    /// options): its root's, or for an inline root one like it.
    template: Request,
    /// The modules it holds and how deep each is, its root at 0.
    depth: HashMap<String, usize>,
    /// Those of them on their way.
    waiting: HashSet<String>,
    progress: Progress,
    /// An `import()`'s graph's settling was queued.
    told: bool,
}

/// A module script ready to run: its root's name, an inline one's text
/// and line, the modules of its graph the realm was not given, imports
/// first, and the request its graph was fetched by, whose options its
/// `import()`s take.
pub(crate) struct ModuleRun {
    pub(crate) url: String,
    pub(crate) inline: Option<(String, u32)>,
    pub(crate) modules: Vec<ModuleSource>,
    pub(crate) template: Request,
}

/// What registering an import map came to.
pub(crate) enum Registered {
    /// It is merged into the document's, with the warnings the console
    /// says.
    Map(Vec<String>),
    /// It is no import map: the exception it reports.
    Error(MapError),
}

/// A document's modules.
#[derive(Default)]
pub(crate) struct Modules {
    map: HashMap<String, Entry>,
    /// The bytes of the modules fetched.
    bytes: usize,
    graphs: HashMap<u32, Graph>,
    next: u32,
    /// The fetches in flight, by the engine's numbers, and their URLs.
    fetching: HashMap<FetchId, String>,
    /// Their results, not yet taken in.
    arrivals: Vec<(String, Result<Response, LoadError>)>,
    /// The URLs to fetch, each with the graph whose request it takes and
    /// the base URL of the module that imports it, its referrer.
    to_fetch: Vec<(String, u32, Option<String>)>,
    import_map: ImportMap,
    /// The specifiers resolved so far (HTML's resolved module set), once
    /// each.
    resolved: Vec<Resolved>,
    resolved_set: HashSet<Resolved>,
    /// The `import()`s that wanted a module in a run: each its number,
    /// the module's name and the host's number of its fetch options.
    imports: Vec<(u32, String, u32)>,
    /// The blobs `blob:` URLs an `import()` named stood for when it was
    /// called, by URL: their fetches take these, though the URLs were
    /// revoked since.
    pinned: HashMap<String, (std::sync::Arc<[u8]>, String)>,
    /// What the console is to say of the modules that failed.
    said: Vec<String>,
}

impl Modules {
    /// HTML's "resolve a module specifier", by the document's import map,
    /// the specifier kept in the resolved module set.
    pub(crate) fn resolve(&mut self, specifier: &str, base: &Url) -> Result<Url, String> {
        let (url, record) = self.import_map.resolve_noting(specifier, base)?;
        if self.resolved.len() < MAX_RESOLVED && self.resolved_set.insert(record.clone()) {
            self.resolved.push(record);
        }
        Ok(url)
    }

    /// HTML's "register an import map" of `text`, whose base URL is
    /// `base`: merged into the document's, whenever it comes.
    pub(crate) fn register(&mut self, text: &str, base: &Url) -> Registered {
        match ImportMap::parse(text, base) {
            Ok((map, mut warnings)) => {
                match self.import_map.merge(map, &self.resolved, &mut warnings) {
                    Ok(()) => Registered::Map(warnings),
                    Err(error) => Registered::Error(error),
                }
            }
            Err(error) => Registered::Error(error),
        }
    }

    /// `import()` `id` wants the module named `url` fetched with the
    /// fetch options numbered `options`; refused past [`MAX_MODULES`]
    /// waiting.
    pub(crate) fn want(&mut self, id: u32, url: String, options: u32) -> Result<(), String> {
        let waiting = self
            .graphs
            .values()
            .filter(|graph| matches!(graph.purpose, Purpose::Import(_)))
            .count();
        if waiting + self.imports.len() >= MAX_MODULES {
            return Err(format!(
                "No more than {MAX_MODULES} dynamic imports may wait for their modules."
            ));
        }
        self.imports.push((id, url, options));
        Ok(())
    }

    /// The blob `url` stands for now, which its fetch takes (a `blob:`
    /// URL's entry, resolved as `import()` resolves it).
    pub(crate) fn pin(&mut self, url: &Url, blob: (std::sync::Arc<[u8]>, String)) {
        if self.pinned.len() < MAX_MODULES {
            self.pinned.insert(url.to_string(), blob);
        }
    }

    /// The `import()`s that wanted a module since this was last asked.
    pub(crate) fn take_wanted(&mut self) -> Vec<(u32, String, u32)> {
        std::mem::take(&mut self.imports)
    }

    /// Starts the graph of `root` for `purpose`, its modules fetched by
    /// requests like `template`; its number.
    pub(crate) fn start(
        &mut self,
        purpose: Purpose,
        root: Root,
        template: Request,
        scan: Scan<'_>,
    ) -> u32 {
        self.next = self.next.wrapping_add(1);
        let id = self.next;
        let (url, inline, first) = match root {
            Root::Inline { text, line, base } => {
                let imports =
                    scan(base.as_str(), &text).and_then(|found| self.resolve_all(&found, &base));
                let first: Vec<(String, usize)> = imports
                    .iter()
                    .flatten()
                    .map(|url| (url.clone(), 1))
                    .collect();
                (base.to_string(), Some((text, line, imports)), first)
            }
            Root::Module(name) => (name.clone(), None, vec![(name, 0)]),
        };
        let depth = first.iter().cloned().collect();
        let graph = Graph {
            purpose,
            url,
            inline,
            template,
            depth,
            waiting: HashSet::new(),
            progress: Progress::Waiting,
            told: false,
        };
        self.graphs.insert(id, graph);
        self.visit(
            id,
            first
                .into_iter()
                .map(|(url, depth)| (url, depth, None))
                .collect(),
        );
        id
    }

    /// `found`, the imports a module at `base` names, resolved to names
    /// and without repeats; `None` if one does not resolve, or its
    /// attributes are refused.
    fn resolve_all(&mut self, found: &[ModuleRequest], base: &Url) -> Option<Vec<String>> {
        let mut urls: Vec<String> = Vec::with_capacity(found.len());
        for request in found {
            let kind = request.kind?;
            let url = self.resolve(&request.specifier, base).ok()?;
            let url = module_name(url.as_str(), kind);
            if !urls.contains(&url) {
                urls.push(url);
            }
            if urls.len() > MAX_MODULES {
                return None;
            }
        }
        Some(urls)
    }

    /// Goes through graph `id` from `queue`'s modules, breadth first:
    /// those the map has not are fetched and waited for, those it has
    /// lead to their imports; a module that failed fails the graph, and
    /// so does one past [`MAX_MODULES`] or [`MAX_MODULE_DEPTH`].
    fn visit(&mut self, id: u32, mut queue: VecDeque<(String, usize, Option<String>)>) {
        let Modules {
            map,
            graphs,
            to_fetch,
            said,
            ..
        } = self;
        let Some(graph) = graphs.get_mut(&id) else {
            return;
        };
        while let Some((url, depth, referrer)) = queue.pop_front() {
            if graph.progress != Progress::Waiting {
                return;
            }
            match map.get(&url) {
                None if map.len() >= MAX_MODULES => {
                    said.push(format!(
                        "Failed to load module script {url}: the page has {MAX_MODULES} modules"
                    ));
                    graph.progress = Progress::Failed;
                }
                None => {
                    map.insert(url.clone(), Entry::Fetching);
                    to_fetch.push((url.clone(), id, referrer));
                    graph.waiting.insert(url);
                }
                Some(Entry::Fetching) => {
                    graph.waiting.insert(url);
                }
                Some(Entry::Failed) => graph.progress = Progress::Failed,
                // A module preload's is its one module (HTML: its
                // descendants fetched at most optionally).
                Some(Entry::Fetched { .. }) if graph.purpose == Purpose::Preload => {}
                Some(Entry::Fetched { base, imports, .. }) => {
                    for import in imports.iter().flatten() {
                        if graph.depth.contains_key(import) {
                            continue;
                        }
                        if depth + 1 > MAX_MODULE_DEPTH {
                            said.push(format!(
                                "Failed to load module script {import}: its graph is deeper than \
                                 {MAX_MODULE_DEPTH}"
                            ));
                            graph.progress = Progress::Failed;
                            break;
                        }
                        graph.depth.insert(import.clone(), depth + 1);
                        queue.push_back((import.clone(), depth + 1, Some(base.clone())));
                    }
                }
            }
        }
        if graph.progress == Progress::Waiting && graph.waiting.is_empty() {
            graph.progress = Progress::Ready;
        }
    }

    /// The fetches to start, each its URL and request: a graph's root by
    /// its element's `integrity`, any other module by the import map's
    /// (HTML's "resolve a module integrity metadata").
    pub(crate) fn take_fetches(&mut self) -> Vec<(String, Request)> {
        let wanted = std::mem::take(&mut self.to_fetch);
        let mut fetches = Vec::with_capacity(wanted.len());
        for (url, id, referrer) in wanted {
            let parsed = Url::parse(module_of_name(&url).0);
            let (Some(graph), Ok(parsed)) = (self.graphs.get(&id), parsed) else {
                self.arrivals
                    .push((url, Err(LoadError::Network(net::NetworkError::Aborted))));
                continue;
            };
            let mut request = graph.template.clone();
            request.abort = AbortHandle::default();
            if url != graph.url || request.integrity.is_empty() {
                request.integrity = self.import_map.integrity(&parsed);
            }
            // A JSON or text module is fetched as `fetch()` fetches (its
            // policy `connect-src`), a CSS module as a style sheet.
            match module_of_name(&url).1 {
                ModuleKind::Js => {}
                ModuleKind::Json | ModuleKind::Text => request.destination = Destination::Empty,
                ModuleKind::Css => request.destination = Destination::Style,
            }
            request.url = parsed;
            // A module another imports is fetched with that one's base URL
            // as its referrer, under the graph's policy (HTML's "fetch the
            // descendants of a module script").
            if let Some(referrer) = referrer.and_then(|r| Url::parse(&r).ok()) {
                request.referrer = net::Referrer::Url(Box::new(referrer));
            }
            // A `blob:` URL pinned when `import()` resolved it: its blob,
            // as Fetch's scheme fetch answers it.
            if let Some((bytes, kind)) = self.pinned.remove(request.url.as_str()) {
                let response = net::blob(&request, Some((&bytes, &kind)));
                self.arrivals
                    .push((url, response.map_err(LoadError::Network)));
                continue;
            }
            fetches.push((url, request));
        }
        fetches
    }

    /// Fetch `id` is the module at `url`'s.
    pub(crate) fn fetch_started(&mut self, id: FetchId, url: String) {
        self.fetching.insert(id, url);
    }

    /// Takes the result of the module at `url`'s fetch, made without the
    /// engine's network: a worker's, which waits for each.
    pub(crate) fn arrived_at(&mut self, url: String, result: Result<Response, LoadError>) {
        self.arrivals.push((url, result));
    }

    /// Whether results wait to be taken in.
    pub(crate) fn has_arrivals(&self) -> bool {
        !self.arrivals.is_empty()
    }

    /// Whether fetch `id` is a module's.
    pub(crate) fn owns(&self, id: FetchId) -> bool {
        self.fetching.contains_key(&id)
    }

    /// What the console is to say since this was last asked.
    pub(crate) fn take_said(&mut self) -> Vec<String> {
        std::mem::take(&mut self.said)
    }

    /// Takes the result of fetch `id`, if it is a module's: whether it
    /// was.
    pub(crate) fn arrived(&mut self, id: FetchId, result: Result<Response, LoadError>) -> bool {
        let Some(url) = self.fetching.remove(&id) else {
            return false;
        };
        self.arrivals.push((url, result));
        true
    }

    /// Whether module fetches are on their way or waiting to be taken in.
    pub(crate) fn busy(&self) -> bool {
        !self.fetching.is_empty()
            || !self.arrivals.is_empty()
            || !self.to_fetch.is_empty()
            || !self.imports.is_empty()
            || self.graphs.values().any(|graph| match graph.purpose {
                Purpose::Import(_) => !graph.told,
                Purpose::Preload => true,
                Purpose::Element => false,
            })
    }

    /// Takes in the modules that came: each checked, decoded, its imports
    /// found by `scan` and resolved, and the graphs that wait for it gone
    /// on with; the console told of those that failed.
    pub(crate) fn advance(&mut self, scan: Scan<'_>) {
        for (url, result) in std::mem::take(&mut self.arrivals) {
            let kind = module_of_name(&url).1;
            let entry =
                match module_of(result, kind) {
                    Ok((_, text)) if self.bytes + text.len() > MAX_MODULE_BYTES => {
                        self.said.push(format!(
                            "Failed to load module script {url}: the page's modules are past \
                         {MAX_MODULE_BYTES} bytes"
                        ));
                        Entry::Failed
                    }
                    Ok((base, text)) => {
                        self.bytes += text.len();
                        // Only JavaScript imports.
                        let imports = match kind {
                            ModuleKind::Js => {
                                scan(&url, &text).and_then(|found| self.resolve_all(&found, &base))
                            }
                            _ => Some(Vec::new()),
                        };
                        Entry::Fetched {
                            base: base.to_string(),
                            text: Some(text),
                            imports,
                        }
                    }
                    Err(why) => {
                        self.said.extend(why.map(|why| match why {
                        Why::Type(essence) if kind == ModuleKind::Js => format!(
                            "Failed to load module script: Expected a JavaScript-or-Wasm module \
                             script but the server responded with a MIME type of \"{essence}\". \
                             Strict MIME type checking is enforced for module scripts per HTML \
                             spec."
                        ),
                        Why::Type(essence) => format!(
                            "Failed to load module script: Expected a {} module script but the \
                             server responded with a MIME type of \"{essence}\". Strict MIME \
                             type checking is enforced for module scripts per HTML spec.",
                            if kind == ModuleKind::Css { "CSS" } else { "JSON" }
                        ),
                        Why::Other(why) => format!("Failed to load module script {url}: {why}"),
                    }));
                        Entry::Failed
                    }
                };
            self.map.insert(url.clone(), entry);
            let waiting: Vec<(u32, usize)> = self
                .graphs
                .iter_mut()
                .filter_map(|(&id, graph)| {
                    graph
                        .waiting
                        .remove(&url)
                        .then(|| (id, graph.depth.get(&url).copied().unwrap_or(0)))
                })
                .collect();
            for (id, depth) in waiting {
                self.visit(id, VecDeque::from([(url.clone(), depth, None)]));
            }
        }
    }

    /// Where graph `id` stands; failed if there is none.
    pub(crate) fn progress(&self, id: u32) -> Progress {
        self.graphs
            .get(&id)
            .map_or(Progress::Failed, |graph| graph.progress)
    }

    /// Graph `id`'s module script, to run now: the graph goes, and the
    /// realm is given its modules it has not.
    pub(crate) fn take_run(&mut self, id: u32) -> Option<ModuleRun> {
        let graph = self.graphs.remove(&id)?;
        let roots = match &graph.inline {
            Some((_, _, imports)) => imports.clone().unwrap_or_default(),
            None => vec![graph.url.clone()],
        };
        let modules = self.hand(roots);
        Some(ModuleRun {
            url: graph.url,
            inline: graph.inline.map(|(text, line, _)| (text, line)),
            modules,
            template: graph.template,
        })
    }

    /// Graph `id` goes, unrun.
    pub(crate) fn drop_graph(&mut self, id: u32) {
        self.graphs.remove(&id);
    }

    /// The `import()`s whose graphs are fetched, or failed, and were not
    /// said to be: their ids.
    pub(crate) fn imports_done(&mut self) -> Vec<u32> {
        let mut done = Vec::new();
        for graph in self.graphs.values_mut() {
            if let Purpose::Import(import) = graph.purpose
                && graph.progress != Progress::Waiting
                && !graph.told
            {
                graph.told = true;
                done.push(import);
            }
        }
        done.sort_unstable();
        done
    }

    /// `import()` `import`'s graph, taken to settle its promise: its
    /// module's URL and the modules the realm was not given, or the
    /// `TypeError`'s message for one that could not be fetched.
    pub(crate) fn take_import(
        &mut self,
        import: u32,
    ) -> Option<(String, Result<Vec<ModuleSource>, String>)> {
        let id = *self
            .graphs
            .iter()
            .find(|(_, graph)| graph.purpose == Purpose::Import(import))?
            .0;
        if self.progress(id) == Progress::Ready {
            let run = self.take_run(id)?;
            return Some((run.url, Ok(run.modules)));
        }
        let graph = self.graphs.remove(&id)?;
        let url = module_of_name(&graph.url).0;
        let message = format!("Failed to fetch dynamically imported module: {url}");
        Some((graph.url, Err(message)))
    }

    /// The modules reachable from `roots` the realm was not given, each
    /// after its imports (depth first, post-order, without recursion),
    /// marked given.
    fn hand(&mut self, roots: Vec<String>) -> Vec<ModuleSource> {
        let mut given = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for root in roots {
            if !seen.insert(root.clone()) {
                continue;
            }
            let mut stack: Vec<(String, usize)> = vec![(root, 0)];
            while let Some((url, next)) = stack.last_mut() {
                let import = match self.map.get(url.as_str()) {
                    Some(Entry::Fetched {
                        imports: Some(imports),
                        ..
                    }) => imports.get(*next).cloned(),
                    _ => None,
                };
                if let Some(import) = import {
                    *next += 1;
                    if seen.insert(import.clone()) {
                        stack.push((import, 0));
                    }
                    continue;
                }
                let Some((url, _)) = stack.pop() else {
                    break;
                };
                if let Some(Entry::Fetched { base, text, .. }) = self.map.get_mut(&url)
                    && let Some(text) = text.take()
                {
                    given.push(ModuleSource {
                        url,
                        base: base.clone(),
                        text,
                    });
                }
            }
        }
        given
    }
}

/// Why a module's fetch gave no module: its type, or anything else.
enum Why {
    Type(String),
    Other(String),
}

/// The module of `kind` a response is (HTML's "fetch a single module
/// script"): its base URL and text, UTF-8 decoded, if it came with an ok
/// status and a MIME type of its kind (JavaScript, JSON, `text/css`, or
/// any for text); else why not, `None` for a refusal by the document's
/// policies, which say so themselves.
fn module_of(
    result: Result<Response, LoadError>,
    kind: ModuleKind,
) -> Result<(Url, String), Option<Why>> {
    let response = match result {
        Ok(response) => response,
        Err(LoadError::Network(net::NetworkError::Csp(_))) => return Err(None),
        Err(error) => return Err(Some(Why::Other(error.to_string()))),
    };
    if !response.ok() {
        return Err(Some(Why::Other(format!("status {}", response.status))));
    }
    let fits = |mime: &net::mime::MimeType| match kind {
        ModuleKind::Js => mime.is_javascript(),
        ModuleKind::Json => is_json(mime.essence()),
        ModuleKind::Css => mime.essence() == "text/css",
        ModuleKind::Text => true,
    };
    match &response.mime_type {
        Some(mime) if fits(mime) => {}
        None if kind == ModuleKind::Text => {}
        Some(mime) => return Err(Some(Why::Type(mime.essence().to_owned()))),
        None => return Err(Some(Why::Type(String::new()))),
    }
    let bytes = response
        .body
        .strip_prefix(b"\xEF\xBB\xBF")
        .unwrap_or(&response.body);
    Ok((
        response.url().clone(),
        String::from_utf8_lossy(bytes).into_owned(),
    ))
}

/// Whether `essence` is a JSON MIME type (MIME Sniffing's): `application/
/// json`, `text/json`, or a subtype ending in `+json`.
fn is_json(essence: &str) -> bool {
    essence == "application/json"
        || essence == "text/json"
        || essence
            .split_once('/')
            .is_some_and(|(_, subtype)| subtype.ends_with("+json"))
}
