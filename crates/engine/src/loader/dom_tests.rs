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
                "tags(document.getElementsByClassName('  y   x '))",
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
