//! The bindings (M25) against the tests' small tree: their shapes, node
//! objects' identity and release, the exceptions, the work charged.

use crate::tests::{Recorder, realm, realm_with, script};
use crate::{Completion, ErrorKind, Limits, WORK_PER_STEP};

/// What `source` ends with, run with a host whose tree has an `html`
/// element with a `body` holding a `p` and a text node.
fn run(source: &str) -> (Completion, Recorder) {
    let mut realm = realm();
    let mut host = Recorder::default();
    let html = host.tree.element(0, "html");
    let body = host.tree.element(html, "body");
    host.tree.element(body, "p");
    host.tree.text(body, "text");
    let done = realm.run(&script(source), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    (done, host)
}

fn string(source: &str) -> String {
    match run(source).0 {
        Completion::String(text) => text,
        other => panic!("{source} ended with {other:?}"),
    }
}

#[test]
fn the_interfaces_have_web_idls_shapes() {
    for (source, expected) in [
        (
            "Object.prototype.toString.call(document)",
            "[object HTMLDocument]",
        ),
        (
            "Object.prototype.toString.call(document.documentElement)",
            "[object HTMLHtmlElement]",
        ),
        (
            "String(document instanceof Document && document instanceof Node)",
            "true",
        ),
        (
            "String(Object.getPrototypeOf(HTMLElement.prototype) === Element.prototype)",
            "true",
        ),
        (
            "String(Object.getPrototypeOf(Node.prototype) === EventTarget.prototype)",
            "true",
        ),
        (
            "String(Object.getPrototypeOf(Text.prototype) === CharacterData.prototype)",
            "true",
        ),
        (
            "String(window instanceof Window && window instanceof EventTarget)",
            "true",
        ),
        (
            "String(Object.getOwnPropertyDescriptor(window, 'Node').enumerable)",
            "false",
        ),
        (
            "String(Object.getOwnPropertyDescriptor(Node.prototype, 'firstChild').enumerable)",
            "true",
        ),
        ("String(Node.ELEMENT_NODE + document.TEXT_NODE)", "4"),
        (
            "Node.name + ' ' + HTMLDivElement.name",
            "Node HTMLDivElement",
        ),
        (
            "Object.prototype.toString.call(document.createElement('DIV'))",
            "[object HTMLDivElement]",
        ),
        (
            "Object.prototype.toString.call(document.createElement('h3'))",
            "[object HTMLHeadingElement]",
        ),
        (
            "Object.prototype.toString.call(document.createElement('video'))",
            "[object HTMLVideoElement]",
        ),
        (
            "String(document.createElement('audio') instanceof HTMLMediaElement)",
            "true",
        ),
        (
            "Object.prototype.toString.call(document.createElement('section'))",
            "[object HTMLElement]",
        ),
        (
            "Object.prototype.toString.call(document.createElement('my-thing'))",
            "[object HTMLElement]",
        ),
        (
            "Object.prototype.toString.call(document.createElement('blink'))",
            "[object HTMLUnknownElement]",
        ),
        ("typeof native + typeof bootstrap", "undefinedundefined"),
    ] {
        assert_eq!(string(source), expected, "{source}");
    }
}

#[test]
fn constructors_and_methods_refuse_what_is_not_theirs() {
    for (source, expected) in [
        (
            "try { new Node() } catch (e) { e.name + ': ' + e.message }",
            "TypeError: Failed to construct 'Node': Illegal constructor",
        ),
        (
            "try { new HTMLDivElement() } catch (e) { e.message }",
            "Failed to construct 'HTMLDivElement': Illegal constructor",
        ),
        (
            "try { new Window() } catch (e) { e.message }",
            "Failed to construct 'Window': Illegal constructor",
        ),
        (
            "try { Object.getOwnPropertyDescriptor(Node.prototype, 'firstChild').get.call({}) } \
             catch (e) { e.name + ': ' + e.message }",
            "TypeError: Illegal invocation",
        ),
        (
            "try { Node.prototype.appendChild.call(document.documentElement, {}) } \
             catch (e) { e.name + ': ' + e.message }",
            "TypeError: Failed to execute 'appendChild' on 'Node': parameter 1 is not of \
             type 'Node'.",
        ),
        // The DOM lets a script make an EventTarget and a Text.
        ("String(new EventTarget() instanceof EventTarget)", "true"),
        (
            "var t = new Text('hi'); t.data + t.nodeName + t.length",
            "hi#text2",
        ),
        ("new Text().data", ""),
    ] {
        assert_eq!(string(source), expected, "{source}");
    }
}

#[test]
fn the_tree_reads_through_the_host() {
    let source = "var html = document.documentElement, body = html.firstChild; \
                  var p = body.firstChild, t = body.lastChild; \
                  [html.tagName, html.localName, body.nodeName, p.nodeType, t.nodeType, \
                   t.data, t.nodeName, document.nodeName, document.nodeType, \
                   p.nextSibling === t, t.previousSibling === p, p.previousSibling, \
                   t.nextSibling, body.parentNode === html, html.parentNode === document, \
                   document.parentNode, document.ownerDocument, p.ownerDocument === document, \
                   body.textContent, document.textContent].join(',')";
    assert_eq!(
        string(source),
        "HTML,html,BODY,1,3,text,#text,#document,9,true,true,,,true,true,,,true,text,"
    );
}

#[test]
fn one_node_is_one_object_while_it_lives() {
    let source = "var a = document.documentElement.firstChild; \
                  var b = document.documentElement.lastChild; \
                  String(a === b && a.firstChild === b.firstChild \
                         && a.firstChild.parentNode === a && document.firstChild.parentNode === document)";
    assert_eq!(string(source), "true");
    // A page that replaces the built-ins changes nothing the bindings do.
    let source = "Map.prototype.get = function () { return undefined }; \
                  WeakRef.prototype.deref = function () { return {} }; \
                  Function.prototype.call = function () { throw new Error('no') }; \
                  String(document.documentElement === document.documentElement)";
    assert_eq!(string(source), "true");
}

#[test]
fn an_object_let_go_of_releases_its_node() {
    let (_, host) = run("for (var i = 0; i < 10; i++) document.createElement('p'); \
                         var kept = document.createElement('b');");
    let tree = &host.tree;
    let made: Vec<u64> = (0..tree.len() as u64).filter(|&i| i > 4).collect();
    assert_eq!(made.len(), 11);
    let ps: Vec<u64> = made
        .iter()
        .copied()
        .filter(|&i| tree.tag(i as usize) == "p")
        .collect();
    assert_eq!(ps.len(), 10);
    for p in &ps {
        assert!(
            tree.released.contains(p),
            "p {p} released: {:?}",
            tree.released
        );
    }
    let b = made
        .iter()
        .copied()
        .find(|&i| tree.tag(i as usize) == "b")
        .unwrap();
    assert!(tree.exposed.contains(&b) && !tree.released.contains(&b));
    // The document's object lives as long as the realm.
    assert!(tree.exposed.contains(&0));
}

#[test]
fn a_node_given_again_after_its_object_died_gets_a_new_one() {
    // The first object is collected before its finalizer runs; the
    // second is made meanwhile and must not be released by the first's.
    let (done, host) = run("var body = document.documentElement.firstChild; \
                            var first = body.firstChild; first.x = 1; first = null; \
                            var second = body.firstChild; String(second.x)");
    assert_eq!(done, Completion::String("undefined".to_owned()));
    let p = host.tree.children(2)[0] as u64;
    assert!(host.tree.exposed.contains(&p), "{:?}", host.tree.released);
}

#[test]
fn a_failed_call_throws_the_doms_exception_in_the_reference_browsers_words() {
    for (source, expected) in [
        (
            "try { document.createElement('a b') } catch (e) { \
             [e instanceof DOMException, e.name, e.message].join('|') }",
            "true|InvalidCharacterError|Failed to execute 'createElement' on 'Document': bad name",
        ),
        (
            "var html = document.documentElement; \
             try { html.firstChild.appendChild(html) } catch (e) { e.name + '|' + e.message }",
            "HierarchyRequestError|Failed to execute 'appendChild' on 'Node': no",
        ),
        (
            "try { document.documentElement.removeChild(document.createElement('i')) } \
             catch (e) { e.name + '|' + e.code }",
            "NotFoundError|8",
        ),
        // A page's DOMException is not the one thrown.
        (
            "DOMException = function () {}; try { document.createElement('') } \
             catch (e) { Object.prototype.toString.call(e) + e.name }",
            "[object DOMException]InvalidCharacterError",
        ),
    ] {
        assert_eq!(string(source), expected, "{source}");
    }
}

#[test]
fn nodes_move_through_the_host() {
    let (done, host) = run("var body = document.documentElement.firstChild; \
                            var b = document.createElement('B'); \
                            body.appendChild(b); \
                            var t = b.appendChild(document.createTextNode('x')); \
                            body.removeChild(body.firstChild); \
                            [body.firstChild.nodeName, b.previousSibling.nodeName, \
                             b.nextSibling, body.textContent, t === b.firstChild].join()");
    assert_eq!(
        done,
        Completion::String("#text,#text,,textx,true".to_owned())
    );
    let body = host.tree.children(1)[0];
    assert_eq!(host.tree.children(body).len(), 2);
}

#[test]
fn a_lone_surrogate_crosses_as_a_replacement_character() {
    assert_eq!(
        string("document.createTextNode('a\\ud800b').data"),
        "a\u{fffd}b"
    );
    assert_eq!(
        string("new Text({ toString() { return '\\udc00' } }).data"),
        "\u{fffd}"
    );
}

#[test]
fn the_hosts_work_is_charged_to_the_budget() {
    let limits = Limits {
        steps: 100,
        ..Limits::default()
    };
    let source = "for (var i = 0; i < 50; i++) document.documentElement.firstChild; 'done'";
    // Free calls: the loop is far within its budget.
    let mut realm = realm_with(limits);
    let mut host = Recorder::default();
    host.tree.element(0, "html");
    assert_eq!(
        realm.run(&script(source), &mut host),
        Ok(Completion::String("done".to_owned()))
    );
    // Ten steps a call: the budget runs out within a dozen calls.
    host.tree.work_per_call = 10 * WORK_PER_STEP;
    let error = realm.run(&script(source), &mut host).expect_err("stopped");
    assert_eq!(error.kind, ErrorKind::Budget);
    // The next run has a budget of its own.
    host.tree.work_per_call = 0;
    assert!(realm.run(&script(source), &mut host).is_ok());
}
