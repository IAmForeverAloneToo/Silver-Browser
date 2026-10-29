//! html5lib's tree construction tests (M32, `docs/design/svg.md`; D57,
//! `docs/design/text-dom-scripts.md`): every `.dat` file in
//! `tests/html5lib/tree-construction/`, each test's input parsed as a
//! document or as a fragment for its context element, its tree dumped as
//! html5lib writes one (`<svg linearGradient>`, `xlink href="..."`) and
//! compared with the expected one. The DOM keeps no comment, doctype or
//! processing instruction nodes yet (G51), so their lines are taken out
//! of the expected tree and the text either side of one joined, as the
//! parser joins it. Nor
//! has it a template's contents, a fragment of their own: the parser puts
//! them under the `template` as its children, so the expected tree's
//! `content` line is taken out and what is under it moved up a level.
//! What fails, by a gap of the tree builder's or of the DOM's, is named
//! in `expectations.txt` beside the files with its reason; a test named
//! there that passes is a failure too.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use dom::{Document, Namespace, NodeData, NodeId};
use html::{Parser, Stop};

/// One test of a `.dat` file.
struct Test {
    data: String,
    fragment: Option<(Namespace, String)>,
    scripting: bool,
    expected: Vec<String>,
}

/// The tests of a `.dat` file's `text`.
fn tests(text: &str) -> Vec<Test> {
    let mut out = Vec::new();
    for chunk in text.split("#data\n").skip(1) {
        let mut sections: BTreeMap<&str, String> = BTreeMap::new();
        let mut name = "data";
        let mut body = String::new();
        for line in chunk.split_inclusive('\n') {
            let bare = line.trim_end_matches('\n');
            if bare.starts_with('#') && !bare.starts_with("#!") && !body.ends_with("\\") {
                sections.insert(name, std::mem::take(&mut body));
                name = match bare {
                    "#errors" => "errors",
                    "#new-errors" => "new-errors",
                    "#document-fragment" => "fragment",
                    "#script-off" => "script-off",
                    "#script-on" => "script-on",
                    "#document" => "document",
                    _ => "other",
                };
                continue;
            }
            body.push_str(line);
        }
        sections.insert(name, body);
        let data = sections.get("data").cloned().unwrap_or_default();
        // The data's last newline is the format's, not the input's.
        let data = data.strip_suffix('\n').unwrap_or(&data).to_owned();
        let fragment = sections.get("fragment").map(|context| {
            let context = context.trim();
            match context.split_once(' ') {
                Some(("svg", local)) => (Namespace::Svg, local.to_owned()),
                Some(("math", local)) => (Namespace::MathMl, local.to_owned()),
                _ => (Namespace::Html, context.to_owned()),
            }
        });
        let document = sections.get("document").cloned().unwrap_or_default();
        out.push(Test {
            data,
            fragment,
            scripting: sections.contains_key("script-on"),
            expected: expected_lines(&document),
        });
    }
    out
}

/// The expected tree's lines, without comments and doctypes, which the
/// DOM does not keep: a text node's lines joined into one, and two text
/// nodes a comment parted joined as the parser joins them.
fn expected_lines(document: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in document.trim_end_matches('\n').split('\n') {
        match line.strip_prefix("| ") {
            Some(rest) => lines.push(rest.to_owned()),
            // A text node's following line.
            None => {
                if let Some(last) = lines.last_mut() {
                    last.push('\n');
                    last.push_str(line);
                }
            }
        }
    }
    // A template's `content` line out, what is under it a level up.
    let mut contents: Vec<usize> = Vec::new();
    let mut flat: Vec<String> = Vec::new();
    for line in lines {
        let body = line.trim_start();
        let depth = line.len() - body.len();
        while contents.last().is_some_and(|&at| at >= depth) {
            contents.pop();
        }
        if body == "content" {
            contents.push(depth);
            continue;
        }
        let depth = depth - 2 * contents.len();
        flat.push(format!("{}{body}", " ".repeat(depth)));
    }
    let mut out: Vec<String> = Vec::new();
    for line in flat {
        let body = line.trim_start();
        if body.starts_with("<!--") || body.starts_with("<!DOCTYPE") || body.starts_with("<?") {
            continue;
        }
        let depth = line.len() - body.len();
        if let Some(text) = body.strip_prefix('"')
            && let Some(last) = out.last_mut()
        {
            let last_body = last.trim_start();
            if last.len() - last_body.len() == depth && last_body.starts_with('"') {
                let joined = format!("{}{}", &last[..last.len() - 1], text);
                *last = joined;
                continue;
            }
        }
        out.push(line);
    }
    out
}

/// How html5lib names an attribute of a foreign element in its
/// namespace: `xlink href`, `xml lang`, `xmlns xlink`, those that
/// "adjust foreign attributes" (HTML section 13.2.6.1) puts in one;
/// another, `xml:base` among them, keeps its name, in none.
fn attribute_name(ns: &Namespace, name: &str) -> String {
    const ADJUSTED: [&str; 11] = [
        "xlink:actuate",
        "xlink:arcrole",
        "xlink:href",
        "xlink:role",
        "xlink:show",
        "xlink:title",
        "xlink:type",
        "xml:lang",
        "xml:space",
        "xmlns",
        "xmlns:xlink",
    ];
    if *ns == Namespace::Html || !ADJUSTED.contains(&name) {
        return name.to_owned();
    }
    if name == "xmlns" {
        return "xmlns xmlns".to_owned();
    }
    name.replacen(':', " ", 1)
}

/// The tree under `parent` as html5lib dumps it.
fn dump(document: &Document, parent: NodeId, depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for child in document.children(parent) {
        match document.data(child) {
            Some(NodeData::Text(text)) => out.push(format!("{indent}\"{text}\"")),
            Some(NodeData::Element { tag, attrs, ns }) => {
                let name = match ns {
                    Namespace::Html => tag.clone(),
                    Namespace::Svg => format!("svg {tag}"),
                    Namespace::MathMl => format!("math {tag}"),
                    Namespace::Other(_) => tag.clone(),
                };
                out.push(format!("{indent}<{name}>"));
                let mut shown: Vec<String> = attrs
                    .iter()
                    .map(|(name, value)| {
                        format!("{indent}  {}=\"{value}\"", attribute_name(ns, name))
                    })
                    .collect();
                shown.sort();
                out.extend(shown);
                dump(document, child, depth + 1, out);
            }
            _ => {}
        }
    }
}

/// The tree `test`'s input makes.
fn run(test: &Test) -> Vec<String> {
    let mut document = Document::new();
    let mut out = Vec::new();
    match &test.fragment {
        Some((ns, context)) => {
            let container = document.create_element("html").expect("a container");
            html::parse_fragment(
                &mut document,
                container,
                (ns, context),
                test.scripting,
                dom::MAX_DEPTH,
                &test.data,
            );
            dump(&document, container, 0, &mut out);
        }
        None => {
            let mut parser = Parser::new(test.scripting);
            parser.feed(&test.data);
            parser.end();
            while parser.parse(&mut document, usize::MAX) != Stop::Done {}
            dump(&document, document.root(), 0, &mut out);
        }
    }
    out
}

#[test]
fn html5libs_tree_construction_tests_pass() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/html5lib/tree-construction");
    let expectations = fs::read_to_string(dir.join("expectations.txt")).unwrap_or_default();
    let mut expected_to_fail: BTreeMap<String, String> = BTreeMap::new();
    for line in expectations.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (id, why) = line.split_once(' ').unwrap_or((line, ""));
        expected_to_fail.insert(id.to_owned(), why.trim().to_owned());
    }
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("the tests are vendored")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "dat"))
        .collect();
    files.sort();
    let mut unexpected = Vec::new();
    let mut ran = 0;
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        // Taken byte for byte: some tests' data holds a CR, and
        // `.gitattributes` keeps the files' line ends as committed.
        let text = fs::read_to_string(&file).expect("a test file");
        let (mut ran_here, mut passed_here) = (0, 0);
        for (index, test) in tests(&text).iter().enumerate() {
            let id = format!("{name}#{index}");
            ran += 1;
            let got = run(test);
            let passed = got == test.expected;
            ran_here += 1;
            passed_here += usize::from(passed);
            match (passed, expected_to_fail.contains_key(&id)) {
                (false, false) => unexpected.push(format!(
                    "FAIL {id}: {:?}\n  expected:\n    {}\n  got:\n    {}",
                    test.data,
                    test.expected.join("\n    "),
                    got.join("\n    ")
                )),
                (true, true) => unexpected.push(format!(
                    "PASS {id}: it passes, so expectations.txt must no longer name it"
                )),
                _ => {}
            }
        }
        eprintln!("{name}: {passed_here} of {ran_here} pass");
    }
    assert!(ran > 100, "{ran} tests ran");
    assert!(
        unexpected.is_empty(),
        "{} unexpected results of {ran}:\n{}",
        unexpected.len(),
        unexpected.join("\n")
    );
}
