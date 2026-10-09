//! The engine giving scripts the DOM (M25, docs/design/dom-scripting.md):
//! node objects over the document's nodes, the tree read and written
//! through them, the exceptions, the work charged, and the nodes scripts
//! let go of freed.

use super::script_tests::{scripted, scripted_bytes};
use super::tests::load_all;
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
            // Kept as written, and a shorthand unexpanded (G50).
            (
                "(function () { var x = document.createElement('i'); x.style.color = 'RED'; \
                 x.style.margin = '1px'; x.style.width = 'banana'; \
                 return [x.style.color, x.style.length, x.style[1], x.style.marginTop, \
                 x.style.cssText, x.getAttribute('style')]; })()",
                "ok [\"RED\",2,\"margin\",\"\",\"color: RED; margin: 1px;\",\
                 \"color: RED; margin: 1px;\"]",
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
             console.log(n.userAgent.indexOf('Mozilla/5.0 (compatible; SilverBrowser/') === 0, \
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
    assert_eq!(
        console,
        [
            "true true true true true",
            "http:|127.0.0.1|/dir/page.html|?q=1|#frag",
            "true true",
            "UTF-8 text/html CSS1Compat",
            "Mozilla Netscape 5.0 (compatible) Gecko 20100101 \"\" \"\" en-US en-US,en true false \
             false false 0 0 false true undefined undefined",
            "true true",
            "400 200",
            "true true true 24 24 1 true true true true 0 null [object Navigator] [object Screen] \
             [object Location]",
        ]
    );
    // D12's escape hatch: what the reference browser says, by Chrome's
    // compatibility mode.
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_compat_user_agent(true);
    load_all(&mut tab, server.url("/dir/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert!(
        console[4].starts_with("Mozilla Netscape 5.0 (")
            && console[4].contains(" Gecko 20030107 \"Google Inc.\""),
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
             static auto auto \"1px 2px\" \"\" true align-content align-items undefined \
             rgb(10, 20, 30)",
            "inline rgb(255, 0, 0) rgb(255, 0, 0) \"\" \"\"",
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
