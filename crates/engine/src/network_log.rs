//! A tab's network log (M36.3, docs/design/devtools.md, "The network
//! log"): what its client did, told by the client as it did it, kept for
//! the DevTools' network panel. One client serves a tab, the engine's in
//! one process and the tab's network service's in the browser in many,
//! so the log has every fetch of the tab's: documents, frames, sheets,
//! scripts, images, fetch() and XHR, workers.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use devtools::Json;
use net::Url;
use net::observe::{Observed, Observer};

/// The most requests the log keeps, the oldest dropped.
pub const MAX_NETWORK_ENTRIES: usize = 1000;

/// The most bytes of a response's body the log keeps.
pub const MAX_BODY_PREVIEW: usize = 1 << 20;

/// What the log holds, and when it began: its times are milliseconds
/// since.
pub struct NetworkLog {
    began: Instant,
    inner: Mutex<Log>,
}

#[derive(Default)]
struct Log {
    entries: VecDeque<Entry>,
    /// Counts each change, so that a reader knows there is news.
    version: u64,
}

/// A request and what came of it.
struct Entry {
    fetch: u64,
    method: String,
    /// Its URLs, the first asked for and each a redirect went to.
    urls: Vec<String>,
    destination: &'static str,
    initiator: Option<String>,
    request_headers: Vec<(String, String)>,
    redirects: Vec<u16>,
    status: Option<u16>,
    response_headers: Vec<(String, String)>,
    cached: bool,
    /// The protocol the response came in: `http/1.1` or `h2` (M38).
    protocol: &'static str,
    start: f64,
    head: Option<f64>,
    end: Option<f64>,
    size: u64,
    body: Vec<u8>,
    /// The body is not shown: what a page's script could not read of
    /// it, an opaque response's.
    hidden: bool,
    error: Option<String>,
}

impl Default for NetworkLog {
    fn default() -> NetworkLog {
        NetworkLog {
            began: Instant::now(),
            inner: Mutex::default(),
        }
    }
}

/// A header list's fields as text, as Latin-1 decodes their bytes.
fn fields(headers: &net::HeaderList) -> Vec<(String, String)> {
    let text = |bytes: &[u8]| bytes.iter().map(|&b| char::from(b)).collect::<String>();
    headers
        .iter()
        .map(|(name, value)| (text(name), text(value)))
        .collect()
}

impl Observer for NetworkLog {
    fn observe(&self, fetch: u64, what: Observed<'_>) {
        let now = self.began.elapsed().as_secs_f64() * 1000.0;
        let mut log = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        log.version += 1;
        if let Observed::Request {
            method,
            url,
            headers,
            destination,
            initiator,
        } = &what
            && !log.entries.iter().any(|entry| entry.fetch == fetch)
        {
            if log.entries.len() == MAX_NETWORK_ENTRIES {
                log.entries.pop_front();
            }
            log.entries.push_back(Entry {
                fetch,
                method: (*method).to_owned(),
                urls: vec![url.to_string()],
                destination: destination_name(*destination),
                initiator: initiator.map(Url::to_string),
                request_headers: fields(headers),
                redirects: Vec::new(),
                status: None,
                response_headers: Vec::new(),
                cached: false,
                protocol: "",
                start: now,
                head: None,
                end: None,
                size: 0,
                body: Vec::new(),
                hidden: false,
                error: None,
            });
            return;
        }
        let Some(entry) = log
            .entries
            .iter_mut()
            .rev()
            .find(|entry| entry.fetch == fetch)
        else {
            return;
        };
        match what {
            // A redirect's hop: its URL is the request's now.
            Observed::Request {
                method,
                url,
                headers,
                ..
            } => {
                entry.method = method.to_owned();
                if entry.urls.last().map(String::as_str) != Some(url.as_str()) {
                    entry.urls.push(url.to_string());
                }
                entry.request_headers = fields(headers);
            }
            Observed::Redirect { status, .. } => entry.redirects.push(status),
            Observed::Response {
                status,
                headers,
                cached,
                protocol,
            } => {
                entry.protocol = match protocol {
                    Some(net::http::Version::Http2) => "h2",
                    Some(net::http::Version::Http11) => "http/1.1",
                    Some(net::http::Version::Http10) => "http/1.0",
                    None => "",
                };
                entry.status = Some(status);
                entry.response_headers = fields(headers);
                entry.cached = cached;
                entry.head = Some(now);
                entry.body.clear();
                entry.size = 0;
            }
            Observed::Body(bytes) => {
                entry.size += bytes.len() as u64;
                let room = MAX_BODY_PREVIEW.saturating_sub(entry.body.len());
                entry
                    .body
                    .extend_from_slice(&bytes[..bytes.len().min(room)]);
            }
            Observed::End(result) => {
                entry.end = Some(now);
                match result {
                    Ok(net::ResponseType::Opaque | net::ResponseType::OpaqueRedirect) => {
                        entry.hidden = true;
                        entry.body = Vec::new();
                    }
                    Ok(_) => {}
                    Err(error) => entry.error = Some(error.to_string()),
                }
            }
        }
    }
}

/// A destination's name, as Fetch writes it, or `document`.
fn destination_name(destination: net::Destination) -> &'static str {
    use net::Destination::*;
    match destination {
        Document => "document",
        Iframe => "iframe",
        Style => "style",
        Script => "script",
        Image => "image",
        Worker => "worker",
        Empty => "fetch",
        Report => "report",
    }
}

impl NetworkLog {
    /// How many changes the log has had: news when it is not what it was.
    pub fn version(&self) -> u64 {
        self.lock().version
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Log> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Forgets every request.
    pub fn clear(&self) {
        let mut log = self.lock();
        log.entries.clear();
        log.version += 1;
    }

    /// Every request, oldest first, as the panel lists them.
    pub fn list(&self) -> Json {
        Json::Array(self.lock().entries.iter().map(summary).collect())
    }

    /// Request `fetch` whole, as the panel shows one: its headers, its
    /// timing and its body as text when it is text.
    pub fn entry(&self, fetch: u64) -> Option<Json> {
        let log = self.lock();
        let entry = log.entries.iter().find(|entry| entry.fetch == fetch)?;
        let header_json = |fields: &[(String, String)]| {
            Json::Array(
                fields
                    .iter()
                    .map(|(n, v)| Json::Array(vec![Json::from(n.as_str()), Json::from(v.as_str())]))
                    .collect(),
            )
        };
        let mut members = match summary(entry) {
            Json::Object(members) => members,
            _ => Vec::new(),
        };
        let body = if entry.hidden {
            Json::Null
        } else {
            match std::str::from_utf8(&entry.body) {
                Ok(text) => Json::from(text),
                Err(_) => Json::from(base64(&entry.body)),
            }
        };
        let text = !entry.hidden && std::str::from_utf8(&entry.body).is_ok();
        members.extend([
            (
                "urls".to_owned(),
                Json::Array(entry.urls.iter().map(|u| Json::from(u.as_str())).collect()),
            ),
            ("initiator".to_owned(), Json::from(entry.initiator.clone())),
            (
                "requestHeaders".to_owned(),
                header_json(&entry.request_headers),
            ),
            (
                "responseHeaders".to_owned(),
                header_json(&entry.response_headers),
            ),
            (
                "redirects".to_owned(),
                Json::Array(
                    entry
                        .redirects
                        .iter()
                        .map(|&s| Json::from(u32::from(s)))
                        .collect(),
                ),
            ),
            ("body".to_owned(), body),
            ("bodyIsText".to_owned(), Json::from(text)),
            (
                "bodyCut".to_owned(),
                Json::from(entry.size > entry.body.len() as u64 && !entry.hidden),
            ),
        ]);
        Some(Json::Object(members))
    }
}

/// A request in a line of the panel's list.
fn summary(entry: &Entry) -> Json {
    let mime = entry
        .response_headers
        .iter()
        .rev()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        });
    Json::object([
        ("id", Json::from(entry.fetch)),
        ("method", Json::from(entry.method.as_str())),
        (
            "url",
            Json::from(entry.urls.last().map_or("", String::as_str)),
        ),
        ("type", Json::from(entry.destination)),
        ("status", Json::from(entry.status.map(u32::from))),
        ("mime", Json::from(mime)),
        ("cached", Json::from(entry.cached)),
        ("protocol", Json::from(entry.protocol)),
        ("size", Json::from(entry.size)),
        ("start", Json::from(entry.start)),
        ("head", Json::from(entry.head)),
        ("end", Json::from(entry.end)),
        ("hidden", Json::from(entry.hidden)),
        ("error", Json::from(entry.error.clone())),
    ])
}

/// `bytes` in base64, for a body the panel shows as an image.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, &b| n << 8 | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use net::test_server::{Reply, TestServer};

    use super::*;
    use crate::testing::*;

    fn field<'a>(value: &'a Json, name: &str) -> &'a Json {
        value
            .get(name)
            .unwrap_or_else(|| panic!("no {name} in {value:?}"))
    }

    #[test]
    fn a_tabs_requests_are_logged_with_their_heads_bodies_and_times() {
        let server = TestServer::plain(|request| match request.target.as_str() {
            "/" => Reply::ok(
                "text/html",
                b"<link rel=stylesheet href=/s.css><img src=/i.png><img src=/gone.png>\
                  <script>fetch('/data.json')</script>",
            ),
            "/s.css" => Reply::ok("text/css", b"p { color: red }"),
            "/i.png" => Reply::ok("image/png", b"not really a png"),
            "/data.json" => Reply::ok("application/json", b"{\"a\":1}"),
            _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"no"),
        });
        let mut tab = Tab::new();
        tab.engine.set_scripting(true);
        tab.engine.navigate(server.url("/"));
        let log = tab.engine.network.log().clone();
        let deadline = std::time::Instant::now() + WAIT;
        let list = loop {
            tab.engine.poll_network();
            tab.engine.run_tasks();
            let list = log.list();
            let done = list.as_array().is_some_and(|entries| {
                // The page's five, and its icon's.
                entries.len() == 6 && entries.iter().all(|e| e.get("end") != Some(&Json::Null))
            });
            if done {
                break list;
            }
            assert!(std::time::Instant::now() < deadline, "{list:?}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let entries = list.as_array().expect("a list");
        let by_url = |path: &str| {
            entries
                .iter()
                .find(|e| field(e, "url").as_str() == Some(server.url(path).as_str()))
                .unwrap_or_else(|| panic!("no {path} in {entries:?}"))
        };
        assert_eq!(field(by_url("/"), "type").as_str(), Some("document"));
        assert_eq!(field(by_url("/s.css"), "type").as_str(), Some("style"));
        assert_eq!(field(by_url("/i.png"), "type").as_str(), Some("image"));
        assert_eq!(field(by_url("/data.json"), "type").as_str(), Some("fetch"));
        assert_eq!(field(by_url("/gone.png"), "status").as_u64(), Some(404));
        assert_eq!(
            field(by_url("/favicon.ico"), "type").as_str(),
            Some("image")
        );
        assert_eq!(field(by_url("/s.css"), "mime").as_str(), Some("text/css"));
        // The protocol each came in (M38): the document streamed, the
        // sheet whole.
        assert_eq!(field(by_url("/"), "protocol").as_str(), Some("http/1.1"));
        assert_eq!(
            field(by_url("/s.css"), "protocol").as_str(),
            Some("http/1.1")
        );
        let css = field(by_url("/s.css"), "id").as_u64().expect("an id");
        let entry = log.entry(css).expect("the entry");
        assert_eq!(field(&entry, "body").as_str(), Some("p { color: red }"));
        assert_eq!(field(&entry, "size").as_u64(), Some(16));
        let sent = field(&entry, "requestHeaders").as_array().expect("headers");
        assert!(
            sent.iter()
                .any(|pair| pair.as_array().and_then(|p| p[0].as_str()) == Some("User-Agent")),
            "{sent:?}"
        );
        assert!(
            field(&entry, "start").as_f64() <= field(&entry, "end").as_f64(),
            "{entry:?}"
        );
        log.clear();
        assert_eq!(log.list(), Json::Array(Vec::new()));
    }

    #[test]
    fn the_log_keeps_its_bounds_and_hides_what_a_page_could_not_read() {
        let log = NetworkLog::default();
        let url = Url::parse("https://a.test/x").expect("a URL");
        let headers = net::HeaderList::new();
        for fetch in 0..(MAX_NETWORK_ENTRIES as u64 + 5) {
            log.observe(
                fetch,
                Observed::Request {
                    method: "GET",
                    url: &url,
                    headers: &headers,
                    destination: net::Destination::Image,
                    initiator: None,
                },
            );
        }
        let list = log.list();
        let entries = list.as_array().expect("a list");
        assert_eq!(entries.len(), MAX_NETWORK_ENTRIES);
        assert_eq!(
            field(&entries[0], "id").as_u64(),
            Some(5),
            "the oldest dropped"
        );
        // A body past the bound is kept to it, its size whole.
        let big = vec![b'x'; MAX_BODY_PREVIEW + 10];
        log.observe(5, Observed::Body(&big));
        let entry = log.entry(5).expect("an entry");
        assert_eq!(
            field(&entry, "body").as_str().map(str::len),
            Some(MAX_BODY_PREVIEW)
        );
        assert_eq!(field(&entry, "size").as_u64(), Some(big.len() as u64));
        assert_eq!(field(&entry, "bodyCut"), &Json::Bool(true));
        // An opaque response's body is not shown.
        log.observe(6, Observed::Body(b"secret"));
        log.observe(6, Observed::End(Ok(net::ResponseType::Opaque)));
        let entry = log.entry(6).expect("an entry");
        assert_eq!(field(&entry, "body"), &Json::Null);
        assert_eq!(field(&entry, "hidden"), &Json::Bool(true));
        // Every change is news.
        let version = log.version();
        log.observe(7, Observed::End(Err(&net::NetworkError::Aborted)));
        assert_eq!(log.version(), version + 1);
        assert_eq!(
            field(&log.entry(7).expect("an entry"), "error").as_str(),
            Some(net::NetworkError::Aborted.to_string().as_str())
        );
    }
}
