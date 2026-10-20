//! The DevTools' router (M36, `docs/design/devtools.md`, "The router is
//! the browser's"): the panel's requests answered by the browser when
//! they are about its own records, passed to the page inspected when
//! they are about the page; the page's answers passed back only to the
//! requests it was given, and its events; and the panel told when the
//! page it inspects navigates.
//!
//! Each end's text is the other's to read but neither's to trust: the
//! page is the web's, and the panel shows what the page chose.

use std::collections::{HashSet, VecDeque};

use devtools::{Json, Request};
use engine::{ConsoleEntry, ConsoleKind, ConsoleLevel, ConsoleSource, Url, View};

/// The most requests waiting on the page's answers.
pub(crate) const MAX_FORWARDED: usize = 1024;

#[derive(Default)]
pub(crate) struct Router {
    /// The requests passed to the page, by their numbers.
    forwarded: HashSet<u64>,
    /// The page's URL as the panel was last told it.
    told: Option<String>,
    /// The page's console since it was shown (M36.1), the oldest dropped
    /// past `MAX_CONSOLE_ENTRIES`: kept while the panel is closed, so
    /// that it shows what was written before it opened.
    console: VecDeque<ConsoleEntry>,
}

impl Router {
    /// The panel opened again: the requests of the one before have no
    /// answers to come.
    pub(crate) fn opened(&mut self) {
        self.forwarded.clear();
    }

    /// Notes the page's URL, `url`: a page navigated to has a console of
    /// its own, and the panel, if `panel` is open, is told of it.
    pub(crate) fn watch(&mut self, url: Option<&Url>, mut panel: Option<&mut impl End>) {
        let url = url.map(|url| url.as_str().to_owned());
        if url == self.told {
            return;
        }
        self.console.clear();
        // The page's requests were the old page's: they have no answers
        // to come.
        for id in self.forwarded.drain() {
            if let Some(panel) = panel.as_mut() {
                panel.devtools_send(devtools::error(id, "the page navigated"));
            }
        }
        if let Some(panel) = panel {
            panel.devtools_send(devtools::event(
                "page.navigated",
                Json::object([("url", Json::from(url.clone()))]),
            ));
        }
        self.told = url;
    }

    /// Keeps the page's console `entries`, and tells the panel, if it is
    /// open, of each.
    pub(crate) fn log(&mut self, entries: Vec<ConsoleEntry>, mut panel: Option<&mut impl End>) {
        for entry in entries {
            if let Some(panel) = panel.as_mut() {
                panel.devtools_send(devtools::event("console.entry", entry_json(&entry)));
            }
            if self.console.len() == engine::MAX_CONSOLE_ENTRIES {
                self.console.pop_front();
            }
            self.console.push_back(entry);
        }
    }

    /// Passes what came since the last call between `panel` and `page`.
    pub(crate) fn route(&mut self, panel: &mut impl End, page: &mut impl End) {
        self.watch(page.url(), Some(&mut *panel));
        for message in panel.take_devtools() {
            let Ok(request) = Request::parse(&message) else {
                continue;
            };
            if let Some(answer) = self.answer(&request) {
                panel.devtools_send(answer);
            } else if self.forwarded.len() >= MAX_FORWARDED {
                panel.devtools_send(devtools::error(request.id, "too many requests"));
            } else {
                self.forwarded.insert(request.id);
                page.inspect(message);
            }
        }
        for message in page.take_inspected() {
            let Ok(value) = Json::parse(&message) else {
                continue;
            };
            let passed = match value.get("id").and_then(Json::as_u64) {
                Some(id) => self.forwarded.remove(&id),
                None => value.get("event").and_then(Json::as_str).is_some(),
            };
            if passed {
                panel.devtools_send(message);
            }
        }
    }

    /// The browser's answer to `request`, when it is about the browser's
    /// own records: `None` for one about the page, which the page answers.
    fn answer(&mut self, request: &Request) -> Option<String> {
        let id = request.id;
        Some(match request.method.as_str() {
            "browser.version" => devtools::result(
                id,
                Json::object([
                    ("name", Json::from("Silver Browser")),
                    ("version", Json::from(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
            // The console's history, oldest first, and its clearing.
            "console.history" => devtools::result(
                id,
                Json::Array(self.console.iter().map(entry_json).collect()),
            ),
            "console.clear" => {
                self.console.clear();
                devtools::result(id, Json::Null)
            }
            method if request.domain() == "browser" => {
                devtools::error(id, &format!("no method {method}"))
            }
            _ => return None,
        })
    }
}

/// A console entry as the panel reads it (docs/design/devtools.md, "The
/// console"): its previews, the page's JSON, parsed again here, and
/// left out when they are not JSON.
pub(crate) fn entry_json(entry: &ConsoleEntry) -> Json {
    let level = match entry.level {
        ConsoleLevel::Log => "log",
        ConsoleLevel::Info => "info",
        ConsoleLevel::Warn => "warn",
        ConsoleLevel::Error => "error",
        ConsoleLevel::Debug => "debug",
    };
    let kind = match entry.kind {
        ConsoleKind::Log => "log",
        ConsoleKind::Dir => "dir",
        ConsoleKind::Table => "table",
        ConsoleKind::Trace => "trace",
        ConsoleKind::Group => "group",
        ConsoleKind::GroupCollapsed => "groupCollapsed",
        ConsoleKind::GroupEnd => "groupEnd",
        ConsoleKind::Clear => "clear",
    };
    let source = match entry.source {
        ConsoleSource::Console => "console",
        ConsoleSource::Exception => "exception",
        ConsoleSource::Network => "network",
        ConsoleSource::Policy => "policy",
        ConsoleSource::Worker => "worker",
        ConsoleSource::Engine => "engine",
    };
    let args = match Json::parse(&entry.previews) {
        Ok(args @ Json::Array(_)) => args,
        _ => Json::Array(Vec::new()),
    };
    let stack = entry
        .stack
        .iter()
        .take(engine::MAX_STACK_FRAMES)
        .map(|frame| {
            Json::object([
                ("function", Json::from(frame.function.as_str())),
                ("url", Json::from(frame.url.as_str())),
                ("line", Json::from(frame.line)),
                ("column", Json::from(frame.column)),
            ])
        })
        .collect();
    Json::object([
        ("level", Json::from(level)),
        ("kind", Json::from(kind)),
        ("source", Json::from(source)),
        ("time", Json::from(entry.time)),
        ("text", Json::from(entry.text.as_str())),
        ("url", Json::from(entry.url.as_str())),
        ("line", Json::from(entry.line)),
        ("column", Json::from(entry.column)),
        ("args", args),
        ("stack", Json::Array(stack)),
    ])
}

/// What the router reads and writes of a view: the panel's or the
/// page's.
pub(crate) trait End {
    fn url(&self) -> Option<&Url>;
    fn devtools_send(&mut self, message: String);
    fn take_devtools(&mut self) -> Vec<String>;
    fn inspect(&mut self, message: String);
    fn take_inspected(&mut self) -> Vec<String>;
}

impl<V: View> End for V {
    fn url(&self) -> Option<&Url> {
        View::url(self)
    }
    fn devtools_send(&mut self, message: String) {
        View::devtools_send(self, message)
    }
    fn take_devtools(&mut self) -> Vec<String> {
        View::take_devtools(self)
    }
    fn inspect(&mut self, message: String) {
        View::inspect(self, message)
    }
    fn take_inspected(&mut self) -> Vec<String> {
        View::take_inspected(self)
    }
}
