//! The engine giving scripts the DOM (M25, docs/design/dom-scripting.md):
//! node objects over the document's nodes, the tree read and written
//! through them, the exceptions, the work charged, and the nodes scripts
//! let go of freed.

use super::script_tests::{scripted, scripted_bytes};
use super::tests::load_all;
use crate::testing::*;
use net::test_server::Reply;

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
             element on document allowed.",
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
    let (mut tab, console) = scripted(page, true);
    assert_eq!(console, ["HELD"]);
    // The objects of `held` and `inner` hold each other, as their tree's
    // (see the bindings' `wrap`): they go with the collector's pass, and
    // the nodes with the sweep after the run that hears of it.
    collected_then(&mut tab, "var after = 1");
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
fn a_templates_contents_live_while_a_script_holds_a_node_in_them() {
    // The template let go of is kept for the node in its contents (D58).
    let page = "<p>x</p><script>                var body = document.documentElement.lastChild;                 var t = body.appendChild(document.createElement('template'));                 var inner = t.content.appendChild(document.createElement('inner'));                 body.removeChild(t); t = null;                </script><script>console.log(inner.parentNode.nodeName)</script>";
    let (tab, console) = scripted(page, true);
    assert_eq!(console, ["#document-fragment"]);
    tab.engine.document().validate().unwrap();
}

#[test]
fn a_node_a_live_range_holds_is_kept() {
    let page = "<p>x</p><script>\
                var gone = document.createElement('gone'); \
                gone.appendChild(document.createElement('child')); \
                var range = document.createRange(); range.setStart(gone, 1); \
                gone = null;\
                </script><script>console.log(range.startContainer.nodeName)</script>";
    let (tab, console) = scripted(page, true);
    assert_eq!(console, ["GONE"]);
    tab.engine.document().validate().unwrap();
}

/// Runs `script` in the tab's page after collecting its realm's garbage,
/// and the tasks it queued; the console's lines.
fn collected_then(tab: &mut Tab, script: &str) -> Vec<String> {
    tab.engine.shown.scripts.collect();
    tab.engine.evaluate_script(script);
    tab.engine.run_tasks();
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

#[test]
fn an_object_lives_while_its_tree_can_be_reached() {
    // Each object here is held only by its tree: a slot's listener, a
    // node's expando of another document, a template's contents, the
    // document a node of it reaches, a range's node.
    let (mut tab, console) = scripted(
        "<p>x</p><script>\
         var host = document.createElement('div'); \
         var root = host.attachShadow({mode: 'closed'}); \
         var slot = root.appendChild(document.createElement('slot')); slot.name = 'n'; \
         slot.addEventListener('slotchange', (e) => console.log('slotchange ' + e.target.mark)); \
         slot.mark = 'kept'; slot = null; root = null; \
         var parsed = new DOMParser().parseFromString('<p>a</p>', 'text/html'); \
         parsed.body.firstChild.mark = 'parsed'; \
         var t = document.createElement('template'); \
         t.content.appendChild(document.createElement('i')).mark = 'content'; \
         var other = document.implementation.createHTMLDocument(''); other.mark = 'owner'; \
         var made = other.createElement('b'); other = null; \
         var range = document.createRange(); \
         var start = document.createElement('s'); start.mark = 'range'; \
         range.setStart(start, 0); start = null; \
         </script>",
        true,
    );
    assert!(console.is_empty(), "{console:?}");
    let seen = collected_then(
        &mut tab,
        "var s = document.createElement('span'); s.slot = 'n'; host.appendChild(s); \
         console.log(parsed.body.firstChild.mark, t.content.firstChild.mark, \
         made.ownerDocument.mark, range.startContainer.mark)",
    );
    assert_eq!(seen, ["parsed content owner range", "slotchange kept"]);
    // Taken out of the document, a node's object keeps with its new tree,
    // and put back, with the document's.
    let seen = collected_then(
        &mut tab,
        "var outer = document.body.appendChild(document.createElement('div')); \
         outer.appendChild(document.createElement('em')).mark = 'moved'; \
         document.body.removeChild(outer);",
    );
    assert!(seen.is_empty(), "{seen:?}");
    let seen = collected_then(
        &mut tab,
        "console.log(outer.firstChild.mark); document.body.appendChild(outer); outer = null",
    );
    assert_eq!(seen, ["moved"]);
    let seen = collected_then(&mut tab, "console.log(document.querySelector('em').mark)");
    assert_eq!(seen, ["moved"]);
}

#[test]
fn a_tree_made_alone_keeps_its_objects_once_another_joins() {
    // G247: a node's object alone in its tree out of the document has no
    // group until a second object joins the tree; then the group holds
    // both, whichever way the second came. Each tree's root has an
    // expando and is let go of; a node under it, held, reaches it.
    let (mut tab, console) = scripted("<p>x</p><script>var k = null</script>", true);
    assert!(console.is_empty(), "{console:?}");
    let cases = [
        (
            "appendChild",
            "var r = document.createElement('div'); r.mark = 'appendChild'; \
             k = r.appendChild(document.createElement('i')); r = null",
        ),
        (
            "insertBefore",
            "var r = document.createElement('div'); r.mark = 'insertBefore'; \
             k = r.insertBefore(document.createElement('i'), null); r = null",
        ),
        (
            "innerHTML",
            "var r = document.createElement('div'); r.mark = 'innerHTML'; \
             r.innerHTML = '<i></i>'; k = r.firstChild; r = null",
        ),
        (
            "parser",
            "var r = document.createRange().createContextualFragment('<i></i>'); \
             r.mark = 'parser'; k = r.firstChild; r = null",
        ),
        (
            "adoptNode",
            "var o = document.implementation.createHTMLDocument(''); \
             var r = o.createElement('div'); r.appendChild(o.createElement('i')); \
             document.adoptNode(r); r.mark = 'adoptNode'; k = r.firstChild; r = null; o = null",
        ),
        (
            "moveBefore",
            "var r = document.createElement('div'); r.mark = 'moveBefore'; \
             var a = r.appendChild(document.createElement('i')); \
             k = r.appendChild(document.createElement('b')); r.moveBefore(k, a); \
             r = null; a = null",
        ),
        (
            "reordered",
            "var r = document.createElement('div'); r.mark = 'reordered';              var a = r.appendChild(document.createElement('i'));              k = r.appendChild(document.createElement('b')); r.insertBefore(k, a);              r = null; a = null",
        ),
        (
            "a child's object",
            "var r = document.createElement('div'); r.append(document.createElement('i')); \
             r.mark = \"a child's object\"; k = r.firstChild; r = null",
        ),
    ];
    for (name, script) in cases {
        let seen = collected_then(&mut tab, script);
        assert!(seen.is_empty(), "{name}: {seen:?}");
        let seen = collected_then(&mut tab, "console.log(k.parentNode.mark)");
        assert_eq!(seen, [name]);
    }
}

#[test]
fn trees_made_and_let_go_of_leave_nothing_behind() {
    // G247: ten thousand trees out of the document, with expandos and
    // without, made and let go of, round after round: once collected,
    // the document's nodes are back where they were, and the realm's
    // heap no bigger than after the first round, whose objects' room its
    // maps keep (they do not shrink).
    let (mut tab, _) = scripted("<p>x</p><script>var a = 1</script>", true);
    collected_then(&mut tab, "var b = 2");
    collected_then(&mut tab, "var c = 3");
    let nodes = tab.engine.document().len();
    let mut heap = 0;
    for round in 0..3 {
        let seen = collected_then(
            &mut tab,
            "(function () { \
               for (var i = 0; i < 10000; i++) { \
                 var t = document.createElement('table'); \
                 var row = t.appendChild(document.createElement('tr')); \
                 row.appendChild(document.createElement('td')).textContent = 'cell ' + i; \
                 if (i % 2 === 0) { t.mark = i; row.mark = t; } \
                 document.createTextNode('alone ' + i); \
               } \
             })()",
        );
        assert!(seen.is_empty(), "{seen:?}");
        collected_then(&mut tab, "var d = 4");
        collected_then(&mut tab, "var e = 5");
        let document = tab.engine.document();
        document.validate().unwrap();
        assert_eq!(document.len(), nodes, "round {round}");
        let now = tab.engine.shown.scripts.heap();
        if round == 0 {
            heap = now;
        } else {
            assert!(
                now <= heap + heap / 100,
                "round {round}: the heap grew from {heap} to {now} bytes"
            );
        }
    }
}

#[test]
fn a_tree_nothing_reaches_is_freed_with_its_objects() {
    // The objects of a tree out of the document hold each other, their
    // listeners and expandos among them, and nothing else holds them.
    let (mut tab, _) = scripted("<p>x</p><script>var a = 1</script>", true);
    let before = tab.engine.document().len();
    let seen = collected_then(
        &mut tab,
        "(function () { \
           var host = document.createElement('div'); \
           var root = host.attachShadow({mode: 'open'}); \
           var slot = root.appendChild(document.createElement('slot')); \
           slot.addEventListener('slotchange', () => host.mark = slot); \
           host.appendChild(document.createElement('span')).mark = host; \
           var parsed = new DOMParser().parseFromString('<p>a</p>', 'text/html'); \
           parsed.body.mark = host; host.other = parsed; \
         })()",
    );
    assert!(seen.is_empty(), "{seen:?}");
    // The collector's pass lets the objects go, the next run tells the
    // host, and its sweep frees the nodes.
    collected_then(&mut tab, "var b = 2");
    collected_then(&mut tab, "var c = 3");
    let document = tab.engine.document();
    document.validate().unwrap();
    assert_eq!(document.len(), before);
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
fn a_script_the_parser_puts_in_a_body_taken_out_does_not_run() {
    // Held by a script, the body taken out still takes what the parser
    // inserts, and a script element there is not connected when it is
    // prepared (HTML section 4.12.1.1): it never runs, as in the
    // reference browser.
    let (mut tab, console) = scripted(
        "<head><script>var log = [];</script></head><body><p>a</p><script>\
         var html = document.documentElement; \
         var b = html.removeChild(html.lastChild); log.push('first');</script>\
         <p>b</p><p>c</p><script>log.push('second')</script><script>log.push('third')</script>",
        true,
    );
    assert!(console.is_empty(), "{console:?}");
    let seen = collected_then(&mut tab, "console.log(log.join(), b.children.length)");
    assert_eq!(seen, ["first 6"]);
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

/// What each expression of `cases` gives in a page with scripts on, as
/// the reference browser printed it: `ok <JSON>` for a value (a node as
/// its name), or `<name>: <message>` for an exception.
fn probed(setup: &str, cases: &[(&str, &str)]) {
    probed_in("<p id=a>x</p>", setup, cases);
}

/// [`probed`] in a page whose body is `body`.
fn probed_in(body: &str, setup: &str, cases: &[(&str, &str)]) {
    let mut page = String::from(body);
    page.push_str(
        "<script>\
         function f(g) { try { var r = g(); return 'ok ' + JSON.stringify(r === undefined ? \
         'undefined' : (r && r.nodeName) || r); } catch (e) { return e.name + ': ' + e.message; } } \
         function names(d) { var r = []; for (var n = d.firstChild; n; n = n.nextSibling) \
         r.push(n.nodeName === '#text' ? n.data : n.nodeName); return r.join(','); } \
         function tags(l) { return Array.prototype.map.call(l, function (n) { \
         return n.nodeName + (n.id ? '#' + n.id : ''); }).join(','); }",
    );
    page.push_str(setup);
    for (expression, _) in cases {
        page.push_str("console.log(f(() => ");
        page.push_str(expression);
        page.push_str("));\n");
    }
    page.push_str("</script>");
    let (_, console) = scripted_bytes(page.into_bytes(), Some(js::Limits::default()));
    let expected: Vec<&str> = cases.iter().map(|(_, result)| *result).collect();
    for (i, (line, want)) in console.iter().zip(&expected).enumerate() {
        assert_eq!(line, want, "case {i}: {}", cases[i].0);
    }
    assert_eq!(console.len(), expected.len(), "{console:?}");
}

#[test]
fn character_data_and_text_as_the_reference_browser_has_them() {
    probed(
        "var t = document.createTextNode('abc');",
        &[
            (
                "t.substringData(5, 1)",
                "IndexSizeError: Failed to execute 'substringData' on 'CharacterData': The offset \
                 5 is greater than the node's length (3).",
            ),
            ("t.substringData(1, 10)", "ok \"bc\""),
            ("t.substringData(3, 1)", "ok \"\""),
            (
                "t.insertData(9, 'z')",
                "IndexSizeError: Failed to execute 'insertData' on 'CharacterData': The offset 9 \
                 is greater than the node's length (3).",
            ),
            (
                "t.deleteData(9, 1)",
                "IndexSizeError: Failed to execute 'deleteData' on 'CharacterData': The offset 9 \
                 is greater than the node's length (3).",
            ),
            (
                "t.replaceData(9, 1, 'q')",
                "IndexSizeError: Failed to execute 'replaceData' on 'CharacterData': The offset 9 \
                 is greater than the node's length (3).",
            ),
            (
                "t.splitText(9)",
                "IndexSizeError: Failed to execute 'splitText' on 'Text': The offset 9 is larger \
                 than the Text node's length.",
            ),
            ("t.cloneNode().data", "ok \"abc\""),
            (
                "(function () { var x = document.createTextNode('q'); x.data = null; return x.data; })()",
                "ok \"\"",
            ),
            (
                "(function () { var d = document.createElement('div'); d.append('a', 'b', \
                 document.createElement('i'), 'c'); return d.childNodes[1].wholeText; })()",
                "ok \"ab\"",
            ),
            (
                "(function () { var d = document.createElement('div'); d.append('hello'); \
                 var r = d.firstChild.splitText(2); \
                 return d.childNodes.length + ' ' + d.firstChild.data + ' ' + r.data; })()",
                "ok \"2 he llo\"",
            ),
            (
                "(function () { var x = document.createTextNode('0123'); x.appendData('45'); \
                 x.insertData(0, '-'); x.deleteData(1, 2); x.replaceData(1, 1, 'three'); \
                 return x.data + ' ' + x.length + ' ' + x.nodeValue; })()",
                "ok \"-three345 9 -three345\"",
            ),
            (
                "(function () { var x = document.createTextNode('a\u{1F600}b'); \
                 return x.length + ' ' + x.substringData(1, 2); })()",
                "ok \"4 \u{1F600}\"",
            ),
        ],
    );
}

#[test]
fn attributes_as_the_reference_browser_has_them() {
    probed(
        "var e = document.createElement('div');",
        &[
            (
                "e.setAttribute('a b', '1')",
                "InvalidCharacterError: Failed to execute 'setAttribute' on 'Element': 'a b' is \
                 not a valid attribute name.",
            ),
            (
                "e.setAttribute('', '1')",
                "InvalidCharacterError: Failed to execute 'setAttribute' on 'Element': '' is not \
                 a valid attribute name.",
            ),
            (
                "e.toggleAttribute('a=b')",
                "InvalidCharacterError: Failed to execute 'toggleAttribute' on 'Element': 'a=b' \
                 is not a valid attribute name.",
            ),
            (
                "(function () { var d = document.createElement('div'); d.className = null; \
                 return d.getAttribute('class'); })()",
                "ok \"null\"",
            ),
            (
                "(function () { var d = document.createElement('div'); d.id = undefined; \
                 return d.id; })()",
                "ok \"undefined\"",
            ),
            (
                "(function () { var d = document.createElement('div'); d.setAttribute('Foo', '1'); \
                 return [d.getAttribute('FOO'), d.hasAttribute('fOo'), d.getAttributeNames().join()]; })()",
                "ok [\"1\",true,\"foo\"]",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 return [d.toggleAttribute('x'), d.toggleAttribute('x'), d.toggleAttribute('x', false), \
                 d.toggleAttribute('x', true), d.toggleAttribute('x', true)]; })()",
                "ok [true,false,false,true,true]",
            ),
            (
                "(function () { var d = document.createElement('div'); var had = d.hasAttributes(); \
                 d.setAttribute('a', '1'); d.removeAttribute('A'); d.removeAttribute('nothing'); \
                 return [had, d.hasAttributes(), d.getAttribute('a'), d.id, d.className]; })()",
                "ok [false,false,null,\"\",\"\"]",
            ),
            ("e.getAttribute('nope')", "ok null"),
        ],
    );
}

#[test]
fn the_tree_changes_as_the_reference_browser_changes_it() {
    probed(
        "var e = document.createElement('div'), t = document.createTextNode('abc'), \
         p = document.documentElement.lastChild.firstChild;",
        &[
            (
                "e.insertAdjacentElement('middle', document.createElement('i'))",
                "SyntaxError: Failed to execute 'insertAdjacentElement' on 'Element': The value \
                 provided ('middle') is not one of 'beforeBegin', 'afterBegin', 'beforeEnd', or \
                 'afterEnd'.",
            ),
            (
                "e.insertAdjacentElement('beforeend', t)",
                "TypeError: Failed to execute 'insertAdjacentElement' on 'Element': parameter 2 \
                 is not of type 'Element'.",
            ),
            (
                "e.insertAdjacentElement('afterend', document.createElement('i'))",
                "ok null",
            ),
            ("e.insertAdjacentText('BeforeEnd', 'q')", "ok \"undefined\""),
            (
                "document.body.replaceChild(document.createElement('i'), e)",
                "NotFoundError: Failed to execute 'replaceChild' on 'Node': The node to be \
                 replaced is not a child of this node.",
            ),
            (
                "document.replaceChild(document.createTextNode('x'), document.documentElement)",
                "HierarchyRequestError: Failed to execute 'replaceChild' on 'Node': Nodes of \
                 type '#text' may not be inserted inside nodes of type '#document'.",
            ),
            (
                "document.body.insertBefore(document.createElement('i'), e)",
                "NotFoundError: Failed to execute 'insertBefore' on 'Node': The node before which \
                 the new node is to be inserted is not a child of this node.",
            ),
            (
                "document.body.insertBefore(document.createElement('i'), null)",
                "ok \"I\"",
            ),
            (
                "document.body.insertBefore(document.createElement('i'))",
                "TypeError: Failed to execute 'insertBefore' on 'Node': 2 arguments required, but \
                 only 1 present.",
            ),
            ("document.body.contains(null)", "ok false"),
            (
                "document.body.contains({})",
                "TypeError: Failed to execute 'contains' on 'Node': parameter 1 is not of type \
                 'Node'.",
            ),
            (
                "document.append(document.createElement('x'))",
                "HierarchyRequestError: Failed to execute 'append' on 'Document': Only one \
                 element on document allowed.",
            ),
            ("e.nodeValue", "ok null"),
            (
                "(function () { var d = document.createElement('div'); d.textContent = null; \
                 return d.childNodes.length; })()",
                "ok 0",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 var a = d.appendChild(document.createElement('a')); d.append('x'); \
                 a.replaceWith(a, 'y'); return [d.childNodes.length, d.firstChild.nodeName, \
                 d.textContent]; })()",
                "ok [3,\"A\",\"yx\"]",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 var a = d.appendChild(document.createElement('a')); \
                 var b = d.appendChild(document.createElement('b')); b.before('t', b, a); \
                 return [d.childNodes[0].data, d.childNodes[1].nodeName, d.childNodes[2].nodeName]; })()",
                "ok [\"t\",\"B\",\"A\"]",
            ),
            (
                "document.createElement('div').getRootNode().nodeName + ' ' + p.getRootNode().nodeName",
                "ok \"DIV #document\"",
            ),
            (
                "document.head.nodeName + document.body.nodeName + (document.defaultView === window)",
                "ok \"HEADBODYtrue\"",
            ),
            ("document.ownerDocument", "ok null"),
            (
                "(function () { var d = document.createElement('div'); \
                 var x = d.appendChild(document.createElement('x')); \
                 var a = d.appendChild(document.createElement('a')); \
                 var b = d.appendChild(document.createElement('b')); b.before(a); \
                 return names(d); })()",
                "ok \"X,A,B\"",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 var x = d.appendChild(document.createElement('x')); \
                 var a = d.appendChild(document.createElement('a')); \
                 var b = d.appendChild(document.createElement('b')); b.before(a, 't'); \
                 return names(d); })()",
                "ok \"X,A,t,B\"",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 var a = d.appendChild(document.createElement('a')); \
                 d.appendChild(document.createElement('x')); a.replaceWith('y', 'z'); \
                 return names(d) + ' ' + a.parentNode; })()",
                "ok \"y,z,X null\"",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 d.append('a', 'b', document.createElement('i')); d.textContent = 'x'; \
                 return names(d); })()",
                "ok \"x\"",
            ),
            (
                "(function () { var x = p.appendChild(document.createElement('x')); \
                 try { document.append(x); } catch (e) {} return x.parentNode; })()",
                "ok \"P\"",
            ),
            (
                "(function () { var x = p.appendChild(document.createElement('x')); \
                 var y = p.appendChild(document.createElement('y')); \
                 try { document.append(x, y); } catch (e) {} return [x.parentNode, y.parentNode]; })()",
                "ok [null,null]",
            ),
            (
                "document.appendChild(document.documentElement)",
                "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': Only one \
                 element on document allowed.",
            ),
            (
                "document.replaceChild(document.documentElement, document.documentElement)",
                "ok \"HTML\"",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 var a = d.appendChild(document.createElement('a')); \
                 var b = d.appendChild(document.createElement('b')); \
                 d.appendChild(document.createElement('c')); \
                 var old = d.replaceChild(b, a); \
                 return [old === a, a.parentNode, d.childNodes.length, d.firstChild === b, \
                 d.lastChild.nodeName, d.replaceChild(b, b) === b]; })()",
                "ok [true,null,2,true,\"C\",true]",
            ),
            (
                "(function () { var c = p.cloneNode(true); \
                 return [c.id, c.textContent, c.parentNode, c === p, p.cloneNode().firstChild]; })()",
                "ok [\"a\",\"x\",null,false,null]",
            ),
            (
                "(function () { var d = document.createElement('div'); d.append('a'); \
                 var i = document.createElement('i'), u = document.createElement('u'); \
                 d.prepend(i, 'b'); d.firstChild.after(u); u.remove(); \
                 d.replaceChildren(i, 'z'); \
                 return [d.childNodes.length, d.firstChild === i, d.lastChild.data, \
                 i.isConnected, p.isConnected, d.contains(i), i.contains(d)]; })()",
                "ok [2,true,\"z\",false,true,true,false]",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 var a = d.appendChild(document.createElement('a')); a.insertAdjacentText('beforebegin', 'q'); \
                 a.insertAdjacentElement('afterBegin', document.createElement('b')); \
                 a.insertAdjacentText('afterend', 'z'); \
                 return [d.childNodes.length, a.firstChild.nodeName, d.textContent, \
                 a.previousElementSibling, a.nextElementSibling, d.firstElementChild === a, \
                 d.lastElementChild === a, d.childElementCount, p.parentElement.nodeName, \
                 document.documentElement.parentElement]; })()",
                "ok [3,\"B\",\"qz\",null,null,true,true,1,\"BODY\",null]",
            ),
        ],
    );
}

#[test]
fn every_kind_of_node_as_the_reference_browser_has_it() {
    // D58: comments, doctypes, processing instructions, fragments, a
    // template's contents and attributes as Attr objects.
    probed_in(
        "<!--c--><p id=a>x<!--in-->y</p><template id=t><b>in</b></template>",
        "var a = document.getElementById('a'), t = document.getElementById('t');",
        &[
            ("document.firstChild.nodeType", "ok 8"),
            ("document.firstChild.data", "ok \"c\""),
            ("document.doctype", "ok null"),
            ("names(a)", "ok \"x,#comment,y\""),
            ("a.textContent", "ok \"xy\""),
            ("a.innerHTML", "ok \"x<!--in-->y\""),
            ("a.cloneNode(true).innerHTML", "ok \"x<!--in-->y\""),
            ("t.content.nodeName", "ok \"#document-fragment\""),
            ("t.childNodes.length", "ok 0"),
            ("t.content.firstChild.nodeName", "ok \"B\""),
            ("t.innerHTML", "ok \"<b>in</b>\""),
            ("document.querySelector('template b')", "ok null"),
            ("t.cloneNode(true).content.firstChild.nodeName", "ok \"B\""),
            (
                "(function () { t.innerHTML = '<i>n</i>';                  return [t.childNodes.length, t.content.firstChild.nodeName]; })()",
                "ok [0,\"I\"]",
            ),
            (
                "(function () { var f = document.createDocumentFragment(); \
                 f.append('a', document.createElement('i')); var d = document.createElement('div'); \
                 d.appendChild(f); return [names(d), f.childNodes.length]; })()",
                "ok [\"a,I\",0]",
            ),
            ("new DocumentFragment().nodeType", "ok 11"),
            ("document.createComment('x').nodeName", "ok \"#comment\""),
            ("new Comment('q').data", "ok \"q\""),
            (
                "document.createProcessingInstruction('x', 'y').target",
                "ok \"x\"",
            ),
            (
                "document.createProcessingInstruction('x', 'y?>')",
                "InvalidCharacterError: Failed to execute 'createProcessingInstruction' on \
                 'Document': The data provided ('y?>') contains '?>'.",
            ),
            (
                "document.implementation.createDocumentType('html', 'p', 's').publicId",
                "ok \"p\"",
            ),
            (
                "document.body.appendChild(document.implementation.createDocumentType('html', \
                 '', ''))",
                "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': Nodes of type \
                 'html' may not be inserted inside nodes of type 'BODY'.",
            ),
            (
                "document.appendChild(document.createTextNode('t'))",
                "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': Nodes of type \
                 '#text' may not be inserted inside nodes of type '#document'.",
            ),
            (
                "document.appendChild(document.createElement('x'))",
                "HierarchyRequestError: Failed to execute 'appendChild' on 'Node': Only one \
                 element on document allowed.",
            ),
            ("a.attributes.length", "ok 1"),
            ("a.attributes[0].name", "ok \"id\""),
            ("a.getAttributeNode('id').value", "ok \"a\""),
            ("a.attributes.id === a.getAttributeNode('id')", "ok true"),
            ("a.getAttributeNode('id').ownerElement === a", "ok true"),
            (
                "(function () { var c = document.createComment('k'); a.prepend(c); c.after('z'); \
                 return names(a); })()",
                "ok \"#comment,z,x,#comment,y\"",
            ),
            (
                "document.appendChild(document.createComment('end')).nodeName",
                "ok \"#comment\"",
            ),
            ("document.lastChild.data", "ok \"end\""),
        ],
    );
}

#[test]
fn a_range_in_an_attr_is_at_its_start_and_holds_nothing() {
    // An Attr is in no tree and has no children: a range set in one is
    // at (attr, 0) at both ends, its own root.
    probed_in(
        "<p id=a title=t>one</p>",
        "var a = document.getElementById('a'), at = a.getAttributeNode('title'), \
         r = document.createRange(); r.selectNodeContents(a); r.setStart(at, 0); \
         function name(f) { try { f(); return 'none'; } catch (e) { return e.name; } }",
        &[
            (
                "[r.startContainer === at, r.endContainer === at, r.commonAncestorContainer === at, \
                 r.collapsed, r.toString(), r.cloneContents().childNodes.length]",
                "ok [true,true,true,true,\"\",0]",
            ),
            (
                "[name(() => r.setEnd(at, 1)), name(() => r.setStartBefore(at)), \
                 name(() => r.selectNode(at)), name(() => r.insertNode(document.createElement('i'))), \
                 name(() => r.surroundContents(document.createElement('i'))), \
                 name(() => r.comparePoint(a, 0))]",
                "ok [\"IndexSizeError\",\"InvalidNodeTypeError\",\"InvalidNodeTypeError\",\
                 \"HierarchyRequestError\",\"HierarchyRequestError\",\"WrongDocumentError\"]",
            ),
            (
                "(function () { var q = r.cloneRange(), d = document.createRange(); \
                 return [q.startContainer === at, q.compareBoundaryPoints(0, r), \
                 name(() => d.compareBoundaryPoints(0, r)), r.isPointInRange(a, 0), \
                 r.intersectsNode(at), d.intersectsNode(at), r.comparePoint(at, 0)]; })()",
                "ok [true,0,\"WrongDocumentError\",false,true,false,0]",
            ),
            (
                "(function () { r.deleteContents(); r.setEnd(a.firstChild, 2); \
                 return [r.startContainer === a.firstChild, r.startOffset, r.collapsed]; })()",
                "ok [true,2,true]",
            ),
            (
                "(function () { var q = document.createRange(); q.selectNodeContents(a); \
                 var before = q.toString(); q.setEnd(at, 0); q.setStart(a.firstChild, 1); \
                 return [before, q.startContainer === a.firstChild, q.collapsed, \
                 name(() => q.insertNode(at))]; })()",
                "ok [\"one\",true,true,\"HierarchyRequestError\"]",
            ),
            (
                "(function () { var q = document.createRange(); q.selectNodeContents(at); \
                 getSelection().removeAllRanges(); getSelection().addRange(q); \
                 return [q.startContainer === at, getSelection().rangeCount]; })()",
                "ok [true,0]",
            ),
        ],
    );
}

#[test]
fn ranges_traversal_and_the_selection_as_the_reference_browser_has_them() {
    // D59: live ranges moved by the tree's changes, their contents
    // taken, cloned and wrapped; the selection; tree walkers and node
    // iterators; the comparisons and `normalize()`.
    probed_in(
        "<p id=a>one<b>two</b>three</p><p id=c>four</p>",
        "var a = document.getElementById('a'), c = document.getElementById('c'), \
         r = document.createRange(), d = document.createElement('div'); \
         d.innerHTML = '<p>x<b>y</b></p><!--c--><i></i>';",
        &[
            (
                "[r.startContainer.nodeName, r.startOffset, r.collapsed]",
                "ok [\"#document\",0,true]",
            ),
            (
                "(r.setStart(a.firstChild, 1), r.setEnd(a.lastChild, 2), r.toString())",
                "ok \"netwoth\"",
            ),
            ("r.commonAncestorContainer.id", "ok \"a\""),
            ("r.cloneContents().textContent", "ok \"netwoth\""),
            (
                "(function () { var f = r.extractContents(); return [f.childNodes.length, \
                 a.innerHTML, r.collapsed, r.startContainer.id, r.startOffset]; })()",
                "ok [3,\"oree\",true,\"a\",1]",
            ),
            (
                "(function () { var t = a.firstChild, q = document.createRange(); \
                 q.setStart(t, 1); t.insertData(0, 'xx'); return q.startOffset; })()",
                "ok 3",
            ),
            (
                "(function () { var q = document.createRange(); q.selectNodeContents(c); \
                 c.firstChild.remove(); return [q.startOffset, q.endOffset]; })()",
                "ok [0,0]",
            ),
            (
                "(function () { var t = document.createTextNode('abcdef'); c.appendChild(t); \
                 var q = document.createRange(); q.setStart(t, 4); q.setEnd(t, 5); \
                 var u = t.splitText(2); return [q.startContainer === u, q.startOffset, \
                 q.endOffset]; })()",
                "ok [true,2,3]",
            ),
            (
                "r.setStart(c, 5)",
                "IndexSizeError: Failed to execute 'setStart' on 'Range': There is no child at \
                 offset 5.",
            ),
            (
                "r.compareBoundaryPoints(5, r)",
                "NotSupportedError: Failed to execute 'compareBoundaryPoints' on 'Range': The \
                 comparison method provided must be one of 'START_TO_START', 'START_TO_END', \
                 'END_TO_END', or 'END_TO_START'.",
            ),
            (
                "(function () { var p = document.createElement('p'); p.textContent = 'hello'; \
                 var q = document.createRange(); q.setStart(p.firstChild, 2); \
                 q.insertNode(document.createElement('br')); return [p.innerHTML, \
                 q.startContainer.nodeName, q.startOffset, q.endContainer.nodeName, \
                 q.endOffset]; })()",
                "ok [\"he<br>llo\",\"#text\",2,\"P\",2]",
            ),
            (
                "(function () { var p = document.createElement('p'); p.innerHTML = 'abc'; \
                 var q = document.createRange(); q.setStart(p.firstChild, 1); \
                 q.setEnd(p.firstChild, 2); q.surroundContents(document.createElement('b')); \
                 return [p.innerHTML, q.startContainer.nodeName, q.startOffset, \
                 q.endOffset]; })()",
                "ok [\"a<b>b</b>c\",\"P\",1,2]",
            ),
            ("r.intersectsNode(document.createElement('x'))", "ok false"),
            ("r.comparePoint(document.body, 2)", "ok 1"),
            (
                "new StaticRange({startContainer: c, startOffset: 0, endContainer: c, \
                 endOffset: 9}).endOffset",
                "ok 9",
            ),
            ("a.compareDocumentPosition(c)", "ok 4"),
            ("c.compareDocumentPosition(a)", "ok 2"),
            ("document.body.compareDocumentPosition(a)", "ok 20"),
            ("a.compareDocumentPosition(document.body)", "ok 10"),
            ("d.cloneNode(true).isEqualNode(d)", "ok true"),
            ("a.isEqualNode(c)", "ok false"),
            (
                "(function () { var e = document.createElement('div'); e.append('a', '', 'b'); \
                 e.appendChild(document.createElement('i')); e.append('c'); e.normalize(); \
                 return e.childNodes.length + ':' + e.firstChild.data; })()",
                "ok \"3:ab\"",
            ),
            (
                "(function () { var w = document.createTreeWalker(d, NodeFilter.SHOW_ELEMENT), \
                 out = []; while (w.nextNode()) out.push(w.currentNode.nodeName); \
                 return out.join(); })()",
                "ok \"P,B,I\"",
            ),
            (
                "(function () { var w = document.createTreeWalker(d, 1, function (n) { \
                 return n.nodeName === 'P' ? NodeFilter.FILTER_REJECT : 1; }), out = []; \
                 while (w.nextNode()) out.push(w.currentNode.nodeName); return out.join(); })()",
                "ok \"I\"",
            ),
            (
                "(function () { var w = document.createTreeWalker(d, 1, function (n) { \
                 return n.nodeName === 'P' ? NodeFilter.FILTER_SKIP : 1; }), out = []; \
                 while (w.nextNode()) out.push(w.currentNode.nodeName); return out.join(); })()",
                "ok \"B,I\"",
            ),
            (
                "(function () { var w = document.createTreeWalker(d, 1); \
                 w.currentNode = d.lastChild; return w.previousNode().nodeName; })()",
                "ok \"B\"",
            ),
            (
                "(function () { var w = document.createTreeWalker(d, 1, function (n) { \
                 return n.nodeName === 'B' ? 1 : NodeFilter.FILTER_SKIP; }); \
                 return w.firstChild().nodeName; })()",
                "ok \"B\"",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 e.innerHTML = '<i></i><p><b></b></p><u></u>'; \
                 var w = document.createTreeWalker(e, 1, function (n) { \
                 return n.nodeName === 'P' ? 2 : 1; }); w.currentNode = e.firstChild; \
                 return w.nextSibling().nodeName; })()",
                "ok \"U\"",
            ),
            (
                "(function () { var it = document.createNodeIterator(d, NodeFilter.SHOW_ELEMENT); \
                 var first = it.nextNode(), second = it.nextNode(); second.remove(); \
                 return [first.nodeName, second.nodeName, it.referenceNode.nodeName, \
                 it.pointerBeforeReferenceNode, it.nextNode().nodeName]; })()",
                "ok [\"DIV\",\"P\",\"DIV\",false,\"I\"]",
            ),
            (
                "(function () { var s = getSelection(); s.selectAllChildren(c); \
                 return [s.rangeCount, s.anchorNode.id, s.type, \
                 s === document.getSelection()]; })()",
                "ok [1,\"c\",\"Range\",true]",
            ),
            (
                "(function () { var s = getSelection(); s.collapse(c, 1); s.extend(c, 0); \
                 return [s.anchorOffset, s.focusOffset, s.direction]; })()",
                "ok [1,0,\"backward\"]",
            ),
        ],
    );
}

#[test]
fn move_before_keeps_the_focus_and_checks_as_insertion_does() {
    let console = logged(
        "<p id=a>a</p><p id=b><input id=i></p><script>\
         var i = document.getElementById('i'), a = document.getElementById('a'); i.focus(); \
         a.moveBefore(i, a.firstChild); \
         console.log(document.activeElement === i, a.firstChild === i); \
         try { a.moveBefore(document.createElement('x'), null); } catch (e) { console.log(e.name); } \
         try { document.body.moveBefore(a, i); } catch (e) { console.log(e.name); }</script>",
    );
    assert_eq!(
        console,
        ["true true", "HierarchyRequestError", "NotFoundError"]
    );
}

#[test]
fn document_open_replaces_the_document_with_what_is_written() {
    let (mut tab, _) = scripted(
        "<p>x</p><script>setTimeout(function () { \
         var old = document.documentElement; document.open(); \
         console.log(document.childNodes.length, old.parentNode); \
         document.write('<p id=n>new</p><script>console.log(\"ran\", \
         document.getElementById(\"n\").textContent)<\\/script>'); \
         document.close(); \
         console.log(document.getElementsByTagName('p').length, document.URL !== ''); }, 0);\
         </script>",
        true,
    );
    let console = tab.run_until(|lines| lines.len() >= 3);
    assert_eq!(console, ["0 null", "ran new", "1 true"]);
    tab.engine.document().validate().unwrap();
}

#[test]
fn collections_as_the_reference_browser_has_them() {
    probed(
        "",
        &[
            (
                "document.body.childNodes === document.body.childNodes",
                "ok true",
            ),
            (
                "document.body.children === document.body.children",
                "ok true",
            ),
            ("document.body.childNodes[999]", "ok \"undefined\""),
            (
                "Object.keys(document.createElement('div').childNodes)",
                "ok []",
            ),
            (
                "(function () { var d = document.createElement('div'); d.append('a'); \
                 return ['0' in d.childNodes, '1' in d.childNodes, 'item' in d.childNodes]; })()",
                "ok [true,false,true]",
            ),
            (
                "(function () { var d = document.createElement('div'); d.append('a'); \
                 var l = d.childNodes; l[5] = 1; return l[5]; })()",
                "ok \"undefined\"",
            ),
            (
                "(function () { var d = document.createElement('div'); d.append('a'); \
                 var l = d.childNodes; l[5] = 1; \
                 return Object.keys(l).join() + '|' + Object.getOwnPropertyNames(l).join(); })()",
                "ok \"0|0\"",
            ),
            (
                "(function () { var d = document.createElement('div'); \
                 d.append(document.createElement('i'), document.createElement('b'), \
                 document.createElement('a')); d.children[0].id = 'q'; \
                 d.children[1].setAttribute('name', 'n'); d.children[2].setAttribute('name', 'm'); \
                 return [d.children.q && d.children.q.nodeName, d.children.namedItem('n').nodeName, \
                 d.children.m && d.children.m.nodeName, d.children.namedItem('nope')]; })()",
                "ok [\"I\",\"B\",\"A\",null]",
            ),
            (
                "Object.prototype.toString.call(document.body.childNodes)",
                "ok \"[object NodeList]\"",
            ),
            (
                "Object.prototype.toString.call(document.body.children)",
                "ok \"[object HTMLCollection]\"",
            ),
            (
                "typeof NodeList.prototype.forEach + typeof HTMLCollection.prototype.forEach",
                "ok \"functionundefined\"",
            ),
            (
                "NodeList.prototype[Symbol.iterator] === Array.prototype.values",
                "ok true",
            ),
            (
                "(function () { var d = document.createElement('div'); var l = d.childNodes, \
                 c = d.children; var n = [l.length, c.length]; d.append('t', document.createElement('i')); \
                 n.push(l.length, c.length, l.item(1).nodeName, c.item(0).nodeName, l.item(5)); \
                 var names = []; for (var x of l) names.push(x.nodeName); \
                 l.forEach(function (x, i) { names.push(i); }); \
                 n.push(names.join(), Array.from(c).length, [...l].length); return n; })()",
                "ok [0,0,2,1,\"I\",\"I\",null,\"#text,I,0,1\",1,2]",
            ),
            (
                "(function () { try { new NodeList() } catch (e) { return e.message } })()",
                "ok \"Failed to construct 'NodeList': Illegal constructor\"",
            ),
        ],
    );
}

#[test]
fn selectors_find_what_the_reference_browser_finds() {
    probed_in(
        "<div id=d class='a b'><p id=a class=x>x</p><p class='x y'>y</p><span id=a>dup</span>\
         </div><ul><li>1</li><li class=X>2</li></ul><b id=''></b>",
        "var d = document.getElementById('d');",
        &[
            ("tags([document.getElementById('a')])", "ok \"P#a\""),
            ("document.getElementById('zz')", "ok null"),
            ("document.getElementById('')", "ok null"),
            (
                "document.querySelector('!!')",
                "SyntaxError: Failed to execute 'querySelector' on 'Document': '!!' is not a \
                 valid selector.",
            ),
            (
                "document.querySelector('')",
                "SyntaxError: Failed to execute 'querySelector' on 'Document': The provided \
                 selector is empty.",
            ),
            (
                "d.querySelectorAll('p[')",
                "SyntaxError: Failed to execute 'querySelectorAll' on 'Element': 'p[' is not a \
                 valid selector.",
            ),
            (
                "d.matches('::')",
                "SyntaxError: Failed to execute 'matches' on 'Element': '::' is not a valid \
                 selector.",
            ),
            (
                "d.closest('1')",
                "SyntaxError: Failed to execute 'closest' on 'Element': '1' is not a valid \
                 selector.",
            ),
            ("document.querySelector('p::before')", "ok null"),
            ("d.matches('div::before')", "ok false"),
            (
                "document.querySelector(':foo')",
                "SyntaxError: Failed to execute 'querySelector' on 'Document': ':foo' is not a \
                 valid selector.",
            ),
            ("document.querySelector(':hover')", "ok null"),
            ("tags([document.querySelector(':scope')])", "ok \"HTML\""),
            (
                "tags(document.querySelectorAll(':scope > html'))",
                "ok \"\"",
            ),
            ("tags(d.querySelectorAll(':scope > p'))", "ok \"P#a,P\""),
            ("d.querySelector(':scope')", "ok null"),
            ("tags(d.querySelectorAll('div p'))", "ok \"P#a,P\""),
            ("tags(d.querySelectorAll('body div p'))", "ok \"P#a,P\""),
            (
                "[d.matches('body > div'), d.matches('.a.b'), d.firstChild.matches(':first-child'), \
                 d.webkitMatchesSelector('div')]",
                "ok [true,true,true,true]",
            ),
            ("tags([d.firstChild.closest('div')])", "ok \"DIV#d\""),
            ("tags([d.closest('div')])", "ok \"DIV#d\""),
            ("d.closest('table')", "ok null"),
            ("tags(d.getElementsByTagName('*'))", "ok \"P#a,P,SPAN#a\""),
            ("tags(d.getElementsByTagName('P'))", "ok \"P#a,P\""),
            (
                "(function () { var l = document.getElementsByTagName('li'); var n = l.length; \
                 document.querySelector('ul').appendChild(document.createElement('li')); \
                 return [n, l.length]; })()",
                "ok [2,3]",
            ),
            ("tags(document.getElementsByClassName('x'))", "ok \"P#a,P\""),
            (
                "tags(document.getElementsByClassName('  y \
                 x '))",
                "ok \"P\"",
            ),
            (
                "document.getElementsByClassName('x').length + \
                 document.getElementsByClassName('X').length * 10",
                "ok 12",
            ),
            ("tags(document.getElementsByClassName('  '))", "ok \"\""),
            ("tags(d.getElementsByClassName('a'))", "ok \"\""),
            (
                "Object.prototype.toString.call(document.querySelectorAll('p'))",
                "ok \"[object NodeList]\"",
            ),
            (
                "document.querySelector()",
                "TypeError: Failed to execute 'querySelector' on 'Document': 1 argument \
                 required, but only 0 present.",
            ),
            ("document.querySelector(undefined)", "ok null"),
            ("document.querySelector(null)", "ok null"),
            (
                "tags(document.querySelectorAll('li, p, span'))",
                "ok \"P#a,P,SPAN#a,LI,LI,LI\"",
            ),
            ("tags(document.querySelectorAll('[class~=y]'))", "ok \"P\""),
            (
                "tags(document.querySelectorAll('li:nth-child(2)'))",
                "ok \"LI\"",
            ),
            ("tags(document.querySelectorAll('p:not(.y)'))", "ok \"P#a\""),
            (
                "tags(document.querySelectorAll(':is(ul, div) > :first-child'))",
                "ok \"P#a,LI\"",
            ),
            // A static list stays as it was made.
            (
                "(function () { var l = document.querySelectorAll('li'); \
                 document.querySelector('ul').appendChild(document.createElement('li')); \
                 return [l.length, document.querySelectorAll('li').length]; })()",
                "ok [3,4]",
            ),
        ],
    );
}

#[test]
fn a_static_list_holds_its_nodes_and_a_live_one_holds_its_root() {
    // The nodes a querySelectorAll found are taken out and let go of but
    // for the list.
    let (tab, console) = scripted(
        "<div id=d><i>1</i><i>2</i></div><script>\
         var d = document.getElementById('d'); \
         var found = d.querySelectorAll('i'); \
         d.parentNode.removeChild(d); d = null;\
         </script><script>console.log(found.length, found[1].textContent)</script>",
        true,
    );
    assert_eq!(console, ["2 2"]);
    tab.engine.document().validate().unwrap();
    // A live list keeps the subtree of the node it is of.
    let (tab, console) = scripted(
        "<div id=d><i>1</i><i>2</i></div><script>\
         var d = document.getElementById('d'); \
         var live = d.getElementsByTagName('i'); \
         d.parentNode.removeChild(d); d = null;\
         </script><script>console.log(live.length, live[0].parentNode.id)</script>",
        true,
    );
    assert_eq!(console, ["2 d"]);
    tab.engine.document().validate().unwrap();
}

#[test]
fn query_selector_stops_at_the_first_match() {
    // Twenty thousand elements, a thousand calls: the first element
    // matches, so each call visits one; were the whole tree visited, the
    // work would be past the budget.
    let mut page = String::from("<div>");
    for _ in 0..20_000 {
        page.push_str("<i></i>");
    }
    page.push_str(
        "</div><script>for (var i = 0; i < 1000; i++) document.querySelector('i'); \
         console.log('finished')</script>",
    );
    let limits = js::Limits {
        steps: 100,
        ..js::Limits::default()
    };
    let (_, console) = scripted_bytes(page.into_bytes(), Some(limits));
    assert_eq!(console, ["finished"]);
}

#[test]
fn markup_reads_and_writes_as_the_reference_browser_has_it() {
    probed_in(
        "<div id=d><p>a</p></div><noscript><b>n</b></noscript>",
        "var d = document.getElementById('d'); \
         function kids(d) { var r = []; for (var n = d.firstChild; n; n = n.nextSibling) \
         r.push(n.nodeName === '#text' ? JSON.stringify(n.data) : n.nodeName); return r.join(','); }",
        &[
            ("d.innerHTML", "ok \"<p>a</p>\""),
            (
                "(function () { var e = document.createElement('i'); \
                 e.setAttribute('a', '<&\"\\'>\\u00a0'); e.setAttribute('b', ''); return e.outerHTML; })()",
                "ok \"<i a=\\\"&lt;&amp;&quot;'&gt;&nbsp;\\\" b=\\\"\\\"></i>\"",
            ),
            (
                "(function () { var e = document.createElement('i'); \
                 e.textContent = '<&>\"\\'\\u00a0'; return e.innerHTML; })()",
                "ok \"&lt;&amp;&gt;\\\"'&nbsp;\"",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 e.append(document.createElement('br'), document.createElement('img'), \
                 document.createElement('param'), document.createElement('keygen'), \
                 document.createElement('image')); return e.innerHTML; })()",
                "ok \"<br><img><param><keygen><image></image>\"",
            ),
            (
                "(function () { var e = document.createElement('br'); \
                 e.append('x', document.createElement('i')); return [e.innerHTML, e.outerHTML]; })()",
                "ok [\"\",\"<br>\"]",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 ['style', 'script', 'xmp', 'iframe', 'noembed', 'noframes', 'plaintext', \
                 'noscript', 'textarea', 'title'].forEach(function (t) { \
                 var c = document.createElement(t); c.textContent = '<&>'; e.appendChild(c); }); \
                 return e.innerHTML; })()",
                "ok \"<style><&></style><script><&></script><xmp><&></xmp><iframe><&></iframe>\
                 <noembed><&></noembed><noframes><&></noframes><plaintext><&></plaintext>\
                 <noscript><&></noscript><textarea>&lt;&amp;&gt;</textarea>\
                 <title>&lt;&amp;&gt;</title>\"",
            ),
            (
                "document.querySelector('noscript').innerHTML",
                "ok \"<b>n</b>\"",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 e.innerHTML = '<p>a<p>b</p><b>c'; return kids(e) + ' / ' + e.innerHTML; })()",
                "ok \"P,P,B / <p>a</p><p>b</p><b>c</b>\"",
            ),
            (
                "(function () { var e = document.createElement('textarea'); \
                 e.innerHTML = 'a</textarea>b<i>'; return kids(e); })()",
                "ok \"\\\"a</textarea>b<i>\\\"\"",
            ),
            (
                "(function () { var e = document.createElement('title'); \
                 e.innerHTML = '<b>&amp;</b>'; return kids(e); })()",
                "ok \"\\\"<b>&</b>\\\"\"",
            ),
            (
                "(function () { var e = document.createElement('script'); \
                 e.innerHTML = 'a<b>&amp;<\\/script>c'; return kids(e); })()",
                "ok \"\\\"a<b>&amp;</script>c\\\"\"",
            ),
            (
                "(function () { var e = document.createElement('html'); \
                 e.innerHTML = '<p>x'; return kids(e) + ' / ' + e.innerHTML; })()",
                "ok \"HEAD,BODY / <head></head><body><p>x</p></body>\"",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 e.innerHTML = '<title>t</title><meta charset=x><body class=q><p>x</p></body>\
                 <html lang=en>'; return kids(e) + ' / ' + e.innerHTML; })()",
                "ok \"TITLE,META,P / <title>t</title><meta charset=\\\"x\\\"><p>x</p>\"",
            ),
            (
                "(function () { var e = document.createElement('div'); e.innerHTML = '<i></i>'; \
                 e.innerHTML = null; return e.childNodes.length; })()",
                "ok 0",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 e.outerHTML = '<p>x</p>'; return e.outerHTML; })()",
                "ok \"<div></div>\"",
            ),
            (
                "(function () { document.documentElement.outerHTML = '<p>'; })()",
                "NoModificationAllowedError: Failed to set the 'outerHTML' property on 'Element': \
                 This element's parent is a Document, which may not be modified.",
            ),
            (
                "(function () { var w = document.createElement('div'); \
                 var e = w.appendChild(document.createElement('i')); w.append('t'); \
                 e.outerHTML = '<b>1</b><u>2</u>'; return w.innerHTML + ' ' + e.parentNode; })()",
                "ok \"<b>1</b><u>2</u>t null\"",
            ),
            (
                "document.createElement('div').insertAdjacentHTML('beforebegin', '<p>')",
                "NoModificationAllowedError: Failed to execute 'insertAdjacentHTML' on 'Element': \
                 The element has no parent.",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 e.insertAdjacentHTML('afterbegin', '<p>1</p>'); \
                 e.insertAdjacentHTML('beforeend', '<p>2</p>'); return e.innerHTML; })()",
                "ok \"<p>1</p><p>2</p>\"",
            ),
            (
                "document.body.insertAdjacentHTML('middle', '<p>')",
                "SyntaxError: Failed to execute 'insertAdjacentHTML' on 'Element': The value \
                 provided ('middle') is not one of 'beforeBegin', 'afterBegin', 'beforeEnd', or \
                 'afterEnd'.",
            ),
            (
                "(function () { var h = document.createElement('html'); \
                 h.insertAdjacentHTML('beforeend', '<p>x'); return h.innerHTML; })()",
                "ok \"<p>x</p>\"",
            ),
            (
                "(function () { var w = document.createElement('div'); \
                 var e = w.appendChild(document.createElement('i')); \
                 e.insertAdjacentHTML('beforebegin', '<b>1</b>'); \
                 e.insertAdjacentHTML('AfterEnd', '<u>2</u>'); return w.innerHTML; })()",
                "ok \"<b>1</b><i></i><u>2</u>\"",
            ),
            ("typeof document.innerHTML", "ok \"undefined\""),
            (
                "document.documentElement.insertAdjacentHTML('beforebegin', '<p>')",
                "NoModificationAllowedError: Failed to execute 'insertAdjacentHTML' on 'Element': \
                 The element has no parent.",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 e.appendChild(document.createElement('i')); \
                 e.insertAdjacentHTML('afterbegin', '<p>1</p>'); return e.innerHTML; })()",
                "ok \"<p>1</p><i></i>\"",
            ),
            (
                "(function () { var e = document.createElement('div'); \
                 e.innerHTML = '<ul><li>1<li>2</ul>'; return e.innerHTML; })()",
                "ok \"<ul><li>1</li><li>2</li></ul>\"",
            ),
        ],
    );
}

#[test]
fn markup_set_from_a_script_is_the_documents_and_its_scripts_do_not_run() {
    let (tab, console) = scripted(
        "<div id=d><p>old</p></div><script>\
         var d = document.getElementById('d'); var old = d.firstChild; \
         d.innerHTML = '<b>new</b><script>console.log(\"ran\")<\\/script>'; \
         var gone = d.firstChild; gone.outerHTML = '<i>new</i>'; \
         console.log(old.parentNode, gone.parentNode, d.firstChild.nodeName, \
         d.childNodes.length)</script><script>old = gone = null</script>",
        true,
    );
    assert_eq!(console, ["null null I 2"]);
    let document = tab.engine.document();
    document.validate().unwrap();
    let d = by_id(&tab.engine, "d");
    assert_eq!(document.text_content(d), "newconsole.log(\"ran\")");
    // What the markup replaced went with the scripts' last holds on it:
    // every node left is in the tree.
    assert_eq!(document.len(), document.subtree(document.root()).count());
}

#[test]
fn markup_nested_under_a_deep_element_goes_no_deeper_than_the_cap() {
    // An element 500 deep, given markup 20 deep: what fits goes in.
    let mut page = String::from("<body>");
    page.push_str(&"<div>".repeat(498));
    page.push_str(
        "<span id=deep></span><script>var deep = document.getElementById('deep'); \
         deep.innerHTML = '<b>'.repeat(20) + 'x'; \
         console.log(deep.textContent, deep.querySelectorAll('b').length)</script>",
    );
    let (tab, console) = scripted_bytes(page.into_bytes(), Some(js::Limits::default()));
    assert_eq!(console.len(), 1);
    assert!(console[0].starts_with("x "), "{console:?}");
    tab.engine.document().validate().unwrap();
}

#[test]
fn class_list_and_dataset_as_the_reference_browser_has_them() {
    probed(
        "var e = document.createElement('div');",
        &[
            (
                "[e.classList.length, e.classList.value, e.hasAttribute('class')]",
                "ok [0,\"\",false]",
            ),
            (
                "(function () { e.classList.remove('x'); return e.hasAttribute('class'); })()",
                "ok false",
            ),
            (
                "(function () { e.classList.add('a', 'b', 'a'); return [e.className, \
                 e.classList.length, e.classList[1], e.classList.item(5), e.classList[5]]; })()",
                "ok [\"a b\",2,\"b\",null,null]",
            ),
            (
                "e.classList.add('')",
                "SyntaxError: Failed to execute 'add' on 'DOMTokenList': The token provided must \
                 not be empty.",
            ),
            (
                "e.classList.add('a b')",
                "InvalidCharacterError: Failed to execute 'add' on 'DOMTokenList': The token \
                 provided ('a b') contains HTML space characters, which are not valid in tokens.",
            ),
            (
                "e.classList.remove('a b')",
                "InvalidCharacterError: Failed to execute 'remove' on 'DOMTokenList': The token \
                 provided ('a b') contains HTML space characters, which are not valid in tokens.",
            ),
            (
                "e.classList.toggle('')",
                "SyntaxError: Failed to execute 'toggle' on 'DOMTokenList': The token provided \
                 must not be empty.",
            ),
            (
                "[e.classList.replace('a', 'c'), e.className, e.classList.replace('zz', 'y'), \
                 e.className]",
                "ok [true,\"c b\",false,\"c b\"]",
            ),
            (
                "e.classList.replace('', 'y')",
                "SyntaxError: Failed to execute 'replace' on 'DOMTokenList': The token provided \
                 must not be empty.",
            ),
            (
                "[e.classList.contains('b'), e.classList.contains(' b'), e.classList.contains('')]",
                "ok [true,false,false]",
            ),
            (
                "e.classList.supports('a')",
                "TypeError: Failed to execute 'supports' on 'DOMTokenList': DOMTokenList has no \
                 supported tokens.",
            ),
            (
                "(function () { e.classList.value = '  q  r q '; \
                 return [e.className, e.classList.length, String(e.classList)]; })()",
                "ok [\"  q  r q \",2,\"  q  r q \"]",
            ),
            ("e.classList === e.classList", "ok true"),
            (
                "[...e.classList].join('|') + ' ' + Object.keys(e.classList).join()",
                "ok \"q|r 0,1\"",
            ),
            (
                "Object.prototype.toString.call(e.classList)",
                "ok \"[object DOMTokenList]\"",
            ),
            (
                "(function () { var x = document.createElement('i'); x.className = '  a  b a '; \
                 x.classList.remove('zz'); return JSON.stringify(x.className); })()",
                "ok \"\\\"a b\\\"\"",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 return [x.classList.toggle('a', false), x.hasAttribute('class'), \
                 x.classList.toggle('a', true), x.classList.toggle('a', true), x.className, \
                 x.classList.toggle('a', false), x.className]; })()",
                "ok [false,false,true,true,\"a\",false,\"\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); x.className = 'a b c'; \
                 return [x.classList.replace('a', 'c'), x.className]; })()",
                "ok [true,\"c b\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 x.setAttribute('data-foo-bar', '1'); x.setAttribute('data-x', '2'); \
                 x.setAttribute('data-', '3'); x.setAttribute('data-Up', '4'); \
                 return [x.dataset.fooBar, x.dataset.x, Object.keys(x.dataset).join(','), \
                 x.dataset.up, 'fooBar' in x.dataset]; })()",
                "ok [\"1\",\"2\",\"fooBar,x,,up\",\"4\",true]",
            ),
            (
                "(function () { var x = document.createElement('i'); x.dataset.helloWorld = 5; \
                 x.dataset.a1 = 'z'; \
                 return x.getAttributeNames().join(',') + ' ' + x.getAttribute('data-hello-world'); })()",
                "ok \"data-hello-world,data-a1 5\"",
            ),
            (
                "(function () { var x = document.createElement('i'); x.dataset['a-b'] = 1; })()",
                "SyntaxError: Failed to set a named property 'a-b' on 'DOMStringMap': 'a-b' is not \
                 a valid property name.",
            ),
            (
                "(function () { var x = document.createElement('i'); x.dataset.q = 1; \
                 delete x.dataset.q; return x.hasAttribute('data-q'); })()",
                "ok false",
            ),
            (
                "Object.prototype.toString.call(document.body.dataset)",
                "ok \"[object DOMStringMap]\"",
            ),
        ],
    );
}

#[test]
fn style_reads_and_writes_the_style_attribute() {
    probed(
        "",
        &[
            (
                "Object.prototype.toString.call(document.body.style)",
                "ok \"[object CSSStyleDeclaration]\"",
            ),
            ("typeof document.body.style.fooBar", "ok \"undefined\""),
            (
                "(function () { var x = document.createElement('i'); x.style.cssFloat = 'left'; \
                 return [x.style.float, x.getAttribute('style')]; })()",
                "ok [\"left\",\"float: left;\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 x.style.setProperty('color', 'red', 'important'); \
                 return [x.getAttribute('style'), x.style.getPropertyPriority('color'), \
                 x.style.color]; })()",
                "ok [\"color: red !important;\",\"important\",\"red\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 x.style.setProperty('--My-Var', ' 10px '); \
                 return [x.getAttribute('style'), x.style.getPropertyValue('--My-Var'), \
                 x.style.getPropertyValue('--my-var')]; })()",
                "ok [\"--My-Var: 10px;\",\"10px\",\"\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); x.style.color = 'red'; \
                 x.style.width = '1px'; return [x.style.removeProperty('color'), \
                 x.getAttribute('style'), x.style.length, x.style[0], x.style.item(0)]; })()",
                "ok [\"red\",\"width: 1px;\",1,\"width\",\"width\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); x.style.color = 'red'; \
                 x.style.color = ''; return [x.getAttribute('style'), x.style.length]; })()",
                "ok [\"\",0]",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 x.style.cssText = 'color: red; bogus: 1; width: 2px !important'; \
                 return [x.style.cssText, x.style.length, x.getAttribute('style')]; })()",
                "ok [\"color: red; width: 2px !important;\",2,\"color: red; width: 2px !important;\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 x.setAttribute('style', 'color:blue;;margin-top:3px'); \
                 return [x.style.color, x.style.marginTop, x.style.length, x.style.cssText]; })()",
                "ok [\"blue\",\"3px\",2,\"color: blue; margin-top: 3px;\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 x.style.webkitTransform = 'none'; \
                 return [x.style.transform, x.getAttribute('style'), typeof x.style.WebkitTransform]; })()",
                "ok [\"none\",\"transform: none;\",\"string\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 x.style['background-color'] = 'red'; return x.style.backgroundColor; })()",
                "ok \"red\"",
            ),
            (
                "(function () { var x = document.createElement('i'); \
                 x.style.setProperty('COLOR', 'red'); \
                 return [x.getAttribute('style'), x.style.getPropertyValue('Color')]; })()",
                "ok [\"color: red;\",\"red\"]",
            ),
            ("document.body.style === document.body.style", "ok true"),
            // A colour's keyword in lowercase, a shorthand kept as its
            // longhands and said from them (G50).
            (
                "(function () { var x = document.createElement('i'); x.style.color = 'RED'; \
                 x.style.margin = '1px'; x.style.width = 'banana'; \
                 return [x.style.color, x.style.length, x.style[1], x.style.marginTop, \
                 x.style.cssText, x.getAttribute('style')]; })()",
                "ok [\"red\",5,\"margin-top\",\"1px\",\"color: red; margin: 1px;\",\
                 \"color: red; margin: 1px;\"]",
            ),
            (
                "(function () { var x = document.createElement('i'); x.style = 'width: 3px'; \
                 return x.getAttribute('style'); })()",
                "ok \"width: 3px;\"",
            ),
        ],
    );
}

#[test]
fn attributes_are_reflected_as_html_says() {
    probed_in(
        "<base href='http://example.com/dir/page.html'><p id=a>x</p>",
        "",
        &[
            (
                "(function () { var a = document.createElement('a'); \
                 a.setAttribute('href', 'x?y#z'); \
                 var i = document.createElement('img'); i.src = '/i.png'; \
                 var b = document.createElement('a'); b.href = 'http://[bad'; \
                 return [a.href, i.src, i.getAttribute('src'), document.createElement('a').href, \
                 b.href]; })()",
                "ok [\"http://example.com/dir/x?y#z\",\"http://example.com/i.png\",\"/i.png\",\
                 \"\",\"http://[bad\"]",
            ),
            (
                "(function () { var x = document.createElement('div'); x.hidden = true; \
                 var s = document.createElement('script'); s.defer = 1; \
                 return [x.getAttribute('hidden'), x.hidden, s.getAttribute('defer'), s.defer, \
                 document.createElement('input').disabled]; })()",
                "ok [\"\",true,\"\",true,false]",
            ),
            (
                "(function () { var x = document.createElement('div'); x.dir = 'RTL'; \
                 var y = document.createElement('div'); y.setAttribute('dir', 'bogus'); \
                 return [x.dir, x.getAttribute('dir'), y.dir]; })()",
                "ok [\"rtl\",\"RTL\",\"\"]",
            ),
            (
                "(function () { var x = document.createElement('img'); x.alt = 'a'; \
                 x.title = 't'; x.lang = 'en'; \
                 return [x.alt, x.title, x.lang, x.getAttributeNames().join()]; })()",
                "ok [\"a\",\"t\",\"en\",\"alt,title,lang\"]",
            ),
            (
                "(function () { var x = document.createElement('img'); \
                 x.setAttribute('width', '12px'); var c = document.createElement('canvas'); \
                 var set = function (v) { x.width = v; return x.getAttribute('width'); }; \
                 return [x.width, c.width, c.height, set(5), set(-1)]; })()",
                "ok [12,300,150,\"5\",\"0\"]",
            ),
            (
                "[document.createElement('form').action.endsWith('/page.html'), \
                 document.createElement('td').colSpan, document.createElement('ol').start]",
                "ok [true,1,1]",
            ),
            (
                "(function () { var t = new EventTarget(); var g = function () {}; \
                 t.addEventListener('x', g); t.addEventListener('x', g); \
                 t.removeEventListener('x', g); \
                 return [typeof window.addEventListener, typeof document.addEventListener]; })()",
                "ok [\"function\",\"function\"]",
            ),
            (
                "document.addEventListener('x')",
                "TypeError: Failed to execute 'addEventListener' on 'EventTarget': 2 arguments required, but only 1 present.",
            ),
            ("document.addEventListener('x', null)", "ok \"undefined\""),
        ],
    );
}

#[test]
fn a_nodes_object_lasts_while_the_node_is_in_the_document() {
    // What a script set on a node's object is there when another reaches
    // the node again; once a node has left the document and been let go
    // of, it is freed (one whose object holds its `style` or `classList`
    // is in a cycle with it, freed when the realm's garbage is next
    // collected).
    let (tab, console) = scripted(
        "<p id=a>x</p><p id=b>y</p><script>document.getElementById('a').mark = 1; \
         document.body.firstChild.listened = 2; document.getElementById('b').mark = 3;</script> \
         <script>var p = document.getElementById('a'); \
         console.log(p.mark, p.listened, p.style === document.getElementById('a').style, \
         p.classList === document.body.firstChild.classList); \
         var b = document.getElementById('b'); console.log(b.mark); b.remove(); b = null; \
         </script><script>console.log('after')</script>",
        true,
    );
    assert_eq!(console, ["1 2 true true", "3", "after"]);
    let document = tab.engine.document();
    assert_eq!(document.len(), document.subtree(document.root()).count());
}

#[test]
fn an_object_held_for_the_document_or_by_what_it_made_stays() {
    // An element made and then put in is held from then on; a classList
    // keeps its element as long as it lives, though nothing else does.
    let (_, console) = scripted(
        "<p>x</p><script>var d = document.createElement('div'); d.mark = 7; d.id = 'made'; \
         document.body.appendChild(d); d = null; \
         var tokens = document.createElement('i').classList; \
         var styled = document.createElement('i').style; \
         var data = document.createElement('i').dataset;</script>\
         <script>console.log(document.getElementById('made').mark); tokens.add('a'); \
         styled.color = 'red'; data.x = 'y'; \
         console.log(tokens.value, styled.cssText, data.x); \
         var s = document.createElement('i').style; s.setProperty('color', 'red', 'bogus'); \
         console.log(s.length)</script>",
        true,
    );
    assert_eq!(console, ["7", "a color: red; y", "0"]);
}

#[test]
fn location_navigator_and_screen_say_what_d12_lets_them() {
    let server = serve(|path| match path {
        "/dir/page.html" | "/dir/page.html?q=1" => Some(html(
            "<p>x</p><script>var l = location; \
             console.log(l.href === document.URL, l === document.location, l === window.location, \
             String(l) === l.href, document.documentURI === l.href); \
             console.log([l.protocol, l.hostname, l.pathname, l.search, l.hash].join('|')); \
             console.log(l.origin === l.protocol + '//' + l.host, l.host === l.hostname + ':' + l.port); \
             console.log(document.characterSet, document.contentType, document.compatMode); \
             var n = navigator; \
             console.log(n.appCodeName, n.appName, n.appVersion, n.product, n.productSub, \
             JSON.stringify(n.vendor), JSON.stringify(n.vendorSub), n.language, n.languages.join(), \
             n.onLine, n.cookieEnabled, n.webdriver, n.pdfViewerEnabled, n.plugins.length, \
             n.mimeTypes.length, n.javaEnabled(), n.languages === n.languages, \
             typeof n.hardwareConcurrency, typeof n.userAgentData); \
             console.log(n.userAgent.indexOf(' Silver/') > 0, \
             ['Win32', 'MacIntel', 'Linux x86_64'].indexOf(n.platform) >= 0); \
             console.log(innerWidth, innerHeight); \
             console.log(screen.width === innerWidth, screen.height === innerHeight, \
             screen.availWidth === innerWidth, screen.colorDepth, screen.pixelDepth, \
             devicePixelRatio, outerWidth === innerWidth, top === window, parent === window, \
             frames === window, window.length, window.opener, \
             Object.prototype.toString.call(navigator), Object.prototype.toString.call(screen), \
             Object.prototype.toString.call(location));\
             </script>",
        )),
        _ => None,
    });
    // What the reference browser says, this browser named after it, by
    // default (D80).
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.frame(400, 200);
    load_all(&mut tab, server.url("/dir/page.html?q=1#frag"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console.len(), 8, "{console:?}");
    assert_eq!(
        console[..4],
        [
            "true true true true true",
            "http:|127.0.0.1|/dir/page.html|?q=1|#frag",
            "true true",
            "UTF-8 text/html CSS1Compat",
        ]
    );
    assert!(
        console[4].starts_with("Mozilla Netscape 5.0 (")
            && console[4].contains(" Gecko 20030107 \"Google Inc.\""),
        "{console:?}"
    );
    assert_eq!(
        console[5..],
        [
            "true true",
            "400 200",
            "true true true 24 24 1 true true true true 0 null [object Navigator] [object Screen] \
             [object Location]",
        ]
    );
    // D12's minimal string, asked for: Gecko's `navigator`.
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_compat_user_agent(false);
    load_all(&mut tab, server.url("/dir/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        console[4..6],
        [
            "Mozilla Netscape 5.0 (compatible) Gecko 20100101 \"\" \"\" en-US en-US,en true true \
             false false 0 0 false true undefined undefined",
            "false true",
        ],
        "{console:?}"
    );
}

#[test]
fn computed_style_reads_the_cascade_as_it_stands() {
    let (_, console) = scripted(
        "<style>#s { color: #0a141e; margin-left: 1.5em; width: 50%; opacity: 0.25; \
         font-family: Arial, sans-serif; --x: 1px  2px; } #s::before { content: 'b'; color: red } \
         .on { background-color: #00f }</style>\
         <noscript id=n>x</noscript><div id=s>s</div><script>\
         var s = document.getElementById('s'), cs = getComputedStyle(s); \
         console.log(cs.color, cs.marginLeft, cs.width, cs.opacity, cs.fontFamily, cs.fontSize, \
         cs.display, cs.backgroundColor, cs.borderTopStyle, cs.borderTopWidth, cs.position, \
         cs.zIndex, cs.top, JSON.stringify(cs.getPropertyValue('--x')), \
         JSON.stringify(cs.cssText), cs.length === cs.length && cs.length > 60, cs[0], \
         cs.item(1), typeof cs.fontWeight, cs.getPropertyValue('COLOR')); \
         console.log(getComputedStyle(document.getElementById('n')).display, \
         getComputedStyle(s, '::before').color, getComputedStyle(s, ':before').color, \
         JSON.stringify(getComputedStyle(s, 'bogus').color), \
         JSON.stringify(getComputedStyle(document.createElement('div')).color)); \
         var gone = document.createElement('div'); gone.innerHTML = '<p>in</p>'; \
         document.body.appendChild(gone); \
         var goneStyle = getComputedStyle(gone.firstChild); var before = goneStyle.display; \
         gone.remove(); console.log(before, JSON.stringify(goneStyle.display)); \
         s.style.color = 'red'; s.className = 'on'; \
         console.log(cs.color, cs.backgroundColor, cs === getComputedStyle(s)); \
         function f(g) { try { g(); return 'no error' } catch (e) { return e.name + ': ' + e.message } } \
         console.log(f(() => { cs.color = 'blue' })); \
         console.log(f(() => cs.setProperty('color', 'blue'))); \
         console.log(f(() => { cs.cssText = '' })); \
         console.log(f(() => getComputedStyle(document.createTextNode('x'))));\
         </script>",
        true,
    );
    assert_eq!(
        console,
        [
            "rgb(10, 20, 30) 24px 50% 0.25 Arial, sans-serif 16px block rgba(0, 0, 0, 0) none 0px \
             static auto auto \"1px 2px\" \"\" true align-content align-items string \
             rgb(10, 20, 30)",
            // A name with no colon is no pseudo-element: the element's
            // own style (CSSOM section 9), as the reference browser has it.
            "inline rgb(255, 0, 0) rgb(255, 0, 0) \"rgb(10, 20, 30)\" \"\"",
            "block \"\"",
            "rgb(255, 0, 0) rgb(0, 0, 255) false",
            "NoModificationAllowedError: Failed to set the 'color' property on \
             'CSSStyleDeclaration': These styles are computed, and therefore the 'color' \
             property is read-only.",
            "NoModificationAllowedError: Failed to execute 'setProperty' on \
             'CSSStyleDeclaration': These styles are computed, and therefore the 'color' \
             property is read-only.",
            "NoModificationAllowedError: Failed to set the 'cssText' property on \
             'CSSStyleDeclaration': These styles are computed, and therefore read-only.",
            "TypeError: Failed to execute 'getComputedStyle' on 'Window': parameter 1 is not of \
             type 'Element'.",
        ]
    );
}

#[test]
fn computed_style_gives_shorthands_gradients_and_the_inherited_currentcolor() {
    // D66: a shorthand from its longhands, the used margins among them;
    // a gradient as the reference browser writes it (G232); a
    // calculation (Values 4 section 10.13); `currentcolor` inherited as
    // the keyword, the inheriting element's colour (G226).
    let page = "<div style='width: 300px'><div id=m style='width: 100px; margin: 0 10%'></div>\
                </div><div id=g style='background-image: linear-gradient(to right, red, blue); \
                background-position-x: calc(10px + 50%)'></div>\
                <div style='color: red; border-color: currentcolor'>\
                <p id=p style='color: blue; border: 1px solid; border-color: inherit'>x</p></div>\
                <script>\
                var m = getComputedStyle(document.getElementById('m')); \
                var g = getComputedStyle(document.getElementById('g')); \
                var p = getComputedStyle(document.getElementById('p')); \
                console.log(m.margin, m.getPropertyValue('border'), m[m.length - 1]); \
                console.log(g.backgroundImage); \
                console.log(g.backgroundPositionX, p.borderColor, p.borderTopColor);\
                </script>";
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", page.as_bytes())),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.frame(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        console,
        [
            "0px 30px 0px none rgb(0, 0, 0) -webkit-border-vertical-spacing",
            "linear-gradient(to right, rgb(255, 0, 0), rgb(0, 0, 255))",
            "calc(50% + 10px) rgb(0, 0, 255) rgb(0, 0, 255)",
        ]
    );
}

#[test]
fn a_script_run_after_a_frame_reaches_the_next_through_invalidation() {
    // What the dom_api fuzz target holds the engine to (M25.3): a script
    // run in the document shown after a frame, as a `javascript:` URL's
    // is, and the next frame what a fresh engine computes for the
    // document as it then stands: the class added restyles its element,
    // the paragraph taken out shows the one after it again.
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<title>t</title><style>.on { margin-left: 7px } p + p { display: none }</style>\
             <p id=a>x</p><p>y</p><p>z</p>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.frame(200, 100);
    crate::differential::run_script(
        &mut tab.engine,
        "console.log(document.title); var a = document.getElementById('a'); \
         a.nextElementSibling.classList.add('on'); a.remove(); \
         console.log(getComputedStyle(document.querySelector('p')).marginLeft);",
    );
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["t", "7px"]);
    let mut fresh = crate::Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    fresh.set_scripting(true);
    crate::differential::compare_with_fresh(&mut tab.engine, &mut fresh, 200, 100)
        .expect("the frame after the run is a fresh engine's");
    let document = tab.engine.document();
    let mut shown: Vec<String> = tab
        .engine
        .fragments(200.0)
        .iter()
        .filter(|fragment| document.tag(fragment.node) == Some("p"))
        .map(|fragment| document.text_content(fragment.node))
        .collect();
    shown.dedup();
    assert_eq!(shown, ["y"]);
}

#[test]
fn a_page_nested_as_deep_as_the_parser_lets_is_drawn_within_the_engines_stack() {
    // A page of elements nested to the parser's cap, blocks, inline
    // boxes holding blocks and grids among them, built into boxes, laid
    // out and drawn on a thread of `STACK_SIZE`, as the shell's main
    // thread is: a debug build's frames are the largest (M25's fuzz run
    // overflowed 1 MiB at 31 levels under the address sanitizer).
    let levels = dom::MAX_DEPTH - 2;
    let pages = [
        format!("{}x", "<div>".repeat(levels)),
        format!("{}x", "<span><div>".repeat(levels / 2)),
        format!(
            "<style>.g {{ display: grid }}</style>{}x",
            "<div class=g>".repeat(levels)
        ),
    ];
    std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(move || {
            for page in pages {
                let mut engine = crate::Engine::with_fonts(
                    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                        .expect("the bundled font parses"),
                );
                engine.load_document(&page);
                engine.frame(200, 100);
                assert!(engine.document().len() > levels);
            }
        })
        .expect("a thread")
        .join()
        .expect("drawn within the stack");
}

#[test]
fn svg_markup_is_svgs_through_inner_html_clones_and_tag_name_lookups() {
    // M32: the parser's SVG keeps its case and namespace, and so does
    // what scripts make of it.
    let (tab, console) = scripted(
        "<svg id=s><lineargradient id=g gradientunits=x></lineargradient></svg>\
         <script>var s = document.getElementById('s'); \
         s.innerHTML = '<clippath id=c><rect/></clippath>'; \
         var c = s.firstChild; \
         console.log(c.localName, s.innerHTML); \
         console.log(document.getElementsByTagName('clipPath').length, \
         document.getElementsByTagName('clippath').length, \
         document.getElementsByTagName('RECT').length); \
         var copy = c.cloneNode(true); s.appendChild(copy); \
         console.log(copy.localName, document.getElementsByTagName('clipPath').length); \
         s.insertAdjacentHTML('beforeend', '<script>console.log(\"ran\")<\\/script>'); \
         console.log(s.lastChild.localName)</script>",
        true,
    );
    assert_eq!(
        console,
        [
            "clipPath <clipPath id=\"c\"><rect></rect></clipPath>",
            "1 0 0",
            "clipPath 2",
            // An SVG script a fragment made never runs.
            "script"
        ]
    );
    let document = tab.engine.document();
    let svg = document
        .subtree(document.root())
        .find(|&node| document.svg_tag(node) == Some("svg"))
        .unwrap();
    let kids: Vec<_> = document
        .children(svg)
        .filter_map(|node| document.element_name(node))
        .map(|(ns, tag)| format!("{ns:?} {tag}"))
        .collect();
    assert_eq!(kids, ["Svg clipPath", "Svg clipPath", "Svg script"]);
    document.validate().unwrap();
}

#[test]
fn svgs_geometry_answers_scripts_as_the_reference_browser_does() {
    // M32: the probe's rect, its box without stroke or transform, its
    // matrix with the transform; a path's length and points.
    let (_tab, console) = scripted(
        "<svg width=200 height=100><rect id=r x=5 y=6 width=7 height=8 stroke=red \
         stroke-width=3 transform='translate(3 4)'></rect>\
         <path id=p d='M0 0 H30 V40' fill=none></path>\
         <g id=g style='display: none'><rect id=h x=1 y=2 width=3 height=4></rect></g></svg>\
         <script>var r = document.getElementById('r'); var b = r.getBBox(); \
         console.log(b.x, b.y, b.width, b.height, b.right, b instanceof DOMRect); \
         var m = r.getCTM(); console.log(m.a, m.d, m.e, m.f); \
         var p = document.getElementById('p'); var at = p.getPointAtLength(50); \
         console.log(p.getTotalLength(), at.x, at.y); \
         console.log(r.isPointInFill({x: 6, y: 7}), r.isPointInFill(new DOMPoint(1, 1)), \
         r.isPointInStroke({x: 5, y: 10}), r.isPointInStroke({x: 8, y: 10})); \
         var g = document.getElementById('g').getBBox(); \
         var h = document.getElementById('h').getBBox(); console.log(g.width, g.height, h.x, h.height); \
         var d = new DOMRect(1, 2, -3, 4); console.log(d.left, d.right, JSON.stringify(d))</script>",
        true,
    );
    assert_eq!(
        console,
        [
            "5 6 7 8 12 true",
            "1 1 3 4",
            "70 30 20",
            "true false true false",
            "0 0 1 4",
            "-2 1 {\"x\":1,\"y\":2,\"width\":-3,\"height\":4,\"top\":2,\"right\":1,\"bottom\":6,\"left\":-2}"
        ]
    );
}

#[test]
fn svgs_geometry_is_asked_again_after_a_change_and_of_another_svg() {
    // M32: the tree a question is answered from is kept while nothing
    // changes; a change, or an element of another svg, asks again.
    let (_tab, console) = scripted(
        "<svg><rect id=r width=10 height=5></rect><text id=t>ab</text></svg>\
         <svg><g transform='translate(7 0)'><rect id=s x=2 width=4 height=3></rect></g></svg>\
         <script>var r = document.getElementById('r'); \
         var a = r.getBBox().width; var t = document.getElementById('t'); \
         var l = t.getComputedTextLength(); r.setAttribute('width', '30'); \
         t.textContent = 'abab'; \
         console.log(a, r.getBBox().width, t.getComputedTextLength() > l, \
         document.getElementById('s').getCTM().e, r.getCTM().e)</script>",
        true,
    );
    assert_eq!(console, ["10 30 true 7 0"]);
}

#[test]
fn svgs_properties_reach_scripts_through_computed_styles_and_style() {
    // M32: SVG's properties are the cascade's, presentation attributes
    // among its declarations, and CSSOM's.
    let (_tab, console) = scripted(
        "<svg><rect id=r fill=red stroke-width=2 transform='translate(3 4)'></rect></svg>\
         <script>var r = document.getElementById('r'); var cs = getComputedStyle(r); \
         console.log(cs.fill, cs.strokeWidth, cs.getPropertyValue('stroke-miterlimit')); \
         r.style.fill = 'url(#g) blue'; r.setAttribute('stroke-width', '5'); \
         console.log(r.style.fill, getComputedStyle(r).fill, getComputedStyle(r).strokeWidth); \
         r.style.paintOrder = 'stroke stroke'; r.style.r = '2'; \
         console.log(r.style.paintOrder === '', r.style.r)</script>",
        true,
    );
    assert_eq!(
        console,
        [
            "rgb(255, 0, 0) 2px 4",
            // The specified value in its canonical form (G50).
            "url(\"#g\") blue url(\"#g\") rgb(0, 0, 255) 5px",
            "true 2"
        ]
    );
}

#[test]
fn create_element_ns_makes_svgs_interfaces_and_refuses_as_the_reference_browser() {
    // Chrome 154's answers (scratchpad m32/p9-ns.html).
    let console = logged(
        "<script>var SVG = 'http://www.w3.org/2000/svg'; \
         var show = function (e) { return [e.constructor.name, e.tagName, e.localName, \
         e.namespaceURI, e.prefix].join(); }; \
         var tryIt = function (f) { try { return f(); } catch (e) { return e.name + ': ' + e.message; } }; \
         console.log(show(document.createElementNS(SVG, 'rect'))); \
         console.log(show(document.createElementNS(SVG, 's:rect'))); \
         console.log(show(document.createElementNS(null, 'Foo'))); \
         console.log(show(document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi'))); \
         console.log(show(document.createElementNS('urn:x', 'x:item'))); \
         console.log(show(document.createElementNS(SVG, 'blink'))); \
         console.log(tryIt(function () { return document.createElementNS(null, 'a:b'); })); \
         console.log(tryIt(function () { return document.createElementNS('urn:x', 'xml:b'); })); \
         console.log(tryIt(function () { return document.createElementNS('urn:x', 'xmlns'); })); \
         console.log(tryIt(function () { return document.createElementNS(SVG, '1abc'); })); \
         console.log(tryIt(function () { return document.createElementNS(SVG, ''); })); \
         console.log(Object.getPrototypeOf(SVGRectElement.prototype).constructor.name, \
         Object.getPrototypeOf(SVGGeometryElement.prototype).constructor.name, \
         Object.getPrototypeOf(SVGGraphicsElement.prototype).constructor.name, \
         Object.getPrototypeOf(SVGElement.prototype).constructor.name, \
         typeof SVGAnimateElement)</script>",
    );
    assert_eq!(
        console,
        [
            "SVGRectElement,rect,rect,http://www.w3.org/2000/svg,",
            "SVGRectElement,s:rect,rect,http://www.w3.org/2000/svg,s",
            "Element,Foo,Foo,,",
            "MathMLElement,mi,mi,http://www.w3.org/1998/Math/MathML,",
            "Element,x:item,item,urn:x,x",
            "SVGElement,blink,blink,http://www.w3.org/2000/svg,",
            "NamespaceError: Failed to execute 'createElementNS' on 'Document': The namespace URI \
             provided ('') is not valid for the qualified name provided ('a:b').",
            "NamespaceError: Failed to execute 'createElementNS' on 'Document': The namespace URI \
             provided ('urn:x') is not valid for the qualified name provided ('xml:b').",
            "NamespaceError: Failed to execute 'createElementNS' on 'Document': The namespace URI \
             provided ('urn:x') is not valid for the qualified name provided ('xmlns').",
            "InvalidCharacterError: Failed to execute 'createElementNS' on 'Document': The \
             qualified name provided ('1abc') contains the invalid character '1'.",
            "InvalidCharacterError: Failed to execute 'createElementNS' on 'Document': The \
             qualified name provided is empty.",
            // SMIL's interfaces are not there (M32.3).
            "SVGGeometryElement SVGGraphicsElement SVGElement Element undefined",
        ]
    );
}

#[test]
fn an_svg_elements_attributes_names_and_namespaced_lookups_are_the_doms() {
    let console = logged(
        "<svg id=s class='a b'><use id=u xlink:href=#a href=#b /><g><rect id=r /></g></svg>\
         <math><mi id=m>x</mi></math>\
         <script>var XL = 'http://www.w3.org/1999/xlink', SVG = 'http://www.w3.org/2000/svg'; \
         var u = document.getElementById('u'), r = document.getElementById('r'); \
         console.log(u.getAttribute('xlink:href'), u.getAttributeNS(XL, 'href'), \
         u.getAttributeNS(null, 'href'), u.hasAttributeNS(XL, 'href'), u.getAttributeNames().join('|')); \
         r.setAttribute('viewBox', '1'); r.setAttributeNS(XL, 'xlink:title', 't'); \
         console.log(r.getAttributeNames().join('|'), r.getAttribute('viewbox'), r.getAttribute('viewBox')); \
         r.removeAttributeNS(XL, 'title'); console.log(r.getAttributeNames().join('|')); \
         console.log(document.getElementsByTagNameNS(SVG, 'use').length, \
         document.getElementsByTagNameNS('*', 'rect').length, document.getElementsByTagNameNS(SVG, '*').length, \
         document.getElementsByTagNameNS(null, 'use').length); \
         var s = document.getElementById('s'); \
         console.log(typeof s.className, s.className.constructor.name, s.className.baseVal, \
         s.className === s.className, s.classList.length); \
         s.className.baseVal = 'c'; console.log(s.getAttribute('class')); \
         console.log(r.ownerSVGElement.id, s.ownerSVGElement, r.viewportElement.id); \
         console.log(r.tagName, r.nodeName, document.getElementById('m').tagName, \
         document.getElementById('m').constructor.name, s.namespaceURI); \
         console.log(typeof u.focus, u.tabIndex, typeof u.onclick, typeof u.click, 'dataset' in u)</script>",
    );
    assert_eq!(
        console,
        [
            "#a #a #b true id|xlink:href|href",
            "id|viewBox|xlink:title null 1",
            "id|viewBox",
            "1 1 4 0",
            "object SVGAnimatedString a b true 2",
            "c",
            "s null s",
            "rect rect mi MathMLElement http://www.w3.org/2000/svg",
            "function -1 object undefined true",
        ]
    );
}

#[test]
fn an_svg_script_runs_as_htmls_does_parsed_made_or_by_its_href() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<svg><script>console.log('parsed')</script>\
             <script href=a.js></script><script xlink:href=b.js></script></svg>\
             <script>var s = document.createElementNS('http://www.w3.org/2000/svg', 'script'); \
             s.textContent = \"console.log('made')\"; document.body.appendChild(s);</script>",
        )),
        "/a.js" => Some(Reply::ok("text/javascript", b"console.log('href')")),
        "/b.js" => Some(Reply::ok("text/javascript", b"console.log('xlink')")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["parsed", "href", "xlink", "made"]);
}

#[test]
fn the_user_agents_rules_are_htmls_or_svgs_and_a_sheet_declares_its_namespaces() {
    // M32: an SVG `a` takes no link colour, but the pointer as the
    // reference browser's sheet gives it, and an SVG `title` is not
    // hidden by HTML's rules; SVG's own make a `text` a block and clip a
    // nested viewport; an author sheet's `@namespace` names its
    // prefixes, one after a style rule is not taken.
    let console = logged(
        "<style>@namespace svg url(http://www.w3.org/2000/svg); \
         svg|rect { fill-opacity: 0.5; } svg|* { color: rgb(1, 2, 3); } \
         p { color: rgb(9, 9, 9); } @namespace q url(http://www.w3.org/2000/svg); \
         q|rect { color: red; }</style>\
         <p id=p style='color: green'>x<svg id=s><a id=a href=#x><text id=t>y</text></a>\
         <title id=ti>z</title><rect id=r /></svg></p>\
         <script>var cs = function (id, p) { return getComputedStyle(document.getElementById(id))\
         .getPropertyValue(p); }; \
         console.log(cs('a', 'color'), cs('a', 'cursor'), cs('ti', 'display'), \
         cs('t', 'display'), cs('s', 'overflow-x'), cs('r', 'color'))</script>",
    );
    assert_eq!(
        console,
        ["rgb(1, 2, 3) pointer inline block hidden rgb(1, 2, 3)"]
    );
}

#[test]
fn an_element_of_display_contents_answers_as_one_with_no_box() {
    // D77: it has no box, so its rect is empty and its client rects none,
    // as the reference browser answers; its children are boxed in its
    // place, and a point over its ::after finds it.
    let page = "<style>body { margin: 0 } #c { display: contents } \
                #c::after { content: ''; position: absolute; left: 0; top: 0; \
                width: 50px; height: 50px } #p { position: relative; height: 100px }</style>\
                <div id=p><span id=c><b id=b>x</b></span></div>\
                <span id=li style='display: inline list-item'>y</span>\
                <script>\
                var c = document.getElementById('c'), r = c.getBoundingClientRect(); \
                console.log(r.x, r.y, r.width, r.height, c.getClientRects().length); \
                console.log(document.getElementById('b').getClientRects().length, \
                getComputedStyle(c).display, document.elementFromPoint(40, 40).id); \
                var range = document.createRange(); range.selectNode(c); \
                console.log(range.getClientRects().length > 0, \
                document.getElementById('li').clientWidth);\
                </script>";
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", page.as_bytes())),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.frame(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["0 0 0 0 0", "1 contents c", "true 0"]);
}

#[test]
fn layout_queries_lay_the_page_out_as_it_stands() {
    // D65: CSSOM View's members and getComputedStyle's used values read a
    // layout of the document as the script left it.
    let page = "<style>body { margin: 0 } \
                #a { width: 100px; height: 50px; padding: 5px; border: 2px solid; margin: 10px } \
                #s { height: 40px; overflow: auto } #s div { height: 200px }</style>\
                <div id=a>x</div><div id=s><div></div></div>\
                <p id=t>one <b>two</b><br>three</p><div id=n style='display: none'>hidden text</div>\
                <script>\
                var a = document.getElementById('a'), s = document.getElementById('s'), \
                t = document.getElementById('t'), r = a.getBoundingClientRect(); \
                console.log(r.x, r.y, r.width, r.height); \
                console.log(a.offsetTop, a.offsetLeft, a.offsetWidth, a.offsetHeight, \
                a.offsetParent === document.body); \
                console.log(a.clientTop, a.clientWidth, a.clientHeight); \
                var cs = getComputedStyle(a); \
                console.log(cs.width, cs.height, cs.marginTop, cs.paddingLeft); \
                console.log(s.scrollHeight, s.clientHeight); \
                s.scrollTop = 30; console.log(s.scrollTop); s.scrollTop = 999; console.log(s.scrollTop); \
                console.log(JSON.stringify(t.innerText), document.getElementById('n').innerText); \
                console.log(JSON.stringify(document.body.innerText.split(String.fromCharCode(10, 10))[0])); \
                console.log(document.elementFromPoint(20, 20).id, window.scrollY, \
                document.scrollingElement.nodeName, t.querySelector('b').getClientRects().length); \
                a.style.width = '200px'; console.log(a.offsetWidth);\
                </script>";
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", page.as_bytes())),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.frame(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        console,
        [
            "10 10 114 64",
            "10 10 114 64 true",
            "2 110 60",
            "100px 50px 10px 5px",
            "200 40",
            "30",
            "160",
            "\"one two\\nthree\" hidden text",
            "\"x\"",
            "a 0 HTML 1",
            "214",
        ]
    );
    // The scroll a script set is the engine's once the run ends.
    let document = tab.engine.document();
    let s = document
        .subtree(document.root())
        .find(|&n| document.attr(n, "id") == Some("s"))
        .expect("the scroller");
    assert_eq!(tab.engine.element_scroll(s), 160.0);
}

#[test]
fn a_selections_string_is_its_rendered_text() {
    // As the reference browser gives it: transformed, white space
    // collapsed, hidden text left out, cut at the range's ends.
    let page = "<p id=p style='text-transform: uppercase'>stra\u{df}e  one\
                <span style='display: none'>gone</span> two</p>\
                <script>\
                var t = document.getElementById('p').firstChild; \
                getSelection().setBaseAndExtent(t, 4, t, 5); \
                console.log(getSelection().toString()); \
                getSelection().setBaseAndExtent(t, 2, document.getElementById('p'), 3); \
                console.log(JSON.stringify(getSelection().toString()));\
                </script>";
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", page.as_bytes())),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.frame(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["SS", "\"RASSE ONE TWO\""]);
}

#[test]
fn font_palette_values_rules_and_font_palette_as_the_reference_has_them() {
    // Chrome 156, probed: a CSSFontPaletteValuesRule of type 0, its name
    // read only, its descriptors "" where the rule has none; the feature
    // values rule after it in the list; font-palette inherited and
    // written as the reference writes it.
    let page = "<style id=s>@font-palette-values --A { font-family: a; base-palette: 3; \
                base-palette: random(10, 30); override-colors: 0 #00ff0080 } \
                @font-feature-values f { @swash { x: random(1, 3) } }</style>\
                <div id=d style=\"font-palette: palette-mix(in hsl longer hue, light, dark 20%)\">\
                <i id=i></i></div><script>\
                var r = document.getElementById('s').sheet.cssRules; \
                console.log(r.length, r[0].constructor.name, r[0].type, r[0].name, r[0].fontFamily, \
                r[0].basePalette, r[0].overrideColors); \
                r[0].name = '--B'; console.log(r[0].name, r[1].type, r[1].swash.size); \
                var d = document.getElementById('d'), i = document.getElementById('i'); \
                console.log(d.style.fontPalette, getComputedStyle(i).fontPalette); \
                </script><style>@keyframes k { to { font-palette: --custom } } \
                #m { animation: k 1s linear -0.25s paused } \
                #n { animation: k 1s linear -2s forwards }</style>\
                <p id=m></p><p id=n></p><script>\
                console.log(getComputedStyle(document.getElementById('m')).fontPalette, \
                getComputedStyle(document.getElementById('n')).fontPalette);</script>";
    let (tab, console) = scripted(page, true);
    assert_eq!(
        console,
        [
            "2 CSSFontPaletteValuesRule 0 --A a 3 0 rgba(0, 255, 0, 0.5)",
            "--A 14 0",
            "palette-mix(in hsl longer hue, light 80%, dark 20%) \
             palette-mix(in hsl longer hue, light 80%, dark 20%)",
            "palette-mix(in oklab, normal 75%, --custom 25%) --custom",
        ]
    );
    drop(tab);
}

#[test]
fn a_loop_over_a_sheets_rules_reads_the_sheet_once() {
    // The site sweep's bbc: `cssRules[i]` and `cssRules.length` at each
    // turn of a loop read the whole sheet each time, 2,200 reads and 13 s
    // past the sweep's patience, the document changed at each turn. A
    // list keeps what it read while nothing could have changed it.
    let mut rules = String::from("<style id=s>");
    for n in 0..300 {
        rules.push_str(&format!(".r{n} {{ top: {n}px }}"));
    }
    rules.push_str(
        "</style><script>\
         var list = document.getElementById('s').sheet.cssRules, last = '';\
         for (var i = 0; i < list.length; i++) {\
           last = list[i].selectorText; document.documentElement.dataset.i = i;\
         }\
         console.log(list.length, last);\
         </script>",
    );
    let page: &'static str = Box::leak(rules.into_boxed_str());
    let (tab, said) = scripted(page, true);
    assert_eq!(said, ["300 .r299"]);
    let reads = tab.engine.shown.scripts.rule_reads();
    assert!(reads <= 2, "the sheet was read {reads} times");
}

#[test]
fn a_sheet_built_rule_by_rule_is_not_read_whole_at_each_rule() {
    // A CSS-in-JS library's pattern (the site sweep's bbc): a rule
    // inserted, the list's length read, 300 times, and some deleted. The
    // list keeps its rules past its own edits, and they are the sheet's.
    let server = serve(|path| match path {
        "/page.html" => Some(Reply::ok(
            "text/html",
            b"<style id=s></style><script>\
              var sheet = document.getElementById('s').sheet, list = sheet.cssRules;\
              function texts() {\
                var out = [];\
                for (var i = 0; i < list.length; i++) { out.push(list[i].cssText); }\
                return out.join('|');\
              }\
              for (var i = 0; i < 300; i++) {\
                sheet.insertRule('.r' + i + ' { top: ' + i + 'px }', list.length);\
                if (i % 50 == 49) { sheet.deleteRule(0); }\
              }\
              sheet.insertRule('@media (min-width: 1px) { b { top: 0 } }', 3);\
              var kept = texts();\
              console.log(list.length);\
              setTimeout(function () { console.log(texts() === kept); });\
              </script>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let said = tab.run_until(|lines| lines.len() >= 2);
    // The second run's list read the sheet again, and found the same.
    assert_eq!(said, ["295", "true"]);
    let reads = tab.engine.shown.scripts.rule_reads();
    assert!(reads <= 3, "the sheet was read {reads} times");
}

#[test]
fn a_rule_list_reads_its_sheet_again_once_it_changed() {
    // What a list keeps goes with an edit through the CSSOM, a `<style>`'s
    // text changed, and the run: a sheet may have come between runs.
    let said = logged(
        "<style id=s>p { top: 0 }</style><script>\
         var s = document.getElementById('s'), list = s.sheet.cssRules;\
         console.log(list.length);\
         s.sheet.insertRule('b { top: 1px }', 1); console.log(list.length);\
         s.textContent = 'i { top: 2px } u { top: 3px } q { top: 4px }';\
         var again = s.sheet.cssRules; console.log(again.length, again[2].selectorText);\
         s.sheet.deleteRule(0); console.log(again.length, again[0].selectorText);\
         </script>",
    );
    assert_eq!(said, ["1", "2", "3 q", "2 u"]);
}

/// A rule list read and kept, `edit` made, then the kept list's texts
/// (`list`, the sheet's, unless `edit` sets `list` to another) said; in a
/// later task, when every list reads its sheet again, whether a fresh
/// read of each of `lists` says the same.
fn kept_then_fresh(style: &str, edit: &str) -> Vec<String> {
    let page = format!(
        "<style id=s>{style}</style><script>\
         function texts(l) {{\
           var out = [];\
           for (var i = 0; i < l.length; i++) {{ out.push(l[i].cssText); }}\
           return out.join(' | ');\
         }}\
         var s = document.getElementById('s'), sheet = s.sheet, list = sheet.cssRules;\
         texts(list);\
         {edit}\
         var kept = texts(list); console.log(kept);\
         setTimeout(function () {{ console.log(texts(list) === kept); }});\
         </script>"
    )
    .into_bytes();
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", &page)),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    tab.run_until(|lines| lines.len() >= 2)
}

#[test]
fn a_kept_rule_list_sees_a_rules_declarations_changed() {
    let said = kept_then_fresh(
        ".a { color: red }",
        "list[0].style.setProperty('top', '1px');",
    );
    assert_eq!(said, [".a { color: red; top: 1px; }", "true"]);
    let said = kept_then_fresh(
        ".a { color: red; top: 1px }",
        "list[0].style.removeProperty('color');",
    );
    assert_eq!(said, [".a { top: 1px; }", "true"]);
    let said = kept_then_fresh(".a { color: red }", "list[0].style.cssText = 'left: 2px';");
    assert_eq!(said, [".a { left: 2px; }", "true"]);
}

#[test]
fn a_kept_rule_list_sees_a_selector_changed() {
    let said = kept_then_fresh(".a { color: red }", "list[0].selectorText = '.b > i';");
    assert_eq!(said, [".b > i { color: red; }", "true"]);
}

#[test]
fn a_kept_rule_list_sees_a_nested_list_changed() {
    // The `@media` rule's own list and the sheet's, which holds its text.
    let said = kept_then_fresh(
        "@media print { .a { top: 0px } }",
        "var inner = list[0].cssRules; texts(inner);\
         inner.length; list[0].insertRule('.b { left: 1px }', 1);\
         list[0].deleteRule(0);\
         console.log(texts(inner));",
    );
    assert_eq!(
        said,
        [
            ".b { left: 1px; }",
            "@media print {\n  .b { left: 1px; }\n}",
            "true"
        ]
    );
}

#[test]
fn a_kept_rule_list_sees_a_constructed_sheet_replaced() {
    let said = kept_then_fresh(
        "",
        "var c = new CSSStyleSheet(); c.replaceSync('.a { top: 0px }');\
         list = c.cssRules; texts(list);\
         c.replaceSync('.b { top: 1px } .c { top: 2px }');",
    );
    assert_eq!(said, [".b { top: 1px; } | .c { top: 2px; }", "true"]);
}

#[test]
fn a_kept_rule_list_sees_a_styles_text_changed() {
    // The list kept reads the `<style>`'s rules as they stand (what the
    // sheet it replaced says is a known gap, roadmap), and so does the
    // new sheet's.
    let said = kept_then_fresh(
        ".a { top: 0px }",
        "s.textContent = '.b { top: 1px }';\
         console.log(texts(list) === texts(s.sheet.cssRules));\
         list = s.sheet.cssRules;",
    );
    assert_eq!(said, ["true", ".b { top: 1px; }", "true"]);
}

#[test]
fn two_references_to_a_sheet_see_one_anothers_inserts() {
    // A list held, a rule inserted through the sheet and through another
    // reference to it: the list and a fresh one say what a fresh read
    // says.
    let said = kept_then_fresh(
        ".a { top: 0px }",
        "var held = list; sheet.insertRule('.b { top: 1px }', 1);\
         document.styleSheets[0].insertRule('.c { top: 2px }', 2);\
         console.log(texts(held) === texts(sheet.cssRules), held === sheet.cssRules);",
    );
    assert_eq!(
        said,
        [
            "true true",
            ".a { top: 0px; } | .b { top: 1px; } | .c { top: 2px; }",
            "true"
        ]
    );
}

#[test]
fn style_sheets_and_their_rules_as_the_reference_browser_has_them() {
    // D66: document.styleSheets, a sheet's rules read and changed, the
    // cascade following; a new text a new sheet.
    let page = "<style id=s>p { color: red } @media (min-width: 1px) { b { top: 0 } }</style>\
                <p id=p>x</p><script>\
                var sheet = document.styleSheets[0], s = document.getElementById('s'), \
                p = document.getElementById('p'); \
                console.log(document.styleSheets.length, sheet === s.sheet, sheet.ownerNode === s, \
                sheet.cssRules.length); \
                console.log(sheet.cssRules[0].cssText, sheet.cssRules[0].selectorText, \
                sheet.cssRules[1].type, sheet.cssRules[1].cssRules[0].cssText); \
                var first = sheet.cssRules[0], media = sheet.cssRules[1]; \
                sheet.insertRule('p { color: rgb(0, 128, 0) }', 1); \
                console.log(sheet.cssRules.length, sheet.cssRules[0] === first, \
                sheet.cssRules[2] === media, getComputedStyle(p).color); \
                first.style.color = 'blue'; console.log(first.cssText); \
                sheet.deleteRule(1); \
                try { sheet.insertRule('nonsense', 0); } catch (e) { console.log(e.name); } \
                try { sheet.replaceSync(''); } catch (e) { console.log(e.name); } \
                console.log(getComputedStyle(p).color, first.parentStyleSheet === sheet); \
                s.textContent = 'i { }';\
                </script><script>\
                console.log(document.styleSheets[0] === sheet, \
                document.styleSheets[0].cssRules[0].cssText);</script>";
    let (tab, console) = scripted(page, true);
    assert_eq!(
        console,
        [
            "1 true true 2",
            "p { color: red; } p 4 b { top: 0; }",
            "3 true true rgb(0, 128, 0)",
            "p { color: blue; }",
            "SyntaxError",
            "NotAllowedError",
            "rgb(0, 0, 255) true",
            "false i { }",
        ]
    );
    drop(tab);
}

#[test]
fn a_rule_a_script_changed_is_the_cascades() {
    // After the load, so that only the CSSOM's change takes the sheets
    // again; an import whose rule goes takes its rules with it.
    let server = serve(|path| match path {
        "/page.html" => Some(Reply::ok(
            "text/html",
            b"<style>@import 'a.css'; @import 'b.css'; p { color: red }</style><p id=p>x</p>\
              <script>setTimeout(function () { var sheet = document.styleSheets[0]; \
              sheet.cssRules[2].style.color = 'blue'; sheet.deleteRule(0); \
              console.log('done'); }, 0);</script>",
        )),
        "/a.css" => Some(Reply::ok("text/css", b"p { color: rgb(1, 2, 3) }")),
        "/b.css" => Some(Reply::ok("text/css", b"i { color: rgb(1, 2, 3) }")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    tab.run_until(|lines| lines.iter().any(|line| line == "done"));
    let document = tab.engine.document();
    let p = document
        .subtree(document.root())
        .find(|&n| document.attr(n, "id") == Some("p"))
        .expect("the paragraph");
    let styles = tab.engine.cached_styles();
    assert_eq!(
        styles.get(p).expect("styled").color,
        style::computed::Color::rgb8(0, 0, 255)
    );
}

/// `page` loaded with scripts on in an 800 by 600 window; the tab and
/// the console's lines.
fn laid_out(page: &'static str) -> (Tab, Vec<String>) {
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", page.as_bytes())),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.frame(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    drop(server);
    (tab, console)
}

#[test]
fn the_insets_resolve_to_their_used_values() {
    // D65: a positioned box's `top` to `left` as the reference browser
    // resolves them, from a sheet the script itself put in.
    let (_, console) = laid_out(
        "<style>body { margin: 0 }</style><div id=c><div id=a></div><div id=r></div>\
         <div id=s></div><div id=t></div></div><script>\
         var s = document.createElement('style'); \
         s.textContent = '#c { position: relative; width: 400px; height: 200px; padding: 10px; \
         border: 5px solid } #a { position: absolute; top: 10%; bottom: auto; left: auto; \
         right: 30px; width: 20px; height: 20px; margin: 3px } \
         #r { position: relative; top: 10%; left: auto; right: 5px } \
         #s { position: sticky; top: 10%; left: auto } #t { top: 5px }'; \
         document.head.append(s); \
         var v = function (id) { var c = getComputedStyle(document.getElementById(id)); \
         return [c.top, c.right, c.bottom, c.left].join(' '); }; \
         console.log(v('a')); console.log(v('r')); console.log(v('s')); console.log(v('t'));\
         </script>",
    );
    assert_eq!(
        console,
        [
            // 10% of the padding box's 220px; the margin box's bottom
            // 220 - 22 - 26 short of its end; 420 - 30 - 26 from its left.
            "22px 30px 172px 364px",
            "20px 5px -20px -5px",
            // Of the viewport, the scroll container here.
            "60px auto auto auto",
            "5px auto auto auto",
        ]
    );
}

#[test]
fn a_fieldsets_absolutely_positioned_box_has_its_border_taken_off_twice() {
    // As the reference browser has it: offsetLeft and offsetTop of an
    // absolutely positioned box in a positioned fieldset.
    let (_, console) = laid_out(
        "<style>body { margin: 0 } fieldset { margin: 0; padding: 3px; border: 5px solid }\
         </style><fieldset id=f style='position: relative; width: 200px; height: 100px'>\
         <div id=a style='position: absolute; left: 0; top: 0; width: 10px; height: 10px'></div>\
         <div id=s style='height: 10px'></div></fieldset><script>\
         var a = document.getElementById('a'), s = document.getElementById('s'); \
         console.log(a.offsetLeft, a.offsetTop, s.offsetLeft, s.offsetTop);</script>",
    );
    assert_eq!(console, ["-5 -5 3 3"]);
}

#[test]
fn style_elements_sheets_namespaces_and_media_lists_follow_the_script() {
    // G50: a `<style>`'s new text is its sheet at once; a removed one's
    // sheet stays, its owner gone; namespace rules and selectors in
    // their canonical forms; media lists that set.
    let (_, console) = laid_out(
        "<style id=s></style><script>\
         var s = document.getElementById('s'); \
         s.textContent = '@namespace x url(y); x|p, *.c:before { color: red } \
         @media SCREEN and (min-width: 1px) { b { top: 0 } }'; \
         var sheet = s.sheet, rules = sheet.cssRules; \
         console.log(rules.length, rules[0].prefix, rules[0].namespaceURI, rules[1].selectorText); \
         console.log(rules[2].media.mediaText); \
         rules[2].media.appendMedium('print'); \
         rules[2].media.deleteMedium('screen and (min-width: 1px)'); \
         console.log(rules[2].conditionText, rules[2].media.length); \
         sheet.media.mediaText = 'PRINT, (color)'; \
         console.log(s.getAttribute('media'), sheet.media[1]); \
         try { rules[2].media.deleteMedium('tv'); } catch (e) { console.log(e.name); } \
         sheet.addRule('i', 'top: 1px'); console.log(rules[rules.length - 1].cssText); \
         sheet.removeRule(rules.length - 1); \
         s.remove(); console.log(sheet.ownerNode, s.sheet, sheet.cssRules.length);\
         </script>",
    );
    assert_eq!(
        console,
        [
            "3 x y x|p, .c::before",
            "screen and (min-width: 1px)",
            "print 1",
            "print, (color) (color)",
            "NotFoundError",
            "i { top: 1px; }",
            "null null 3",
        ]
    );
}

#[test]
fn declarations_are_written_in_their_canonical_forms_and_shorthands() {
    // G50: specified values at their shortest, URLs quoted, commas
    // spaced; the box and border shorthands kept as their longhands and
    // said from them.
    let (_, console) = laid_out(
        "<div id=d></div><script>var d = document.getElementById('d'); \
         d.style.cssText = 'margin-top: .5em; margin-right: 1px; margin-bottom: .5em; \
         margin-left: 1px; color: rgb(1,2,3); background-image: url(a.png)'; \
         console.log(d.style.cssText); \
         d.style.cssText = 'border: 1px solid; border-top-color: red'; \
         console.log(d.style.borderWidth, JSON.stringify(d.style.border), d.style.cssText); \
         d.style.removeProperty('border'); console.log(JSON.stringify(d.style.cssText));\
         </script>",
    );
    assert_eq!(
        console,
        [
            "margin: 0.5em 1px; color: rgb(1, 2, 3); background-image: url(\"a.png\");",
            "1px \"\" border-width: 1px; border-style: solid; \
             border-color: red currentcolor currentcolor; border-image: none;",
            "\"\"",
        ]
    );
}

#[test]
fn scroll_into_view_reads_scroll_margin_and_scroll_padding() {
    // The target's box grown by its margin, in the scrollport less the
    // container's padding; the root's padding for the viewport.
    let (_, console) = laid_out(
        "<style>body { margin: 0 } html { scroll-padding: 10px 0 0 } \
         #s { height: 100px; overflow: auto; scroll-padding-top: 20px } \
         #t { margin-top: 300px; height: 10px; scroll-margin: 5px } \
         #f { height: 50px }</style>\
         <div id=s><div id=t></div><div style='height: 500px'></div></div>\
         <div style='height: 2000px'></div><div id=f></div>         <div style='height: 2000px'></div><script>\
         var s = document.getElementById('s'); \
         document.getElementById('t').scrollIntoView(); \
         console.log(s.scrollTop); \
         document.getElementById('t').scrollIntoView({ block: 'end' }); \
         console.log(s.scrollTop); \
         document.getElementById('f').scrollIntoView(); \
         console.log(scrollY);</script>",
    );
    // 300 less 5 less 20; 315 past a port of 100 is 215; the viewport
    // stops 10px above #f's top at 2100.
    assert_eq!(console, ["275", "215", "2090"]);
}

#[test]
fn a_ranges_rects_are_its_characters_boxes() {
    // Each character's box from the text's fragment, collapsed white
    // space taking none, an element held whole giving its own box.
    let (_, console) = laid_out(
        "<style>body { margin: 0 } p { margin: 0; font: 20px monospace }</style>\
         <p id=p>abc   def<span id=s>gh</span></p><p id=q>ab&shy;cd</p><script>\
         var t = document.getElementById('p').firstChild; \
         var r = document.createRange(); \
         var box = function (a, b) { r.setStart(t, a); r.setEnd(t, b); \
           return r.getBoundingClientRect(); }; \
         var one = box(0, 1), three = box(0, 3), spaces = box(3, 6), caret = box(5, 5); \
         console.log(one.left, one.width > 0, three.width === 3 * one.width); \
         console.log(spaces.width > 0, spaces.left === three.right); \
         console.log(caret.width, caret.left === spaces.right, caret.height > 0); \
         r.setStart(t, 0); r.setEndAfter(document.getElementById('s')); \
         console.log(r.getClientRects().length, r.getBoundingClientRect().width > 7 * one.width); \
         t = document.getElementById('q').firstChild; \
         console.log(box(0, 4).width === 3 * one.width);</script>",
    );
    assert_eq!(
        console,
        ["0 true true", "true true", "0 true true", "2 true", "true"]
    );
}

#[test]
fn a_disabled_sheet_and_a_constructed_sheets_media_reach_the_cascade() {
    let (_, console) = laid_out(
        "<style id=s>p { color: rgb(255, 0, 0) }</style><p id=p>x</p><script>\
         var p = document.getElementById('p'); \
         var color = function () { console.log(getComputedStyle(p).color); }; \
         var sheet = document.getElementById('s').sheet; \
         sheet.disabled = true; color(); \
         sheet.disabled = false; color(); \
         var c = new CSSStyleSheet({ media: 'print' }); \
         c.replaceSync('p { color: rgb(0, 0, 255) }'); \
         document.adoptedStyleSheets = [c]; color(); \
         c.media.mediaText = 'screen'; color(); \
         c.disabled = true; color(); \
         try { new CSSStyleSheet({ baseURL: 'https://test:test/' }); } \
         catch (e) { console.log(e.name); }</script>",
    );
    assert_eq!(
        console,
        [
            "rgb(0, 0, 0)",
            "rgb(255, 0, 0)",
            "rgb(255, 0, 0)",
            "rgb(0, 0, 255)",
            "rgb(255, 0, 0)",
            "NotAllowedError"
        ]
    );
}

#[test]
fn boxes_and_the_viewport_scroll_across() {
    // Horizontal scrolling for scripts: scrollLeft, scrollTo with left,
    // scrollBy, scrollWidth, the window's scrollX, and scrollIntoView's
    // inline alignment; the engine takes the offsets.
    let (tab, console) = laid_out(
        "<style>body { margin: 0 } #s { width: 100px; height: 100px; overflow: auto } \
         #s div { width: 300px; height: 50px } #w { width: 2000px; height: 10px } \
         #t { margin-left: 1500px; width: 10px; height: 10px }</style>\
         <div id=s><div></div></div><div id=w></div><div id=t></div><script>\
         var s = document.getElementById('s'); \
         console.log(s.scrollWidth, s.scrollLeft); \
         s.scrollLeft = 50; console.log(s.scrollLeft); \
         s.scrollTo({ left: 999 }); console.log(s.scrollLeft); \
         s.scrollBy(-30, 0); console.log(s.scrollLeft); \
         var p = s.scrollTo(1); console.log(p instanceof Promise); \
         p.catch(function (e) { console.log(e.name); }); \
         window.scrollTo({ left: 40 }); console.log(window.scrollX); \
         document.getElementById('t').scrollIntoView({ inline: 'center' }); \
         console.log(window.scrollX);</script>",
    );
    assert_eq!(
        console,
        [
            "300 0",
            "50",
            "200",
            "170",
            "true",
            "40",
            "1105",
            "TypeError"
        ]
    );
    let document = tab.engine.document();
    let s = document
        .subtree(document.root())
        .find(|&n| document.attr(n, "id") == Some("s"))
        .expect("the scroller");
    assert_eq!(tab.engine.element_scroll_x(s), 170.0);
    assert_eq!(tab.engine.scroll_x(), 1105.0);
}

#[test]
fn container_queries_are_answered_after_layout_for_scripts_and_frames() {
    // D72: a script's computed style lays the document out and restyles
    // the subtrees of the containers whose sizes changed what their
    // queries and units answer, a nested container a pass later; an
    // unknown query applies nothing; a change of a container's size is
    // answered again, by a script and by a frame.
    let page: &'static str = "<style>#c { container: c / size; width: 300px; height: 100px }\
         #d { container-type: inline-size; width: 50% }\
         @container c (width > 200px) { #p { color: #f00 } #d { width: 75% } }\
         @container (width = 225px) { #q { color: #0f0 } }\
         @container c (foo(width)) or (not (foo(width))) { #p { --unknown: 1 } }\
         @container nothing (width) { #p { --named: 1 } }\
         #q { width: 10cqw }</style>\
         <div id=c><p id=p>p</p><div id=d><p id=q>q</p></div></div>\
         <script>var p = document.getElementById('p'), q = document.getElementById('q');\
         var cs = (e, n) => getComputedStyle(e).getPropertyValue(n);\
         console.log(cs(p, 'color'), cs(q, 'color'), cs(q, 'width'), \
         JSON.stringify(cs(p, '--unknown')), JSON.stringify(cs(p, '--named'))); \
         document.getElementById('c').style.width = '100px'; \
         console.log(cs(p, 'color'), cs(q, 'color'), cs(q, 'width'));</script>";
    let server = serve(move |path| (path == "/page.html").then(|| html(page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_viewport(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        console,
        [
            "rgb(255, 0, 0) rgb(0, 255, 0) 22.5px \"\" \"\"",
            "rgb(0, 0, 0) rgb(0, 0, 0) 5px",
        ]
    );
    // The frame's layout settles them too, the paragraph black at the
    // container's width the script left.
    let _ = tab.engine.layout(800.0);
    assert_eq!(color(&mut tab.engine, "p"), (0.0, 0.0, 0.0));
    assert_eq!(color(&mut tab.engine, "q"), (0.0, 0.0, 0.0));
}

#[test]
fn a_containers_pseudo_elements_query_it_and_one_unstyled_inherits() {
    // D72: a pseudo-element's first query container is its element; one
    // no rule styles has the style its element passes on.
    let page: &'static str = "<style>#t { container-type: inline-size; --v: x }\
         #t::before, #t::first-line { color: #f00 }\
         @container (width >= 300px) { #t::before { color: #0f0 } }\
         </style><div id=o style='width: 200px'><div id=t></div></div>\
         <script>var t = document.getElementById('t');\
         var cs = (p, n) => getComputedStyle(t, p).getPropertyValue(n);\
         console.log(cs('::before', 'color'), \
         cs('::first-line', 'color'), cs('::after', '--v'), cs('::after', 'content'), \
         cs('::after', 'color')); \
         document.getElementById('o').style.width = '300px'; \
         console.log(cs('::before', 'color'));</script>";
    let server = serve(move |path| (path == "/page.html").then(|| html(page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_viewport(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        console,
        [
            "rgb(255, 0, 0) rgb(255, 0, 0) x none rgb(0, 0, 0)",
            "rgb(0, 255, 0)",
        ]
    );
}

#[test]
fn an_anonymous_table_inside_an_element_is_not_measured_as_its_box() {
    // A cell inside a size container's block is wrapped in an anonymous
    // table named after the block, which overflows it: the block's own
    // box alone is what `clientWidth` and `offsetWidth` measure, as the
    // reference browser has it (nested-query-containers.html).
    let page: &'static str = "<style>section { width: 500px }\
         .c { display: table-cell } div { container-type: size; border: solid 1px }\
         </style><section><div class=c><div><div class=c><div></div></div></div></div></section>\
         <script>var out = []; for (const d of document.querySelectorAll('div')) \
         out.push(d.clientWidth + 'x' + d.clientHeight + '/' + d.offsetWidth); console.log(out.join(' '));</script>";
    let server = serve(move |path| (path == "/page.html").then(|| html(page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_viewport(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["2x2/4 0x0/2 2x2/4 0x0/2"]);
}

#[test]
fn nested_containers_settle_a_level_a_pass_up_to_the_bound() {
    // D72: each pass settles one more level of nested containers, each
    // a pixel narrower than the one around it by a query of its width;
    // past `MAX_CONTAINER_PASSES` levels, the deeper ones keep the styles
    // of the sizes of the pass before: the width of the one around them.
    let page: &'static str = "<style id=s>.c { container-type: inline-size } #o { width: 200px }</style>\
         <div id=o class=c></div><script>\
         var sheet = document.getElementById('s').sheet, at = document.getElementById('o');\
         for (var w = 200; w > 100; w--) {\
           sheet.insertRule('@container (width = ' + w + 'px) { .c { max-width: ' + (w - 1) + 'px } }');\
           var child = document.createElement('div'); child.className = 'c'; at.appendChild(child); at = child;\
         }\
         var widths = Array.from(document.querySelectorAll('.c'), c => c.offsetWidth);\
         console.log(widths[0], widths[10], widths[60], widths[64], widths[99]);</script>";
    let server = serve(move |path| (path == "/page.html").then(|| html(page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_viewport(800, 600);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    let widths: Vec<i32> = console[0]
        .split(' ')
        .map(|w| w.parse().expect("a width"))
        .collect();
    assert_eq!(&widths[..3], [200, 190, 140]);
    let bound = style::container::MAX_CONTAINER_PASSES as i32;
    // The levels past the bound did not settle.
    assert!(widths[4] > 200 - 99, "{widths:?}");
    assert!(widths[3] >= 200 - bound, "{widths:?}");
}

/// Runs `test` on a thread of the engine's stack, with an engine whose
/// scripts run, the bundled font its only one (G253).
fn on_engine_stack(test: impl FnOnce(&mut crate::Engine) + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(move || {
            let mut engine = crate::Engine::with_fonts(
                text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                    .expect("the bundled font parses"),
            );
            engine.set_scripting(true);
            test(&mut engine);
        })
        .expect("a thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// The texts `engine`'s console took since this was last asked.
fn console_of(engine: &mut crate::Engine) -> Vec<String> {
    engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// Hosts nested in each other's light trees, `hosts` of them, each one's
/// shadow tree `deep` elements of `display: table-cell` deep with its
/// slot at the bottom, an `i#d` in the last: a flat tree as deep as they
/// add up to, each tree within the DOM's cap (G253).
const SLOTTED_DEEP: &str = "function slotted(hosts, deep) { \
    let at = document.body; \
    for (let h = 0; h < hosts; h++) { \
      const host = document.createElement('section'); at.append(host); \
      let inner = host.attachShadow({mode: 'open'}); \
      const sheet = document.createElement('style'); \
      sheet.textContent = 'div { display: table-cell }'; inner.append(sheet); \
      for (let i = 0; i < deep; i++) { const d = document.createElement('div'); inner.append(d); inner = d; } \
      inner.append(document.createElement('slot')); at = host; } \
    const i = document.createElement('i'); i.id = 'd'; i.textContent = 'x'; at.append(i); }";

#[test]
fn a_script_at_its_stack_limit_lays_out_the_deepest_pages_and_throws_no_deeper() {
    // G253: the native stack under a script's: one recursing all but to
    // its limit, then laying out the deepest page there is at the bottom
    // (boxes nested to `layout::MAX_BOX_DEPTH` about table cells, real
    // tables to the parser's cap, shadow trees' slots past the DOM's),
    // lays it out, and one going deeper throws a RangeError it catches,
    // the page working on; never the process's end. On a thread of
    // 26 MiB, this ended the process (measured).
    let pages = [
        format!(
            "<style>.c {{ display: table-cell }}</style>{}<i id=d>x</i>",
            "<div class=c>".repeat(dom::MAX_DEPTH - 4)
        ),
        format!(
            "{}<i id=d>x</i>",
            "<table><tr><td>".repeat(dom::MAX_DEPTH / 4 - 2)
        ),
        // Its shadow trees made by the script below.
        "<body>".to_owned(),
    ];
    on_engine_stack(move |engine| {
        for page in pages {
            engine.load_document(&page);
            engine.frame(200, 100);
            if page == "<body>" {
                crate::differential::run_script(engine, &format!("{SLOTTED_DEEP} slotted(2, 300)"));
            }
            crate::differential::run_script(
                engine,
                "const d = document.getElementById('d'); \
                 let w = 0; \
                 function deep(n, lay) { \
                   if (n === 0) { \
                     if (!lay) return 0; \
                     d.style.width = (++w) + 'px'; return d.getBoundingClientRect().width; } \
                   return 1 + deep(n - 1, lay); } \
                 let limit = 0; \
                 try { for (let n = 100; ; n += 100) { deep(n, false); limit = n; } } \
                 catch (e) { console.log('limit ' + (limit >= 3000) + ' ' + e.name); } \
                 const out = []; \
                 for (const back of [400, 100, 25, 10, -500]) { \
                   try { out.push(typeof deep(limit - back, true)); } catch (e) { out.push(e.name); } } \
                 console.log(out.join(' '));",
            );
            crate::differential::run_script(engine, "console.log('after')");
            assert_eq!(
                console_of(engine),
                [
                    "limit true RangeError",
                    "number number number number RangeError",
                    "after"
                ],
                "{page:.60}"
            );
        }
    });
}

#[test]
fn a_flat_tree_deeper_than_a_tree_may_be_is_drawn_to_the_cap() {
    // G253: hosts slotted into each other's shadow trees, each tree within
    // the DOM's cap, nest past it in the flat tree the boxes are built
    // from: drawn, its boxes stopping at the cap, the content past it
    // making none; before, the boxes' recursion ended the process.
    on_engine_stack(|engine| {
        engine.load_document("<body></body>");
        engine.frame(200, 100);
        crate::differential::run_script(
            engine,
            &format!(
                "document.body.innerHTML = '<span id=top>top</span>'; \
                 {SLOTTED_DEEP} slotted(3, 300); \
                 const top = document.getElementById('top').getBoundingClientRect(); \
                 const last = document.getElementById('d').getBoundingClientRect(); \
                 console.log((top.width > 0) + ' ' + (last.width + last.height));"
            ),
        );
        engine.frame(200, 100);
        assert_eq!(console_of(engine), ["true 0"]);
    });
}

#[test]
fn every_walk_of_a_page_as_deep_as_it_goes_ends_or_throws() {
    // G253: a page nested to the DOM's cap and, through shadow trees'
    // slots, past it in the flat tree; every walk of it a script or the
    // engine makes either ends or throws, the process never ending.
    on_engine_stack(|engine| {
        engine.load_document(&format!(
            "<style>.x div {{ color: #00f }}</style><body>{}<b id=light tabindex=0>x</b>",
            "<div>".repeat(dom::MAX_DEPTH - 4)
        ));
        engine.frame(200, 100);
        let walks = "const deep = document.getElementById('light'); \
            const nested = (n) => { let o = {}; for (let i = 0; i < n; i++) o = { o }; return o; }; \
            const walks = { \
              innerHTML: () => document.body.innerHTML.length, \
              outerHTML: () => document.documentElement.outerHTML.length, \
              textContent: () => document.body.textContent.length, \
              normalize: () => document.body.normalize(), \
              cloneNode: () => document.body.cloneNode(true).childNodes.length, \
              importNode: () => document.importNode(document.body, true).childNodes.length, \
              range: () => { const r = document.createRange(); r.selectNodeContents(document.body); \
                return r.toString().length + r.cloneContents().childNodes.length; }, \
              selection: () => { getSelection().selectAllChildren(document.body); \
                return getSelection().toString().length; }, \
              hitTest: () => String(document.elementFromPoint(5, 5)), \
              focus: () => { deep.focus(); return document.activeElement === deep; }, \
              computedStyle: () => getComputedStyle(document.getElementById('d')).color, \
              invalidation: () => { document.body.classList.toggle('x'); \
                return getComputedStyle(deep).color; }, \
              structuredClone: () => typeof structuredClone(nested(10000)), \
              jsonStringify: () => JSON.stringify(nested(10000)).length, \
              jsonParse: () => typeof JSON.parse('['.repeat(10000) + ']'.repeat(10000)), \
            }; \
            for (const [name, walk] of Object.entries(walks)) { \
              try { walk(); console.log(name + ' ended'); } \
              catch (e) { console.log(name + ' threw ' + e.name); } }";
        crate::differential::run_script(
            engine,
            &format!("{SLOTTED_DEEP} slotted(2, 300); {walks}"),
        );
        // The engine's own walks: the frame, hit testing, the sequential
        // focus navigation each way, the inspector's tree opened all the
        // way down.
        engine.frame(200, 100);
        let _ = engine.hit_test(5.0, 5.0);
        let _ = engine.focus_next(false);
        let _ = engine.focus_next(true);
        let document = engine.document().clone();
        let open: Vec<String> = document
            .subtree(document.root())
            .map(|node| node.to_bits().to_string())
            .collect();
        engine.inspect(&format!(
            "{{\"id\":1,\"method\":\"dom.tree\",\"params\":{{\"open\":[{}]}}}}",
            open.join(",")
        ));
        let lines = console_of(engine);
        eprintln!("{lines:#?}");
        assert_eq!(lines.len(), 15, "{lines:?}");
        assert!(
            lines
                .iter()
                .all(|line| line.ends_with(" ended") || line.contains(" threw ")),
            "{lines:?}"
        );
    });
}

/// The flat-tree steps of the frame after a script made `hosts` hosts
/// slotted into each other's shadow trees, each shadow tree a chain of
/// `deep` plain blocks with its slot at the bottom (G253).
fn steps_of_a_slotted_restyle(engine: &mut crate::Engine, hosts: usize, deep: usize) -> u64 {
    engine.load_document("<body></body>");
    engine.frame(200, 100);
    crate::differential::run_script(
        engine,
        &format!(
            "let at = document.body; \
             for (let h = 0; h < {hosts}; h++) {{ \
               const host = document.createElement('section'); at.append(host); \
               let inner = host.attachShadow({{mode: 'open'}}); \
               for (let i = 0; i < {deep}; i++) {{ \
                 const d = document.createElement('div'); inner.append(d); inner = d; }} \
               inner.append(document.createElement('slot')); at = host; }} \
             at.append('deep');"
        ),
    );
    dom::take_flat_steps();
    engine.frame(200, 100);
    dom::take_flat_steps()
}

#[test]
fn a_restyle_walks_the_flat_tree_once_however_deep_slots_nest_it() {
    // G253: through shadow trees' slots the flat tree is as deep as what
    // is slotted; a restyle walked to its top for each changed node, and
    // the layout cache for each change, which made a frame quadratic in
    // it (2,000 elements 0.7 s, 4,000 3 s). Counted in steps, not time:
    // chains twice as long take about twice the steps.
    on_engine_stack(|engine| {
        let short = steps_of_a_slotted_restyle(engine, 3, 150);
        let long = steps_of_a_slotted_restyle(engine, 3, 300);
        assert!(short > 0, "the steps are counted");
        assert!(
            long as f64 <= 2.2 * short as f64,
            "{short} steps for chains of 150, {long} for 300"
        );
    });
}
