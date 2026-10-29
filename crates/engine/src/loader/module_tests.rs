//! Module scripts (D61): inline and external ones with their static
//! imports, deferred and `async`, `nomodule`, each module evaluated once
//! a realm by its URL, their errors, `import()` and `import.meta`, and
//! import maps. The outputs are what the reference browser gives.

use net::test_server::Reply;

use super::tests::load_all;
use crate::testing::*;

fn scripting() -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab
}

/// The texts of the console's entries since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

fn javascript(body: &str) -> Reply {
    Reply::ok("text/javascript", body.as_bytes())
}

/// A script of `body` sent after `wait` milliseconds.
fn later(body: &str, wait: u64) -> Reply {
    let mut bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body.as_bytes());
    Reply::Pieces(vec![(std::time::Duration::from_millis(wait), bytes)])
}

#[test]
fn module_scripts_are_deferred_and_run_their_imports_first() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script type=module>console.log('inline module', document.readyState)</script>\
             <script type=module src=a.js></script>\
             <script>console.log('classic')</script>\
             <script defer src=d.js></script>\
             <script nomodule>console.log('nomodule')</script>\
             <script type=module nomodule>console.log('module nomodule')</script>\
             <script type=speculationrules>{}</script>\
             <p>x</p>",
        )),
        "/a.js" => Some(javascript(
            "import { b } from './lib/b.js'; import './lib/c.js';\n\
             console.log('a', b, document.currentScript, this);",
        )),
        "/lib/b.js" => Some(javascript(
            "import './c.js'; console.log('b'); export const b = 1;",
        )),
        "/lib/c.js" => Some(javascript("console.log('c')")),
        "/d.js" => Some(javascript("console.log('defer')")),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(
        lines(&mut tab),
        [
            "classic",
            "inline module interactive",
            "c",
            "b",
            "a 1 null undefined",
            "defer",
            "module nomodule",
        ]
    );
}

#[test]
fn an_async_module_script_runs_once_its_graph_has_come() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script type=module async>import './slow.js'; console.log('async inline')</script>\
             <script type=module async src=quick.js></script>\
             <script type=module>console.log('deferred')</script>\
             <script src=block.js></script><p>x</p>",
        )),
        "/slow.js" => Some(later("console.log('slow')", 900)),
        "/block.js" => Some(later("console.log('block')", 300)),
        "/quick.js" => Some(javascript("console.log('quick')")),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    // The async one that came while the parser waited ran then; the
    // other once its import came, after the parse.
    assert_eq!(
        lines(&mut tab),
        ["quick", "block", "deferred", "slow", "async inline"]
    );
}

#[test]
fn each_module_is_evaluated_once_by_its_url() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script type=module src=a.js onload=\"console.log('load 1')\"></script>\
             <script type=module src=a.js onload=\"console.log('load 2')\"></script>\
             <script type=module src=a.js#two></script>\
             <script type=module>import { n } from './a.js'; console.log('inline', n)</script>",
        )),
        "/a.js" => Some(javascript(
            "console.log('a', import.meta.url); export const n = 1;",
        )),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let a = server.url("/a.js");
    assert_eq!(
        lines(&mut tab),
        [
            format!("a {a}"),
            "load 1".to_owned(),
            "load 2".to_owned(),
            format!("a {a}#two"),
            "inline 1".to_owned(),
        ]
    );
}

#[test]
fn a_module_that_cannot_be_fetched_fires_error_and_one_that_throws_is_reported() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>onerror = (m, f, l, c, e) => console.log('onerror', m, l, e.name)</script>\
             <script type=module src=missing.js onerror=\"console.log('error missing')\"></script>\
             <script type=module src=text.js onerror=\"console.log('error text')\"></script>\
             <script type=module src=deep.js onerror=\"console.log('error deep')\"></script>\
             <script type=module src=syntax.js onload=\"console.log('load syntax')\"></script>\
             <script type=module>\nimport { nope } from './ok.js';</script>\
             <script type=module>import 'bare';</script>\
             <script type=module>\n\nthrow new Error('boom')</script>",
        )),
        "/text.js" => Some(Reply::ok("text/plain", b"console.log('text')")),
        "/deep.js" => Some(javascript("import './missing.js'")),
        "/syntax.js" => Some(javascript("import './ok.js';\nlet let = 1;")),
        "/ok.js" => Some(javascript("console.log('ok'); export const yes = 1;")),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let page = server.url("/page.html");
    let missing = server.url("/missing.js");
    let syntax = server.url("/syntax.js");
    let ok = server.url("/ok.js");
    let bare = "Failed to resolve module specifier \"bare\". Relative references must start \
                with either \"/\", \"./\", or \"../\".";
    // What the console says of a fetch that failed, said as its response
    // came, which the network decides.
    let mut lines = lines(&mut tab);
    let mut failed: Vec<String> = lines
        .iter()
        .filter(|line| line.starts_with("Failed to load module script"))
        .cloned()
        .collect();
    lines.retain(|line| !line.starts_with("Failed to load module script"));
    failed.sort();
    assert_eq!(
        failed,
        [
            format!("Failed to load module script {missing}: status 404"),
            "Failed to load module script: Expected a JavaScript-or-Wasm module script but the \
             server responded with a MIME type of \"text/plain\". Strict MIME type checking is \
             enforced for module scripts per HTML spec."
                .to_owned(),
        ]
    );
    assert_eq!(
        lines,
        [
            "error missing".to_owned(),
            "error text".to_owned(),
            "error deep".to_owned(),
            // A syntax error in a module it imports, reported as the
            // script runs, which then has its `load` event.
            "onerror Uncaught SyntaxError: variable name expected 2 SyntaxError".to_owned(),
            format!("Uncaught SyntaxError: variable name expected at {syntax}:2:5"),
            "load syntax".to_owned(),
            // An export its import does not have: nothing of the graph
            // runs, ok.js neither. QuickJS words it, at no place.
            format!(
                "onerror Uncaught SyntaxError: Could not find export 'nope' in module '{ok}' 0 \
                 SyntaxError"
            ),
            format!(
                "Uncaught SyntaxError: Could not find export 'nope' in module '{ok}' at \
                 {page}:0:0"
            ),
            format!("onerror Uncaught TypeError: {bare} 0 TypeError"),
            format!("Uncaught TypeError: {bare} at {page}:0:0"),
            // Its text starts on the document's line 2.
            "onerror Uncaught Error: boom 4 Error".to_owned(),
            format!("Uncaught Error: boom at {page}:4:11"),
        ]
    );
}

#[test]
fn a_graph_deeper_than_the_bound_fails_as_a_fetch_does() {
    let server = serve(|path| {
        if path == "/page.html" {
            return Some(html(
                "<script type=module src=c0.js onerror=\"console.log('error')\"></script>",
            ));
        }
        let n: usize = path.strip_prefix("/c")?.strip_suffix(".js")?.parse().ok()?;
        Some(javascript(&format!("import './c{}.js';", n + 1)))
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let deep = server.url(&format!("/c{}.js", js::MAX_MODULE_DEPTH + 1));
    assert_eq!(
        lines(&mut tab),
        [
            format!(
                "Failed to load module script {deep}: its graph is deeper than {}",
                js::MAX_MODULE_DEPTH
            ),
            "error".to_owned(),
        ]
    );
}

#[test]
fn import_fetches_its_graph_and_settles_in_a_task() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>\n\
             import('./m.js').then(m => console.log('m', m.default, m.url, m.dep));\n\
             import('./gone.js').catch(e => console.log(e.name, e.message));\n\
             import('nope').catch(e => console.log(e.name));\n\
             console.log('sync');\n\
             </script>\
             <script type=module>\n\
             console.log(import.meta.resolve('./x/y.js'), import.meta.url);\n\
             const m = await import('./m.js');\n\
             console.log('again', m.default);\n\
             </script>",
        )),
        "/m.js" => Some(javascript(
            "import { dep } from './dep.js'; export default 'm'; export { dep };\n\
             export const url = import.meta.url;",
        )),
        "/dep.js" => Some(javascript("export const dep = 'dep';")),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let mut lines = tab.run_until(|lines| lines.len() >= 7);
    let (page, gone, m) = (
        server.url("/page.html"),
        server.url("/gone.js"),
        server.url("/m.js"),
    );
    lines.retain(|line| !line.starts_with("Failed to load module script"));
    // The `import()`s' order among themselves is the network's.
    let (first, rest) = lines.split_at(3);
    assert_eq!(
        first,
        [
            "sync",
            "TypeError",
            &format!("{} {page}", server.url("/x/y.js"))
        ]
    );
    let mut rest = rest.to_vec();
    rest.sort();
    assert_eq!(
        rest,
        [
            format!("TypeError Failed to fetch dynamically imported module: {gone}"),
            "again m".to_owned(),
            format!("m m {m} dep"),
        ]
    );
}

#[test]
fn an_import_map_maps_bare_specifiers_before_the_first_module() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script type=importmap>{\n\
               \"imports\": { \"lib\": \"./lib/index.js\", \"lib/\": \"./lib/\" },\n\
               \"scopes\": { \"/lib/\": { \"lib\": \"./lib/scoped.js\" } },\n\
               \"integrity\": { \"./lib/checked.js\": \"sha256-AAAA\" },\n\
               \"extra\": true\n\
             }</script>\
             <script type=importmap onerror=\"console.log('error second')\">{}</script>\
             <script type=module>\n\
             import { x } from 'lib'; import 'lib/more.js';\n\
             console.log('x', x, import.meta.resolve('lib'));\n\
             import('lib/checked.js').catch(e => console.log(e.name));\n\
             </script>\
             <script type=importmap onerror=\"console.log('error late')\">{}</script>",
        )),
        "/lib/index.js" => Some(javascript("import { s } from 'lib'; export const x = s;")),
        "/lib/scoped.js" => Some(javascript("export const s = 'scoped';")),
        "/lib/more.js" => Some(javascript("console.log('more')")),
        "/lib/checked.js" => Some(javascript("console.log('checked')")),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.len() >= 9);
    assert_eq!(
        lines,
        [
            "An invalid top-level key \"extra\" was present in the import map.".to_owned(),
            "Only one import map is supported.".to_owned(),
            "error second".to_owned(),
            "An import map is added after module script load was triggered.".to_owned(),
            "error late".to_owned(),
            "more".to_owned(),
            format!("x scoped {}", server.url("/lib/index.js")),
            format!(
                "Failed to load module script {}: the response did not match the integrity \
                 metadata",
                server.url("/lib/checked.js")
            ),
            "TypeError".to_owned(),
        ]
    );
}

#[test]
fn an_import_map_that_is_none_is_reported_where_it_is() {
    let (tab, console) = super::script_tests::scripted(
        "<script>onerror = (m, f, l) => console.log('onerror', m, l)</script>\n\
         <script type=importmap>{ \"imports\": [] }</script>\
         <script type=module>import 'lib'</script>",
        true,
    );
    // The map is none, so the module's specifier is bare.
    let imports = "TypeError: The \"imports\" top-level key needs to be a JSON object.";
    let lib = "TypeError: Failed to resolve module specifier \"lib\". Relative references \
               must start with either \"/\", \"./\", or \"../\".";
    let page = tab.engine.url().unwrap().to_string();
    assert_eq!(
        console,
        [
            format!("onerror Uncaught {imports} 2"),
            format!("Uncaught {imports} at {page}:2:11"),
            format!("onerror Uncaught {lib} 0"),
            format!("Uncaught {lib} at {page}:0:0"),
        ]
    );
}

#[test]
fn a_module_from_another_origin_needs_cors_and_a_script_may_insert_one() {
    let other = serve(|path| match path {
        "/open.js" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/javascript"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            b"console.log('open')",
        )),
        "/closed.js" => Some(javascript("console.log('closed')")),
        _ => None,
    });
    let (open, closed) = (other.url("/open.js"), other.url("/closed.js"));
    let page = format!(
        "<script type=module src={open}></script>\
         <script type=module src={closed} onerror=\"console.log('error closed')\"></script>\
         <script>\
         const s = document.createElement('script'); s.type = 'module';\
         s.textContent = 'console.log(\"inserted\", document.currentScript)';\
         document.head.append(s); console.log('after insert');\
         </script>"
    );
    let server = serve(move |path| match path {
        "/page.html" => Some(html(&page)),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let mut lines = lines(&mut tab);
    // Said when its response came, which the network decides.
    let refused = format!("Failed to load module script {closed}: ");
    let before = lines.len();
    lines.retain(|line| !line.starts_with(&refused));
    assert_eq!(lines.len(), before - 1, "{lines:?}");
    assert_eq!(
        lines,
        ["after insert", "inserted null", "open", "error closed"]
    );
}

#[test]
fn a_module_worker_runs_its_graph_and_imports_more() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>\
             const w = new Worker('w.js', { type: 'module' });\
             w.onmessage = e => console.log(e.data);\
             const broken = new Worker('broken.js', { type: 'module' });\
             broken.onerror = e => console.log('broken', e.constructor.name);\
             </script>",
        )),
        "/w.js" => Some(javascript(
            "import { n } from './lib/n.js';\n\
             const m = await import('./lib/m.js');\n\
             const gone = await import('./gone.js').catch(e => e.name);\n\
             postMessage([n, m.default, gone, import.meta.url.split('/').pop(), typeof importScripts].join(' '));",
        )),
        "/lib/n.js" => Some(javascript("export const n = 1;")),
        "/lib/m.js" => Some(javascript(
            "import { n } from './n.js'; export default n + 1;",
        )),
        "/broken.js" => Some(javascript("import './missing.js';")),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let mut lines = tab.run_until(|lines| lines.len() >= 2);
    lines.sort();
    assert_eq!(lines, ["1 2 TypeError w.js function", "broken Event"]);
}

#[test]
fn a_module_preload_fetches_its_module_once_and_runs_nothing() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let server = serve(move |path| match path {
        "/page.html" => Some(html(
            "<script>function insert() {\
               const s = document.createElement('script'); s.type = 'module';\
               s.textContent = 'import { m } from \"./m.js\"; console.log(\"module\", m)';\
               document.documentElement.append(s);\
             }</script>\
             <link rel=modulepreload href=m.js onload=\"console.log('load'); insert()\">\
             <link rel=modulepreload href=missing.js onerror=\"console.log('error')\">\
             <link rel=modulepreload as=style href=m.js onerror=\"console.log('error as')\">",
        )),
        "/m.js" => {
            counted.fetch_add(1, Ordering::SeqCst);
            Some(javascript("console.log('m'); export const m = 1;"))
        }
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let mut lines = tab.run_until(|lines| lines.len() >= 6);
    let failed = format!(
        "Failed to load module script {}: status 404",
        server.url("/missing.js")
    );
    lines.retain(|line| *line != failed);
    // The preloaded module ran only once a module script imported it,
    // which took it from the map.
    let at = |line: &str| lines.iter().position(|l| l == line).expect(line);
    assert!(
        at("load") < at("m") && at("m") < at("module 1"),
        "{lines:?}"
    );
    lines.sort();
    assert_eq!(lines, ["error", "error as", "load", "m", "module 1"]);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}
