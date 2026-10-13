//! The web-platform-tests subset (M17, T1.1; its grid tests M18's): the
//! tests under `tests/wpt/`, which `examples/wpt.rs` chose and copied
//! from upstream, run as `wptrunner` would run them, against
//! `expectations.txt`.
//!
//! The directory is served from the `net` crate's test server on
//! 127.0.0.1, so a test and its stylesheets load through the engine's
//! own loader by their URLs. Each test is loaded in an engine of its
//! own, with the bundled font, at 800 by 600, on as many threads as
//! there are cores, its scripts run as a browser under `wptrunner` runs
//! them (M24). A test's animations and transitions are drawn at
//! their first frame, the document's timeline at zero (M22), as the
//! reference browser's capture of a test that does not wait shows them:
//! paused, or at a negative delay, they show the moment they ask for.
//! A page's clock is a hand's, moved by the runner: a test that keeps
//! `reftest-wait` on its root element (a crashtest, `test-wait`) is sent
//! `TestRendered` after its first frame and has its timers, animation
//! frames and rendering steps run, the clock moved on to each, until it
//! takes the class off, and two frames more, as `wptrunner` waits for
//! it (M26.11); one that never does fails after ten seconds of its time.
//!
//! - a reftest is rendered, and so is each of its references, and it
//!   passes when it matches one of the references it must match and none
//!   it must not, within its `fuzzy` metadata, by `wptrunner`'s
//!   comparison;
//! - a check-layout test is checked natively, as `check-layout-th.js`
//!   checks it: each element its selectors find, the element's parent,
//!   and the element's descendants, by their `data-*` attributes, within
//!   a pixel, the CSSOM View values computed from the engine's fragments;
//! - a crashtest must load and render without panicking;
//! - a testharness.js test (M27) runs the real `testharness.js` under
//!   the runner's own `testharnessreport.js`, which writes each
//!   subtest's result into the page, as `wptrunner`'s report hands them
//!   to its driver; an `.any.js` or `.window.js` test is served in the
//!   page wptserve makes of it for a window, a file's `.headers` are
//!   sent with it, and an `.asis` file is sent as the whole response,
//!   as wptserve sends them. The page's clock waits for the network while a
//!   fetch is in flight and moves on to its timers otherwise; the
//!   harness keeps no timeout of its own, and a test still running
//!   after ten seconds of its time (a minute for a long one), or with
//!   nothing left to wait for, times out. Pages of other kinds that load
//!   the harness have the runner stand in for it, as before.
//!
//! The acceptance layouts of M17 to M20 (`tests/layouts/`, M17.10,
//! M18.7, M19's and M20.5) are check-layout pages too, their expectations
//! captured from the reference browser by `examples/layouts.rs`, and run
//! here the same way, every element of the body checked, each at 800 by
//! 600 or the size its `reference-size` asks for.
//!
//! `expectations.txt` names each test expected to fail, with the reason,
//! and each testharness subtest expected to fail as its test's path, a
//! tab, the subtest's name, a tab and the reason; a testharness test
//! whose harness does not finish is named as a whole. A test or subtest
//! that fails without being named fails the run, and so does one named
//! that passes, so that the file stays the true list of what does not
//! work and why. Every run writes `target/wpt/report.html`: each test
//! with its result, and a failing reftest's render, its reference's and
//! their difference side by side; and `target/wpt/testharness.txt`,
//! each testharness test and subtest that failed, for keeping the
//! expectations.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dom::{Document, NodeId};
use engine::differential::run_script;
use engine::{Engine, FrameTime, LoadState};
use layout::{Fragment, FragmentKind};
use net::test_server::{Reply, TestServer};
use style::computed::{Display, LEFT, Position, RIGHT, TOP};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// How long a page may take to load.
const LOAD: Duration = Duration::from_secs(30);

/// How long, on its own clock, a test may wait before it is drawn.
const WAIT: Duration = Duration::from_secs(10);

/// A frame's time at sixty frames a second.
const FRAME: Duration = Duration::from_micros(16_667);

/// The vendored directory.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt")
}

/// What the report and the images go in.
fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt")
}

/// One test of the manifest.
#[derive(Debug, Clone)]
enum Test {
    Reftest {
        path: String,
        /// Each reference, `true` for one the test must match.
        refs: Vec<(bool, String)>,
        /// The fuzziness by reference, an empty one applying to all:
        /// the allowed largest channel difference and number of
        /// differing pixels, each a range.
        fuzzy: Vec<(String, Fuzzy)>,
    },
    CheckLayout {
        path: String,
        selectors: Vec<String>,
        /// The viewport's size: 800 by 600, or the size an acceptance
        /// layout asks for.
        size: (u32, u32),
    },
    Crashtest {
        path: String,
    },
    Testharness {
        path: String,
        /// Whether its `timeout` is long.
        long: bool,
    },
}

type Fuzzy = ((u32, u32), (u32, u32));

impl Test {
    fn path(&self) -> &str {
        match self {
            Test::Reftest { path, .. }
            | Test::CheckLayout { path, .. }
            | Test::Crashtest { path }
            | Test::Testharness { path, .. } => path,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Test::Reftest { .. } => "reftest",
            Test::CheckLayout { .. } => "check-layout",
            Test::Crashtest { .. } => "crashtest",
            Test::Testharness { .. } => "testharness",
        }
    }
}

/// The manifest `examples/wpt.rs` writes.
fn manifest() -> Vec<Test> {
    let text = fs::read_to_string(root().join("MANIFEST")).expect("the manifest is there");
    let mut tests = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let mut fields = line.split('\t');
        let (Some(kind), Some(path)) = (fields.next(), fields.next()) else {
            panic!("a manifest line without a kind and a path: {line}");
        };
        let path = path.to_string();
        tests.push(match kind {
            "reftest" => {
                let mut refs = Vec::new();
                let mut fuzzy = Vec::new();
                for field in fields {
                    if let Some(reference) = field.strip_prefix("==") {
                        refs.push((true, reference.to_string()));
                    } else if let Some(reference) = field.strip_prefix("!=") {
                        refs.push((false, reference.to_string()));
                    } else if let Some(spec) = field.strip_prefix("fuzzy=") {
                        fuzzy.push(parse_fuzzy(spec).unwrap_or_else(|| {
                            panic!("a malformed fuzziness in the manifest: {line}")
                        }));
                    } else {
                        panic!("an unknown reftest field in the manifest: {line}");
                    }
                }
                Test::Reftest { path, refs, fuzzy }
            }
            "checklayout" => Test::CheckLayout {
                path,
                selectors: fields.map(str::to_string).collect(),
                size: (WIDTH, HEIGHT),
            },
            "crashtest" => Test::Crashtest { path },
            "testharness" => Test::Testharness {
                path,
                long: fields.any(|field| field == "timeout=long"),
            },
            _ => panic!("an unknown kind in the manifest: {line}"),
        });
    }
    tests
}

/// `ref:a-b;c-d`.
fn parse_fuzzy(spec: &str) -> Option<(String, Fuzzy)> {
    let (reference, ranges) = spec.rsplit_once(':')?;
    let (channel, pixels) = ranges.split_once(';')?;
    let range = |r: &str| -> Option<(u32, u32)> {
        let (low, high) = r.split_once('-')?;
        Some((low.parse().ok()?, high.parse().ok()?))
    };
    Some((reference.to_string(), (range(channel)?, range(pixels)?)))
}

/// `expectations.txt`: each test expected to fail, and why; a subtest
/// by its test's path and its name, a tab between.
fn expectations() -> BTreeMap<String, String> {
    let text = fs::read_to_string(root().join("expectations.txt")).unwrap_or_default();
    let mut out = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (path, reason) = match line.split_once('\t') {
            Some((path, rest)) => {
                let (name, reason) = rest.split_once('\t').unwrap_or((rest, ""));
                (format!("{path}\t{}", unescape(name)), reason)
            }
            None => {
                let (path, reason) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
                (path.to_string(), reason)
            }
        };
        let path = path.as_str();
        let reason = reason.trim();
        assert!(!reason.is_empty(), "expectations.txt: {path} has no reason");
        assert!(
            out.insert(path.to_string(), reason.to_string()).is_none(),
            "expectations.txt: {path} is named twice"
        );
    }
    out
}

/// A subtest's name as `expectations.txt` writes it, in ASCII: each
/// `\u{HEX}` the character it names.
fn unescape(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(at) = rest.find("\\u{") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 3..];
        let named = after.split_once('}').and_then(|(hex, tail)| {
            let c = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
            Some((c, tail))
        });
        match named {
            Some((c, tail)) => {
                out.push(c);
                rest = tail;
            }
            None => {
                out.push_str("\\u{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The runner's host, as wptserve's `{{host}}` (M33): a name under
/// `localhost`, which the engine resolves to this machine, so that its
/// subdomains are hosts of the same site and `ALT_HOST`'s of another.
const HOST: &str = "web-platform.localhost";
const ALT_HOST: &str = "not-web-platform.localhost";

/// The client's configuration with the runner's hosts and their
/// subdomains at 127.0.0.1, where its one server listens: a name under
/// `localhost` would try `::1` first, which on Windows takes a while to
/// be refused at each new connection.
fn runner_hosts() -> net::transport::Config {
    let mut config = net::transport::Config::default();
    let here = std::net::IpAddr::from([127, 0, 0, 1]);
    for site in [HOST, ALT_HOST] {
        for sub in ["", "www.", "www1.", "www2."] {
            config = config.resolve(&format!("{sub}{site}"), here);
        }
    }
    config
}

/// The URL the runner loads `path` at: on [`HOST`], the server's port.
fn test_url(server: &TestServer, path: &str) -> net::Url {
    let port = server.url("/").port().expect("the server's port");
    net::Url::parse(&format!("http://{HOST}:{port}/{path}")).expect("a test's URL")
}

/// `text`, a `.sub` template, filled as wptserve fills one (M33): the
/// hosts and their subdomains, the ports (the server's for `http`, and
/// for what the runner does not serve, `https` and the WebSockets, a
/// port the engine never connects to), the request's location, query
/// (a parameter it does not have empty) and fields, a new UUID each time
/// one is asked for, and the variables a template sets (`{{$id:...}}`)
/// and reads (`{{$id}}`). What it does not know stays as written.
fn substitute(text: &str, request: &net::test_server::ServerRequest, port: u16) -> String {
    static UUIDS: AtomicU64 = AtomicU64::new(1);
    let mut variables: HashMap<String, String> = HashMap::new();
    let host_of = |site: &str, sub: &str| match sub {
        "" => site.to_owned(),
        sub => format!("{sub}.{site}"),
    };
    let header = request.header("Host").unwrap_or(HOST);
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target.as_str(), ""));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("{{") {
        out.push_str(&rest[..at]);
        let Some(end) = rest[at..].find("}}") else {
            rest = &rest[at..];
            break;
        };
        let mut inner = rest[at + 2..at + end].trim();
        // `$name:expression` sets a variable to what the expression
        // gives; `$name` alone reads it.
        let mut set = None;
        if let Some(variable) = inner.strip_prefix('$') {
            match variable.split_once(':') {
                Some((name, expression)) => {
                    set = Some(name.to_owned());
                    inner = expression.trim();
                }
                None => {
                    let value = variables.get(variable).cloned().unwrap_or_default();
                    out.push_str(&value);
                    rest = &rest[at + end + 2..];
                    continue;
                }
            }
        }
        let key = |name: &str| {
            inner
                .strip_prefix(name)
                .and_then(|r| r.strip_prefix('['))
                .and_then(|r| r.strip_suffix(']'))
        };
        let filled = if inner == "host" {
            Some(HOST.to_owned())
        } else if let Some(sub) = key("domains") {
            Some(host_of(HOST, sub))
        } else if let Some(pair) = key("hosts") {
            pair.split_once("][").map(|(site, sub)| match site {
                "alt" => host_of(ALT_HOST, sub),
                _ => host_of(HOST, sub),
            })
        } else if let Some(pair) = key("ports") {
            pair.split_once("][").map(|(scheme, _)| match scheme {
                "http" => port.to_string(),
                _ => "1".to_owned(),
            })
        } else if let Some(part) = key("location") {
            let hostname = header.rsplit_once(':').map_or(header, |(name, _)| name);
            match part {
                "host" => Some(header.to_owned()),
                "hostname" => Some(hostname.to_owned()),
                "port" => Some(port.to_string()),
                "scheme" => Some("http".to_owned()),
                "server" => Some(format!("http://{header}")),
                "path" | "pathname" => Some(path.to_owned()),
                "query" => Some(if query.is_empty() {
                    String::new()
                } else {
                    format!("?{query}")
                }),
                _ => None,
            }
        } else if let Some(name) = key("GET") {
            let found = query.split('&').find_map(|pair| {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                (form_decoded(key) == name).then(|| form_decoded(value))
            });
            Some(found.unwrap_or_default())
        } else if let Some(name) = key("headers") {
            request.header(name).map(str::to_owned)
        } else if inner == "uuid()" {
            let n = UUIDS.fetch_add(1, Ordering::Relaxed);
            Some(format!("00000000-0000-4000-8000-{n:012x}"))
        } else {
            None
        };
        match filled {
            Some(value) => {
                if let Some(name) = set {
                    variables.insert(name, value.clone());
                }
                out.push_str(&value);
            }
            None => out.push_str(&rest[at..at + end + 2]),
        }
        rest = &rest[at + end + 2..];
    }
    out.push_str(rest);
    out
}

/// `text` of a query, `application/x-www-form-urlencoded`: `+` a space,
/// each `%XX` its byte, read as UTF-8.
fn form_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The content type a vendored file is served with.
fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("xht" | "xhtml") => "application/xhtml+xml",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("png" | "apng") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("avif") => "image/avif",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        _ => "application/octet-stream",
    }
}

/// The runner's `testharnessreport.js` (M27): it turns the harness's own
/// timeout and log off, as `wptrunner`'s does, and writes the results
/// into the page, a line each, for [`harness_results`] to read: the
/// harness's status and message, then each subtest's status, name and
/// message, tabs between, tabs and line breaks in them made spaces.
const REPORT: &str = r#"setup({explicit_timeout: true, output: false});
add_completion_callback(function (tests, status) {
  function clean(text) {
    return String(text === undefined || text === null ? "" : text).replace(/[\t\n\r]/g, " ");
  }
  var lines = [status.status + "\t" + clean(status.message)];
  for (var i = 0; i < tests.length; i++) {
    lines.push(tests[i].status + "\t" + clean(tests[i].name) + "\t" + clean(tests[i].message));
  }
  var out = document.createElement("pre");
  out.id = "wpt-results";
  out.textContent = lines.join("\n");
  (document.body || document.documentElement).appendChild(out);
});
"#;

/// The page wptserve makes of the `.any.js` or `.window.js` test whose
/// source is `source`, served beside it as `name`, for a window: its
/// title and the scripts its `// META:` lines name, `self.GLOBAL` for an
/// `.any.js` one, the harness, then the test.
fn wrapper(name: &str, source: &str, any: bool) -> String {
    let mut head = String::new();
    for line in source.lines().map(str::trim) {
        let Some(meta) = line
            .strip_prefix("//")
            .and_then(|rest| rest.trim_start().strip_prefix("META:"))
        else {
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            break;
        };
        match meta.trim().split_once('=') {
            Some(("title", title)) => {
                let _ = writeln!(head, "<title>{}</title>", escape(title.trim()));
            }
            Some(("script", src)) => {
                let _ = writeln!(head, "<script src=\"{}\"></script>", escape(src.trim()));
            }
            _ => {}
        }
    }
    let global = if any {
        "<script>\nself.GLOBAL = {\n  isWindow: function() { return true; },\n  \
         isWorker: function() { return false; },\n  isShadowRealm: function() { return false; },\n};\n\
         </script>\n"
    } else {
        ""
    };
    format!(
        "<!doctype html>\n<meta charset=utf-8>\n{global}\
         <script src=\"/resources/testharness.js\"></script>\n\
         <script src=\"/resources/testharnessreport.js\"></script>\n{head}\
         <div id=log></div>\n<script src=\"{}\"></script>\n",
        escape(name)
    )
}

/// A test's text, its bytes that are no UTF-8 replaced: a test may hold
/// some on purpose (M31's encodings tests).
fn read_lossy(path: &Path) -> std::io::Result<String> {
    fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The page wptserve makes of a test for a dedicated worker (M31): the
/// harness, which takes the tests the worker `script` runs and reports.
fn worker_page(script: &str, source: &str) -> String {
    let mut head = String::new();
    for line in source.lines().map(str::trim) {
        let Some(meta) = line
            .strip_prefix("//")
            .and_then(|rest| rest.trim_start().strip_prefix("META:"))
        else {
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            break;
        };
        if let Some(("title", title)) = meta.trim().split_once('=') {
            let _ = writeln!(head, "<title>{}</title>", escape(title.trim()));
        }
    }
    format!(
        "<!doctype html>\n<meta charset=utf-8>\n{head}\
         <script src=\"/resources/testharness.js\"></script>\n\
         <script src=\"/resources/testharnessreport.js\"></script>\n\
         <div id=log></div>\n\
         <script>\nfetch_tests_from_worker(new Worker(\"{}\"));\n</script>\n",
        escape(script)
    )
}

/// The script wptserve makes of an `.any.js` test for a dedicated worker
/// (M31): `self.GLOBAL`, the harness and the scripts its `// META:`
/// lines name, imported, then the test, then `done()`.
fn worker_script(name: &str, source: &str) -> String {
    let mut imports = String::new();
    for line in source.lines().map(str::trim) {
        let Some(meta) = line
            .strip_prefix("//")
            .and_then(|rest| rest.trim_start().strip_prefix("META:"))
        else {
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            break;
        };
        if let Some(("script", src)) = meta.trim().split_once('=') {
            let _ = writeln!(imports, "importScripts({:?});", src.trim());
        }
    }
    format!(
        "self.GLOBAL = {{\n  isWindow: function() {{ return false; }},\n  \
         isWorker: function() {{ return true; }},\n  \
         isShadowRealm: function() {{ return false; }},\n}};\n\
         importScripts(\"/resources/testharness.js\");\n{imports}\
         importScripts({name:?});\ndone();\n"
    )
}

/// The server of `root`, the vendored directory or the layouts'. With
/// `harness`, the testharness tests' (M27): `testharnessreport.js` is
/// the runner's, and an `.any.html` or `.window.html` page is made of
/// its test; without, the harness's scripts are not found, the runner
/// standing in for them. A file's `.headers` beside it are sent with it.
fn serve(root: PathBuf, harness: bool) -> TestServer {
    TestServer::plain(move |request| {
        let path = request.target.split(['?', '#']).next().unwrap_or("");
        let file = path.trim_start_matches('/');
        let safe = !file.split('/').any(|part| part == ".." || part.is_empty());
        let not_found = || {
            Reply::Send(
                b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\n\r\nnot found"
                    .to_vec(),
            )
        };
        if !safe {
            return not_found();
        }
        match (harness, file) {
            (true, "resources/testharnessreport.js") => {
                return Reply::ok("text/javascript", REPORT.as_bytes());
            }
            (false, "resources/testharness.js" | "resources/testharnessreport.js") => {
                return not_found();
            }
            _ => {}
        }
        if harness && !root.join(file).is_file() {
            // A test for a dedicated worker (M31): its page, and an
            // `.any.js` one's script.
            let worker = file
                .strip_suffix(".any.worker.html")
                .map(|stem| (stem, ".any.js", ".any.worker.js"))
                .or_else(|| {
                    file.strip_suffix(".worker.html")
                        .map(|stem| (stem, ".worker.js", ".worker.js"))
                });
            // The test's `.headers` go with the pages and scripts made of
            // it, as wptserve sends them (M33: a policy's tests).
            let headers_of = |test: &str| fs::read_to_string(root.join(format!("{test}.headers")));
            if let Some((stem, test, script)) = worker
                && let Ok(source) = read_lossy(&root.join(format!("{stem}{test}")))
            {
                let name = format!("{}{script}", stem.rsplit('/').next().unwrap_or(stem));
                let page = worker_page(&name, &source);
                let headers = headers_of(&format!("{stem}{test}")).ok();
                return reply("text/html; charset=utf-8", page.into_bytes(), headers);
            }
            if let Some(stem) = file.strip_suffix(".any.worker.js")
                && let Ok(source) = read_lossy(&root.join(format!("{stem}.any.js")))
            {
                let name = format!("{}.any.js", stem.rsplit('/').next().unwrap_or(stem));
                let script = worker_script(&name, &source);
                let headers = headers_of(&format!("{stem}.any.js")).ok();
                return reply("text/javascript", script.into_bytes(), headers);
            }
            for (page, script, any) in [
                (".any.html", ".any.js", true),
                (".window.html", ".window.js", false),
            ] {
                if let Some(stem) = file.strip_suffix(page)
                    && let Ok(source) = read_lossy(&root.join(format!("{stem}{script}")))
                {
                    let name = format!("{}{script}", stem.rsplit('/').next().unwrap_or(stem));
                    let page = wrapper(&name, &source, any);
                    let headers = headers_of(&format!("{stem}{script}")).ok();
                    return reply("text/html; charset=utf-8", page.into_bytes(), headers);
                }
            }
        }
        let Ok(mut body) = fs::read(root.join(file)) else {
            return not_found();
        };
        // The request's port, as its Host says, for a template (M33).
        let port = request
            .header("Host")
            .and_then(|host| host.rsplit_once(':'))
            .and_then(|(_, port)| port.parse().ok())
            .unwrap_or(80);
        if file.contains(".sub.") {
            body = substitute(&String::from_utf8_lossy(&body), request, port).into_bytes();
        }
        if file.ends_with(".asis") {
            return Reply::SendAndClose(body);
        }
        let headers = fs::read_to_string(root.join(format!("{file}.headers"))).or_else(|_| {
            fs::read_to_string(root.join(format!("{file}.sub.headers")))
                .map(|text| substitute(&text, request, port))
        });
        reply(content_type(file), body, headers.ok())
    })
}

/// A response of `body` as `content_type`, with the fields of a
/// `.headers` file's text, each in place of one of its name before.
fn reply(content_type: &str, body: Vec<u8>, headers: Option<String>) -> Reply {
    let mut fields = vec![("Content-Type".to_string(), content_type.to_string())];
    for (name, value) in headers
        .iter()
        .flat_map(|text| text.lines())
        .filter_map(|line| line.split_once(':'))
    {
        let (name, value) = (name.trim(), value.trim());
        fields.retain(|(old, _)| !old.eq_ignore_ascii_case(name));
        fields.push((name.to_string(), value.to_string()));
    }
    let mut bytes = b"HTTP/1.1 200 OK\r\n".to_vec();
    for (name, value) in &fields {
        bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    bytes.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    bytes.extend_from_slice(&body);
    Reply::Send(bytes)
}

/// A page's clock, in nanoseconds since it was made: moved only by the
/// runner.
#[derive(Clone, Default)]
struct HandClock(Arc<AtomicU64>);

impl HandClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.0.load(Ordering::Relaxed))
    }

    fn set(&self, at: Duration) {
        let nanos = u64::try_from(at.as_nanos()).unwrap_or(u64::MAX);
        self.0.fetch_max(nanos, Ordering::Relaxed);
    }
}

/// A page loaded: its engine, what its waker sends, and its clock.
struct Loaded {
    engine: Engine,
    woken: Receiver<()>,
    clock: HandClock,
}

/// An engine with the bundled font and a hand's clock that has loaded
/// `path` from `server`; or why it could not.
fn load_waking(server: &TestServer, path: &str) -> Result<Loaded, String> {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.set_scripting(true);
    engine.set_network(net::Client::new(runner_hosts()));
    let clock = HandClock::default();
    let hand = clock.clone();
    engine.set_clock(Arc::new(move || hand.now()));
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(test_url(server, path));
    let deadline = Instant::now() + LOAD;
    loop {
        engine.poll_network();
        match engine.load_state() {
            LoadState::Loaded => {
                return Ok(Loaded {
                    engine,
                    woken,
                    clock,
                });
            }
            LoadState::Failed(error) => return Err(format!("{path} failed to load: {error}")),
            LoadState::Loading => {}
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| format!("{path} took more than {LOAD:?} to load"))?;
        let _ = woken.recv_timeout(left);
    }
}

/// An engine with the bundled font that has loaded `path` from `server`,
/// or why it could not.
fn load(server: &TestServer, path: &str) -> Result<Engine, String> {
    load_waking(server, path).map(|loaded| loaded.engine)
}

/// Where the page's clock goes from `now`: to the next frame, a frame's
/// time on when one is wanted now (`frame_wanted`), else when its
/// animations next change (`next_frame`), or to the next task due,
/// whichever comes first, and a millisecond on at least; `None` when
/// neither is.
fn next_time(
    now: Duration,
    frame_wanted: bool,
    next_frame: Option<Duration>,
    next_task: Option<Duration>,
) -> Option<Duration> {
    let frame = if frame_wanted {
        Some(now + FRAME)
    } else {
        next_frame
    };
    [frame, next_task]
        .into_iter()
        .flatten()
        .min()
        .map(|at| at.max(now + Duration::from_millis(1)))
}

#[test]
fn a_harness_is_held_to_the_expectations_subtest_by_subtest() {
    let harness = |status, tests: &[(&str, u8)]| Harness {
        status,
        message: "why".to_string(),
        tests: tests
            .iter()
            .map(|(name, status)| (name.to_string(), *status, "said".to_string()))
            .collect(),
    };
    let expected = |keys: &[&str]| -> BTreeMap<String, String> {
        keys.iter()
            .map(|key| (key.to_string(), "a reason".to_string()))
            .collect()
    };
    let ran = harness(0, &[("a", 0), ("b", 1), ("c", 1)]);
    // Each failing subtest named: nothing unexpected.
    assert!(harness_unexpected("t.html", &ran, &expected(&["t.html\tb", "t.html\tc"])).is_empty());
    // One not named, one named that passes, one named that did not run,
    // and the whole test named though its harness finished.
    let unexpected = harness_unexpected(
        "t.html",
        &ran,
        &expected(&["t.html", "t.html\ta", "t.html\tb", "t.html\tgone"]),
    );
    assert_eq!(unexpected.len(), 4, "{unexpected:?}");
    assert!(unexpected[0].starts_with("PASS t.html: its harness finished"));
    assert!(unexpected[1].starts_with("PASS t.html\ta:"));
    assert!(unexpected[2].starts_with("FAIL t.html\tc: said"));
    assert!(unexpected[3].starts_with("GONE t.html\tgone:"));
    // A harness that did not finish: the test named whole, or it is
    // unexpected.
    let error = harness(1, &[("a", 0)]);
    assert!(harness_unexpected("t.html", &error, &expected(&["t.html"])).is_empty());
    assert_eq!(
        harness_unexpected("t.html", &error, &expected(&[])),
        ["ERROR t.html: why"]
    );
}

#[test]
fn an_expectation_names_a_subtest_in_ascii() {
    assert_eq!(
        unescape("sort: \\u{FFFD}=x&\\u{1F308}"),
        "sort: \u{FFFD}=x&\u{1F308}"
    );
    // What names no character stays as it is.
    assert_eq!(unescape("a \\u{zz} b \\u{D800}"), "a \\u{zz} b \\u{D800}");
}

#[test]
fn a_template_is_filled_as_wptserve_fills_it() {
    let request = net::test_server::ServerRequest {
        method: "GET".to_string(),
        target: "/dir/t.sub.html?logs=%5B%22a+b%22%5D&x=1".to_string(),
        headers: vec![
            ("Host".to_string(), format!("{HOST}:8000")),
            ("X-Y".to_string(), "z".to_string()),
        ],
        body: Vec::new(),
        connection: 0,
        index: 0,
        sni: None,
    };
    let filled = substitute(
        "{{host}} {{domains[www1]}} {{hosts[alt][]}} {{hosts[][www2]}} \
         {{ports[http][0]}} {{ports[https][0]}} {{location[server]}} \
         {{location[path]}}{{location[query]}} {{GET[logs]}} [{{GET[none]}}] \
         {{headers[x-y]}} {{$id:uuid()}} {{$id}} {{unknown}}",
        &request,
        8000,
    );
    let words: Vec<&str> = filled.split(' ').collect();
    assert_eq!(
        &words[..9],
        [
            HOST,
            &format!("www1.{HOST}"),
            ALT_HOST,
            &format!("www2.{HOST}"),
            "8000",
            "1",
            &format!("http://{HOST}:8000"),
            "/dir/t.sub.html?logs=%5B%22a+b%22%5D&x=1",
            "[\"a",
        ]
    );
    assert_eq!(words[9], "b\"]");
    assert_eq!(words[10], "[]");
    assert_eq!(words[11], "z");
    assert_eq!(
        words[12], words[13],
        "a variable is the value it was set to"
    );
    assert!(words[12].starts_with("00000000-0000-4000-8000-"));
    assert_eq!(words[14], "{{unknown}}");
    assert_eq!(form_decoded("a%2Fb+c%zz"), "a/b c%zz");
}

#[test]
fn the_clock_moves_on_to_the_next_frame_or_task() {
    let ms = Duration::from_millis;
    assert_eq!(
        next_time(ms(100), true, None, Some(ms(500))),
        Some(ms(100) + FRAME)
    );
    assert_eq!(
        next_time(ms(100), false, Some(ms(300)), Some(ms(200))),
        Some(ms(200))
    );
    assert_eq!(
        next_time(ms(100), false, Some(ms(150)), Some(ms(200))),
        Some(ms(150))
    );
    assert_eq!(
        next_time(ms(100), false, None, Some(ms(50))),
        Some(ms(101)),
        "a task overdue: a millisecond on"
    );
    assert_eq!(next_time(ms(100), false, None, None), None);
}

/// Whether the root element's `class` holds `class`.
fn waits(engine: &Engine, class: &str) -> bool {
    let document = engine.document();
    document
        .document_element()
        .and_then(|root| document.attr(root, "class"))
        .is_some_and(|classes| classes.split_ascii_whitespace().any(|c| c == class))
}

/// Runs the page's event loop until its root element no longer has
/// `class` (`reftest-wait`, or a crashtest's `test-wait`), as `wptrunner`
/// waits for it: the tasks due and the network's replies taken, a frame
/// drawn by time when one is wanted, its rendering steps and animation
/// frame callbacks with it, and the clock moved on to the next frame
/// wanted or task due; or to what the network sends, waited for. After
/// the first frame, `TestRendered` is dispatched at the root element,
/// bubbling, as the harness dispatches it; after the class goes, two
/// frames more. Fails past [`WAIT`] of the page's time, or with nothing
/// left to wait for.
fn wait_for(loaded: &mut Loaded, path: &str, class: &str) -> Result<(), String> {
    let Loaded {
        engine,
        woken,
        clock,
    } = loaded;
    if !waits(engine, class) {
        return Ok(());
    }
    let nothing = || format!("{path} kept {class} on, with nothing to wait for");
    let deadline = Instant::now() + LOAD;
    let mut rendered = false;
    while waits(engine, class) {
        let now = clock.now();
        if now > WAIT {
            return Err(format!("{path} kept {class} on for {WAIT:?}"));
        }
        engine.poll_network();
        engine.run_tasks();
        if engine.needs_frame() || !rendered {
            engine.frame_at(WIDTH, HEIGHT, FrameTime::at(now));
        }
        if !rendered {
            rendered = true;
            run_script(
                engine,
                "document.documentElement.dispatchEvent(\
                 new Event('TestRendered', { bubbles: true }));",
            );
            continue;
        }
        if !waits(engine, class) {
            break;
        }
        // The clock waits for the network and the decoders, as a
        // testharness test's does (M33): an image a timer's screenshot
        // waits for is there however fast the machine fetched it.
        if engine.fetching() || engine.decoding() {
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(nothing)?;
            let _ = woken.recv_timeout(left.min(Duration::from_millis(100)));
            continue;
        }
        match next_time(
            now,
            engine.needs_frame(),
            engine.next_frame_at(),
            engine.next_task_at(),
        ) {
            Some(at) => clock.set(at),
            // Only the network can change anything now.
            None => {
                let left = deadline
                    .checked_duration_since(Instant::now())
                    .ok_or_else(nothing)?;
                woken
                    .recv_timeout(left.min(Duration::from_secs(2)))
                    .map_err(|_| nothing())?;
            }
        }
    }
    // Two frames more, as `wptrunner` waits two animation frames after
    // the class goes before it takes the screenshot: what the page
    // queued for them, its events and callbacks, is drawn.
    for _ in 0..2 {
        let now = clock.now() + FRAME;
        clock.set(now);
        engine.poll_network();
        engine.run_tasks();
        engine.frame_at(WIDTH, HEIGHT, FrameTime::at(now));
    }
    Ok(())
}

/// The RGB pixels of the page at `path`, drawn once what it draws is
/// decoded (M23): a frame asks for its images' pixels, and the one after
/// they are decoded draws them.
fn render(server: &TestServer, path: &str) -> Result<Vec<u8>, String> {
    let mut loaded = load_waking(server, path)?;
    wait_for(&mut loaded, path, "reftest-wait")?;
    let Loaded {
        mut engine, woken, ..
    } = loaded;
    let deadline = Instant::now() + LOAD;
    let rgba = loop {
        let rgba = engine.frame(WIDTH, HEIGHT);
        if !engine.decoding() {
            break rgba;
        }
        while engine.decoding() {
            engine.poll_network();
            if !engine.decoding() {
                break;
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| format!("{path}'s images took more than {LOAD:?} to decode"))?;
            let _ = woken.recv_timeout(left);
        }
    };
    Ok(rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect())
}

/// The largest channel difference and the number of differing pixels.
fn differences(a: &[u8], b: &[u8]) -> (u32, u32) {
    let mut largest = 0u32;
    let mut pixels = 0u32;
    for (a, b) in a.chunks(3).zip(b.chunks(3)) {
        let d = a
            .iter()
            .zip(b)
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .max()
            .unwrap_or(0);
        if d > 0 {
            pixels += 1;
            largest = largest.max(d);
        }
    }
    (largest, pixels)
}

/// Whether two renders count as equal under `fuzzy`, as `wptrunner`
/// decides (`RefTestImplementation.check_pass`): exactly equal without
/// fuzziness, else differences within both ranges — or none at all,
/// when a range allows none.
fn equal(largest: u32, pixels: u32, fuzzy: Option<Fuzzy>) -> bool {
    match fuzzy {
        None | Some(((0, 0), (0, 0))) => pixels == 0,
        Some(((channel_low, channel_high), (pixels_low, pixels_high))) => {
            (pixels == 0 && pixels_low == 0)
                || (largest == 0 && channel_low == 0)
                || ((channel_low..=channel_high).contains(&largest)
                    && (pixels_low..=pixels_high).contains(&pixels))
        }
    }
}

/// What running a test gave.
struct Outcome {
    passed: bool,
    message: String,
    /// For a failing reftest: the test's render and the closest
    /// reference's, with the reference's path.
    images: Option<(Vec<u8>, Vec<u8>, String)>,
    /// For a testharness test whose harness wrote its results.
    harness: Option<Harness>,
}

impl Outcome {
    fn pass() -> Self {
        Outcome {
            passed: true,
            message: String::new(),
            images: None,
            harness: None,
        }
    }

    fn fail(message: String) -> Self {
        Outcome {
            passed: false,
            message,
            images: None,
            harness: None,
        }
    }
}

/// What the runner's `testharnessreport.js` wrote: the harness's status
/// and message, and each subtest's name, status and message. A status
/// is testharness.js's: 0 for an harness that finished (OK) or a subtest
/// that passed.
struct Harness {
    status: u8,
    message: String,
    tests: Vec<(String, u8, String)>,
}

/// testharness.js's names of a harness's statuses.
const HARNESS_STATUSES: [&str; 4] = ["OK", "ERROR", "TIMEOUT", "PRECONDITION_FAILED"];

/// testharness.js's names of a subtest's statuses.
const SUBTEST_STATUSES: [&str; 5] = ["PASS", "FAIL", "TIMEOUT", "NOTRUN", "PRECONDITION_FAILED"];

fn status_name(names: &[&'static str], status: u8) -> &'static str {
    names.get(usize::from(status)).copied().unwrap_or("UNKNOWN")
}

/// The page's time a testharness test may take, as `wptrunner` gives
/// it: ten seconds, a minute for one whose `timeout` is long.
const HARNESS_WAIT: Duration = Duration::from_secs(10);
const HARNESS_WAIT_LONG: Duration = Duration::from_secs(60);
/// The real time a page with workers is given before each move of its
/// clock (M31).
const WORKER_SLICE: Duration = Duration::from_millis(20);

/// The most real time a page with workers waits out a move of its clock
/// (M33).
const WORKER_WAIT: Duration = Duration::from_secs(1);

/// The results the report wrote into the page, once it has; a subtest
/// that names the server's port, which each run's server chooses, has it
/// as `PORT` (M33).
fn harness_results(engine: &Engine) -> Option<Harness> {
    let port = engine
        .url()
        .and_then(net::Url::port)
        .map(|port| port.to_string());
    let document = engine.document();
    let out = document
        .subtree(document.root())
        .find(|&id| document.attr(id, "id") == Some("wpt-results"))?;
    let text = document.text_content(out);
    let mut lines = text.split('\n');
    let (status, message) = lines.next()?.split_once('\t')?;
    let tests = lines
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let status = fields.next()?.parse().ok()?;
            let name = fields.next()?.to_string();
            let name = match &port {
                Some(port) => name.replace(port.as_str(), "PORT"),
                None => name,
            };
            Some((name, status, fields.next().unwrap_or("").to_string()))
        })
        .collect();
    Some(Harness {
        status: status.parse().ok()?,
        message: message.to_string(),
        tests,
    })
}

/// Runs the testharness test at `path` until its harness writes its
/// results, as [`wait_for`] runs a page's event loop: the network's
/// replies taken and the tasks due run, a frame drawn by time when one
/// is wanted, and the clock moved on to the next frame or task, but only
/// once no fetch is in flight, so that a page waiting on the server is
/// not timed out by its own timers running ahead.
fn run_testharness(server: &TestServer, path: &str, long: bool) -> Outcome {
    let mut loaded = match load_waking(server, path) {
        Ok(loaded) => loaded,
        Err(error) => return Outcome::fail(error),
    };
    let Loaded {
        engine,
        woken,
        clock,
    } = &mut loaded;
    let wait = if long {
        HARNESS_WAIT_LONG
    } else {
        HARNESS_WAIT
    };
    let deadline = Instant::now() + LOAD;
    // When the page's clock last moved, in real time.
    let mut moved = Instant::now();
    loop {
        let now = clock.now();
        engine.poll_network();
        engine.run_tasks();
        if engine.needs_frame() {
            engine.frame_at(WIDTH, HEIGHT, FrameTime::at(now));
        }
        if let Some(harness) = harness_results(engine) {
            let passed = harness.status == 0 && harness.tests.iter().all(|(_, s, _)| *s == 0);
            let failed = harness.tests.iter().filter(|(_, s, _)| *s != 0).count();
            let message = match harness.status {
                0 => format!("{failed} of {} subtests failed", harness.tests.len()),
                status => format!(
                    "harness {}: {}",
                    status_name(&HARNESS_STATUSES, status),
                    harness.message
                ),
            };
            return Outcome {
                passed,
                message,
                images: None,
                harness: Some(harness),
            };
        }
        if now > wait {
            return Outcome::fail(format!("TIMEOUT: still running after {wait:?} of its time"));
        }
        let left = deadline.checked_duration_since(Instant::now());
        let Some(left) = left else {
            return Outcome::fail(format!("TIMEOUT: still running after {LOAD:?}"));
        };
        if engine.fetching() {
            let _ = woken.recv_timeout(left.min(Duration::from_millis(100)));
            continue;
        }
        let next = next_time(
            now,
            engine.needs_frame(),
            engine.next_frame_at(),
            engine.next_task_at(),
        );
        // A page's workers run in real time (M31): what they send is
        // taken for a slice of real time before the page's clock moves
        // on, however often they send; and a move longer than a slice is
        // waited out in real time, up to a second (M33), so that a worker
        // the page's own timeout waits for has the time it would have.
        let slice = moved.elapsed();
        if engine.has_workers() {
            let jump = next.map_or(Duration::ZERO, |at| at.saturating_sub(now));
            let real = jump.clamp(WORKER_SLICE, WORKER_WAIT);
            if slice < real {
                let _ = woken.recv_timeout(real - slice);
                continue;
            }
        }
        match next {
            Some(at) => {
                clock.set(at);
                moved = Instant::now();
            }
            // Only the network, a decoder or a worker can change
            // anything now; a worker may take its time.
            None => {
                let quiet = if engine.has_workers() {
                    left
                } else {
                    left.min(Duration::from_secs(1))
                };
                if woken.recv_timeout(quiet).is_err() {
                    return Outcome::fail("TIMEOUT: nothing left to wait for".to_string());
                }
            }
        }
    }
}

fn run_reftest(
    server: &TestServer,
    path: &str,
    refs: &[(bool, String)],
    fuzzy: &[(String, Fuzzy)],
) -> Outcome {
    let test = match render(server, path) {
        Ok(test) => test,
        Err(error) => return Outcome::fail(error),
    };
    let fuzziness = |reference: &str| {
        fuzzy
            .iter()
            .find(|(r, _)| r == reference)
            .or_else(|| fuzzy.iter().find(|(r, _)| r.is_empty()))
            .map(|(_, f)| *f)
    };
    let mut matched = !refs.iter().any(|(must_match, _)| *must_match);
    let mut messages = Vec::new();
    let mut closest: Option<(u32, Vec<u8>, String)> = None;
    for (must_match, reference) in refs {
        let rendered = match render(server, reference) {
            Ok(rendered) => rendered,
            Err(error) => return Outcome::fail(error),
        };
        let (largest, pixels) = differences(&test, &rendered);
        let same = equal(largest, pixels, fuzziness(reference));
        if *must_match {
            if same {
                matched = true;
            } else {
                messages.push(format!(
                    "{pixels} pixels differ from {reference}, by up to {largest}"
                ));
                if closest.as_ref().is_none_or(|(p, _, _)| pixels < *p) {
                    closest = Some((pixels, rendered, reference.clone()));
                }
            }
        } else if same {
            return Outcome::fail(format!(
                "renders the same as {reference}, which it must not"
            ));
        }
    }
    if matched {
        return Outcome::pass();
    }
    Outcome {
        passed: false,
        message: messages.join("; "),
        images: closest.map(|(_, reference, name)| (test, reference, name)),
        harness: None,
    }
}

/// An element's boxes, from the engine's fragments: its first box
/// fragment's border box, or the union of its inline boxes on their
/// lines, in document coordinates, and the left border's width.
#[derive(Debug, Clone, Copy)]
struct Boxes {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    border_left: f32,
    border_top: f32,
    border_right: f32,
    border_bottom: f32,
    inline: bool,
}

/// The boxes of every element that has one.
fn boxes(layout: &layout::DocumentLayout) -> HashMap<NodeId, Boxes> {
    let mut out: HashMap<NodeId, Boxes> = HashMap::new();
    for f in layout.iter() {
        if f.generated.is_some() {
            continue;
        }
        let inline = match f.kind {
            FragmentKind::Box => false,
            FragmentKind::InlineBox => true,
            FragmentKind::Text(_) | FragmentKind::Marker(_) | FragmentKind::Replaced(_) => continue,
        };
        match out.get_mut(&f.node) {
            None => {
                out.insert(f.node, own(f, inline));
            }
            Some(b) if b.inline && inline => {
                let (right, bottom) = ((b.x + b.w).max(f.x + f.w), (b.y + b.h).max(f.y + f.h));
                b.x = b.x.min(f.x);
                b.y = b.y.min(f.y);
                b.w = right - b.x;
                b.h = bottom - b.y;
            }
            Some(_) => {}
        }
    }
    out
}

fn own(f: &Fragment, inline: bool) -> Boxes {
    let border = f.style.used_border_width();
    Boxes {
        x: f.x,
        y: f.y,
        w: f.w,
        h: f.h,
        border_left: border[LEFT],
        border_top: border[TOP],
        border_right: border[RIGHT],
        border_bottom: border[style::computed::BOTTOM],
        inline,
    }
}

/// A rounding as a browser's integer CSSOM values have it: half up,
/// for a size as for a position. The reference browser's `offsetWidth`
/// and `clientWidth` are the size rounded, wherever the box starts —
/// not the distance between its two edges rounded, which M17's layouts
/// found a pixel apart for a box a third of a pixel into one.
fn round(v: f32) -> f64 {
    (f64::from(v) + 0.5).floor()
}

/// The values `check-layout-th.js` reads, for one page.
struct Checker<'e> {
    document: &'e Document,
    boxes: HashMap<NodeId, Boxes>,
    styles: &'e style::Styles,
}

impl Checker<'_> {
    /// CSSOM View's `offsetParent`: none for the root, the body, an
    /// element with no box, or a fixed one no ancestor is a containing
    /// block for, as one whose transform properties or `will-change`
    /// make it so is (M19); else the nearest ancestor that is a
    /// containing block for absolutely positioned descendants, by its
    /// position or, as the reference browser has it, by those
    /// properties, or is a cell or a table when the element is not
    /// positioned, or is the body.
    fn offset_parent(&self, id: NodeId) -> Option<NodeId> {
        let tag = |n: NodeId| self.document.tag(n).unwrap_or("").to_ascii_lowercase();
        let position = |n: NodeId| self.styles.get(n).map(|s| s.position);
        // Whether the element's effects make it a containing block for
        // every descendant (`all`) or for the absolutely positioned
        // ones; the transform properties apply to a box that is not an
        // inline one alone.
        let contains = |n: NodeId, all: bool| {
            let transformable = self.boxes.get(&n).is_some_and(|b| !b.inline);
            self.styles.get(n).is_some_and(|s| {
                if all {
                    s.effects().contains_all_of(transformable)
                } else {
                    s.effects().contains_absolutes_of(transformable)
                }
            })
        };
        if !self.boxes.contains_key(&id) || matches!(tag(id).as_str(), "html" | "body") {
            return None;
        }
        if position(id) == Some(Position::Fixed)
            && !self
                .document
                .ancestors(id)
                .any(|a| self.document.is_element(a) && contains(a, true))
        {
            return None;
        }
        let positioned = position(id).is_some_and(|p| p != Position::Static);
        for ancestor in self.document.ancestors(id) {
            if !self.document.is_element(ancestor) {
                continue;
            }
            let t = tag(ancestor);
            if position(ancestor).is_some_and(|p| p != Position::Static)
                || contains(ancestor, false)
                || t == "body"
                || (!positioned && matches!(t.as_str(), "td" | "th" | "table"))
            {
                return Some(ancestor);
            }
        }
        None
    }

    /// `offsetLeft` and `offsetTop`: the border box from the offset
    /// parent's padding box, or from the document when there is none or
    /// it is the body.
    fn offset(&self, id: NodeId) -> (f64, f64) {
        let Some(b) = self.boxes.get(&id) else {
            return (0.0, 0.0);
        };
        let parent = self
            .offset_parent(id)
            .filter(|&p| {
                self.document
                    .tag(p)
                    .is_some_and(|t| !t.eq_ignore_ascii_case("body"))
            })
            .and_then(|p| self.boxes.get(&p));
        match parent {
            Some(p) => (
                round(b.x - p.x - p.border_left),
                round(b.y - p.y - p.border_top),
            ),
            None => (round(b.x), round(b.y)),
        }
    }

    /// The value a `data-*` expectation is checked against, as a number
    /// or, for the display, a keyword.
    fn value(&self, id: NodeId, attribute: &str) -> Value {
        let b = self.boxes.get(&id).copied();
        let number = |f: &dyn Fn(Boxes) -> f64| Value::Number(b.map_or(0.0, f));
        match attribute {
            "data-expected-width" => number(&|b| round(b.w)),
            "data-expected-height" => number(&|b| round(b.h)),
            "data-offset-x" => Value::Number(self.offset(id).0),
            "data-offset-y" => Value::Number(self.offset(id).1),
            "data-expected-client-width" => number(&|b| {
                if b.inline {
                    0.0
                } else {
                    round(b.w - b.border_left - b.border_right)
                }
            }),
            "data-expected-client-height" => number(&|b| {
                if b.inline {
                    0.0
                } else {
                    round(b.h - b.border_top - b.border_bottom)
                }
            }),
            "data-expected-bounding-client-rect-width" => number(&|b| f64::from(b.w)),
            "data-expected-bounding-client-rect-height" => number(&|b| f64::from(b.h)),
            "data-total-x" => Value::Number(
                self.offset(id).0
                    + b.filter(|b| !b.inline)
                        .map_or(0.0, |b| round(b.border_left)),
            ),
            "data-total-y" => Value::Number(
                self.offset(id).1 + b.filter(|b| !b.inline).map_or(0.0, |b| round(b.border_top)),
            ),
            "data-expected-display" => Value::Keyword(
                self.styles
                    .get(id)
                    .map_or("none", |s| display_name(s.display)),
            ),
            other => panic!("the rule lets no {other} through"),
        }
    }

    /// The failures among `id`'s expectations, each described.
    fn check(&self, id: NodeId, failures: &mut Vec<String>) {
        for (name, expected) in self.document.attrs(id).unwrap_or(&[]) {
            if !name.starts_with("data-") || name.starts_with("data-test") {
                continue;
            }
            let actual = self.value(id, name);
            let ok = match &actual {
                Value::Number(actual) => expected
                    .trim()
                    .parse::<f64>()
                    .is_ok_and(|expected| (actual - expected).abs() < 1.0),
                Value::Keyword(actual) => actual == expected,
            };
            if !ok {
                failures.push(format!(
                    "{} {name}: {actual} where {expected} was expected",
                    describe(self.document, id)
                ));
            }
        }
    }
}

enum Value {
    Number(f64),
    Keyword(&'static str),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{n}"),
            Value::Keyword(k) => f.write_str(k),
        }
    }
}

/// `display`'s keyword for a computed value.
fn display_name(display: Display) -> &'static str {
    match display {
        Display::Block => "block",
        Display::Inline => "inline",
        Display::InlineBlock => "inline-block",
        Display::None => "none",
        Display::Table => "table",
        Display::InlineTable => "inline-table",
        Display::TableRowGroup => "table-row-group",
        Display::TableHeaderGroup => "table-header-group",
        Display::TableFooterGroup => "table-footer-group",
        Display::TableRow => "table-row",
        Display::TableColumnGroup => "table-column-group",
        Display::TableColumn => "table-column",
        Display::TableCell => "table-cell",
        Display::TableCaption => "table-caption",
        Display::Flex => "flex",
        Display::InlineFlex => "inline-flex",
        Display::Grid => "grid",
        Display::InlineGrid => "inline-grid",
        Display::ListItem => "list-item",
    }
}

/// An element as a failure names it: its tag, id and classes.
fn describe(document: &Document, id: NodeId) -> String {
    let mut out = format!("<{}", document.tag(id).unwrap_or("?"));
    for attribute in ["id", "class"] {
        if let Some(value) = document.attr(id, attribute) {
            let _ = write!(out, " {attribute}=\"{value}\"");
        }
    }
    out.push('>');
    out
}

fn run_check_layout(
    server: &TestServer,
    path: &str,
    selectors: &[String],
    (width, height): (u32, u32),
) -> Outcome {
    let mut engine = match load(server, path) {
        Ok(engine) => engine,
        Err(error) => return Outcome::fail(error),
    };
    // A frame first: it sets the viewport the layout is for.
    engine.frame(width, height);
    let boxes = boxes(engine.layout(width as f32));
    engine.styles();
    let document = engine.document().clone();
    let styles = engine.styles().clone();
    let checker = Checker {
        document: &document,
        boxes,
        styles: &styles,
    };
    let mut failures = Vec::new();
    for selector in selectors {
        let Ok(list) = style::parse_selector_list(&css::tokenize(selector)) else {
            return Outcome::fail(format!("the selector {selector} does not parse"));
        };
        let found: Vec<NodeId> = document
            .subtree(document.root())
            .filter(|&n| {
                document.is_element(n) && style::matches_selector_list(&document, n, &list)
            })
            .collect();
        for node in found {
            if let Some(parent) = document.parent_element(node) {
                checker.check(parent, &mut failures);
            }
            for n in document.subtree(node) {
                if document.is_element(n) {
                    checker.check(n, &mut failures);
                }
            }
        }
    }
    if failures.is_empty() {
        Outcome::pass()
    } else {
        Outcome::fail(failures.join("; "))
    }
}

fn run_crashtest(server: &TestServer, path: &str) -> Outcome {
    let ran = load_waking(server, path).and_then(|mut loaded| {
        wait_for(&mut loaded, path, "test-wait")?;
        loaded.engine.frame(WIDTH, HEIGHT);
        Ok(())
    });
    match ran {
        Ok(()) => Outcome::pass(),
        Err(error) => Outcome::fail(error),
    }
}

/// Runs one test, a panic being a failure; a testharness test from
/// `harness`, the server that serves the harness.
fn run(server: &TestServer, harness: &TestServer, test: &Test) -> Outcome {
    let ran = panic::catch_unwind(AssertUnwindSafe(|| match test {
        Test::Reftest { path, refs, fuzzy } => run_reftest(server, path, refs, fuzzy),
        Test::CheckLayout {
            path,
            selectors,
            size,
        } => run_check_layout(server, path, selectors, *size),
        Test::Crashtest { path } => run_crashtest(server, path),
        Test::Testharness { path, long } => run_testharness(harness, path, *long),
    }));
    ran.unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        Outcome::fail(format!("panicked: {message}"))
    })
}

fn write_png(path: &Path, rgb: &[u8]) {
    let file = File::create(path).expect("the output directory is writable");
    let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgb))
        .expect("the image is written");
}

/// The differing pixels in red over the reference, dimmed.
fn difference_image(test: &[u8], reference: &[u8]) -> Vec<u8> {
    test.chunks(3)
        .zip(reference.chunks(3))
        .flat_map(|(t, r)| {
            if t == r {
                [r[0] / 2 + 0x40, r[1] / 2 + 0x40, r[2] / 2 + 0x40]
            } else {
                [0xff, 0x00, 0x00]
            }
        })
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Writes `target/wpt/report.html` and a failing reftest's images.
fn report(results: &[(Test, Outcome)], expected: &BTreeMap<String, String>) {
    let out = out_dir();
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).expect("the output directory is created");
    let passed = results.iter().filter(|(_, o)| o.passed).count();
    let mut html = format!(
        "<!doctype html><meta charset=utf-8><title>web-platform-tests</title>\
         <style>body{{font:14px sans-serif;margin:2em}}td{{padding:2px 8px;vertical-align:top}}\
         .pass{{color:#070}}.fail{{color:#b00}}.expected{{color:#a60}}\
         img{{width:400px;border:1px solid #ccc}}</style>\
         <h1>web-platform-tests</h1><p>{passed} of {} passed.</p><table>",
        results.len()
    );
    for (i, (test, outcome)) in results.iter().enumerate() {
        let path = test.path();
        let (class, word) = match (outcome.passed, expected.get(path)) {
            (true, None) => ("pass", "PASS".to_string()),
            (true, Some(_)) => ("fail", "UNEXPECTED PASS".to_string()),
            (false, Some(reason)) => ("expected", format!("FAIL (expected: {})", escape(reason))),
            (false, None) => ("fail", "FAIL".to_string()),
        };
        let _ = write!(
            html,
            "<tr><td class={class}>{word}</td><td>{}</td><td>{}<br>{}",
            test.kind(),
            escape(path),
            escape(&outcome.message)
        );
        if let Some(harness) = &outcome.harness {
            for (name, status, message) in &harness.tests {
                if *status == 0 {
                    continue;
                }
                let named = expected.contains_key(&format!("{path}\t{name}"));
                let _ = write!(
                    html,
                    "<br><span class={}>{}</span> {}: {}",
                    if named { "expected" } else { "fail" },
                    status_name(&SUBTEST_STATUSES, *status),
                    escape(name),
                    escape(message)
                );
            }
        }
        if let Some((rendered, reference, name)) = &outcome.images {
            let stem = format!("{i:04}");
            write_png(&out.join(format!("{stem}-test.png")), rendered);
            write_png(&out.join(format!("{stem}-ref.png")), reference);
            write_png(
                &out.join(format!("{stem}-diff.png")),
                &difference_image(rendered, reference),
            );
            let _ = write!(
                html,
                "<br><img src={stem}-test.png title=test> <img src={stem}-ref.png title=\"{}\"> \
                 <img src={stem}-diff.png title=difference>",
                escape(name)
            );
        }
        html.push_str("</td></tr>");
    }
    html.push_str("</table>");
    fs::write(out.join("report.html"), html).expect("the report is written");
    // Each testharness test that failed, and each subtest, a line each
    // with tabs between, for keeping expectations.txt: the path, the
    // subtest's name (none for the whole test), the status and the
    // message.
    let mut failures = String::new();
    for (test, outcome) in results {
        if !matches!(test, Test::Testharness { .. }) || outcome.passed {
            continue;
        }
        let path = test.path();
        match &outcome.harness {
            Some(harness) if harness.status == 0 => {
                for (name, status, message) in &harness.tests {
                    if *status != 0 {
                        let status = status_name(&SUBTEST_STATUSES, *status);
                        let _ = writeln!(failures, "{path}\t{name}\t{status}\t{message}");
                    }
                }
            }
            _ => {
                let _ = writeln!(failures, "{path}\t\tFAIL\t{}", outcome.message);
            }
        }
    }
    fs::write(out.join("testharness.txt"), failures).expect("the failures are written");
}

#[test]
fn the_web_platform_tests_subset_runs_as_expected() {
    let tests = manifest();
    let expected = expectations();
    for key in expected.keys() {
        let path = key.split('\t').next().unwrap_or(key);
        assert!(
            tests.iter().any(|t| t.path() == path),
            "expectations.txt names {path}, which is not in the manifest"
        );
    }
    let server = serve(root(), false);
    let harness = serve(root(), true);
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Outcome>>> = Mutex::new((0..tests.len()).map(|_| None).collect());
    let threads = thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(tests.len().max(1));
    thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(test) = tests.get(i) else {
                        break;
                    };
                    let outcome = run(&server, &harness, test);
                    results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(outcome);
                }
            });
        }
    });
    let results: Vec<(Test, Outcome)> = tests
        .into_iter()
        .zip(results.into_inner().unwrap_or_else(|e| e.into_inner()))
        .map(|(test, outcome)| (test, outcome.expect("every test ran")))
        .collect();
    report(&results, &expected);
    let mut unexpected = Vec::new();
    let mut subtests = (0, 0);
    for (test, outcome) in &results {
        if let Some(harness) = &outcome.harness {
            subtests.0 += harness.tests.len();
            subtests.1 += harness.tests.iter().filter(|(_, s, _)| *s == 0).count();
            unexpected.extend(harness_unexpected(test.path(), harness, &expected));
            continue;
        }
        match (outcome.passed, expected.contains_key(test.path())) {
            (false, false) => unexpected.push(format!("FAIL {}: {}", test.path(), outcome.message)),
            (true, true) => unexpected.push(format!(
                "PASS {}: it passes, so expectations.txt must no longer name it",
                test.path()
            )),
            _ => {}
        }
    }
    let passed = results.iter().filter(|(_, o)| o.passed).count();
    eprintln!(
        "web-platform-tests: {passed} of {} passed ({} of {} testharness subtests), \
         {} expected to fail; report in {}",
        results.len(),
        subtests.1,
        subtests.0,
        expected.len(),
        out_dir().join("report.html").display()
    );
    assert!(
        unexpected.is_empty(),
        "{} unexpected results:\n{}",
        unexpected.len(),
        unexpected.join("\n")
    );
}

/// What is unexpected of the testharness test at `path` whose harness
/// wrote `harness`: a harness that did not finish, unless the test is
/// named whole; one that did, if it is; each subtest that failed and is
/// not named, or passed and is; and each subtest named that did not run.
fn harness_unexpected(
    path: &str,
    harness: &Harness,
    expected: &BTreeMap<String, String>,
) -> Vec<String> {
    let whole = expected.contains_key(path);
    if harness.status != 0 {
        return match whole {
            true => Vec::new(),
            false => vec![format!(
                "{} {path}: {}",
                status_name(&HARNESS_STATUSES, harness.status),
                harness.message
            )],
        };
    }
    let mut unexpected = Vec::new();
    if whole {
        unexpected.push(format!(
            "PASS {path}: its harness finished, so expectations.txt must name its failing \
             subtests rather than the test"
        ));
    }
    for (name, status, message) in &harness.tests {
        match (
            *status == 0,
            expected.contains_key(&format!("{path}\t{name}")),
        ) {
            (false, false) => unexpected.push(format!(
                "{} {path}\t{name}: {message}",
                status_name(&SUBTEST_STATUSES, *status)
            )),
            (true, true) => unexpected.push(format!(
                "PASS {path}\t{name}: it passes, so expectations.txt must no longer name it"
            )),
            _ => {}
        }
    }
    let prefix = format!("{path}\t");
    for key in expected.keys().filter(|key| key.starts_with(&prefix)) {
        if !harness
            .tests
            .iter()
            .any(|(name, ..)| key[prefix.len()..] == *name)
        {
            unexpected.push(format!(
                "GONE {key}: expectations.txt names a subtest that did not run"
            ));
        }
    }
    unexpected
}

/// The viewport's size an acceptance layout asks for with its
/// `<meta name="reference-size" content="WxH">` (M20), as
/// `examples/layouts.rs` captures it, else 800 by 600.
fn size_of(source: &str) -> (u32, u32) {
    let needle = "<meta name=\"reference-size\" content=\"";
    let Some(at) = source.find(needle) else {
        return (WIDTH, HEIGHT);
    };
    let size = source[at + needle.len()..].split('"').next().unwrap_or("");
    size.split_once('x')
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .unwrap_or_else(|| panic!("the page's size {size:?} is not WxH"))
}

#[test]
fn the_acceptance_layouts_lay_out_as_the_reference_browser_lays_them_out() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/layouts");
    let mut pages: Vec<String> = fs::read_dir(&dir)
        .expect("the layouts are there")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".html"))
        .collect();
    pages.sort();
    assert_eq!(
        pages.len(),
        19,
        "M17's navigation bar, card grid, holy grail, media objects and application shell, \
         M18's grid garden, dashboard, photo gallery, form grid, page of named areas and calendar, \
         M19's carousel, rotated panels, modal and fixed header in a transform, \
         M20's responsive page at three widths and M26's form controls"
    );
    let server = serve(dir.clone(), false);
    let mut failures = Vec::new();
    for page in &pages {
        // A page without its expectations would pass on nothing.
        let source = fs::read_to_string(dir.join(page)).expect("the page reads");
        assert!(
            source.matches("data-expected-width").count() >= 10,
            "{page} has not been captured"
        );
        let test = Test::CheckLayout {
            path: page.clone(),
            selectors: vec!["body".to_string()],
            size: size_of(&source),
        };
        let outcome = run(&server, &server, &test);
        if !outcome.passed {
            failures.push(format!("{page}: {}", outcome.message.replace("; ", "\n  ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
