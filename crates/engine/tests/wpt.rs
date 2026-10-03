//! The web-platform-tests subset (M17, T1.1): the tests under
//! `tests/wpt/`, which `examples/wpt.rs` chose and copied from upstream,
//! run as `wptrunner` would run them, against `expectations.txt`.
//!
//! The directory is served from the `net` crate's test server on
//! 127.0.0.1, so a test and its stylesheets load through the engine's
//! own loader by their URLs. Each test is loaded in an engine of its
//! own, with the bundled font, at 800 by 600, on as many threads as
//! there are cores:
//!
//! - a reftest is rendered, and so is each of its references, and it
//!   passes when it matches one of the references it must match and none
//!   it must not, within its `fuzzy` metadata, by `wptrunner`'s
//!   comparison;
//! - a check-layout test is checked natively, as `check-layout-th.js`
//!   checks it: each element its selectors find, the element's parent,
//!   and the element's descendants, by their `data-*` attributes, within
//!   a pixel, the CSSOM View values computed from the engine's fragments;
//! - a crashtest must load and render without panicking.
//!
//! The acceptance layouts of M17 (`tests/layouts/`, M17.10) are
//! check-layout pages too, their expectations captured from the
//! reference browser by `examples/layouts.rs`, and run here the same
//! way, every element of the body checked.
//!
//! `expectations.txt` names each test expected to fail, with the reason.
//! A test that fails without being named fails the run, and so does one
//! named that passes, so that the file stays the true list of what does
//! not work and why. Every run writes `target/wpt/report.html`: each test
//! with its result, and a failing reftest's render, its reference's and
//! their difference side by side.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Duration, Instant};

use dom::{Document, NodeId};
use engine::{Engine, LoadState};
use layout::{Fragment, FragmentKind};
use net::test_server::{Reply, TestServer};
use style::computed::{Display, LEFT, Position, RIGHT, TOP};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// How long a page may take to load.
const LOAD: Duration = Duration::from_secs(30);

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
    },
    Crashtest {
        path: String,
    },
}

type Fuzzy = ((u32, u32), (u32, u32));

impl Test {
    fn path(&self) -> &str {
        match self {
            Test::Reftest { path, .. }
            | Test::CheckLayout { path, .. }
            | Test::Crashtest { path } => path,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Test::Reftest { .. } => "reftest",
            Test::CheckLayout { .. } => "check-layout",
            Test::Crashtest { .. } => "crashtest",
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
            },
            "crashtest" => Test::Crashtest { path },
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

/// `expectations.txt`: each test expected to fail, and why.
fn expectations() -> BTreeMap<String, String> {
    let text = fs::read_to_string(root().join("expectations.txt")).unwrap_or_default();
    let mut out = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (path, reason) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let reason = reason.trim();
        assert!(!reason.is_empty(), "expectations.txt: {path} has no reason");
        assert!(
            out.insert(path.to_string(), reason.to_string()).is_none(),
            "expectations.txt: {path} is named twice"
        );
    }
    out
}

/// The content type a vendored file is served with.
fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("xht" | "xhtml") => "application/xhtml+xml",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

/// The server of `root`, the vendored directory or the layouts'.
fn serve(root: PathBuf) -> TestServer {
    TestServer::plain(move |request| {
        let path = request.target.split(['?', '#']).next().unwrap_or("");
        let file = path.trim_start_matches('/');
        let safe = !file.split('/').any(|part| part == ".." || part.is_empty());
        match fs::read(root.join(file)) {
            Ok(body) if safe => Reply::ok(content_type(file), &body),
            _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"not found"),
        }
    })
}

/// An engine with the bundled font that has loaded `path` from `server`,
/// or why it could not.
fn load(server: &TestServer, path: &str) -> Result<Engine, String> {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(server.url(&format!("/{path}")));
    let deadline = Instant::now() + LOAD;
    loop {
        engine.poll_network();
        match engine.load_state() {
            LoadState::Loaded => return Ok(engine),
            LoadState::Failed(error) => return Err(format!("{path} failed to load: {error}")),
            LoadState::Loading => {}
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| format!("{path} took more than {LOAD:?} to load"))?;
        let _ = woken.recv_timeout(left);
    }
}

/// The RGB pixels of the page at `path`.
fn render(server: &TestServer, path: &str) -> Result<Vec<u8>, String> {
    let mut engine = load(server, path)?;
    let rgba = engine.frame(WIDTH, HEIGHT);
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
}

impl Outcome {
    fn pass() -> Self {
        Outcome {
            passed: true,
            message: String::new(),
            images: None,
        }
    }

    fn fail(message: String) -> Self {
        Outcome {
            passed: false,
            message,
            images: None,
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
            FragmentKind::Text(_) => continue,
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
    /// element with no box or a fixed one; else the nearest ancestor that
    /// is positioned, or is a cell or a table when the element is not
    /// positioned, or is the body.
    fn offset_parent(&self, id: NodeId) -> Option<NodeId> {
        let tag = |n: NodeId| self.document.tag(n).unwrap_or("").to_ascii_lowercase();
        let position = |n: NodeId| self.styles.get(n).map(|s| s.position);
        if !self.boxes.contains_key(&id)
            || matches!(tag(id).as_str(), "html" | "body")
            || position(id) == Some(Position::Fixed)
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

fn run_check_layout(server: &TestServer, path: &str, selectors: &[String]) -> Outcome {
    let mut engine = match load(server, path) {
        Ok(engine) => engine,
        Err(error) => return Outcome::fail(error),
    };
    // A frame first: it sets the viewport the layout is for.
    engine.frame(WIDTH, HEIGHT);
    let boxes = boxes(engine.layout(WIDTH as f32));
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
    match load(server, path) {
        Ok(mut engine) => {
            engine.frame(WIDTH, HEIGHT);
            Outcome::pass()
        }
        Err(error) => Outcome::fail(error),
    }
}

/// Runs one test, a panic being a failure.
fn run(server: &TestServer, test: &Test) -> Outcome {
    let ran = panic::catch_unwind(AssertUnwindSafe(|| match test {
        Test::Reftest { path, refs, fuzzy } => run_reftest(server, path, refs, fuzzy),
        Test::CheckLayout { path, selectors } => run_check_layout(server, path, selectors),
        Test::Crashtest { path } => run_crashtest(server, path),
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
}

#[test]
fn the_web_platform_tests_subset_runs_as_expected() {
    let tests = manifest();
    let expected = expectations();
    for path in expected.keys() {
        assert!(
            tests.iter().any(|t| t.path() == path),
            "expectations.txt names {path}, which is not in the manifest"
        );
    }
    let server = serve(root());
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
                    let outcome = run(&server, test);
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
    for (test, outcome) in &results {
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
        "web-platform-tests: {passed} of {} passed, {} expected to fail; report in {}",
        results.len(),
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
        5,
        "a navigation bar, a card grid, the holy grail, media objects and an application shell"
    );
    let server = serve(dir.clone());
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
        };
        let outcome = run(&server, &test);
        if !outcome.passed {
            failures.push(format!("{page}: {}", outcome.message.replace("; ", "\n  ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
