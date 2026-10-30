//! html5lib's scripted tree construction tests (D57): the `scripted_*`
//! files of `crates/html/tests/html5lib/tree-construction/`, which the
//! `html` crate's harness leaves to the engine. Each input is loaded as
//! a page with scripts on, its scripts run as the parser meets them (and
//! `document.write` writes into its input), as the format means
//! `#script-on`, and the tree is dumped as html5lib writes one.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use dom::{Document, Namespace, NodeData, NodeId};

use super::tests::load_all;
use crate::testing::*;

/// One test: its input and the expected tree's lines.
struct Test {
    data: String,
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
            if bare.starts_with('#') && !body.ends_with('\\') {
                sections.insert(name, std::mem::take(&mut body));
                name = match bare {
                    "#document" => "document",
                    _ => "other",
                };
                continue;
            }
            body.push_str(line);
        }
        sections.insert(name, body);
        let data = sections.get("data").cloned().unwrap_or_default();
        let data = data.strip_suffix('\n').unwrap_or(&data).to_owned();
        let mut expected: Vec<String> = Vec::new();
        let document = sections.get("document").cloned().unwrap_or_default();
        for line in document.trim_end_matches('\n').split('\n') {
            match line.strip_prefix("| ") {
                Some(rest) => expected.push(rest.to_owned()),
                None => {
                    if let Some(last) = expected.last_mut() {
                        last.push('\n');
                        last.push_str(line);
                    }
                }
            }
        }
        out.push(Test { data, expected });
    }
    out
}

/// The tree under `parent` as html5lib dumps it (HTML elements and text
/// are all these tests hold).
fn dump(document: &Document, parent: NodeId, depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for child in document.children(parent) {
        match document.data(child) {
            Some(NodeData::Text(text)) => out.push(format!("{indent}\"{text}\"")),
            Some(NodeData::Element { tag, attrs, ns }) => {
                let name = match ns {
                    Namespace::Svg => format!("svg {tag}"),
                    Namespace::MathMl => format!("math {tag}"),
                    _ => tag.clone(),
                };
                out.push(format!("{indent}<{name}>"));
                let mut shown: Vec<String> = attrs
                    .iter()
                    .map(|(name, value)| format!("{indent}  {name}=\"{value}\""))
                    .collect();
                shown.sort();
                out.extend(shown);
                dump(document, child, depth + 1, out);
            }
            Some(NodeData::Comment(data)) => out.push(format!("{indent}<!-- {data} -->")),
            Some(NodeData::DocumentType { name, .. }) => {
                out.push(format!("{indent}<!DOCTYPE {name}>"));
            }
            _ => {}
        }
    }
}

#[test]
fn html5libs_scripted_tree_tests_pass_with_scripts_run_mid_parse() {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../html/tests/html5lib/tree-construction");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("the tests are vendored")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("scripted_") && name.ends_with(".dat"))
        })
        .collect();
    files.sort();
    let mut ran = 0;
    let mut failed = Vec::new();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&file).expect("a test file");
        for (index, test) in tests(&text).into_iter().enumerate() {
            let data = test.data.clone();
            let server = serve(move |path| (path == "/").then(|| html(&data)));
            let mut tab = Tab::new();
            tab.engine.set_scripting(true);
            load_all(&mut tab, server.url("/"));
            let document = tab.engine.document();
            let mut got = Vec::new();
            dump(document, document.root(), 0, &mut got);
            ran += 1;
            if got != test.expected {
                failed.push(format!(
                    "{name}#{index}: {:?}\n  expected:\n    {}\n  got:\n    {}",
                    test.data,
                    test.expected.join("\n    "),
                    got.join("\n    ")
                ));
            }
        }
    }
    assert!(ran >= 6, "{ran} tests ran");
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
