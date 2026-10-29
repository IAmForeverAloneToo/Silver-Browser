//! Module scripts in a realm (D61): static imports found by declaring a
//! module apart, a graph linked and evaluated once a realm, its errors,
//! `import()` waiting for the engine's fetch, `import.meta`, and the
//! bootstrap's modules out of reach.

use crate::tests::{Recorder, realm, script};
use crate::{ErrorKind, MAX_MODULE_DEPTH, ModuleScript, ModuleSource, Task};

const PAGE: &str = "https://example.com/page.html";

fn module(url: &str, text: &str) -> ModuleSource {
    ModuleSource {
        url: url.to_owned(),
        base: url.to_owned(),
        text: text.to_owned(),
    }
}

fn inline<'a>(text: &'a str, modules: &'a [ModuleSource]) -> ModuleScript<'a> {
    ModuleScript {
        url: PAGE,
        inline: Some((text, 1)),
        modules,
    }
}

/// The console's lines since they were last taken.
fn lines(host: &mut Recorder) -> Vec<String> {
    std::mem::take(&mut host.console)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

#[test]
fn a_modules_static_imports_are_found_by_declaring_it_apart() {
    let mut realm = realm();
    let found = realm
        .module_requests(
            "https://example.com/a.js",
            "import x from './x.js'; export * from './y.js'; import './x.js';\n\
             import('./dynamic.js'); export { z } from './z.js';",
        )
        .expect("it parses");
    assert_eq!(found, ["./x.js", "./y.js", "./z.js"]);
    let error = realm
        .module_requests("https://example.com/b.js", "import {")
        .expect_err("a syntax error");
    assert_eq!(error.kind, ErrorKind::Exception);
    assert!(error.message.starts_with("SyntaxError"), "{error:?}");
    // Nothing was declared in the realm: a module importing x.js, which
    // it was not given, cannot be linked.
    let mut host = Recorder::default();
    let error = realm
        .run_module(&inline("import x from './x.js'", &[]), &mut host)
        .expect_err("x.js was never given");
    assert_eq!(
        error.message,
        "TypeError: Failed to load module script: https://example.com/x.js"
    );
}

#[test]
fn a_module_script_links_its_graph_and_each_module_runs_once() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let modules = [
        module(
            "https://example.com/lib/a.js",
            "console.log('a', import.meta.url); export const a = 1;",
        ),
        module(
            "https://example.com/lib/b.js",
            "import { a } from './a.js'; console.log('b', a); export default 2;",
        ),
    ];
    let first = "import b from './lib/b.js'; import { a } from './lib/a.js';\n\
                 console.log('inline', a + b, import.meta.url, import.meta.resolve('./c.js'),\n\
                 Object.keys(import.meta).join(), Object.getPrototypeOf(import.meta), this);";
    realm
        .run_module(&inline(first, &modules), &mut host)
        .expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    // A second script importing a.js again: its body does not run again.
    let again = "import { a } from './lib/a.js'; console.log('again', a)";
    realm
        .run_module(&inline(again, &[]), &mut host)
        .expect("it runs");
    // An external module script that is a module declared already.
    let external = ModuleScript {
        url: "https://example.com/lib/b.js",
        inline: None,
        modules: &[],
    };
    realm.run_module(&external, &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(
        lines(&mut host),
        [
            "a https://example.com/lib/a.js",
            "b 1",
            "inline 3 https://example.com/page.html https://example.com/c.js url,resolve null undefined",
            "again 1",
        ]
    );
}

#[test]
fn errors_linking_and_evaluating_a_module_are_reported() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let modules = [module("https://example.com/a.js", "export const a = 1;")];
    // An export it has not: a syntax error linking, returned.
    let error = realm
        .run_module(
            &inline("import { nope } from './a.js'", &modules),
            &mut host,
        )
        .expect_err("it does not link");
    assert_eq!(error.kind, ErrorKind::Exception);
    assert!(error.message.starts_with("SyntaxError"), "{error:?}");
    // A bare specifier: a type error.
    let error = realm
        .run_module(&inline("import 'lodash'", &[]), &mut host)
        .expect_err("it does not resolve");
    assert_eq!(
        error.message,
        "TypeError: Failed to resolve module specifier \"lodash\". Relative references must \
         start with either \"/\", \"./\", or \"../\"."
    );
    // An exception evaluating it, and a top-level await's rejection:
    // reported by the bootstrap as uncaught, at the document's lines.
    realm
        .run_module(
            &inline(
                "import { a } from './a.js';\nthrow new Error('boom ' + a)",
                &[],
            ),
            &mut host,
        )
        .expect("it links");
    realm
        .run_module(
            &inline("await null;\n\nthrow new TypeError('later')", &[]),
            &mut host,
        )
        .expect("it links");
    assert!(realm.checkpoint(&mut host).is_empty());
    let reported: Vec<(&str, &str, u32)> = host
        .reported
        .iter()
        .map(|e| (e.message.as_str(), e.url.as_str(), e.line))
        .collect();
    assert_eq!(
        reported,
        [("Error: boom 1", PAGE, 2), ("TypeError: later", PAGE, 3)]
    );
}

#[test]
fn import_waits_for_the_graph_the_engine_fetches() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let source = "import('./m.js').then(m => console.log('m', m.default, m[Symbol.toStringTag]));\n\
                  import('./gone.js').catch(e => console.log(String(e)));\n\
                  import('nope').catch(e => console.log(e.name));";
    realm.run(&script(source), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(lines(&mut host), ["TypeError"]);
    assert_eq!(
        host.imports,
        [
            (1, "https://example.com/m.js".to_owned()),
            (2, "https://example.com/gone.js".to_owned()),
        ]
    );
    let gone = Task::Imported {
        id: 2,
        url: "https://example.com/gone.js".to_owned(),
        modules: Vec::new(),
        error: Some(
            "Failed to fetch dynamically imported module: https://example.com/gone.js".to_owned(),
        ),
    };
    realm.enter(&gone, &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    // Its graph's modules, n.js among them, given with it.
    let found = Task::Imported {
        id: 1,
        url: "https://example.com/m.js".to_owned(),
        modules: vec![
            module(
                "https://example.com/m.js",
                "import { n } from './n.js'; export default n + 1;",
            ),
            module("https://example.com/n.js", "export const n = 6;"),
        ],
        error: None,
    };
    realm.enter(&found, &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(
        lines(&mut host),
        [
            "TypeError: Failed to fetch dynamically imported module: https://example.com/gone.js",
            "m 7 Module",
        ]
    );
    // Imported again: no fetch, the same module.
    let again =
        "import('./m.js').then(m => import('./n.js').then(n => console.log(m.default, n.n)))";
    realm.run(&script(again), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(lines(&mut host), ["7 6"]);
    assert_eq!(host.imports.len(), 2);
}

/// A chain of `depth` modules, each importing the next, given leaves
/// first as the engine gives them.
fn chain(depth: usize) -> Vec<ModuleSource> {
    (0..depth)
        .rev()
        .map(|i| {
            let text = match i + 1 < depth {
                true => format!(
                    "import {{ v as w }} from './m{}.js'; export const v = w + 1;",
                    i + 1
                ),
                false => "export const v = 1;".to_owned(),
            };
            module(&format!("https://example.com/m{i}.js"), &text)
        })
        .collect()
}

#[test]
fn a_graph_as_deep_as_the_bound_links_within_the_stack() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let modules = chain(MAX_MODULE_DEPTH);
    realm
        .run_module(
            &inline("import { v } from './m0.js'; console.log(v)", &modules),
            &mut host,
        )
        .expect("it links");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(lines(&mut host), [MAX_MODULE_DEPTH.to_string()]);
}

#[test]
fn the_bootstraps_modules_are_none_a_page_may_import() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let source = "for (const name of ['bootstrap', 'bootstrap-gpu']) {\n\
                    import(name).then(() => console.log('imported'), e => console.log(e.name));\n\
                  }";
    realm.run(&script(source), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(lines(&mut host), ["TypeError", "TypeError"]);
    assert!(host.imports.is_empty());
}
