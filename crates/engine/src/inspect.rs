//! The DevTools' two ends in an engine (M36, `docs/design/devtools.md`):
//! the panel's page, which the browser alone shows and whose window has
//! the bridge to it; and the page inspected, which answers the panel's
//! requests about itself. The browser's router passes messages between
//! the two; neither end trusts what the other's text says.

use devtools::{Json, Request};

use crate::loader::{Incoming, LoadState};
use crate::{Engine, Handling, Navigation, Url};

/// The most answers and events a page inspected may have waiting for the
/// browser: it takes them each turn.
pub const MAX_INSPECTED_MESSAGES: usize = 1024;

impl Engine {
    /// Shows the DevTools panel's page (M36): the browser's own document,
    /// its scripts on whatever the browser's setting, its window given
    /// the global `devtools`. No navigation reaches it: this is its only
    /// way in.
    pub fn show_devtools_panel(&mut self) {
        self.cancel_navigation();
        let url = Url::parse(devtools::PANEL_URL).expect("the panel's URL parses");
        let id = self.new_page_id();
        self.panel = true;
        self.scripting = true;
        self.navigation = Some(Navigation {
            url: url.clone(),
            incoming: Incoming::Fetching(id),
            handling: Handling::Replace,
            started: self.time_origin(),
            creator: None,
            sandbox: self.creation_sandbox(),
            referrer: String::new(),
        });
        self.load_state = LoadState::Loading;
        let mut headers = net::HeaderList::new();
        headers.append(b"Content-Type", b"text/html;charset=utf-8");
        headers.append(b"Content-Security-Policy", devtools::PANEL_CSP.as_bytes());
        let head = net::Response {
            url_list: vec![url],
            status: 200,
            status_message: b"OK".to_vec(),
            headers,
            body: Vec::new(),
            mime_type: net::MimeType::parse("text/html;charset=utf-8"),
            address_space: None,
            response_type: net::ResponseType::Basic,
            cors_exposed: Vec::new(),
        };
        self.document_head(id, head.clone());
        self.document_body(id, devtools::frontend::page().as_bytes());
        self.document_arrived(id, Ok(head));
    }

    /// An engine showing the DevTools panel's page (M36), on this one's
    /// fonts, clock, waker, preferences and scale, drawing in software.
    pub fn devtools_panel(&self) -> Engine {
        let mut panel = Engine::with_fonts(self.layout_fonts.clone());
        panel.set_clock(self.clock.clone());
        if let Some(waker) = &self.waker {
            panel.use_waker(waker.clone());
        }
        panel.set_preferences(self.preferences);
        panel.set_scale_factor(self.scale);
        panel.show_devtools_panel();
        panel
    }

    /// Whether the engine shows the DevTools panel's page.
    pub fn is_devtools_panel(&self) -> bool {
        self.panel
    }

    /// The panel's page hears `message` from the browser, as a task.
    pub fn devtools_send(&mut self, message: String) {
        if !self.panel || !self.shown.page.is_panel() {
            return;
        }
        let now = self.now();
        self.shown.scripts.post(now, js::Task::DevTools(message));
        self.wake();
    }

    /// What the panel's page sent the browser since this was last asked.
    pub fn take_devtools(&mut self) -> Vec<String> {
        if !self.shown.page.is_panel() {
            return Vec::new();
        }
        self.shown.scripts.take_devtools()
    }

    /// The panel's request `message` about the page shown: its answer is
    /// in [`Engine::take_inspected`]. What is not a request is dropped.
    pub fn inspect(&mut self, message: &str) {
        let Ok(request) = Request::parse(message) else {
            return;
        };
        let answer = match self.answer(&request) {
            Ok(result) => devtools::result(request.id, result),
            Err(why) => devtools::error(request.id, &why),
        };
        self.tell_panel(answer);
    }

    /// The answers and events for the panel since this was last asked.
    pub fn take_inspected(&mut self) -> Vec<String> {
        std::mem::take(&mut self.inspected)
    }

    /// Queues `message` for the panel, unless too many wait.
    fn tell_panel(&mut self, message: String) {
        if self.inspected.len() < MAX_INSPECTED_MESSAGES {
            self.inspected.push(message);
        }
    }

    /// The result of `request`, or why there is none.
    fn answer(&mut self, request: &Request) -> Result<Json, String> {
        match request.method.as_str() {
            // The page's URL and title.
            "page.info" => Ok(Json::object([
                ("url", Json::from(self.url().map(Url::to_string))),
                ("title", Json::from(self.title())),
            ])),
            method => Err(format!("no method {method}")),
        }
    }
}

#[cfg(test)]
#[path = "inspect_tests.rs"]
mod tests;
