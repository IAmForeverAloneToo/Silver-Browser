//! Documents beside the page's (gap batch H, D67): what scripts make of
//! them through `DOMParser`, `document.implementation`, the `Document`
//! constructor and XMLHttpRequest, each node's owner document, and the
//! page's rendering, which they leave alone.

use std::time::{Duration, Instant};

use net::test_server::Reply;

use crate::testing::*;
use crate::{Engine, differential};

fn test_engine() -> Engine {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.set_scripting(true);
    engine
}

/// What `script` logs, run in a page of `markup`.
fn said(markup: &str, script: &str) -> Vec<String> {
    let mut engine = test_engine();
    engine.load_document(markup);
    engine.take_console();
    differential::run_script(&mut engine, script);
    engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

#[test]
fn a_parsed_document_answers_for_itself_and_not_the_page() {
    let lines = said(
        "<p id=x>page</p>",
        "const d = new DOMParser().parseFromString(\
           '<!DOCTYPE html><title> T  t </title><p id=x>a</p><script>window.ran = 1</script>', \
           'text/html'); \
         console.log([d.title, d.compatMode, d.contentType, d.characterSet, d.readyState, \
           d.URL === document.URL, d.getElementById('x').textContent, \
           d.body.ownerDocument === d, d.defaultView === null, d.location === null, \
           d instanceof HTMLDocument, d.querySelector('p').isConnected, \
           document.getElementById('x').textContent, window.ran === undefined].join()); \
         document.body.appendChild(document.importNode(d.querySelector('script'), true));          document.body.appendChild(d.querySelector('script').cloneNode(true));          document.body.appendChild(d.querySelector('script'));          console.log(window.ran === undefined);          const q = new DOMParser().parseFromString('<p>', 'text/html'); \
         console.log(q.compatMode); \
         d.title = 'new'; \
         console.log(d.title + ' ' + document.title.length);",
    );
    assert_eq!(
        lines,
        [
            "T t,CSS1Compat,text/html,UTF-8,complete,true,a,true,true,true,true,true,page,true",
            "true",
            "BackCompat",
            "new 0",
        ]
    );
}

#[test]
fn an_xml_document_keeps_names_namespaces_and_cdata_and_serializes_them() {
    let lines = said(
        "<p>page</p>",
        "const x = document.implementation.createDocument(null, 'Root', null); \
         const e = x.createElement('Child'); \
         x.documentElement.appendChild(e); \
         e.setAttributeNS('urn:a', 'a:k', 'v'); \
         e.appendChild(x.createCDATASection('c')); \
         const at = e.attributes[0]; \
         console.log([x.documentElement.tagName, e.tagName, e.getAttributeNS('urn:a', 'k'), \
           at.prefix, at.localName, at.namespaceURI, e.firstChild.nodeType, \
           e.firstChild.nodeName, x instanceof XMLDocument, x.contentType].join()); \
         console.log(new XMLSerializer().serializeToString(x)); \
         const n = document.createElementNS('urn:n', 'n:item'); \
         console.log(n.prefix + ' ' + n.localName + ' ' + document.createElement('a:b').localName); \
         const p = new DOMParser().parseFromString('<r xmlns:p=\"urn:p\"><p:i p:a=\"1\"/></r>', \
           'text/xml'); \
         const i = p.documentElement.firstChild; \
         console.log([i.prefix, i.localName, i.namespaceURI, i.getAttributeNS('urn:p', 'a'), \
           p.querySelector('r').localName].join()); \
         try { x.createCDATASection('a]]>b'); } catch (error) { console.log(error.name); } \
         try { document.createCDATASection('a'); } catch (error) { console.log(error.name); }",
    );
    assert_eq!(
        lines,
        [
            "Root,Child,v,a,k,urn:a,4,#cdata-section,true,application/xml",
            "<Root><Child xmlns:ns1=\"urn:a\" ns1:k=\"v\"><![CDATA[c]]></Child></Root>",
            "n item a:b",
            "p,i,urn:p,1,r",
            "InvalidCharacterError",
            "NotSupportedError",
        ]
    );
}

#[test]
fn a_node_belongs_to_its_trees_document_and_moves_with_adoption() {
    let lines = said(
        "<p>page</p>",
        "const d = document.implementation.createHTMLDocument('t'); \
         const p = document.querySelector('p'); \
         d.body.appendChild(p); \
         console.log([p.ownerDocument === d, document.querySelector('p') === null, \
           d.title, d.doctype.name, d.createElement('DIV').tagName].join()); \
         const copy = document.importNode(d.body, true); \
         console.log([copy.ownerDocument === document, copy.parentNode === null, \
           copy.firstChild.textContent].join()); \
         document.adoptNode(p); \
         console.log([p.ownerDocument === document, p.parentNode === null, \
           d.body.firstChild === null].join()); \
         const made = d.createElement('span'); \
         console.log(made.ownerDocument === d); \
         document.body.appendChild(made); \
         console.log(made.ownerDocument === document); \
         try { document.adoptNode(d); } catch (error) { console.log(error.name); } \
         const plain = new Document(); \
         console.log([plain instanceof XMLDocument, plain.contentType, \
           plain.createElement('a').tagName, plain.documentElement].join());",
    );
    assert_eq!(
        lines,
        [
            "true,true,t,html,DIV",
            "true,true,page",
            "true,true,true",
            "true",
            "true",
            "NotSupportedError",
            "false,application/xml,a,",
        ]
    );
}

#[test]
fn a_templates_contents_are_owned_by_an_inert_document() {
    let lines = said(
        "<template id=t></template>",
        "const t = document.getElementById('t'); \
         t.innerHTML = '<b>x</b>'; \
         const inert = t.content.ownerDocument; \
         console.log([inert !== document, inert === t.content.firstChild.ownerDocument, \
           inert.defaultView === null, inert.contentType, \
           document.createElement('template').content.ownerDocument === inert].join()); \
         const b = t.content.firstChild; \
         b.remove(); \
         console.log(b.ownerDocument === inert); \
         const d = new DOMParser().parseFromString('<template><i></i></template>', 'text/html'); \
         const other = d.querySelector('template').content.ownerDocument; \
         console.log([other !== inert, other !== d].join());",
    );
    assert_eq!(
        lines,
        ["true,true,true,text/html,true", "true", "true,true"]
    );
}

#[test]
fn another_documents_style_sheets_are_its_own() {
    let lines = said(
        "<style>p { color: red }</style>",
        "const d = new DOMParser().parseFromString(\
           '<style>a { color: green } b { color: blue }</style><style></style>', 'text/html'); \
         console.log([d.styleSheets.length, d.styleSheets[0].cssRules.length, \
           d.styleSheets[0].cssRules[1].selectorText, \
           d.styleSheets[0].ownerNode === d.querySelector('style'), \
           document.styleSheets.length].join());",
    );
    assert_eq!(lines, ["2,2,b,true,1"]);
}

#[test]
fn a_document_beside_the_page_disturbs_neither_its_rendering_nor_its_tree() {
    let mut engine = test_engine();
    engine.load_document("<style>p { color: green }</style><p>a</p>");
    engine.frame(200, 100);
    differential::run_script(
        &mut engine,
        "window.kept = new DOMParser().parseFromString(\
           '<style>p { color: red; display: none }</style><p>x</p>', 'text/html'); \
         kept.body.appendChild(kept.createElement('div')); \
         kept.querySelector('p').setAttribute('style', 'display: none'); \
         window.xml = document.implementation.createDocument('urn:x', 'r', null);",
    );
    assert_eq!(engine.document().other_documents().len(), 2);
    engine.frame(200, 100);
    let mut fresh = test_engine();
    if let Err(difference) = differential::compare_with_fresh(&mut engine, &mut fresh, 200, 100) {
        panic!("{difference}");
    }
    engine.document().validate().unwrap();
}

#[test]
fn a_document_no_script_holds_is_freed() {
    let mut engine = test_engine();
    engine.load_document("<p>a</p>");
    differential::run_script(
        &mut engine,
        "window.held = new DOMParser().parseFromString('<p>held</p>', 'text/html').body; \
         new DOMParser().parseFromString('<p>dropped</p>', 'text/html');",
    );
    // The objects let go of are released by the next run's end.
    differential::run_script(&mut engine, "window.held = window.held;");
    differential::run_script(&mut engine, "0");
    let document = engine.document();
    let others = document.other_documents();
    assert_eq!(others.len(), 1, "the one a script holds a node of stays");
    assert!(
        document
            .subtree(others[0])
            .any(|n| document.text(n) == Some("held"))
    );
    document.validate().unwrap();
}

#[test]
fn ranges_and_iterators_a_run_drops_make_room_for_more_in_it() {
    // dom/ranges' tests, which run once documents are made, make many
    // more ranges in one run than the host keeps at once
    // (dom::MAX_RANGES): those dropped are let go of when more are
    // wanted, before the registry hears of them after the run.
    let lines = said(
        "<p>a</p>",
        "const d = document.implementation.createHTMLDocument(''); \
         let kept = []; \
         for (let i = 0; i < 10000; i++) { \
           const r = (i % 2 ? d : document).createRange(); \
           if (i % 1000 === 0) kept.push(r); \
           document.createNodeIterator(document.body); \
         } \
         console.log([kept.length, d.createRange().startContainer === d, \
           kept[9].startContainer === document, \
           Node.DOCUMENT_POSITION_FOLLOWING, Node.prototype.DOCUMENT_POSITION_CONTAINS].join());",
    );
    assert_eq!(lines, ["10,true,true,4,8"]);
}

#[test]
fn a_child_replaced_by_itself_is_taken_out_and_put_back() {
    // DOM's "replace": the ranges in it move to its parent, as dom/ranges'
    // tests found once they ran.
    let lines = said(
        "<div><p>a</p><p>b</p></div>",
        "const div = document.querySelector('div'); \
         const p = div.firstChild; \
         const r = document.createRange(); \
         r.setStart(p, 0); \
         div.replaceChild(p, p); \
         console.log([r.startContainer === div, r.startOffset, div.firstChild === p].join());",
    );
    assert_eq!(lines, ["true,0,true"]);
}

/// The page of the test below: an XML parse that fails, an XML
/// document's contextual fragment, and two responses made documents.
const RESPONSES: &str = r#"<script>
const bad = new DOMParser().parseFromString('<r><a>1</a><b>', 'application/xml');
const report = bad.getElementsByTagName('parsererror');
console.log([report.length, report[0].namespaceURI, bad.contentType,
  bad.querySelector('a').textContent].join());
const xml = document.implementation.createDocument(null, 'r', null);
const range = xml.createRange();
range.setStart(xml.documentElement, 0);
const made = range.createContextualFragment(
  '<body xmlns="http://www.w3.org/1999/xhtml"><p>x</p></body>');
console.log(made.firstChild.localName + ' ' + (made.ownerDocument === xml));
const x = new XMLHttpRequest();
x.open('GET', '/r.xml');
x.onload = () => {
  const doc = x.responseXML;
  console.log([doc.documentElement.firstChild.textContent, doc.contentType,
    typeof x.response === 'string', doc.URL.endsWith('/r.xml')].join());
  const h = new XMLHttpRequest();
  h.open('GET', '/h.html');
  h.responseType = 'document';
  h.onload = () => console.log(h.response.title + ' ' + h.response.compatMode + ' ' +
    (x.responseXML === doc));
  h.send();
};
x.send();
</script>"#;

#[test]
fn an_xml_parse_reports_its_error_and_a_response_makes_a_document() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(RESPONSES)),
        "/r.xml" => Some(Reply::ok("text/xml", b"<r><i>v</i></r>")),
        "/h.html" => Some(html("<title>t</title>")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lines: Vec<String> = Vec::new();
    while lines.len() < 4 {
        assert!(Instant::now() < deadline, "logged only {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
    }
    assert_eq!(
        lines,
        [
            "1,http://www.w3.org/1999/xhtml,application/xml,1",
            "p true",
            "v,text/xml,true,true",
            "t BackCompat true",
        ]
    );
}
