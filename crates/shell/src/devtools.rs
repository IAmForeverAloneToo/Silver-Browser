//! The DevTools' router (M36, `docs/design/devtools.md`, "The router is
//! the browser's"): the panel's requests answered by the browser when
//! they are about its own records, passed to the page inspected when
//! they are about the page; the page's answers passed back only to the
//! requests it was given, and its events; and the panel told when the
//! page it inspects navigates.
//!
//! Each end's text is the other's to read but neither's to trust: the
//! page is the web's, and the panel shows what the page chose.

use std::collections::HashSet;

use devtools::{Json, Request};
use engine::{Url, View};

/// The most requests waiting on the page's answers.
pub(crate) const MAX_FORWARDED: usize = 1024;

#[derive(Default)]
pub(crate) struct Router {
    /// The requests passed to the page, by their numbers.
    forwarded: HashSet<u64>,
    /// The page's URL as the panel was last told it.
    told: Option<String>,
}

impl Router {
    /// Passes what came since the last call between `panel` and `page`.
    pub(crate) fn route(&mut self, panel: &mut impl End, page: &mut impl End) {
        let url = page.url().map(|url| url.as_str().to_owned());
        if url != self.told {
            // The page's requests were the old page's: they have no
            // answers to come.
            for id in self.forwarded.drain() {
                panel.devtools_send(devtools::error(id, "the page navigated"));
            }
            panel.devtools_send(devtools::event(
                "page.navigated",
                Json::object([("url", Json::from(url.clone()))]),
            ));
            self.told = url;
        }
        for message in panel.take_devtools() {
            let Ok(request) = Request::parse(&message) else {
                continue;
            };
            match request.domain() {
                "browser" => panel.devtools_send(answer(&request)),
                _ if self.forwarded.len() >= MAX_FORWARDED => {
                    panel.devtools_send(devtools::error(request.id, "too many requests"));
                }
                _ => {
                    self.forwarded.insert(request.id);
                    page.inspect(message);
                }
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

/// The browser's answer to `request` about itself.
fn answer(request: &Request) -> String {
    match request.method.as_str() {
        "browser.version" => devtools::result(
            request.id,
            Json::object([
                ("name", Json::from("Silver Browser")),
                ("version", Json::from(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
        method => devtools::error(request.id, &format!("no method {method}")),
    }
}
