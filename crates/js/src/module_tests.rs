//! Module scripts in a realm (D61): static imports found by declaring a
//! module apart, a graph linked and evaluated once a realm, its errors,
//! `import()` waiting for the engine's fetch, `import.meta`, and the
//! bootstrap's modules out of reach.

use crate::tests::{Recorder, realm, script};
use crate::{ErrorKind, MAX_MODULE_DEPTH, ModuleKind, ModuleScript, ModuleSource, Script, Task};

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
        options: 0,
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
    let specifiers: Vec<&str> = found.iter().map(|r| r.specifier.as_str()).collect();
    assert_eq!(specifiers, ["./x.js", "./y.js", "./z.js"]);
    assert!(found.iter().all(|r| r.kind == Some(ModuleKind::Js)));
    // A regular expression in it compiles there too (found by css-logical's
    // test modules, whose imports were taken for none).
    let found = realm
        .module_requests(
            "https://example.com/r.js",
            "import { a } from './a.js'; export const b = 'a-b'.replace(/-(.)/g, '$1');",
        )
        .expect("it parses");
    assert_eq!(found.len(), 1);
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
        options: 0,
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

/// A classic script at `url` whose base URL is `base` and whose fetch
/// options are numbered `options`.
fn script_at<'a>(source: &'a str, url: &'a str, base: &'a str, options: u32) -> Script<'a> {
    Script {
        source,
        url,
        line: 1,
        muted: false,
        base,
        options,
    }
}

#[test]
fn import_in_code_a_string_compiled_resolves_against_the_script_that_made_it() {
    let mut realm = realm();
    let mut host = Recorder::default();
    // An external script: `eval`, an indirect `eval`, `Function` and a
    // timer's string resolve against its URL and fetch with its options;
    // a function it made keeps them when another script calls it later.
    let external = "eval(\"import('./a.js')\");\n\
                    (0, eval)(\"import('./b.js')\");\n\
                    Function(\"return import('./c.js')\")();\n\
                    setTimeout(\"import('./d.js')\");\n\
                    var later = () => eval(\"import('./e.js')\");";
    let url = "https://example.com/lib/s.js";
    realm
        .run(&script_at(external, url, "", 5), &mut host)
        .expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    realm.enter(&Task::Timer(1), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    // An inline script whose base URL was another when it was prepared:
    // its own code resolves against that, and calls the other's function.
    let inline = "eval(\"import('./f.js')\"); later();";
    let base = "https://example.com/dir/x";
    realm
        .run(&script_at(inline, PAGE, base, 7), &mut host)
        .expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    let urls: Vec<&str> = host.imports.iter().map(|(_, url)| url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://example.com/lib/a.js",
            "https://example.com/lib/b.js",
            "https://example.com/lib/c.js",
            "https://example.com/lib/d.js",
            "https://example.com/dir/f.js",
            "https://example.com/lib/e.js",
        ]
    );
    assert_eq!(host.import_options, [5, 5, 5, 5, 7, 5]);
}

#[test]
fn json_css_and_text_modules_are_their_values_and_attributes_are_checked() {
    let mut realm = realm();
    let mut host = Recorder::default();
    // The imports a module names, with the kinds they ask for.
    let found = realm
        .module_requests(
            "https://example.com/a.js",
            "import a from './a.json' with { type: 'json' };\n\
             import b from './b.txt' with { type: 'text' };\n\
             import c from './c.js' with { type: 'nope' };\n\
             import d from './d.js' with { other: 'x' };",
        )
        .expect("it parses");
    let kinds: Vec<Option<ModuleKind>> = found.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        [Some(ModuleKind::Json), Some(ModuleKind::Text), None, None]
    );
    // A JSON module and a text module given by their names: each module
    // is its value, once.
    let json = crate::module_name("https://example.com/a.json", ModuleKind::Json);
    let text = crate::module_name("https://example.com/b.txt", ModuleKind::Text);
    let modules = [
        module(&json, "{\"n\": [1, 2], \"__proto__\": 3}"),
        module(&text, "plain \"text\""),
    ];
    let first = "import a from './a.json' with { type: 'json' };\n\
                 import b from './b.txt' with { type: 'text' };\n\
                 import again from './a.json' with { type: 'json' };\n\
                 console.log(a.n.join(), Object.getOwnPropertyNames(a).join(), a === again, b);";
    realm
        .run_module(&inline(first, &modules), &mut host)
        .expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(lines(&mut host), ["1,2 n,__proto__ true plain \"text\""]);
    // A JSON module that does not parse fails linking with its syntax
    // error; a static import with a key other than `type` is a syntax
    // error, a type unknown a type error.
    let bad = crate::module_name("https://example.com/bad.json", ModuleKind::Json);
    let error = realm
        .run_module(
            &inline(
                "import x from './bad.json' with { type: 'json' }",
                &[module(&bad, "{x")],
            ),
            &mut host,
        )
        .expect_err("it does not parse");
    assert!(error.message.starts_with("SyntaxError"), "{error:?}");
    let error = realm
        .run_module(
            &inline("import './a.js' with { other: 'x' }", &[]),
            &mut host,
        )
        .expect_err("a key refused");
    assert_eq!(
        error.message,
        "SyntaxError: Invalid attribute key \"other\"."
    );
    let error = realm
        .run_module(
            &inline("import './a.js' with { type: 'nope' }", &[]),
            &mut host,
        )
        .expect_err("a type unknown");
    assert_eq!(
        error.message,
        "TypeError: \"nope\" is not a valid module type."
    );
    // `import()` rejects both with type errors before any fetch, and asks
    // for a JSON module by its name.
    let source = "import('./x.js', { with: { other: 'x' } }).catch(e => console.log(String(e)));\n\
                  import('./x.js', { with: { type: 'nope' } }).catch(e => console.log(String(e)));\n\
                  import('./y.json', { with: { type: 'json' } }).then(m => console.log(m.default));";
    realm.run(&script(source), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(
        lines(&mut host),
        [
            "TypeError: Invalid attribute key \"other\".",
            "TypeError: \"nope\" is not a valid module type.",
        ]
    );
    let json = crate::module_name("https://example.com/y.json", ModuleKind::Json);
    let urls: Vec<&str> = host.imports.iter().map(|(_, url)| url.as_str()).collect();
    assert_eq!(urls, [json.as_str()]);
    // Its graph come, the promise settles with the module of its type.
    let came = Task::Imported {
        id: host.imports[0].0,
        url: json.clone(),
        modules: vec![module(&json, "\"given\"")],
        error: None,
    };
    realm.enter(&came, &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(lines(&mut host), ["given"]);
}

#[test]
fn import_meta_resolve_converts_its_argument_to_a_string() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let source = "try { import.meta.resolve(); } catch (e) { console.log(e.message) }\n\
                  console.log(import.meta.resolve({ toString() { return './t.js'; } }))";
    realm
        .run_module(&inline(source, &[]), &mut host)
        .expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(
        lines(&mut host),
        [
            "Failed to resolve module specifier \"undefined\". Relative references must start \
             with either \"/\", \"./\", or \"../\".",
            "https://example.com/t.js",
        ]
    );
}

#[test]
fn a_module_that_fails_to_parse_throws_one_error_object_each_time() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let source = "import('./bad.js').catch(e => { globalThis.first = e; \
                  return import('./bad.js'); }).catch(e => console.log(e.name, e === first));";
    realm.run(&script(source), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    let came = Task::Imported {
        id: 1,
        url: "https://example.com/bad.js".to_owned(),
        modules: vec![module("https://example.com/bad.js", "export const = ;")],
        error: None,
    };
    realm.enter(&came, &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(lines(&mut host), ["SyntaxError true"]);
    // The second asked the engine for nothing.
    assert_eq!(host.imports.len(), 1);
}
