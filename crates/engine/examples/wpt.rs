//! The web-platform-tests subset the engine runs (M17, T1.1; its grid
//! tests M18's, its transforms tests M19's, its media queries,
//! conditional rules and cascade tests M20's, its tests of custom
//! properties, values, lists, counters, generated content, decoration
//! and alignment M21's, its animation, transition and easing tests
//! M22's, its tests with scripts M24's and M25's, and its tests of
//! events, controls and the selectors they match M26's).
//!
//! `cargo run -p engine --example wpt -- pull <checkout>` reads a
//! checkout of web-platform-tests and copies into `tests/wpt/` the tests
//! of the directories below that use only what this engine has, with
//! their references and stylesheets at their own paths, and writes the
//! manifest `tests/wpt.rs` runs and a README saying where they came from
//! and what was left out. The rule (design note `docs/design/flexbox.md`,
//! "The web-platform-tests runner"):
//!
//! - a reftest, a check-layout test or a crashtest, its references and
//!   its stylesheets on the same checkout;
//! - no script but `testharness.js`, `testharnessreport.js` and
//!   `check-layout-th.js`, which the runner stands in for, the helpers
//!   `/common/reftest-wait.js` and `/common/rendering-utils.js`, which
//!   are copied and run (M26), calls to `checkLayout` with a selector in
//!   an inline script or `onload`, and inline scripts and event handler
//!   attributes that use nothing of the platform but `console` (M24),
//!   the DOM, `window` and the interfaces M25 gives, and the events,
//!   timers, animation frames, focus and controls' state M26 gives, none
//!   of the members their design notes leave to other milestones, and
//!   no `data-*` name the runner does not check and no `style` property
//!   the cascade does not take named in them (M25); a test that keeps
//!   `reftest-wait` on is drawn when its scripts take it off (M26);
//! - no element that makes no box here: SVG, canvases, frames, media,
//!   fieldsets, meters and `br`, and no `input` of a type that needs a
//!   picker (G61); an `img` is taken (M23) but one with a
//!   `srcset` or `sizes`, which this engine does not read (G35), and the
//!   images it and its page's stylesheets name, by `url()` or
//!   `image-set()`, are copied with it where they are on the checkout;
//!   no SVG image (M32) and no AVIF one (G34);
//! - every stylesheet only qualified rules whose selectors the matcher
//!   parses, and every declaration one the cascade accepts, as
//!   `style::supports` says, custom properties and declarations with
//!   `var()`, `env()` or `attr()` among them as the cascade takes them
//!   (M21); and no
//!   at-rule but `@charset`, `@import`, whose sheet is on the checkout
//!   and held to the rule too, `@media`, `@supports` and `@layer`
//!   (M20), the rules inside them held to it as well but for those in an
//!   `@supports` block whose condition is false here, which the cascade
//!   drops whatever they hold, and `@keyframes` and `@-webkit-keyframes`
//!   (M22), each keyframe's declarations held to it as a style rule's
//!   are: so no `@font-face`, no `@container`, and no rule inside a
//!   style rule;
//! - no Ahem font, no `dir=rtl`, and only the `data-*` expectations the
//!   runner checks;
//! - in an XHTML file, which this engine reads with its HTML parser (M15),
//!   nothing that parser reads otherwise than an XML parser does: no
//!   CDATA section and no self-closing element that is not void.
//! - no `.sub` file, a template wptserve fills in before it is served,
//!   which the runner's server does not (M23).
//!
//! And the testharness.js tests of the directories of
//! [`TESTHARNESS_DIRECTORIES`] (M27), by another rule, since what such a
//! test needs of the engine shows subtest by subtest, each that fails
//! named in `expectations.txt` with why: what the runner's one server
//! can serve. So an `.any.js` test for a window and for a dedicated
//! worker (M31), as its `global` says, a `.window.js` or `.worker.js`
//! test, or a page that loads `testharness.js`, with the scripts it
//! loads, and no Python handler, no `.sub` template, no other host,
//! nothing over `https:`, no shared or service worker, no module worker
//! (G41), no `blob:` URL (G68), no `SharedArrayBuffer` (G83) and no
//! `BroadcastChannel` (G84). The real `testharness.js` is copied
//! with them, and the files of the directories of
//! [`SUPPORT_DIRECTORIES`] they read, but for their Python handlers and
//! templates.
//!
//! The manifest has a line per test, its fields separated by tabs: the
//! kind and the path, then for a reftest each reference as `==path` or
//! `!=path` and each fuzziness as `fuzzy=ref:a-b;c-d` (an empty `ref`
//! applying to every reference), for a check-layout test its selectors.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};

use dom::{Document, NodeId};

/// The directories of the checkout whose tests are taken.
const DIRECTORIES: &[&str] = &[
    "css/css-flexbox",
    "css/css-grid",
    "css/css-transforms",
    "css/css-will-change",
    "css/mediaqueries",
    "css/css-conditional",
    "css/css-cascade",
    "css/css-variables",
    "css/css-values",
    "css/css-lists",
    "css/css-counter-styles",
    "css/css-content",
    "css/css-text-decor",
    "css/css-text/text-align",
    "css/css-text/text-justify",
    "css/CSS2/lists",
    "css/CSS2/generated-content",
    "css/css-animations",
    "css/css-transitions",
    "css/css-easing",
    "css/css-images",
    "css/css-backgrounds",
    "html/semantics/embedded-content/the-img-element",
    "css/selectors/invalidation",
    "dom/nodes",
    "html/dom",
    "css/selectors",
    "html/semantics/forms",
];

/// The directories whose testharness.js tests are taken (M27), each with
/// the start the name of a test taken must have.
const TESTHARNESS_DIRECTORIES: &[(&str, &str)] = &[
    ("fetch/api/headers", ""),
    ("fetch/api/request", ""),
    ("fetch/api/response", ""),
    ("fetch/api/basic", ""),
    ("xhr", ""),
    ("url", "urlsearchparams-"),
    ("FileAPI/blob", ""),
    ("webstorage", ""),
    // The WebAssembly JavaScript interface (M29): the namespace, its
    // classes and their methods. Not `wasm/core`, Wasm 3.0's own suite:
    // its harness makes a 64-bit table before any test, and its wrappers
    // compare values through GC types, neither of which the interpreter
    // has (docs/design/wasm.md, section 2).
    ("wasm/jsapi", "interface."),
    ("wasm/jsapi", "prototypes."),
    ("wasm/jsapi/constructor", ""),
    ("wasm/jsapi/global", ""),
    ("wasm/jsapi/instance", ""),
    ("wasm/jsapi/memory", ""),
    ("wasm/jsapi/module", ""),
    ("wasm/jsapi/table", ""),
    // The 2D context (M30): the element's and an OffscreenCanvas's for a
    // window, but for the directories of [`CANVAS_LEFT_OUT`].
    ("html/canvas/element", ""),
    ("html/canvas/offscreen", ""),
    // Workers and messages (M31): dedicated workers, channel messaging
    // without frames, and the structured clone.
    ("workers", ""),
    ("webmessaging", ""),
    ("html/infrastructure/safe-passing-of-structured-data", ""),
    ("html/webappapis/structured-clone", ""),
];

/// The directories of the canvas's tests left out, and why: what the
/// design note leaves to later work (docs/design/canvas.md, section 2).
const CANVAS_LEFT_OUT: &[(&str, &str)] = &[
    ("/filters/", "canvas filters (G81)"),
    ("/wide-gamut-canvas/", "colour spaces past sRGB (G81)"),
    ("/color-type/", "colour types past 8 bits (G81)"),
    ("/global-hdr-headroom/", "HDR (G81)"),
    ("/video/", "video (with media)"),
    ("/layers/", "layers (beginLayer, not in the standard)"),
    ("/manual/", "the manual reftests (by the reftest rule)"),
];

/// Helpers that define, among others, functions using another host or
/// a Python handler: taken with a test that calls none of those
/// functions, the helper's own text not held to the rule.
const HOST_HELPERS: &[(&str, &[&str])] = &[(
    "html/canvas/resources/canvas-tests.js",
    &[
        "addCrossOriginYellowImage",
        "addCrossOriginRedirectYellowImage",
        "forEachCanvasSource",
    ],
)];

/// The directories of files the testharness tests read, copied whole
/// but for wptserve's Python handlers and templates.
const SUPPORT_DIRECTORIES: &[&str] = &[
    "fetch/api/resources",
    "fetch/api/cors/resources",
    "fetch/api/request/resources",
    "xhr/resources",
    "FileAPI/support",
    "webstorage/resources",
    "html/canvas/resources",
    "images",
    // M31's directories whole: the scripts a test's workers run are named
    // in its scripts, not by its markup.
    "workers",
    "webmessaging",
    "html/infrastructure/safe-passing-of-structured-data",
    "html/webappapis/structured-clone",
];

/// How deep `@import`s may go; the engine fetches no deeper.
const MAX_IMPORT_DEPTH: usize = 16;

/// Elements that make no box here, or not the box the tests mean.
const NO_BOX: &[&str] = &[
    "svg", "canvas", "iframe", "frame", "frameset", "object", "embed", "video", "audio", "picture",
    "math", "fieldset", "legend", "meter", "progress", "output", "details", "summary", "dialog",
    "marquee", "br", "hr",
];

/// The `input` types drawn as text fields until G61 gives them pickers
/// of their own (M26).
const PICKERS: &[&str] = &[
    "file",
    "color",
    "range",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "image",
];

/// The scripts a test may load: the harness, which the runner stands in
/// for.
const HARNESS: &[&str] = &[
    "testharness.js",
    "testharnessreport.js",
    "check-layout-th.js",
];

/// The helpers a test may load, copied with it and run (M26): what they
/// hold is the reference runner's, and the functions a test calls of
/// them, [`HELPER_FUNCTIONS`], use only what M26 gives.
const HELPERS: &[&str] = &["common/reftest-wait.js", "common/rendering-utils.js"];

/// The functions of [`HELPERS`] a test's script may call.
const HELPER_FUNCTIONS: &[&str] = &[
    "takeScreenshot",
    "takeScreenshotDelayed",
    "waitForAtLeastOneFrame",
    "failIfNot",
];

/// The `data-*` expectations the runner checks.
const CHECKED: &[&str] = &[
    "data-expected-width",
    "data-expected-height",
    "data-offset-x",
    "data-offset-y",
    "data-expected-client-width",
    "data-expected-client-height",
    "data-expected-bounding-client-rect-width",
    "data-expected-bounding-client-rect-height",
    "data-total-x",
    "data-total-y",
    "data-expected-display",
];

/// HTML's void elements, the ones a self-closing tag may close.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [command, checkout] if command == "pull" => match pull(Path::new(checkout)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("wpt pull: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo run -p engine --example wpt -- pull <checkout>");
            ExitCode::from(2)
        }
    }
}

/// How a test is run.
enum Kind {
    /// Rendered and compared with its references.
    Reftest {
        /// Each reference, `true` for one the test must match.
        refs: Vec<(bool, String)>,
        /// The fuzziness metadata, by reference (`None` for every one):
        /// the allowed ranges of the largest channel difference and of
        /// the number of differing pixels.
        fuzzy: Vec<(Option<String>, String)>,
    },
    /// Laid out and checked against its `data-*` attributes.
    CheckLayout { selectors: Vec<String> },
    /// Loaded and rendered, and must not panic.
    Crashtest,
    /// Run under testharness.js, each subtest checked (M27).
    Testharness { long: bool },
}

/// What a page links and declares, and the files it needs.
#[derive(Default)]
struct Page {
    /// `rel=match` and `rel=mismatch` links, resolved.
    refs: Vec<(bool, String)>,
    /// `<meta name=fuzzy>` contents.
    fuzzy: Vec<String>,
    /// The selectors its `checkLayout` calls name.
    selectors: Vec<String>,
    /// Whether it loads `check-layout-th.js`.
    check_layout: bool,
    /// Its stylesheets and the images it and they name, resolved; and
    /// the helper scripts it loads (M26).
    stylesheets: Vec<String>,
    /// The names its inline scripts declare (M26).
    declared: HashSet<String>,
}

fn pull(checkout: &Path) -> Result<(), String> {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt");
    let (commit, date) = upstream(checkout)?;
    let mut manifest = Vec::new();
    let mut files = BTreeSet::new();
    let mut left_out: BTreeMap<String, usize> = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for directory in DIRECTORIES {
        let mut tests = Vec::new();
        walk(&checkout.join(directory), &mut tests)?;
        tests.sort();
        for path in tests {
            let rel = relative(checkout, &path)?;
            if !is_test(&rel) {
                continue;
            }
            match take(checkout, &rel) {
                Ok((kind, needs)) => {
                    let (name, line) = manifest_line(&rel, &kind);
                    *counts.entry(name).or_default() += 1;
                    manifest.push(line);
                    files.insert(rel);
                    files.extend(needs);
                }
                Err(reason) => *left_out.entry(reason).or_default() += 1,
            }
        }
    }
    for (directory, start) in TESTHARNESS_DIRECTORIES {
        let mut tests = Vec::new();
        walk(&checkout.join(directory), &mut tests)?;
        tests.sort();
        for path in tests {
            let rel = relative(checkout, &path)?;
            let name = rel.rsplit('/').next().unwrap_or(&rel);
            if !name.starts_with(start) || !is_testharness_test(name) {
                continue;
            }
            if let Some((_, why)) = CANVAS_LEFT_OUT
                .iter()
                .find(|(part, _)| rel.starts_with("html/canvas/") && rel.contains(part))
            {
                *left_out.entry((*why).to_string()).or_default() += 1;
                continue;
            }
            match take_testharness(checkout, &rel) {
                Ok((pages, long, needs)) => {
                    for page in pages {
                        let (name, line) = manifest_line(&page, &Kind::Testharness { long });
                        *counts.entry(name).or_default() += 1;
                        manifest.push(line);
                    }
                    files.insert(rel);
                    files.extend(needs);
                }
                Err(reason) => *left_out.entry(reason).or_default() += 1,
            }
        }
    }
    if counts.contains_key("testharness") {
        files.insert("resources/testharness.js".to_string());
        // The data the tests read beside them: anything but a page, a
        // script, a Python handler or a template.
        for (directory, _) in TESTHARNESS_DIRECTORIES {
            let mut data = Vec::new();
            walk(&checkout.join(directory), &mut data)?;
            for path in data {
                let rel = relative(checkout, &path)?;
                let name = rel.rsplit('/').next().unwrap_or(&rel);
                let kind = name.rsplit('.').next().unwrap_or("");
                // A canvas test's picture of its expected output, named
                // as it is, which only a person looks at.
                let expected = kind == "png"
                    && path.with_extension("html").is_file()
                    && rel.starts_with("html/canvas/");
                if !matches!(kind, "html" | "htm" | "xhtml" | "js" | "py" | "yml" | "md")
                    && !name.contains(".sub.")
                    && !expected
                {
                    files.insert(rel);
                }
            }
        }
        for directory in SUPPORT_DIRECTORIES {
            let mut support = Vec::new();
            walk_all(&checkout.join(directory), &mut support)?;
            for path in support {
                let rel = relative(checkout, &path)?;
                let name = rel.rsplit('/').next().unwrap_or(&rel);
                if !name.ends_with(".py") && !name.contains(".sub.") {
                    files.insert(rel);
                }
            }
        }
    }
    // The vendored files are replaced wholesale, every directory of
    // them, the images' too; the expectations stay.
    for entry in fs::read_dir(&out).map_err(|e| format!("{}: {e}", out.display()))? {
        let old = entry.map_err(|e| e.to_string())?.path();
        if old.is_dir() {
            fs::remove_dir_all(&old).map_err(|e| format!("{}: {e}", old.display()))?;
        }
    }
    for file in &files {
        let to = out.join(file);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::copy(checkout.join(file), &to).map_err(|e| format!("{file}: {e}"))?;
    }
    let licence =
        fs::read_to_string(checkout.join("LICENSE.md")).map_err(|e| format!("LICENSE.md: {e}"))?;
    fs::write(out.join("LICENSE.md"), licence).map_err(|e| e.to_string())?;
    let mut text = String::from(
        "# The engine's web-platform-tests subset: kind, path, then for a\n\
         # reftest its references and fuzziness, for a check-layout test its\n\
         # selectors, separated by tabs. Written by `wpt pull`.\n",
    );
    for line in &manifest {
        text.push_str(line);
        text.push('\n');
    }
    fs::write(out.join("MANIFEST"), text).map_err(|e| e.to_string())?;
    fs::write(
        out.join("README.md"),
        readme(&commit, &date, &counts, &left_out),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "{} tests ({}), {} files, from {commit}",
        manifest.len(),
        counts
            .iter()
            .map(|(kind, n)| format!("{n} {kind}"))
            .collect::<Vec<_>>()
            .join(", "),
        files.len()
    );
    Ok(())
}

/// The checkout's commit and its date.
fn upstream(checkout: &Path) -> Result<(String, String), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["log", "-1", "--format=%H %cs"])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut words = text.split_whitespace();
    match (words.next(), words.next()) {
        (Some(commit), Some(date)) if output.status.success() => {
            Ok((commit.to_string(), date.to_string()))
        }
        _ => Err(format!("{} is not a git checkout", checkout.display())),
    }
}

/// Every file under `dir`, but in the directories that hold references
/// and support files rather than tests.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !matches!(name, "reference" | "support" | "resources") {
                walk(&path, out)?;
            }
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Every file under `dir`, in every directory.
fn walk_all(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            walk_all(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Whether the file `name` is a testharness test, as wptserve names
/// them: an `.any.js`, `.window.js` or `.worker.js` test (M31), or a
/// page.
fn is_testharness_test(name: &str) -> bool {
    let page = [".html", ".htm", ".xhtml"]
        .iter()
        .any(|ext| name.ends_with(ext));
    let stem = name.split('.').next().unwrap_or(name);
    (name.ends_with(".any.js")
        || name.ends_with(".window.js")
        || name.ends_with(".worker.js")
        || page)
        && !stem.ends_with("-manual")
        && !stem.ends_with("-ref")
}

/// The `// META:` lines at the top of an `.any.js` or `.window.js`
/// test: each name and value.
fn metas(source: &str) -> Vec<(String, String)> {
    let mut metas = Vec::new();
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
        if let Some((name, value)) = meta.trim().split_once('=') {
            metas.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    metas
}

/// The directories of M31's tests: held to [`workers_rule`] besides.
const WORKER_DIRECTORIES: &[&str] = &[
    "workers/",
    "webmessaging/",
    "html/infrastructure/safe-passing-of-structured-data/",
    "html/webappapis/structured-clone/",
];

/// Fails on what M31 leaves to others (docs/design/workers.md, section
/// 2): a module worker, a `blob:` URL, a `SharedArrayBuffer` and a
/// `BroadcastChannel`; a `helper` script's `SharedArrayBuffer` taken, as
/// `/common/sab.js` makes one only when a test asks.
fn workers_rule(text: &str, helper: bool) -> Result<(), String> {
    let module = [
        "type: 'module'",
        "type: \"module\"",
        "type:'module'",
        "type:\"module\"",
    ];
    if module.iter().any(|m| text.contains(m)) {
        return Err("a module worker (G41)".to_string());
    }
    if text.contains("createObjectURL") {
        return Err("a blob: URL (G68)".to_string());
    }
    if !helper && (text.contains("SharedArrayBuffer") || text.contains("Atomics.")) {
        return Err("SharedArrayBuffer (G83)".to_string());
    }
    if text.contains("BroadcastChannel") {
        return Err("BroadcastChannel (G84)".to_string());
    }
    Ok(())
}

/// A script's text without its `//` comment lines: a generated worker
/// test names its generator (`gentest.py`) in them (M31).
fn without_line_comments(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` without its HTML comments: what a generated test says it was
/// generated by (`gentest.py`, the canvas's) is no file it loads.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Fails on what only wptserve can serve: a Python handler, a pipe
/// that shapes a response, or a page of another host than the runner's
/// one; and on what has no global here: a worker, or a frame or a
/// window a script makes.
fn served_here(text: &str) -> Result<(), String> {
    if text.contains(".py") {
        return Err("Python handler".to_string());
    }
    if text.contains("pipe=") {
        return Err("wptserve pipe".to_string());
    }
    if ["SharedWorker", "serviceWorker", "ServiceWorker"]
        .iter()
        .any(|worker| text.contains(worker))
    {
        return Err("a shared or service worker".to_string());
    }
    // In any case: a tag name is (`createElement("IFRAME")`, M28).
    let lower = text.to_ascii_lowercase();
    if [
        "createelement(\"iframe\")",
        "createelement('iframe')",
        "window.open(",
        ".contentwindow",
    ]
    .iter()
    .any(|frame| lower.contains(frame))
    {
        return Err("a frame".to_string());
    }
    let hosts = [
        "get_host_info",
        "{{host",
        "{{domains",
        "{{hosts",
        "REMOTE_ORIGIN",
        "REMOTE_HOST",
        "OTHER_ORIGIN",
        "www1.",
        "www2.",
    ];
    if hosts.iter().any(|host| text.contains(host)) {
        return Err("another host".to_string());
    }
    Ok(())
}

/// The testharness test at `rel`, if the rule takes it: the pages the
/// runner loads (for an `.any.js`, `.window.js` or `.worker.js` test,
/// those wptserve makes of it: an `.any.js` one's for a window and for a
/// dedicated worker, as its `global` says, M31), whether its timeout is
/// long, and the scripts it loads; else why it is left out.
fn take_testharness(
    checkout: &Path,
    rel: &str,
) -> Result<(Vec<String>, bool, Vec<String>), String> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if name.contains(".sub.") {
        return Err("wptserve substitution (.sub)".to_string());
    }
    if name.contains(".https.") || name.contains(".h2.") {
        return Err("https:".to_string());
    }
    if [".serviceworker.", ".sharedworker.", ".worker-module."]
        .iter()
        .any(|worker| name.contains(worker))
        || (name.contains(".worker.") && !name.ends_with(".worker.js"))
    {
        return Err("a shared or service worker".to_string());
    }
    let source = fs::read(checkout.join(rel)).map_err(|_| format!("missing file {rel}"))?;
    let source = String::from_utf8_lossy(&source).into_owned();
    let text = if rel.ends_with(".js") {
        without_line_comments(&source)
    } else {
        without_comments(&source)
    };
    served_here(&text)?;
    let workers = WORKER_DIRECTORIES.iter().any(|d| rel.starts_with(d));
    if workers {
        if rel.contains("/broadcastchannel/") {
            return Err("BroadcastChannel (G84)".to_string());
        }
        // A module worker made through a helper, its type a variable:
        // the test's path names it.
        if rel.contains("module") {
            return Err("a module worker (G41)".to_string());
        }
        workers_rule(&text, false)?;
    }
    let mut scripts = Vec::new();
    let (pages, long) = if let Some(stem) = rel
        .strip_suffix(".any.js")
        .map(|stem| (stem, ".any"))
        .or_else(|| rel.strip_suffix(".window.js").map(|stem| (stem, ".window")))
        .or_else(|| rel.strip_suffix(".worker.js").map(|stem| (stem, ".worker")))
    {
        let metas = metas(&source);
        let globals: Vec<&str> = metas
            .iter()
            .filter(|(name, _)| name == "global")
            .flat_map(|(_, value)| value.split(','))
            .map(str::trim)
            .collect();
        // An `.any.js` test is for a window and a dedicated worker when
        // it names no global, as wptserve's default is.
        let names =
            |wanted: &[&str]| globals.is_empty() || globals.iter().any(|g| wanted.contains(g));
        let pages = match stem.1 {
            ".any" => {
                let mut pages = Vec::new();
                if names(&["window", "default"]) {
                    pages.push(format!("{}.any.html", stem.0));
                }
                if names(&["worker", "dedicatedworker", "default"]) {
                    pages.push(format!("{}.any.worker.html", stem.0));
                }
                if pages.is_empty() {
                    return Err("not for a window or a dedicated worker".to_string());
                }
                pages
            }
            kind => vec![format!("{}{kind}.html", stem.0)],
        };
        for (name, value) in &metas {
            if name == "script" {
                scripts.push(resolve(rel, value)?);
            }
        }
        let long = metas
            .iter()
            .any(|(name, value)| name == "timeout" && value == "long");
        (pages, long)
    } else {
        let node = html::parse_document(&source).map_err(|e| format!("unparsed page: {e:?}"))?;
        let document = Document::from_node(&node);
        let mut harness = false;
        let mut long = false;
        for id in document.subtree(document.root()) {
            match document.tag(id).map(str::to_ascii_lowercase).as_deref() {
                Some("script") => {
                    if let Some(src) = document.attr(id, "src") {
                        let name = src.rsplit('/').next().unwrap_or(src);
                        if name == "testharness.js" {
                            harness = true;
                        } else if !HARNESS.contains(&name) {
                            scripts.push(resolve(rel, src)?);
                        }
                    }
                }
                Some("meta") => {
                    long |= document
                        .attr(id, "name")
                        .is_some_and(|n| n.eq_ignore_ascii_case("timeout"))
                        && document.attr(id, "content") == Some("long");
                }
                Some("iframe" | "frame" | "object" | "embed") => {
                    return Err("a frame".to_string());
                }
                _ => {}
            }
        }
        if !harness {
            return Err("not a testharness test".to_string());
        }
        (vec![rel.to_string()], long)
    };
    for script in &scripts {
        if script.contains(".sub.") {
            return Err("wptserve substitution (.sub)".to_string());
        }
        if let Some((_, functions)) = HOST_HELPERS.iter().find(|(helper, _)| helper == script) {
            if functions.iter().any(|function| source.contains(function)) {
                return Err("another host".to_string());
            }
            continue;
        }
        let text = fs::read(checkout.join(script)).map_err(|_| format!("missing file {script}"))?;
        let text = String::from_utf8_lossy(&text);
        served_here(&text)?;
        if workers {
            workers_rule(&text, true)?;
        }
    }
    Ok((pages, long, scripts))
}

/// `path` relative to the checkout, with `/` between its parts.
fn relative(checkout: &Path, path: &Path) -> Result<String, String> {
    let rel = path
        .strip_prefix(checkout)
        .map_err(|_| format!("{} is outside the checkout", path.display()))?;
    Ok(rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/"))
}

/// Whether `rel` names a test rather than a reference: a page whose name
/// does not say it is one.
fn is_test(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let page = [".html", ".htm", ".xht", ".xhtml"]
        .iter()
        .any(|ext| name.ends_with(ext));
    let stem = name.split('.').next().unwrap_or(name);
    // A `-manual` test needs a person at the pointer, and wptrunner
    // does not run it either.
    page && !stem.ends_with("-ref")
        && !stem.ends_with("-notref")
        && !stem.contains("-ref-")
        && !stem.ends_with("-print")
        && !stem.ends_with("-manual")
}

/// The test at `rel`, if the rule takes it: how it is run and the files
/// it needs besides itself; else why it is left out.
fn take(checkout: &Path, rel: &str) -> Result<(Kind, Vec<String>), String> {
    // A `.sub` file is a template wptserve fills in with its hosts and
    // ports, which the runner's one server does not.
    if rel
        .rsplit('/')
        .next()
        .is_some_and(|name| name.contains(".sub."))
    {
        return Err("wptserve substitution (.sub)".to_string());
    }
    let page = read_page(checkout, rel)?;
    let mut needs = page.stylesheets.clone();
    if !page.refs.is_empty() {
        for (_, reference) in &page.refs {
            let referenced = read_page(checkout, reference)?;
            needs.push(reference.clone());
            needs.extend(referenced.stylesheets);
        }
        let fuzzy = page
            .fuzzy
            .iter()
            .map(|content| parse_fuzzy(rel, content))
            .collect::<Result<Vec<_>, String>>()?;
        return Ok((
            Kind::Reftest {
                refs: page.refs,
                fuzzy,
            },
            needs,
        ));
    }
    if page.check_layout && !page.selectors.is_empty() {
        return Ok((
            Kind::CheckLayout {
                selectors: page.selectors,
            },
            needs,
        ));
    }
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if rel.contains("/crashtests/") || name.contains("-crash.") {
        return Ok((Kind::Crashtest, needs));
    }
    Err("not a reftest, a check-layout test or a crashtest".to_string())
}

/// Reads the page at `rel` and checks it against the rule.
fn read_page(checkout: &Path, rel: &str) -> Result<Page, String> {
    let source = fs::read(checkout.join(rel)).map_err(|_| format!("missing file {rel}"))?;
    let source = String::from_utf8_lossy(&source);
    if rel.ends_with(".xht") || rel.ends_with(".xhtml") {
        xml_only(&source)?;
    }
    let node = html::parse_document(&source).map_err(|e| format!("unparsed page: {e:?}"))?;
    let document = Document::from_node(&node);
    let mut page = Page {
        declared: page_declarations(&document),
        ..Page::default()
    };
    for id in document.subtree(document.root()) {
        if !document.is_element(id) {
            continue;
        }
        element(checkout, rel, &document, id, &mut page)?;
    }
    Ok(page)
}

/// Checks one element of the page at `rel`, and takes what it links and
/// declares into `page`.
fn element(
    checkout: &Path,
    rel: &str,
    document: &Document,
    id: NodeId,
    page: &mut Page,
) -> Result<(), String> {
    // Any namespace's: an `<svg>` is SVG's, a `<math>` MathML's.
    let tag = document.local_name(id).unwrap_or("").to_ascii_lowercase();
    if NO_BOX.contains(&tag.as_str()) {
        return Err(format!("element <{tag}>"));
    }
    if tag == "input"
        && let Some(kind) = document.attr(id, "type")
        && PICKERS.contains(&kind.trim().to_ascii_lowercase().as_str())
    {
        return Err(format!("input type={}", kind.trim().to_ascii_lowercase()));
    }
    for (name, value) in document.attrs(id).unwrap_or(&[]) {
        let name = name.to_ascii_lowercase();
        if name == "dir" && value.trim().eq_ignore_ascii_case("rtl") {
            return Err("dir=rtl".to_string());
        }
        if name.starts_with("on") {
            match check_layout_calls(value) {
                Ok(selectors) if name == "onload" => page.selectors.extend(selectors),
                _ if uses_only_m26_on(value, &page.declared) => writes_supported(value)?,
                _ => return Err("script".to_string()),
            }
        }
        if name.starts_with("data-")
            && !name.starts_with("data-test")
            && !CHECKED.contains(&name.as_str())
        {
            return Err(format!("unchecked attribute {name}"));
        }
        if name == "style" {
            let declarations = css::parse_style_attribute(value).unwrap_or_default();
            declarations_supported(&declarations)?;
            for d in &declarations {
                images(checkout, rel, &d.value, &mut page.stylesheets)?;
            }
        }
    }
    if tag == "img" {
        if document.attr(id, "srcset").is_some() || document.attr(id, "sizes").is_some() {
            return Err("img srcset".to_string());
        }
        if let Some(src) = document.attr(id, "src") {
            unsupported_image(src)?;
        }
        if let Some(src) = document.attr(id, "src")
            && !is_data(src)
            && !src.trim().is_empty()
        {
            let path = resolve(rel, src)?;
            if checkout.join(&path).is_file() && !page.stylesheets.contains(&path) {
                page.stylesheets.push(path);
            }
        }
    }
    match tag.as_str() {
        "style" => sheet_supported(
            checkout,
            rel,
            &document.text_content(id),
            0,
            &mut page.stylesheets,
        ),
        "script" => match document.attr(id, "src") {
            Some(src) => {
                let name = src.rsplit('/').next().unwrap_or(src);
                if HARNESS.contains(&name) {
                    page.check_layout |= name == "check-layout-th.js";
                    Ok(())
                } else if let Ok(path) = resolve(rel, src)
                    && HELPERS.contains(&path.as_str())
                {
                    if !page.stylesheets.contains(&path) {
                        page.stylesheets.push(path);
                    }
                    Ok(())
                } else {
                    Err("script".to_string())
                }
            }
            None => {
                let text = document.text_content(id);
                match check_layout_calls(&text) {
                    Ok(selectors) => page.selectors.extend(selectors),
                    Err(_) if uses_only_m26_on(&text, &page.declared) => {
                        writes_supported(&text)?;
                    }
                    Err(error) => return Err(error),
                }
                Ok(())
            }
        },
        "link" => {
            let (Some(rels), Some(href)) = (document.attr(id, "rel"), document.attr(id, "href"))
            else {
                return Ok(());
            };
            for kind in rels.split_whitespace() {
                match kind.to_ascii_lowercase().as_str() {
                    "stylesheet" => {
                        let path = resolve(rel, href)?;
                        let text = fs::read_to_string(checkout.join(&path))
                            .map_err(|_| format!("missing file {path}"))?;
                        sheet_supported(checkout, &path, &text, 0, &mut page.stylesheets)?;
                        page.stylesheets.push(path);
                    }
                    "match" => page.refs.push((true, resolve(rel, href)?)),
                    "mismatch" => page.refs.push((false, resolve(rel, href)?)),
                    _ => {}
                }
            }
            Ok(())
        }
        "meta" => {
            if document
                .attr(id, "name")
                .is_some_and(|n| n.eq_ignore_ascii_case("fuzzy"))
                && let Some(content) = document.attr(id, "content")
            {
                page.fuzzy.push(content.to_string());
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Fails on what an XML parser reads one way and the HTML parser
/// another: a CDATA section, whose markers stay in an HTML element's
/// text (in a stylesheet, swallowing its rules), and a self-closing tag
/// of an element that is not void, which the HTML parser leaves open.
fn xml_only(source: &str) -> Result<(), String> {
    if source.contains("<![CDATA[") {
        return Err("CDATA section in XHTML".to_string());
    }
    let mut rest = source;
    while let Some(at) = rest.find("/>") {
        let before = &rest[..at];
        if let Some(open) = before.rfind('<') {
            let tag: String = before[open + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if !tag.is_empty() && !VOID.contains(&tag.as_str()) {
                return Err(format!("self-closing <{tag}/> in XHTML"));
            }
        }
        rest = &rest[at + 2..];
    }
    Ok(())
}

/// The selectors of a script made of nothing but `checkLayout` calls,
/// `checkLayout('selector')` or `window.checkLayout("selector", true)`,
/// each perhaps followed by `;`.
fn check_layout_calls(script: &str) -> Result<Vec<String>, String> {
    let not = || Err("script".to_string());
    let mut selectors = Vec::new();
    let mut rest = strip_comments(script);
    loop {
        let s = rest.trim_start();
        if s.is_empty() {
            return Ok(selectors);
        }
        let s = s.strip_prefix("window.").unwrap_or(s);
        let Some(s) = s.strip_prefix("checkLayout") else {
            return not();
        };
        let Some(s) = s.trim_start().strip_prefix('(') else {
            return not();
        };
        let s = s.trim_start();
        let Some(quote) = s.chars().next().filter(|q| *q == '\'' || *q == '"') else {
            return not();
        };
        let Some(end) = s[1..].find(quote) else {
            return not();
        };
        let selector = &s[1..1 + end];
        style::parse_selector_list(&css::tokenize(selector))
            .map_err(|_| format!("selector {selector}"))?;
        selectors.push(selector.to_string());
        let mut s = s[end + 2..].trim_start();
        if let Some(after) = s.strip_prefix(',') {
            let after = after.trim_start();
            s = after
                .strip_prefix("true")
                .or_else(|| after.strip_prefix("false"))
                .map_or(after, str::trim_start);
        }
        let Some(s) = s.strip_prefix(')') else {
            return not();
        };
        let s = s.trim_start();
        rest = s.strip_prefix(';').unwrap_or(s).to_string();
    }
}

/// The names a script may use as M24 ran it: ECMAScript's keywords and
/// global objects, and `console`.
const ECMASCRIPT: &[&str] = &[
    "arguments",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "get",
    "if",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "undefined",
    "NaN",
    "Infinity",
    "globalThis",
    "Math",
    "JSON",
    "Object",
    "Array",
    "String",
    "Number",
    "Boolean",
    "Symbol",
    "BigInt",
    "Promise",
    "Proxy",
    "Reflect",
    "Error",
    "EvalError",
    "RangeError",
    "ReferenceError",
    "SyntaxError",
    "TypeError",
    "URIError",
    "AggregateError",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "WeakRef",
    "Date",
    "RegExp",
    "ArrayBuffer",
    "DataView",
    "Uint8Array",
    "Int32Array",
    "Float64Array",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "decodeURIComponent",
    "console",
];

/// The names of the platform a script may use as M26 runs it, beside
/// ECMAScript's: the document, the window and what it reports, and the
/// interfaces the bindings expose (docs/design/dom-scripting.md, section
/// 1), each element interface among them by its `HTML...Element` name;
/// and M26's (docs/design/events.md, section 1): the event interfaces,
/// timers and animation frames, microtasks, `performance`, the abort
/// signals, the controls' collections and validity, and the window's
/// `event`.
const M26_GLOBALS: &[&str] = &[
    "Event",
    "CustomEvent",
    "UIEvent",
    "FocusEvent",
    "MouseEvent",
    "PointerEvent",
    "WheelEvent",
    "KeyboardEvent",
    "InputEvent",
    "SubmitEvent",
    "ErrorEvent",
    "PageTransitionEvent",
    "AnimationEvent",
    "TransitionEvent",
    "AbortController",
    "AbortSignal",
    "ValidityState",
    "HTMLFormControlsCollection",
    "HTMLOptionsCollection",
    "RadioNodeList",
    "Performance",
    "performance",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "queueMicrotask",
    "reportError",
    "event",
    "document",
    "window",
    "self",
    "location",
    "navigator",
    "screen",
    "getComputedStyle",
    "innerWidth",
    "innerHeight",
    "outerWidth",
    "outerHeight",
    "devicePixelRatio",
    "DOMException",
    "EventTarget",
    "Node",
    "CharacterData",
    "Text",
    "Element",
    "Document",
    "HTMLDocument",
    "NodeList",
    "HTMLCollection",
    "DOMTokenList",
    "CSSStyleDeclaration",
    "DOMStringMap",
    "Location",
    "Navigator",
    "Screen",
    "Window",
];

/// The members a script may not use as M26 runs it, by the names a test
/// reaches them by after a dot: those M25's design note leaves to other
/// milestones (section 2): a layout flush and CSSOM View's layout
/// queries (G46), more kinds of node (G47), `document.open` and `close`
/// (G48), the CSSOM's sheets (G50), namespaces (G51), traversal and
/// ranges (G52), shadow trees (G53); M26's: pickers and the values they
/// give (G61), pointer capture (G58), editing hosts (G59); DOM's
/// `moveBefore` (G66); and those of what neither has a part of:
/// animations' objects, fonts, media, canvases and frames.
const NOT_M26: &[&str] = &[
    "offsetWidth",
    "offsetHeight",
    "offsetTop",
    "offsetLeft",
    "offsetParent",
    "clientWidth",
    "clientHeight",
    "clientTop",
    "clientLeft",
    "scrollWidth",
    "scrollHeight",
    "scrollTop",
    "scrollLeft",
    "scrollX",
    "scrollY",
    "pageXOffset",
    "pageYOffset",
    "getBoundingClientRect",
    "getClientRects",
    "scrollIntoView",
    "scrollTo",
    "scrollBy",
    "scroll",
    "innerText",
    "outerText",
    "createComment",
    "createDocumentFragment",
    "createProcessingInstruction",
    "createCDATASection",
    "createAttribute",
    "importNode",
    "adoptNode",
    "attributes",
    "getAttributeNode",
    "setAttributeNode",
    "content",
    "doctype",
    "open",
    "close",
    "styleSheets",
    "sheet",
    "cssRules",
    "insertRule",
    "deleteRule",
    "adoptedStyleSheets",
    "replaceSync",
    "createElementNS",
    "getElementsByTagNameNS",
    "setAttributeNS",
    "getAttributeNS",
    "namespaceURI",
    "createTreeWalker",
    "createNodeIterator",
    "createRange",
    "getSelection",
    "compareDocumentPosition",
    "isEqualNode",
    "normalize",
    "moveBefore",
    "valueAsNumber",
    "valueAsDate",
    "stepUp",
    "stepDown",
    "showPicker",
    "files",
    "setRangeText",
    "setPointerCapture",
    "releasePointerCapture",
    "contentEditable",
    "isContentEditable",
    "designMode",
    "attachShadow",
    "shadowRoot",
    "animate",
    "getAnimations",
    "timeline",
    "fonts",
    "computedStyleMap",
    "attributeStyleMap",
    "matchMedia",
    "naturalWidth",
    "naturalHeight",
    "complete",
    "decode",
    "play",
    "getContext",
    "contentDocument",
    "contentWindow",
    "elementFromPoint",
    "elementsFromPoint",
    "showModal",
    "showPopover",
    "hidePopover",
    "togglePopover",
    "checkVisibility",
    "requestFullscreen",
    "execCommand",
];

/// Whether `script` uses nothing of the platform but `console` and what
/// M25 and M26 give a script: each name it uses but after a dot is
/// ECMAScript's, `console`, one of [`M26_GLOBALS`] or an element
/// interface's, a helper's ([`HELPER_FUNCTIONS`]), or one it declares
/// (by `var`, `let`, `const`, `function` or `class`, or as a function's
/// parameter); and none after a dot is one of [`NOT_M26`], but for a
/// layout query read as a statement of its own ([`flushes`]). Its
/// comments and strings are passed over; one with a template
/// substitution, or a string that does not end, is not read and not
/// taken.
#[cfg(test)]
fn uses_only_m26(script: &str) -> bool {
    uses_only_m26_on(script, &HashSet::new())
}

/// [`uses_only_m26`] for a script of a page whose scripts declare
/// `page`'s names, which it may use too: a handler calling a function a
/// script declares, or one script another's.
fn uses_only_m26_on(script: &str, page: &HashSet<String>) -> bool {
    let Some(code) = code_only(script) else {
        return false;
    };
    let words = words(&code);
    let mut declared = declared(&code, &words);
    // A function's parameters: the words between the parenthesis after
    // `function` (and its name) or before `=>`, and a word before `=>`.
    for (open, close) in parameter_lists(&code) {
        for &(name, end, _) in &words {
            if end > open && end <= close {
                declared.insert(name);
            }
        }
    }
    for &(name, end, _) in &words {
        if code[end..].trim_start().starts_with("=>") {
            declared.insert(name);
        }
    }
    // An object literal's key: after `{` or `,`, before `:`.
    let key = |name: &str, end: usize| {
        code[..end - name.len()].trim_end().ends_with(['{', ','])
            && code[end..].trim_start().starts_with(':')
    };
    words.iter().all(|&(name, end, after_dot)| {
        if after_dot {
            !NOT_M26.contains(&name) || flushes(&code, name, end)
        } else if key(name, end) {
            true
        } else {
            ECMASCRIPT.contains(&name)
                || M26_GLOBALS.contains(&name)
                || HELPER_FUNCTIONS.contains(&name)
                || is_element_interface(name)
                || declared.contains(name)
                || page.contains(name)
        }
    })
}

/// Each word of `code`, a script's code alone: where it starts and ends,
/// and whether a dot is before it.
fn words(code: &str) -> Vec<(&str, usize, bool)> {
    let mut words = Vec::new();
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
        if !word(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && word(bytes[i]) {
            i += 1;
        }
        if bytes[start].is_ascii_digit() {
            continue;
        }
        let before = code[..start].trim_end();
        let after_dot = before.ends_with('.') && !before.ends_with("..");
        words.push((&code[start..i], i, after_dot));
    }
    words
}

/// The names `code`'s `var`, `let`, `const`, `function` and `class`
/// declare: a name declared follows its keyword with nothing but white
/// space between, so not the first word of an anonymous function's body.
fn declared<'a>(code: &str, words: &[(&'a str, usize, bool)]) -> HashSet<&'a str> {
    let mut declared = HashSet::new();
    for pair in words.windows(2) {
        let ((keyword, end, _), (name, name_end, _)) = (pair[0], pair[1]);
        if matches!(keyword, "var" | "let" | "const" | "function" | "class")
            && code[end..name_end - name.len()].trim().is_empty()
        {
            declared.insert(name);
        }
    }
    declared
}

/// The names a page's inline scripts declare, which its other scripts
/// and its event handlers may use.
fn page_declarations(document: &Document) -> HashSet<String> {
    let mut names = HashSet::new();
    for id in document.subtree(document.root()) {
        if document.tag(id) == Some("script")
            && document.attr(id, "src").is_none()
            && let Some(code) = code_only(&document.text_content(id))
        {
            names.extend(
                declared(&code, &words(&code))
                    .into_iter()
                    .map(str::to_string),
            );
        }
    }
    names
}

/// The layout queries a test reads to have the reference browser bring
/// its style and layout up to date before what it changes next, as
/// `document.body.offsetTop;`: here a script's changes all reach the
/// first frame, and the value, which M25 does not give (G46), is not
/// used.
const FLUSHES: &[&str] = &[
    "offsetTop",
    "offsetLeft",
    "offsetWidth",
    "offsetHeight",
    "clientTop",
    "clientLeft",
    "clientWidth",
    "clientHeight",
    "scrollTop",
    "scrollLeft",
    "scrollWidth",
    "scrollHeight",
];

/// Whether `name`, a member of [`FLUSHES`] ending at `end` in `code`, is
/// read as a statement of its own: the chain of names and dots before it
/// starts the statement, and the statement ends after it.
fn flushes(code: &str, name: &str, end: usize) -> bool {
    if !FLUSHES.contains(&name) {
        return false;
    }
    let chain = code[..end]
        .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'));
    let before = chain.trim_end_matches([' ', '\t']);
    let after = code[end..].trim_start_matches([' ', '\t']);
    (before.is_empty() || before.ends_with([';', '{', '}', '\n', '\r']))
        && (after.is_empty() || after.starts_with([';', '}', '\n', '\r']))
}

/// Whether what `script` names for its writes is held to the rule as
/// the page's own markup is: each `data-*` attribute it names, by its
/// name or through `dataset`, one the runner checks, and each property
/// it sets through `style`, by its IDL name or `setProperty`, one the
/// cascade takes. The values, which may be computed, are not read.
fn writes_supported(script: &str) -> Result<(), String> {
    let name_char = |c: char| c.is_ascii_alphanumeric() || c == '-';
    let mut attributes: Vec<String> = script
        .match_indices("data-")
        .map(|(at, _)| {
            let rest = &script[at + "data-".len()..];
            let end = rest.find(|c| !name_char(c)).unwrap_or(rest.len());
            format!("data-{}", &rest[..end])
        })
        .collect();
    let mut properties: Vec<String> = Vec::new();
    for (at, _) in script.match_indices("setProperty(") {
        let rest = script[at + "setProperty(".len()..].trim_start();
        if let Some(rest) = rest.strip_prefix(['\'', '"']) {
            let end = rest.find(|c| !name_char(c)).unwrap_or(rest.len());
            properties.push(rest[..end].to_ascii_lowercase());
        }
    }
    let code = code_only(script).unwrap_or_default();
    for (prefix, into_attribute) in [(".dataset.", true), (".style.", false)] {
        for (at, _) in code.match_indices(prefix) {
            let rest = &code[at + prefix.len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                .unwrap_or(rest.len());
            let name = kebab(&rest[..end]);
            if into_attribute {
                attributes.push(format!("data-{name}"));
            } else if !matches!(
                name.as_str(),
                "css-text"
                    | "set-property"
                    | "remove-property"
                    | "get-property-value"
                    | "length"
                    | "item"
                    | "css-float"
                    | ""
            ) {
                properties.push(name);
            }
        }
    }
    for name in attributes {
        if name != "data-" && !name.starts_with("data-test") && !CHECKED.contains(&name.as_str()) {
            return Err(format!("unchecked attribute {name}"));
        }
    }
    for name in properties {
        if name.starts_with("--") {
            continue;
        }
        let declarations =
            css::parse_style_attribute(&format!("{name}: initial")).unwrap_or_default();
        if declarations.is_empty() {
            return Err(format!("declaration of {name}"));
        }
        declarations_supported(&declarations)?;
    }
    Ok(())
}

/// A camel-cased IDL name as the hyphenated name it stands for:
/// `writingMode` as `writing-mode`, `webkitFoo` as `-webkit-foo`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    if name.starts_with("webkit") {
        out.push('-');
    }
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether `name` is an HTML element interface's, `HTMLElement` and
/// `HTMLDivElement` among them.
fn is_element_interface(name: &str) -> bool {
    name.strip_prefix("HTML")
        .and_then(|rest| rest.strip_suffix("Element"))
        .is_some_and(|middle| middle.bytes().all(|b| b.is_ascii_alphabetic()))
}

/// `script`'s code alone: its comments gone and each string blanked, so
/// that nothing in either reads as a name; `None` for a template with a
/// substitution, or a string or comment that does not end.
fn code_only(script: &str) -> Option<String> {
    let mut out = String::with_capacity(script.len());
    let mut chars = script.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut star = false;
                loop {
                    let next = chars.next()?;
                    if star && next == '/' {
                        break;
                    }
                    star = next == '*';
                }
                out.push(' ');
            }
            '\'' | '"' | '`' => {
                out.push_str("''");
                let mut escaped = false;
                loop {
                    let next = chars.next()?;
                    if escaped {
                        escaped = false;
                    } else if next == '\\' {
                        escaped = true;
                    } else if next == c {
                        break;
                    } else if c == '`' && next == '$' && chars.peek() == Some(&'{') {
                        return None;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    Some(out)
}

/// The byte ranges of `code`'s parameter lists: the parentheses after
/// `function` and its name, and those just before `=>`.
fn parameter_lists(code: &str) -> Vec<(usize, usize)> {
    let mut lists = Vec::new();
    let matching = |open: usize| {
        let mut depth = 0;
        for (i, c) in code[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(open + i);
                    }
                }
                _ => {}
            }
        }
        None
    };
    for (at, _) in code.match_indices("function") {
        if let Some(open) = code[at..].find('(').map(|o| at + o)
            && let Some(close) = matching(open)
        {
            lists.push((open, close));
        }
    }
    for (at, _) in code.match_indices("=>") {
        let before = code[..at].trim_end();
        if before.ends_with(')') {
            let close = before.len() - 1;
            let mut depth = 0;
            for (i, c) in code[..=close].char_indices().rev() {
                match c {
                    ')' => depth += 1,
                    '(' => {
                        depth -= 1;
                        if depth == 0 {
                            lists.push((i, close));
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    lists
}

/// `script` without its `//` and `/* */` comments.
fn strip_comments(script: &str) -> String {
    let mut out = String::new();
    let mut rest = script;
    loop {
        let line = rest.find("//");
        let block = rest.find("/*");
        match (line, block) {
            (Some(l), b) if b.is_none_or(|b| l < b) => {
                out.push_str(&rest[..l]);
                rest = rest[l..].find('\n').map_or("", |n| &rest[l + n..]);
            }
            (_, Some(b)) => {
                out.push_str(&rest[..b]);
                rest = rest[b..].find("*/").map_or("", |e| &rest[b + e + 2..]);
            }
            _ => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// Fails unless every rule of the stylesheet `text` is one the cascade
/// takes as written; `at` is the file its URLs resolve against, the
/// sheet's own or the page of a `<style>`, and `depth` how many imports
/// deep it is. The sheets it imports go in `needs`.
fn sheet_supported(
    checkout: &Path,
    at: &str,
    text: &str,
    depth: usize,
    needs: &mut Vec<String>,
) -> Result<(), String> {
    let sheet = css::parse_stylesheet(text).map_err(|e| format!("stylesheet: {e}"))?;
    nesting_supported(text)?;
    rules_supported(checkout, at, &sheet.rules, depth, needs)
}

/// Fails on a block inside a style rule's block: a rule inside a rule,
/// CSS Nesting, which the parser does not have and drops with the
/// declarations around it. A block is a style rule's when what opens it
/// does not begin with an at-keyword.
fn nesting_supported(text: &str) -> Result<(), String> {
    // For each block open, whether it is a style rule's.
    let mut open: Vec<bool> = Vec::new();
    // Whether what is read since the last rule or declaration ended
    // began with an at-keyword; `None` before anything is read.
    let mut at_rule: Option<bool> = None;
    for token in css::tokenize(text) {
        match token {
            css::Token::Whitespace | css::Token::Cdo | css::Token::Cdc => {}
            css::Token::OpenCurly => {
                if open.last() == Some(&true) {
                    return Err("nested rule".to_string());
                }
                open.push(at_rule != Some(true));
                at_rule = None;
            }
            css::Token::CloseCurly => {
                open.pop();
                at_rule = None;
            }
            css::Token::Semicolon => at_rule = None,
            css::Token::AtKeyword(_) if at_rule.is_none() => at_rule = Some(true),
            _ => {
                at_rule.get_or_insert(false);
            }
        }
    }
    Ok(())
}

/// Fails unless the cascade takes every one of `rules` as written; `at`,
/// `depth` and `needs` as for [`sheet_supported`].
fn rules_supported(
    checkout: &Path,
    at: &str,
    rules: &[css::Rule],
    depth: usize,
    needs: &mut Vec<String>,
) -> Result<(), String> {
    for rule in rules {
        let at_rule = match rule {
            css::Rule::Qualified(rule) => {
                style::parse_selector_list(&rule.prelude).map_err(|_| "selector".to_string())?;
                declarations_supported(&rule.declarations)?;
                for d in &rule.declarations {
                    images(checkout, at, &d.value, needs)?;
                }
                continue;
            }
            css::Rule::At(at_rule) => at_rule,
        };
        let name = at_rule.name.to_ascii_lowercase();
        let inner = match &at_rule.block {
            Some(css::Block::Rules(inner)) => inner.as_slice(),
            _ => &[],
        };
        match name.as_str() {
            "charset" => {}
            "import" => {
                let href = at_rule
                    .prelude
                    .iter()
                    .find_map(|token| match token {
                        css::Token::Url { value, .. } | css::Token::String { value, .. } => {
                            Some(value.as_str())
                        }
                        _ => None,
                    })
                    .ok_or_else(|| "@import without a URL".to_string())?;
                let path = resolve(at, href)?;
                if depth >= MAX_IMPORT_DEPTH || needs.contains(&path) {
                    continue;
                }
                let text = fs::read_to_string(checkout.join(&path))
                    .map_err(|_| format!("missing file {path}"))?;
                needs.push(path.clone());
                sheet_supported(checkout, &path, &text, depth + 1, needs)?;
            }
            "media" | "layer" => rules_supported(checkout, at, inner, depth, needs)?,
            // A false or invalid condition drops the block, whatever it
            // holds.
            "supports" => {
                if style::supports_condition(&at_rule.prelude) == Some(true) {
                    rules_supported(checkout, at, inner, depth, needs)?;
                }
            }
            // A keyframe whose selector or a rule whose name does not
            // parse is dropped by the draft, as the cascade drops it.
            "keyframes" | "-webkit-keyframes" => {
                for keyframe in inner {
                    match keyframe {
                        css::Rule::Qualified(keyframe) => {
                            declarations_supported(&keyframe.declarations)?;
                        }
                        css::Rule::At(rule) => {
                            return Err(format!("@{} in @{name}", rule.name.to_ascii_lowercase()));
                        }
                    }
                }
            }
            _ => return Err(format!("@{name}")),
        }
    }
    Ok(())
}

/// Whether `url` is a data URL, which names no file.
fn is_data(url: &str) -> bool {
    url.trim_start()
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("data:"))
}

/// Fails on an image this engine does not read: an SVG one (M32) or an
/// AVIF one (G34), by its name or its data URL's type.
fn unsupported_image(url: &str) -> Result<(), String> {
    let url = url.trim().to_ascii_lowercase();
    let path = url.split(['?', '#']).next().unwrap_or(&url);
    if path.ends_with(".svg") || path.ends_with(".svgz") || url.starts_with("data:image/svg") {
        return Err("SVG image".to_string());
    }
    if path.ends_with(".avif") || url.starts_with("data:image/avif") {
        return Err("AVIF image".to_string());
    }
    Ok(())
}

/// Puts in `needs` the images a declaration's `value` names (M23): each
/// `url()`, and each string of an `image-set()`, resolved against `at`,
/// where it is a file on the checkout. One that is not, a broken image
/// the test may mean, a link elsewhere and a data URL are left as they
/// are. Fails on an image this engine does not read.
fn images(
    checkout: &Path,
    at: &str,
    value: &[css::Token],
    needs: &mut Vec<String>,
) -> Result<(), String> {
    let mut functions: Vec<String> = Vec::new();
    for token in value {
        let url = match token {
            css::Token::Url { value, .. } => Some(value.as_str()),
            css::Token::Function(name) => {
                functions.push(name.to_ascii_lowercase());
                None
            }
            css::Token::OpenParen => {
                functions.push(String::new());
                None
            }
            css::Token::CloseParen => {
                functions.pop();
                None
            }
            css::Token::String { value, .. }
                if matches!(
                    functions.last().map(String::as_str),
                    Some("url" | "image-set" | "-webkit-image-set")
                ) =>
            {
                Some(value.as_str())
            }
            _ => None,
        };
        if let Some(url) = url {
            unsupported_image(url)?;
        }
        let Some(url) = url.filter(|url| !is_data(url)) else {
            continue;
        };
        if let Ok(path) = resolve(at, url)
            && checkout.join(&path).is_file()
            && !needs.contains(&path)
        {
            needs.push(path);
        }
    }
    Ok(())
}

/// Fails unless the cascade accepts every one of `declarations`.
fn declarations_supported(declarations: &[css::Declaration]) -> Result<(), String> {
    for d in declarations {
        if d.name == "font-family"
            && d.value.iter().any(|t| match t {
                css::Token::Ident(s) | css::Token::String { value: s, .. } => {
                    s.eq_ignore_ascii_case("ahem")
                }
                _ => false,
            })
        {
            return Err("Ahem".to_string());
        }
        if !style::supports(&d.name, &d.value) {
            return Err(format!("declaration of {}", d.name));
        }
    }
    Ok(())
}

/// `href` on the page at `rel`, as a path from the checkout's root: a
/// link to another origin is not on the checkout.
fn resolve(rel: &str, href: &str) -> Result<String, String> {
    let href = href.split(['?', '#']).next().unwrap_or(href).trim();
    if href.contains("://") || href.starts_with("//") || href.is_empty() {
        return Err(format!("link to {href}"));
    }
    let base = if href.starts_with('/') {
        PathBuf::new()
    } else {
        Path::new(rel)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    };
    let mut parts: Vec<String> = Vec::new();
    for component in base.join(href.trim_start_matches('/')).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts
                    .pop()
                    .ok_or_else(|| format!("link out of the checkout: {href}"))?;
            }
            _ => {}
        }
    }
    Ok(parts.join("/"))
}

/// `<meta name=fuzzy content>`: the reference it is for, if it names
/// one, and its ranges as `a-b;c-d`.
fn parse_fuzzy(rel: &str, content: &str) -> Result<(Option<String>, String), String> {
    let (reference, ranges) = match content.split_once(':') {
        Some((reference, ranges)) if !reference.contains(['=', ';']) => {
            (Some(resolve(rel, reference.trim())?), ranges)
        }
        _ => (None, content),
    };
    let mut parts = Vec::new();
    for part in ranges.split(';') {
        let value = part.split_once('=').map_or(part, |(_, v)| v).trim();
        let (low, high) = value.split_once('-').unwrap_or((value, value));
        let (Ok(low), Ok(high)) = (low.trim().parse::<u32>(), high.trim().parse::<u32>()) else {
            return Err(format!("fuzzy {content}"));
        };
        parts.push(format!("{low}-{high}"));
    }
    if parts.len() != 2 {
        return Err(format!("fuzzy {content}"));
    }
    Ok((reference, parts.join(";")))
}

/// The test's kind's name and its manifest line.
fn manifest_line(rel: &str, kind: &Kind) -> (&'static str, String) {
    let mut fields = Vec::new();
    let name = match kind {
        Kind::Reftest { refs, fuzzy } => {
            for (matches, reference) in refs {
                fields.push(format!("{}{reference}", if *matches { "==" } else { "!=" }));
            }
            for (reference, ranges) in fuzzy {
                fields.push(format!(
                    "fuzzy={}:{ranges}",
                    reference.as_deref().unwrap_or("")
                ));
            }
            "reftest"
        }
        Kind::CheckLayout { selectors } => {
            fields.extend(selectors.iter().cloned());
            "checklayout"
        }
        Kind::Crashtest => "crashtest",
        Kind::Testharness { long } => {
            if *long {
                fields.push("timeout=long".to_string());
            }
            "testharness"
        }
    };
    let mut line = format!("{name}\t{rel}");
    for field in fields {
        line.push('\t');
        line.push_str(&field);
    }
    (name, line)
}

/// The README of `tests/wpt/`.
fn readme(
    commit: &str,
    date: &str,
    counts: &BTreeMap<&str, usize>,
    left_out: &BTreeMap<String, usize>,
) -> String {
    let total: usize = counts.values().sum();
    let quoted: Vec<String> = DIRECTORIES.iter().map(|d| format!("`{d}`")).collect();
    let directories = match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => quoted.join(""),
    };
    let testharness = TESTHARNESS_DIRECTORIES
        .iter()
        .map(|(directory, start)| match start.is_empty() {
            true => format!("`{directory}`"),
            false => format!("`{directory}` (the `{start}*` tests)"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!(
        "# web-platform-tests subset\n\n\
         The tests here are copied from\n\
         [web-platform-tests](https://github.com/web-platform-tests/wpt),\n\
         under its BSD licence (`LICENSE.md` beside this file), at commit\n\n\
         ```text\n\
         {commit} ({date})\n\
         ```\n\n\
         They are the tests of {directories}\n\
         that use only what this engine has, chosen by the rule in\n\
         `examples/wpt.rs` and copied with their references, stylesheets\n\
         and images at their own paths by\n\n\
         ```sh\n\
         cargo run -p engine --example wpt -- pull <checkout>\n\
         ```\n\n\
         which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs\n\
         them; `expectations.txt` names those expected to fail, and why.\n\n\
         With them are the testharness.js tests of {testharness} (M27, M29, M31)\n\
         that the runner's one server can serve: no Python handler, no `.sub`\n\
         template, no other host, nothing over `https:`, no shared or\n\
         service worker and no module worker. `resources/testharness.js` is\n\
         the real one; the runner serves its own `testharnessreport.js`, and\n\
         the pages wptserve makes of an `.any.js` test (for a window and,\n\
         as its `global` says, a dedicated worker), a `.window.js` test or a\n\
         `.worker.js` test, and an `.any.js` test's worker script. Each\n\
         subtest they fail is named in `expectations.txt`, and why.\n\n\
         ## Taken\n\n\
         | Kind | Tests |\n\
         | --- | ---: |\n"
    );
    for (kind, n) in counts {
        text.push_str(&format!("| {kind} | {n} |\n"));
    }
    text.push_str(&format!("| all | {total} |\n\n"));
    text.push_str(
        "## Left out\n\n\
         The first reason the rule found for each test it left out:\n\n\
         | Reason | Tests |\n\
         | --- | ---: |\n",
    );
    let mut reasons: Vec<(&String, &usize)> = left_out.iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (reason, n) in reasons {
        text.push_str(&format!("| {} | {n} |\n", reason.replace('|', "\\|")));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A checkout holding `files`, by path and content, removed when
    /// dropped.
    struct Checkout(PathBuf);

    impl Checkout {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root = env::temp_dir().join(format!("wpt-rule-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            for (path, content) in files {
                let path = root.join(path);
                fs::create_dir_all(path.parent().expect("a file has a parent"))
                    .expect("the temporary directory is writable");
                fs::write(path, content).expect("the temporary directory is writable");
            }
            Checkout(root)
        }

        /// The manifest line of the test at `rel` and the files it needs,
        /// or why the rule leaves it out.
        fn take(&self, rel: &str) -> Result<(String, Vec<String>), String> {
            take(&self.0, rel).map(|(kind, needs)| (manifest_line(rel, &kind).1, needs))
        }
    }

    impl Drop for Checkout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_rule_takes_reftests_check_layout_tests_and_crashtests() {
        let checkout = Checkout::new(
            "kinds",
            &[
                (
                    "css/x/t.html",
                    "<link rel=match href=t-ref.html><link rel=stylesheet href=s.css>\
                     <meta name=fuzzy content='maxDifference=0-2;totalPixels=0-10'>\
                     <div class=a style='width: 10px'></div>",
                ),
                ("css/x/t-ref.html", "<div style='width: 10px'></div>"),
                ("css/x/s.css", "@charset \"utf-8\"; .a { color: red }"),
                (
                    "css/x/c.html",
                    "<script src=/resources/check-layout-th.js></script>\
                     <body onload=\"checkLayout('.a')\">\
                     <div class=a data-expected-width=10></div>",
                ),
                (
                    "css/x/s.html",
                    "<div class=a data-expected-height=0></div><script>\
                     // the harness runs it\n window.checkLayout(\".a\", true); \
                     /* and again */ checkLayout('div');</script>\
                     <script src=../../resources/check-layout-th.js></script>",
                ),
                ("css/x/crashtests/k.html", "<div></div>"),
                ("css/x/w-crash.html", "<div></div>"),
            ],
        );
        assert_eq!(
            checkout.take("css/x/t.html"),
            Ok((
                "reftest\tcss/x/t.html\t==css/x/t-ref.html\tfuzzy=:0-2;0-10".to_string(),
                vec!["css/x/s.css".to_string(), "css/x/t-ref.html".to_string()]
            ))
        );
        assert_eq!(
            checkout.take("css/x/c.html"),
            Ok(("checklayout\tcss/x/c.html\t.a".to_string(), vec![]))
        );
        assert_eq!(
            checkout.take("css/x/s.html").map(|(line, _)| line),
            Ok("checklayout\tcss/x/s.html\t.a\tdiv".to_string())
        );
        for crash in ["css/x/crashtests/k.html", "css/x/w-crash.html"] {
            assert_eq!(
                checkout.take(crash),
                Ok((format!("crashtest\t{crash}"), vec![]))
            );
        }
    }

    #[test]
    fn the_images_a_test_names_are_copied_with_it() {
        let checkout = Checkout::new(
            "images",
            &[
                (
                    "css/x/t.html",
                    "<link rel=match href=t-ref.html><link rel=stylesheet href=s/s.css>\
                     <style>.b { background-image: image-set(\"c.png\" 1x) }</style>\
                     <img src=a.png><img src=missing.png><img src='data:image/png;base64,AA'>\
                     <div style='background: url(\"/images/b.png\")'></div>",
                ),
                ("css/x/t-ref.html", "<img src=a.png>"),
                (
                    "css/x/s/s.css",
                    ".a { list-style-image: url(../d.png); content: \"e.png\" }",
                ),
                ("css/x/a.png", "a"),
                ("css/x/c.png", "c"),
                ("css/x/d.png", "d"),
                ("css/x/e.png", "e"),
                ("images/b.png", "b"),
            ],
        );
        let (_, mut needs) = checkout.take("css/x/t.html").expect("taken");
        needs.sort();
        needs.dedup();
        assert_eq!(
            needs,
            [
                "css/x/a.png",
                "css/x/c.png",
                "css/x/d.png",
                "css/x/s/s.css",
                "css/x/t-ref.html",
                "images/b.png",
            ],
            "a missing image, a data URL and a string that is no URL are not copied"
        );
    }

    #[test]
    fn every_vendored_test_is_in_a_directory_the_rule_takes() {
        // The subset is pulled by the directories: none of them is lost
        // without a pull that drops its tests.
        let manifest =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt/MANIFEST"))
                .expect("the manifest");
        let mut used: BTreeSet<&str> = BTreeSet::new();
        let testharness: Vec<&str> = TESTHARNESS_DIRECTORIES.iter().map(|(d, _)| *d).collect();
        for line in manifest
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let mut fields = line.split('\t');
            let kind = fields.next().expect("a kind");
            let path = fields.next().expect("a path");
            // A testharness test by the directories of its own rule (M27).
            let directories = match kind {
                "testharness" => testharness.as_slice(),
                _ => DIRECTORIES,
            };
            let directory = directories
                .iter()
                .filter(|d| path.starts_with(&format!("{d}/")))
                .max_by_key(|d| d.len())
                .unwrap_or_else(|| panic!("{path} is in no directory of the rule"));
            used.insert(directory);
        }
        for directory in DIRECTORIES.iter().chain(&testharness) {
            assert!(used.contains(directory), "{directory} gives no test");
        }
    }

    #[test]
    fn a_testharness_test_is_taken_when_the_runners_server_can_serve_it() {
        let harness = "<script src=/resources/testharness.js></script>\
                       <script src=/resources/testharnessreport.js></script>";
        let checkout = Checkout::new(
            "testharness",
            &[
                (
                    "fetch/a/t.any.js",
                    "// META: script=../r/u.js\n// META: timeout=long\ntest(() => {});",
                ),
                ("fetch/r/u.js", "function u() {}"),
                ("fetch/a/w.window.js", "test(() => {});"),
                (
                    "fetch/a/p.html",
                    &format!("{harness}<script src=h.js></script>"),
                ),
                ("fetch/a/h.js", "function h() {}"),
                (
                    "fetch/a/worker.any.js",
                    "// META: global=worker\ntest(() => {});",
                ),
                ("fetch/a/py.any.js", "fetch('resources/echo.py');"),
                (
                    "fetch/a/inc.any.js",
                    "// META: script=../r/py.js\ntest(() => {});",
                ),
                ("fetch/r/py.js", "const url = 'x.py';"),
                (
                    "fetch/a/host.any.js",
                    "const u = get_host_info().HTTP_REMOTE_ORIGIN;",
                ),
                ("fetch/a/pipe.any.js", "fetch('x?pipe=trickle(d1)');"),
                ("fetch/a/made.any.js", "new Worker('w.js');"),
                ("fetch/a/frame.any.js", "document.createElement('iframe');"),
                (
                    "fetch/a/upper.any.js",
                    "document.createElement(\"IFRAME\");",
                ),
                (
                    "fetch/a/f.html",
                    &format!("{harness}<iframe src=x.html></iframe>"),
                ),
                ("fetch/a/s.https.any.js", "test(() => {});"),
                ("fetch/a/t.sub.any.js", "test(() => {});"),
                ("fetch/a/none.html", "<p>no harness</p>"),
                // M31's: workers.
                ("fetch/a/shared.any.js", "new SharedWorker('w.js');"),
                (
                    "fetch/a/only.any.js",
                    "// META: global=sharedworker\ntest(() => {});",
                ),
                (
                    "fetch/a/gen.worker.js",
                    "// generated by gentest.py\nimportScripts('/resources/testharness.js');\n\
                     test(() => {});\ndone();",
                ),
                ("fetch/a/x.sharedworker.html", &format!("{harness}<p>")),
                ("fetch/a/blob.any.js", "URL.createObjectURL(new Blob([]));"),
                (
                    "workers/a/blob.any.js",
                    "URL.createObjectURL(new Blob([]));",
                ),
                (
                    "workers/a/module.html",
                    &format!("{harness}<script>new Worker('w.js', {{ type: 'module' }});</script>"),
                ),
                ("workers/a/sab.any.js", "new SharedArrayBuffer(8);"),
                ("workers/a/bc.any.js", "new BroadcastChannel('b');"),
                (
                    "workers/a/helped.any.js",
                    "// META: script=/common/sab.js\ntest(() => {});",
                ),
                (
                    "common/sab.js",
                    "function make(type) { return new SharedArrayBuffer(8); }",
                ),
            ],
        );
        let take = |rel: &str| take_testharness(&checkout.0, rel);
        let pages = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();
        // An `.any.js` test is a window's and a dedicated worker's unless
        // its `global` says otherwise (M31).
        assert_eq!(
            take("fetch/a/t.any.js"),
            Ok((
                pages(&["fetch/a/t.any.html", "fetch/a/t.any.worker.html"]),
                true,
                vec!["fetch/r/u.js".to_string()]
            ))
        );
        assert_eq!(
            take("fetch/a/w.window.js"),
            Ok((pages(&["fetch/a/w.window.html"]), false, vec![]))
        );
        assert_eq!(
            take("fetch/a/p.html"),
            Ok((
                pages(&["fetch/a/p.html"]),
                false,
                vec!["fetch/a/h.js".to_string()]
            ))
        );
        assert_eq!(
            take("fetch/a/worker.any.js"),
            Ok((pages(&["fetch/a/worker.any.worker.html"]), false, vec![]))
        );
        assert_eq!(
            take("fetch/a/made.any.js"),
            Ok((
                pages(&["fetch/a/made.any.html", "fetch/a/made.any.worker.html"]),
                false,
                vec![]
            ))
        );
        // A generated worker test's comments name no file it loads.
        assert_eq!(
            take("fetch/a/gen.worker.js"),
            Ok((pages(&["fetch/a/gen.worker.html"]), false, vec![]))
        );
        // A helper that makes a SharedArrayBuffer only when asked.
        assert_eq!(
            take("workers/a/helped.any.js"),
            Ok((
                pages(&[
                    "workers/a/helped.any.html",
                    "workers/a/helped.any.worker.html"
                ]),
                false,
                vec!["common/sab.js".to_string()]
            ))
        );
        // What M31 leaves to others, in its own directories only.
        assert_eq!(
            take("fetch/a/blob.any.js"),
            Ok((
                pages(&["fetch/a/blob.any.html", "fetch/a/blob.any.worker.html"]),
                false,
                vec![]
            ))
        );
        for (rel, why) in [
            ("fetch/a/py.any.js", "Python handler"),
            ("fetch/a/inc.any.js", "Python handler"),
            ("fetch/a/host.any.js", "another host"),
            ("fetch/a/pipe.any.js", "wptserve pipe"),
            ("fetch/a/shared.any.js", "a shared or service worker"),
            (
                "fetch/a/only.any.js",
                "not for a window or a dedicated worker",
            ),
            ("fetch/a/x.sharedworker.html", "a shared or service worker"),
            ("workers/a/blob.any.js", "a blob: URL (G68)"),
            ("workers/a/module.html", "a module worker (G41)"),
            ("workers/a/sab.any.js", "SharedArrayBuffer (G83)"),
            ("workers/a/bc.any.js", "BroadcastChannel (G84)"),
            ("fetch/a/frame.any.js", "a frame"),
            ("fetch/a/upper.any.js", "a frame"),
            ("fetch/a/f.html", "a frame"),
            ("fetch/a/s.https.any.js", "https:"),
            ("fetch/a/t.sub.any.js", "wptserve substitution (.sub)"),
            ("fetch/a/none.html", "not a testharness test"),
        ] {
            assert_eq!(take(rel), Err(why.to_string()), "{rel}");
        }
        assert!(is_testharness_test("t.any.js") && is_testharness_test("p.html"));
        assert!(is_testharness_test("t.worker.js"));
        assert!(!is_testharness_test("u.js") && !is_testharness_test("m-manual.html"));
    }

    #[test]
    fn a_canvas_test_takes_its_helper_unless_it_calls_another_hosts_function() {
        let harness = "<script src=/resources/testharness.js></script>";
        let helper = "<script src=/html/canvas/resources/canvas-tests.js></script>";
        let checkout = Checkout::new(
            "canvas",
            &[
                (
                    "html/canvas/resources/canvas-tests.js",
                    "function addCrossOriginYellowImage() { get_host_info(); }",
                ),
                (
                    "html/canvas/element/a/t.html",
                    &format!("{harness}{helper}<script>_addTest(() => {{}});</script>"),
                ),
                (
                    "html/canvas/element/a/x.html",
                    &format!("{harness}{helper}<script>addCrossOriginYellowImage();</script>"),
                ),
                (
                    "html/canvas/element/a/g.html",
                    &format!("<!-- generated by gentest.py -->{harness}{helper}"),
                ),
            ],
        );
        let take = |rel: &str| take_testharness(&checkout.0, rel);
        assert_eq!(
            take("html/canvas/element/a/t.html"),
            Ok((
                vec!["html/canvas/element/a/t.html".to_string()],
                false,
                vec!["html/canvas/resources/canvas-tests.js".to_string()]
            ))
        );
        assert_eq!(
            take("html/canvas/element/a/x.html"),
            Err("another host".to_string())
        );
        // A comment naming the generator is no Python handler.
        assert!(take("html/canvas/element/a/g.html").is_ok());
    }

    #[test]
    fn a_template_wptserve_fills_in_is_left_out() {
        let checkout = Checkout::new(
            "sub",
            &[
                (
                    "css/x/t.sub.html",
                    "<link rel=match href=r.html><div></div>",
                ),
                ("css/x/r.html", "<div></div>"),
            ],
        );
        assert_eq!(
            checkout.take("css/x/t.sub.html"),
            Err("wptserve substitution (.sub)".to_string())
        );
    }

    #[test]
    fn the_rule_leaves_out_what_this_engine_does_not_have() {
        let reftest = |body: &str| format!("<link rel=match href=r.html>{body}");
        let pages = [
            (reftest("<img srcset='a.png 2x'>"), "img srcset"),
            (reftest("<img src=a.svg>"), "SVG image"),
            (
                reftest("<div style='background: url(\"data:image/svg+xml,<svg/>\")'></div>"),
                "SVG image",
            ),
            (
                reftest("<style>.a { list-style-image: url(b.avif) }</style>"),
                "AVIF image",
            ),
            (reftest("<svg></svg>"), "element <svg>"),
            (reftest("<div onanimationend=go()></div>"), "script"),
            (reftest("<img onerror=go()>"), "script"),
            (reftest("<script>go()</script>"), "script"),
            (reftest("<script src=x.js></script>"), "script"),
            (
                reftest("<div data-foo=1></div>"),
                "unchecked attribute data-foo",
            ),
            (reftest("<div dir=rtl></div>"), "dir=rtl"),
            (reftest("<input type=Range>"), "input type=range"),
            (reftest("<fieldset></fieldset>"), "element <fieldset>"),
            (reftest("<script src=/common/other.js></script>"), "script"),
            (
                "<body onload=\"document.body.scrollTo(0, 1)\"><link rel=match href=r.html>"
                    .to_string(),
                "script",
            ),
            (
                reftest("<div style='writing-mode: vertical-rl'></div>"),
                "declaration of writing-mode",
            ),
            (reftest("<style>.a { font-family: Ahem }</style>"), "Ahem"),
            (
                reftest("<style>@font-face { font-family: x }</style>"),
                "@font-face",
            ),
            (
                reftest("<style>@container (width > 0) { .a { color: red } }</style>"),
                "@container",
            ),
            (
                reftest(
                    "<style>@media all { @keyframes k { to { writing-mode: vertical-rl } } }</style>",
                ),
                "declaration of writing-mode",
            ),
            (
                reftest("<style>@keyframes k { @media all { } }</style>"),
                "@media in @keyframes",
            ),
            (
                reftest("<style>@media all { .a { writing-mode: vertical-rl } }</style>"),
                "declaration of writing-mode",
            ),
            (
                reftest(
                    "<style>@supports (color: red) { .a { writing-mode: vertical-rl } }</style>",
                ),
                "declaration of writing-mode",
            ),
            (
                reftest("<style>@import 'missing.css';</style>"),
                "missing file css/x/missing.css",
            ),
            (
                reftest("<style>@import 'bad.css' screen;</style>"),
                "declaration of writing-mode",
            ),
            (
                reftest("<style>.a { color: red; & .b { color: blue } }</style>"),
                "nested rule",
            ),
            (
                reftest("<style>@media all { .a { color: red; .b { color: blue } } }</style>"),
                "nested rule",
            ),
            (
                reftest("<style>.a:unknown { color: red }</style>"),
                "selector",
            ),
            (
                "<link rel=match href=missing.html>".to_string(),
                "missing file css/x/missing.html",
            ),
            (
                "<link rel=match href=http://example.org/r.html>".to_string(),
                "link to http://example.org/r.html",
            ),
            (
                "<body onload=\"checkLayout('.a'); go()\">".to_string(),
                "script",
            ),
            (
                "<div></div>".to_string(),
                "not a reftest, a check-layout test or a crashtest",
            ),
        ];
        let mut files: Vec<(String, String)> = pages
            .iter()
            .enumerate()
            .map(|(i, (page, _))| (format!("css/x/t{i}.html"), page.clone()))
            .collect();
        files.push(("css/x/r.html".to_string(), "<div></div>".to_string()));
        files.push((
            "css/x/bad.css".to_string(),
            ".a { writing-mode: vertical-rl }".to_string(),
        ));
        files.push((
            "css/x/x.xht".to_string(),
            "<html><link rel=match href=r.html/><div/></html>".to_string(),
        ));
        files.push((
            "css/x/y.xht".to_string(),
            "<html><style><![CDATA[ .a {} ]]></style></html>".to_string(),
        ));
        let files: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        let checkout = Checkout::new("left-out", &files);
        for (i, (page, reason)) in pages.iter().enumerate() {
            assert_eq!(
                checkout.take(&format!("css/x/t{i}.html")).err().as_deref(),
                Some(*reason),
                "{page}"
            );
        }
        assert_eq!(
            checkout.take("css/x/x.xht").err().as_deref(),
            Some("self-closing <div/> in XHTML")
        );
        assert_eq!(
            checkout.take("css/x/y.xht").err().as_deref(),
            Some("CDATA section in XHTML")
        );
    }

    #[test]
    fn scripts_that_use_only_what_m25_and_m26_give_are_taken() {
        for script in [
            "function PASS() { return false; }",
            "document.title = 'a // b'; console.log(\"c\", `d`)",
            "/* x */ const f = (a, b) => a + b; let n = f(1, 2) * Math.PI; var g = c => c;",
            "function h(x) { return [x].map(function (y) { return y.toFixed(2) }) }",
            "// go()\nconsole.log('/* not a comment */ go()')",
            "console.log('it\\'s', \"a \\\" b\")",
            "const k = (a, b = (1)) => a + b; function m(c = Math.max(1, 2), d) { return c + d }",
            "var d = document.createElement('div'); d.className = 'a'; document.body.appendChild(d);",
            "document.getElementById('t').style.color = 'red'; window.x = 1;",
            "getComputedStyle(document.body).display; innerWidth; HTMLDivElement; Node.ELEMENT_NODE",
            "document.querySelector('p').classList.toggle('b'); document.write('<p>x</p>')",
            "setTimeout(function () { takeScreenshot(); }, 10); takeScreenshotDelayed(5)",
            "requestAnimationFrame(() => requestAnimationFrame(takeScreenshot))",
            "document.body.addEventListener('load', function (e) { e.preventDefault(); })",
            "window.onload = function () { document.body.onclick = null; }",
            "var i = document.querySelector('input'); i.focus(); i.value = 'x'; i.checked = true",
            "document.body.dispatchEvent(new MouseEvent('click', { bubbles: true }))",
            "performance.now(); queueMicrotask(function () {}); waitForAtLeastOneFrame()",
            "document.createEvent('Event'); event; new AbortController().signal",
        ] {
            assert!(uses_only_m26(script), "{script}");
        }
        for script in [
            "console.log(document.body.offsetTop)",
            "go()",
            "let t = `${document.cookie}`",
            "x.style.color = 'red'",
            "'unended",
            "setTimeout(f, 0)",
            "window.onload = function() { window.scrollTo(0, 100); };",
            "console.log(...go)",
            "document.createDocumentFragment()",
            "document.body.valueAsNumber = 1",
            "document.body.setPointerCapture(1)",
            "document.body.contentEditable = 'true'",
            "new MutationObserver(function () {})",
            "requestIdleCallback(function () {})",
            "document.body.moveBefore(document.body.firstChild, null)",
            "var o = { a: b }",
            "var t = c ? d : 1",
            "document.styleSheets[0].insertRule('p {}')",
            "document.fonts.ready.then(f)",
            "e.getBoundingClientRect()",
            "HTMLElement2",
            "var h = document.body.offsetTop;",
            "f(document.body.offsetTop);",
            "document.body.offsetTop + 1;",
            "document.body.offsetTop = 1;",
        ] {
            assert!(!uses_only_m26(script), "{script}");
        }
        for script in [
            "document.documentElement.offsetTop;",
            "var d = document.body;\n  d.offsetHeight\n  d.className = 'b';",
            "if (true) { document.body.scrollTop }",
        ] {
            assert!(uses_only_m26(script), "{script}");
        }
        assert!(
            is_element_interface("HTMLElement") && is_element_interface("HTMLTableCellElement")
        );
        assert!(!is_element_interface("HTMLCollection") && !is_element_interface("SVGElement"));
        assert!(!is_element_interface("HTML2Element"));
        assert_eq!(
            writes_supported("e.style.writingMode = m;")
                .err()
                .as_deref(),
            Some("declaration of writing-mode")
        );
        assert_eq!(
            writes_supported("e.style.setProperty('writing-mode', m);")
                .err()
                .as_deref(),
            Some("declaration of writing-mode")
        );
        assert_eq!(
            writes_supported("e.setAttribute('data-expected-scroll-width', 1)")
                .err()
                .as_deref(),
            Some("unchecked attribute data-expected-scroll-width")
        );
        assert_eq!(
            writes_supported("e.dataset.expectedScrollWidth = 1")
                .err()
                .as_deref(),
            Some("unchecked attribute data-expected-scroll-width")
        );
        assert!(
            writes_supported(
                "e.style.height = '1px'; e.style.cssText = ''; e.style.setProperty('--x', 1); \
                 e.setAttribute('data-expected-width', 1); e.dataset.testName = 'a'"
            )
            .is_ok()
        );
        assert_eq!(kebab("webkitBoxFlex"), "-webkit-box-flex");
        let checkout = Checkout::new(
            "scripts",
            &[
                (
                    "css/x/a.html",
                    "<link rel=match href=r.html><a onblur=\"PASS();\"></a>\
                     <script>function PASS() { return false; }</script>",
                ),
                (
                    "css/x/b.html",
                    "<link rel=match href=r.html><div id=d></div><script>\
                     document.getElementById('d').setAttribute('data-expected-scroll-width', 1)\
                     </script>",
                ),
                (
                    "css/x/c.html",
                    "<html class=reftest-wait><link rel=match href=r.html>\
                     <script src=/common/reftest-wait.js></script>\
                     <input id=i onfocus=\"this.value = 'x'\" autofocus>\
                     <body onload=\"requestAnimationFrame(takeScreenshot)\">",
                ),
                (
                    "css/x/d.html",
                    "<link rel=match href=r.html><div onclick=\"this.style.writingMode = 'x'\">",
                ),
                ("css/x/r.html", "<div></div>"),
            ],
        );
        assert!(checkout.take("css/x/a.html").is_ok());
        assert_eq!(
            checkout.take("css/x/b.html").err().as_deref(),
            Some("unchecked attribute data-expected-scroll-width")
        );
        let (_, needs) = checkout.take("css/x/c.html").expect("taken");
        assert!(
            needs.iter().any(|n| n == "common/reftest-wait.js"),
            "{needs:?}"
        );
        assert_eq!(
            checkout.take("css/x/d.html").err().as_deref(),
            Some("declaration of writing-mode"),
            "a handler's writes are held to the rule"
        );
    }

    #[test]
    fn the_rule_takes_media_supports_layers_and_imports() {
        let checkout = Checkout::new(
            "at-rules",
            &[
                (
                    "css/x/t.html",
                    "<link rel=match href=r.html><link rel=stylesheet href=s/a.css media=print>\
                     <style>@import url(s/b.css) layer(x) supports(display: grid) screen;\
                     @layer x, y; @media (min-width: 1px) { @layer y { .a { color: red } } }\
                     @supports (writing-mode: vertical-rl) { .a { writing-mode: vertical-rl } }\
                     @supports nonsense { .a { writing-mode: vertical-rl } }</style>",
                ),
                ("css/x/r.html", "<div></div>"),
                ("css/x/s/a.css", "@import 'c.css'; .a { color: blue }"),
                ("css/x/s/b.css", "@import '../s/c.css'; .b { color: blue }"),
                // Importing each other, as far as the engine goes.
                ("css/x/s/c.css", "@import 'a.css'; .c { color: blue }"),
            ],
        );
        let (_, needs) = checkout.take("css/x/t.html").expect("taken");
        let needs: BTreeSet<String> = needs.into_iter().collect();
        assert_eq!(
            needs,
            BTreeSet::from(
                [
                    "css/x/r.html",
                    "css/x/s/a.css",
                    "css/x/s/b.css",
                    "css/x/s/c.css"
                ]
                .map(String::from)
            )
        );
    }

    #[test]
    fn the_rule_takes_keyframes_animations_and_transitions() {
        // M22: `@keyframes` and `@-webkit-keyframes`, their keyframes'
        // declarations held to the rule; a keyframe the draft drops, its
        // selector not parsing, is taken as the cascade drops it.
        let checkout = Checkout::new(
            "animations",
            &[
                (
                    "css/x/t.html",
                    "<link rel=match href=r.html><style>\
                     @keyframes k { from { opacity: 0; --x: 1 } 50% { animation-timing-function: \
                     steps(2) } to { translate: 10px !important } 120% { color: red } }\
                     @-webkit-keyframes w { to { margin-left: 5px } }\
                     .a { animation: k 1s -0.5s paused; transition: margin 1s ease-in-out }\
                     </style>",
                ),
                ("css/x/r.html", "<div></div>"),
            ],
        );
        assert!(checkout.take("css/x/t.html").is_ok());
    }

    #[test]
    fn the_rule_takes_custom_properties_substitutions_and_lists() {
        let checkout = Checkout::new(
            "values",
            &[
                (
                    "css/x/t.html",
                    "<link rel=match href=r.html>\
                     <style>:root { --w: 10px; --c: green } \
                     .a { width: calc(var(--w) * 2); color: var(--c, red); \
                     margin: env(safe-area-inset-top, 0px) } \
                     li::marker { color: var(--c) } \
                     .b::before { content: counter(list-item) attr(data-test) }</style>\
                     <ol><li class=a>x</li></ol><ul><li class=b data-test=y></li></ul>\
                     <menu><li>z</li></menu>",
                ),
                ("css/x/r.html", "<div></div>"),
                (
                    "css/x/u.html",
                    "<link rel=match href=r.html>\
                     <style>.a { writing-mode: var(--m) }</style>",
                ),
                (
                    "css/x/v.html",
                    "<link rel=match href=r.html>\
                     <style>@property --w { syntax: '<length>' }</style>",
                ),
                (
                    "css/x/w.html",
                    "<link rel=match href=r.html>\
                     <style>@counter-style c { system: cyclic }</style>",
                ),
            ],
        );
        assert!(checkout.take("css/x/t.html").is_ok());
        assert_eq!(
            checkout.take("css/x/u.html").err().as_deref(),
            Some("declaration of writing-mode")
        );
        assert_eq!(
            checkout.take("css/x/v.html").err().as_deref(),
            Some("@property")
        );
        assert_eq!(
            checkout.take("css/x/w.html").err().as_deref(),
            Some("@counter-style")
        );
    }

    #[test]
    fn a_test_is_a_page_whose_name_does_not_make_it_a_reference() {
        for rel in [
            "css/x/a.html",
            "css/x/a-001.xht",
            "css/x/a.htm",
            "css/x/a.xhtml",
        ] {
            assert!(is_test(rel), "{rel}");
        }
        for rel in [
            "css/x/a-ref.html",
            "css/x/a-notref.html",
            "css/x/a-ref-1.html",
            "css/x/a-print.html",
            "css/x/a.css",
            "css/x/a.png",
        ] {
            assert!(!is_test(rel), "{rel}");
        }
    }

    #[test]
    fn links_resolve_on_the_checkout_and_fuzziness_parses() {
        assert_eq!(
            resolve("css/x/t.html", "r.html?a#b"),
            Ok("css/x/r.html".into())
        );
        assert_eq!(
            resolve("css/x/t.html", "../y/r.html"),
            Ok("css/y/r.html".into())
        );
        assert_eq!(
            resolve("css/x/t.html", "/resources/s.js"),
            Ok("resources/s.js".into())
        );
        assert!(resolve("css/x/t.html", "../../../r.html").is_err());
        assert!(resolve("css/x/t.html", "//example.org/r.html").is_err());
        assert!(resolve("css/x/t.html", "").is_err());
        assert_eq!(
            parse_fuzzy("css/x/t.html", "maxDifference=0-2;totalPixels=0-100"),
            Ok((None, "0-2;0-100".into()))
        );
        assert_eq!(
            parse_fuzzy("css/x/t.html", "r.html:5;10-20"),
            Ok((Some("css/x/r.html".into()), "5-5;10-20".into()))
        );
        assert!(parse_fuzzy("css/x/t.html", "0-2").is_err());
        assert!(parse_fuzzy("css/x/t.html", "a-b;0-1").is_err());
    }

    #[test]
    fn a_manifest_line_holds_a_tests_references_and_selectors() {
        let reftest = Kind::Reftest {
            refs: vec![(true, "a.html".into()), (false, "b.html".into())],
            fuzzy: vec![(Some("a.html".into()), "0-1;0-5".into())],
        };
        assert_eq!(
            manifest_line("t.html", &reftest),
            (
                "reftest",
                "reftest\tt.html\t==a.html\t!=b.html\tfuzzy=a.html:0-1;0-5".into()
            )
        );
        let selectors = Kind::CheckLayout {
            selectors: vec![".a".into(), "#b".into()],
        };
        assert_eq!(
            manifest_line("t.html", &selectors),
            ("checklayout", "checklayout\tt.html\t.a\t#b".into())
        );
        assert_eq!(
            manifest_line("t.html", &Kind::Crashtest),
            ("crashtest", "crashtest\tt.html".into())
        );
    }
}
