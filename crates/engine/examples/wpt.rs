//! The web-platform-tests subset the engine runs (M17, T1.1; its grid
//! tests M18's, its transforms tests M19's, its media queries,
//! conditional rules and cascade tests M20's, and its tests of custom
//! properties, values, lists, counters, generated content, decoration
//! and alignment M21's).
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
//!   `check-layout-th.js`, and calls to `checkLayout` with a selector,
//!   in an inline script or `onload`;
//! - no element that makes no box here: images, SVG, canvases, frames,
//!   media, form controls and `br`;
//! - every stylesheet only qualified rules whose selectors the matcher
//!   parses, and every declaration one the cascade accepts, as
//!   `style::supports` says, custom properties and declarations with
//!   `var()`, `env()` or `attr()` among them as the cascade takes them
//!   (M21); and no
//!   at-rule but `@charset`, `@import`, whose sheet is on the checkout
//!   and held to the rule too, and `@media`, `@supports` and `@layer`
//!   (M20), the rules inside them held to it as well but for those in an
//!   `@supports` block whose condition is false here, which the cascade
//!   drops whatever they hold: so no `@font-face`, no `@container`, and
//!   no rule inside a style rule;
//! - no Ahem font, no `dir=rtl`, and only the `data-*` expectations the
//!   runner checks;
//! - in an XHTML file, which this engine reads with its HTML parser (M15),
//!   nothing that parser reads otherwise than an XML parser does: no
//!   CDATA section and no self-closing element that is not void.
//!
//! The manifest has a line per test, its fields separated by tabs: the
//! kind and the path, then for a reftest each reference as `==path` or
//! `!=path` and each fuzziness as `fuzzy=ref:a-b;c-d` (an empty `ref`
//! applying to every reference), for a check-layout test its selectors.

use std::collections::{BTreeMap, BTreeSet};
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
    "css/css-env",
    "css/css-lists",
    "css/css-counter-styles",
    "css/css-content",
    "css/css-text-decor",
    "css/css-text/text-align",
    "css/css-text/text-justify",
    "css/CSS2/lists",
    "css/CSS2/generated-content",
];

/// How deep `@import`s may go; the engine fetches no deeper.
const MAX_IMPORT_DEPTH: usize = 16;

/// Elements that make no box here, or not the box the tests mean.
const NO_BOX: &[&str] = &[
    "img", "svg", "canvas", "iframe", "frame", "frameset", "object", "embed", "video", "audio",
    "picture", "math", "input", "button", "select", "textarea", "fieldset", "legend", "meter",
    "progress", "output", "details", "summary", "dialog", "marquee", "br", "hr",
];

/// The scripts a test may load: the harness, which the runner stands in
/// for.
const HARNESS: &[&str] = &[
    "testharness.js",
    "testharnessreport.js",
    "check-layout-th.js",
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
    /// Its stylesheets, resolved.
    stylesheets: Vec<String>,
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
    // The vendored files are replaced wholesale; the expectations stay.
    for directory in DIRECTORIES {
        let first = directory.split('/').next().unwrap_or(directory);
        let old = out.join(first);
        if old.exists() {
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
    let mut page = Page::default();
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
    let tag = document.tag(id).unwrap_or("").to_ascii_lowercase();
    if NO_BOX.contains(&tag.as_str()) {
        return Err(format!("element <{tag}>"));
    }
    for (name, value) in document.attrs(id).unwrap_or(&[]) {
        let name = name.to_ascii_lowercase();
        if name == "dir" && value.trim().eq_ignore_ascii_case("rtl") {
            return Err("dir=rtl".to_string());
        }
        if name == "class" && tag == "html" && value.split_whitespace().any(|c| c == "reftest-wait")
        {
            return Err("script (reftest-wait)".to_string());
        }
        if name == "onload" {
            page.selectors.extend(check_layout_calls(value)?);
        } else if name.starts_with("on") {
            return Err("script".to_string());
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
                } else {
                    Err("script".to_string())
                }
            }
            None => {
                page.selectors
                    .extend(check_layout_calls(&document.text_content(id))?);
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
            _ => return Err(format!("@{name}")),
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
         `examples/wpt.rs` and copied with their references and stylesheets\n\
         at their own paths by\n\n\
         ```sh\n\
         cargo run -p engine --example wpt -- pull <checkout>\n\
         ```\n\n\
         which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs\n\
         them; `expectations.txt` names those expected to fail, and why.\n\n\
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
    fn the_rule_leaves_out_what_this_engine_does_not_have() {
        let reftest = |body: &str| format!("<link rel=match href=r.html>{body}");
        let pages = [
            (reftest("<img src=a.png>"), "element <img>"),
            (reftest("<div onclick=go()></div>"), "script"),
            (reftest("<script>go()</script>"), "script"),
            (reftest("<script src=x.js></script>"), "script"),
            (
                reftest("<div data-foo=1></div>"),
                "unchecked attribute data-foo",
            ),
            (reftest("<div dir=rtl></div>"), "dir=rtl"),
            (
                "<html class=reftest-wait><link rel=match href=r.html>".to_string(),
                "script (reftest-wait)",
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
                reftest("<style>@media all { @keyframes k { } }</style>"),
                "@keyframes",
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
