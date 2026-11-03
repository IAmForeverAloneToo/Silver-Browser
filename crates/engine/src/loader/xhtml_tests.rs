//! XML documents (gap batch D's parser gaps): XHTML and the other XML
//! types read by the XML parser into XML documents, their XHTML elements
//! styled, laid out and scripted as HTML's, their scripts run as the
//! parser meets them.

use net::test_server::Reply;

use super::tests::load_all;
use crate::testing::*;

/// `body` served as `kind` and loaded with scripts on; the console's
/// lines.
fn loaded(kind: &'static str, body: &'static str) -> (Tab, Vec<String>) {
    let server = serve(move |path| (path == "/page").then(|| Reply::ok(kind, body.as_bytes())));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page"));
    let console = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    (tab, console)
}

const XHTML: &str = "application/xhtml+xml";

#[test]
fn xhtml_is_an_xml_document_whose_scripts_run_as_the_parser_meets_them() {
    let (tab, console) = loaded(
        XHTML,
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"x.dtd\">\
         <html xmlns='http://www.w3.org/1999/xhtml'><head><title>t</title></head><body>\
         <p id='a'>a&nbsp;b</p>\
         <script><![CDATA[\
           console.log(document.contentType, document.getElementById('b'), \
             document.body.childNodes.length);\
           try { document.write('x'); } catch (e) { console.log(e.name); }\
           try { document.open(); } catch (e) { console.log(e.name); }\
           const made = document.createElement('Foo');\
           console.log(made.tagName, made.localName, made.namespaceURI);\
           console.log(document.getElementsByTagName('P').length, \
             document.getElementsByTagName('p').length, document.body.tagName);\
           const div = document.createElement('div');\
           div.setAttribute('DataX', '1');\
           console.log(div.getAttribute('datax'), div.getAttribute('DataX'));\
         ]]></script><p id='b'>b</p></body></html>",
    );
    assert_eq!(
        console,
        [
            "application/xhtml+xml null 2",
            "InvalidStateError",
            "InvalidStateError",
            "Foo Foo http://www.w3.org/1999/xhtml",
            "0 1 body",
            "null 1",
        ]
    );
    let document = tab.engine.document();
    assert!(document.is_xml_document());
    assert_eq!(
        document.text_content(by_id(&tab.engine, "a")),
        "a\u{a0}b",
        "the XHTML DTD's entities"
    );
}

#[test]
fn inner_html_is_xmls_in_an_xml_document() {
    let (_, console) = loaded(
        XHTML,
        "<html xmlns='http://www.w3.org/1999/xhtml'><body><div id='d'><br/><p>a&amp;b</p></div>\
         <script><![CDATA[\
           const d = document.getElementById('d');\
           console.log(d.innerHTML);\
           d.innerHTML = '<B>x</B><svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>';\
           console.log(d.firstChild.localName, d.firstChild.namespaceURI, d.lastChild.localName);\
           try { d.innerHTML = '<p>'; } catch (e) { console.log(e.name); }\
           console.log(d.childNodes.length);\
         ]]></script></body></html>",
    );
    // The children are serialized in no namespace's context, each
    // declaring its own (DOM Parsing section 3.2, D67).
    assert_eq!(
        console,
        [
            "<br xmlns=\"http://www.w3.org/1999/xhtml\" />\
             <p xmlns=\"http://www.w3.org/1999/xhtml\">a&amp;b</p>",
            "B http://www.w3.org/1999/xhtml svg",
            "SyntaxError",
            "2",
        ]
    );
}

#[test]
fn xhtml_is_styled_and_laid_out_as_html() {
    let (mut tab, console) = loaded(
        XHTML,
        "<html xmlns='http://www.w3.org/1999/xhtml'><head><style>p { color: rgb(0, 128, 0) }\
         </style></head><body><p id='p'>x</p><div id='d'><b>y</b></div><script>\
         const p = document.getElementById('p');\
         const style = getComputedStyle(p);\
         console.log(style.display, style.marginTop, getComputedStyle(document.body).marginLeft);\
         console.log(getComputedStyle(document.querySelector('b')).fontWeight);\
         </script></body></html>",
    );
    assert_eq!(console, ["block 16px 8px", "700"]);
    let (r, g, b) = color(&mut tab.engine, "p");
    assert!(r == 0.0 && (g - 128.0 / 255.0).abs() < 0.01 && b == 0.0);
}

#[test]
fn an_xml_document_with_xhtml_shows_as_a_document_and_one_without_as_text() {
    let (tab, console) = loaded(
        "text/xml",
        "<root><p xmlns='http://www.w3.org/1999/xhtml' id='x'>a</p>\
         <script xmlns='http://www.w3.org/1999/xhtml'>\
         console.log(document.contentType, document.createElement('Q').namespaceURI)\
         </script></root>",
    );
    assert_eq!(console, ["text/xml null"]);
    assert!(tab.engine.document().is_xml_document());
    let (tab, _) = loaded("application/xml", "<root><a>text</a></root>");
    assert!(
        !tab.engine.document().is_xml_document(),
        "shown as its source"
    );
    assert!(text(&tab.engine).contains("<root><a>text</a></root>"));
}

#[test]
fn an_xhtml_error_is_shown_above_what_came_before_it() {
    let (tab, console) = loaded(
        XHTML,
        "<html xmlns='http://www.w3.org/1999/xhtml'><body><p>before</p>\
         <script>console.log('ran')</script><p>broken</b></body></html>",
    );
    assert_eq!(console, ["ran"]);
    let shown = text(&tab.engine);
    assert!(
        shown.starts_with("This page contains the following errors:"),
        "{shown}"
    );
    assert!(shown.contains("Opening and ending tag mismatch: p and b"));
    assert!(shown.contains("before"));
}

#[test]
fn the_xml_parser_constructs_a_defined_custom_element_as_it_meets_it() {
    // Constructed with the parse stopped at its start tag, its writes
    // throwing, before its attributes and its insertion; the reactions
    // its attributes queue before it is inserted, its connection's after;
    // a prefixed one taking its prefix; and one with
    // `customelementregistry` made with no registry, so not constructed.
    let (_, console) = loaded(
        XHTML,
        "<html xmlns='http://www.w3.org/1999/xhtml' xmlns:h='http://www.w3.org/1999/xhtml'>\
         <body><script><![CDATA[\
           const log = [];\
           class X extends HTMLElement {\
             static observedAttributes = ['a'];\
             constructor() {\
               super();\
               let threw = 'none';\
               try { document.open(); } catch (e) { threw = e.name; }\
               log.push('ctor ' + this.attributes.length + ' ' + (this.parentNode === null) + ' ' +\
                 threw + ' ' + document.querySelectorAll('x-x').length);\
             }\
             attributeChangedCallback(n, o, v) { log.push('attr ' + v + ' ' + this.isConnected); }\
             connectedCallback() { log.push('connected ' + this.prefix); }\
           }\
           customElements.define('x-x', X);\
         ]]></script><x-x a='1'/><h:x-x a='2'/><x-x customelementregistry=''/>\
         <script><![CDATA[\
           const all = document.querySelectorAll('x-x');\
           console.log(log.join(', '));\
           console.log(all.length, all[2] instanceof X, all[2].customElementRegistry);\
         ]]></script></body></html>",
    );
    assert_eq!(
        console,
        [
            "ctor 0 true InvalidStateError 0, attr 1 false, connected null, \
             ctor 0 true InvalidStateError 1, attr 2 false, connected h",
            "3 false null",
        ]
    );
}
