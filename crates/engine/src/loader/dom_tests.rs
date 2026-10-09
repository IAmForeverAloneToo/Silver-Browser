//! The engine giving scripts the DOM (M25, docs/design/dom-scripting.md):
//! node objects over the document's nodes, the tree read and written
//! through them, the exceptions, the work charged, and the nodes scripts
//! let go of freed.

use super::script_tests::{scripted, scripted_bytes};
use crate::testing::*;

/// The console's lines of `page` loaded with scripts on.
fn logged(page: &'static str) -> Vec<String> {
    scripted(page, true).1
}

#[test]
fn a_script_reads_the_tree_the_parser_built() {
    let console = logged(
        "<title>t</title><p id=a>one</p><p>two</p>\
         <script>var html = document.documentElement, body = html.lastChild; \
         console.log(html.nodeName, html.firstChild.nodeName, body.nodeName, \
         body.firstChild.nodeName, body.firstChild.firstChild.data, \
         body.firstChild.nextSibling.textContent, body.lastChild.nodeName, \
         document.firstChild === html, html.parentNode === document, \
         Object.prototype.toString.call(body.firstChild))</script>",
    );
    assert_eq!(
        console,
        ["HTML HEAD BODY P one two SCRIPT true true [object HTMLParagraphElement]"]
    );
}

#[test]
fn nodes_a_script_makes_and_moves_are_the_documents() {
    let (tab, console) = scripted(
        "<p id=a>a</p><p id=b>b</p>\
         <script>var body = document.documentElement.lastChild; \
         var a = body.firstChild, b = a.nextSibling; \
         var div = document.createElement('DIV'); \
         div.appendChild(document.createTextNode('made')); \
         body.appendChild(div); \
         body.appendChild(a); \
         body.removeChild(b); \
         console.log(div.localName, div.textContent, a.parentNode === body)</script>",
        true,
    );
    assert_eq!(console, ["div made true"]);
    let document = tab.engine.document();
    let body = element(&tab.engine, "body");
    let tags: Vec<_> = document
        .children(body)
        .map(|child| document.tag(child).unwrap_or("#text"))
        .collect();
    assert_eq!(tags, ["script", "div", "p"]);
    document.validate().unwrap();
}

#[test]
fn a_failed_call_throws_what_the_reference_browser_throws() {
    let console = logged(
        "<p>x</p><script>\
         var html = document.documentElement, body = html.lastChild, p = body.firstChild; \
         function f(g) { try { g(); return 'no error' } catch (e) { return e.name + ': ' + e.message } } \
         console.log(f(() => p.appendChild(body))); \
         console.log(f(() => p.firstChild.appendChild(document.createElement('i')))); \
         console.log(f(() => body.removeChild(document.createElement('i')))); \
         console.log(f(() => document.createElement('a b'))); \
         console.log(f(() => document.createElement(''))); \
         console.log(f(() => document.appendChild(document.createTextNode('x')))); \
         console.log(f(() => document.appendChild(document.createElement('html')))); \
         console.log(f(() => body.appendChild(document))); \
         console.log(f(() => p.appendChild('x')));\
         </script>",
    );
    assert_eq!(
        console,
        [
            "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': The new child \
             element contains the parent.",
            "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': This node type \
             does not support this method.",
            "NotFoundError: Failed to execute 'removeChild' on 'Node': The node to be removed \
             is not a child of this node.",
            "InvalidCharacterError: Failed to execute 'createElement' on 'Document': The tag \
             name provided ('a b') is not a valid name.",
            "InvalidCharacterError: Failed to execute 'createElement' on 'Document': The tag \
             name provided ('') is not a valid name.",
            "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': Nodes of type \
             '#text' may not be inserted inside nodes of type '#document'.",
            "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': Only one \
             element on Document allowed.",
            "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': The new child \
             element contains the parent.",
            "TypeError: Failed to execute 'appendChild' on 'Node': parameter 1 is not of type \
             'Node'.",
        ]
    );
}

#[test]
fn nodes_scripts_let_go_of_are_freed_and_those_they_hold_are_not() {
    let page = "<p>x</p><script>\
                for (var i = 0; i < 1000; i++) document.createElement('p').appendChild(document.createTextNode('t')); \
                var kept = document.createElement('kept'); \
                var body = document.documentElement.lastChild; \
                var gone = body.appendChild(document.createElement('gone')); \
                gone.appendChild(document.createElement('child')); \
                body.removeChild(gone); gone = null; \
                var held = body.appendChild(document.createElement('held')); \
                var inner = held.appendChild(document.createElement('inner')); \
                body.removeChild(held); held = null;\
                </script><script>console.log(inner.parentNode.nodeName); inner = null</script>";
    let (tab, console) = scripted(page, true);
    assert_eq!(console, ["HELD"]);
    let document = tab.engine.document();
    document.validate().unwrap();
    // The document, html, head, body and p, "x", the scripts and their
    // texts.
    let parsed = 1 + 4 + 1 + 2 + 2;
    // What is left: the parsed nodes and `kept`, but nothing let go of,
    // `held` among them once the second script let go of `inner`.
    assert_eq!(document.len(), parsed + 1);
    let tags: Vec<_> = document
        .subtree(document.root())
        .filter_map(|n| document.tag(n))
        .collect();
    assert!(!tags.contains(&"gone") && !tags.contains(&"held"));
}

#[test]
fn a_node_the_parser_still_fills_can_be_taken_out_and_freed() {
    // The body is taken out while the parser is inside it, and let go
    // of: the parser's insertions into it go nowhere, and nothing breaks.
    let (tab, console) = scripted(
        "<body><p>a</p><script>var html = document.documentElement; \
         html.removeChild(html.lastChild);</script><p>b</p><p>c</p>\
         <script>console.log(document.documentElement.lastChild.nodeName)</script>",
        true,
    );
    // The second script went into the freed body: it never ran.
    assert!(console.is_empty(), "{console:?}");
    let document = tab.engine.document();
    document.validate().unwrap();
    let tags: Vec<_> = document
        .subtree(document.root())
        .filter_map(|n| document.tag(n))
        .collect();
    assert_eq!(tags, ["html", "head"]);
}

#[test]
fn the_work_of_native_calls_counts_against_the_budget() {
    let mut page = String::from("<div>");
    for _ in 0..2_000 {
        page.push_str("<i>x</i>");
    }
    page.push_str(
        "</div><script>var body = document.documentElement.lastChild; \
         for (var i = 0; i < 1000; i++) body.textContent; console.log('finished')</script>",
    );
    let limits = js::Limits {
        steps: 100,
        ..js::Limits::default()
    };
    let (_, console) = scripted_bytes(page.clone().into_bytes(), Some(limits));
    assert_eq!(console.len(), 1);
    assert!(console[0].starts_with("Script ran too long"), "{console:?}");
    // The same loop over a small document is within the budget.
    let small = page.replace("<i>x</i>", "");
    let (_, console) = scripted_bytes(small.into_bytes(), Some(limits));
    assert_eq!(console, ["finished"]);
}
