//! The Sanitizer API as the reference browser ships it (gap batch H,
//! HTML section 8.5.3): `Sanitizer` and its configuration, the built-in
//! safe default and the safe baseline, `Element.setHTML` and
//! `setHTMLUnsafe`, and `Document.parseHTML` and `parseHTMLUnsafe`, each
//! answer what the reference browser answers.

use crate::{Engine, differential};

fn test_engine() -> Engine {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.set_scripting(true);
    engine.load_document("<div id=d></div>");
    engine
}

/// What each of `cases`, an expression, gives as a string in a page, or
/// the error it throws, its name and message.
fn answers(cases: &[&str]) -> Vec<String> {
    let mut engine = test_engine();
    engine.take_console();
    let mut script = String::from(
        "const J = (x) => JSON.stringify(x);
         const d = document.getElementById('d');
         const h = (html, opts) => { d.setHTML(html, opts); return d.innerHTML; };
         const u = (html, opts) => { d.setHTMLUnsafe(html, opts); return d.innerHTML; };",
    );
    for case in cases {
        script.push_str(&format!(
            "try {{ console.log(String({case})); }} \
             catch (e) {{ console.log(e.name + ': ' + e.message); }}\n"
        ));
    }
    differential::run_script(&mut engine, &script);
    engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

#[test]
fn the_default_configuration_and_the_safe_baseline_are_the_reference_browsers() {
    // Each JSON as the reference browser gives it, by its length and a
    // hash of its code units.
    let said = answers(&[
        "(() => { const s = J(new Sanitizer().get()); let h = 0;            for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;            return s.length + ' ' + h; })()",
        "(() => { const z = new Sanitizer({}); z.removeUnsafe(); const s = J(z.get());            let h = 0; for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;            return s.length + ' ' + h; })()",
        "J(new Sanitizer().get()).length",
        "J(new Sanitizer('default').get()) === J(new Sanitizer().get())",
        "(() => { const s = new Sanitizer({}); s.removeUnsafe(); return J(s.get()).length; })()",
        "Object.keys(new Sanitizer().get()).join()",
        "J(new Sanitizer().get().elements.find(e => e.name === 'mo'))",
        "J(new Sanitizer().get().elements.slice(27, 29))",
        "(() => { const s = new Sanitizer({}); s.removeUnsafe(); \
           return J(s.get().removeElements.map(e => e.name)); })()",
        "(() => { const s = new Sanitizer({}); s.removeUnsafe(); \
           const a = s.get().removeAttributes; return a.length + a[0].name + a[138].name; })()",
        "Object.getOwnPropertyNames(Sanitizer.prototype).join()",
    ]);
    assert_eq!(
        said,
        [
            "15766 2981565958",
            "6238 363104216",
            "15766",
            "true",
            "6238",
            "attributes,comments,dataAttributes,elements,processingInstructions",
            "{\"name\":\"mo\",\"namespace\":\"http://www.w3.org/1998/Math/MathML\",\"attributes\":\
             [{\"name\":\"fence\",\"namespace\":null},{\"name\":\"form\",\"namespace\":null},\
             {\"name\":\"largeop\",\"namespace\":null},{\"name\":\"lspace\",\"namespace\":null},\
             {\"name\":\"maxsize\",\"namespace\":null},{\"name\":\"minsize\",\"namespace\":null},\
             {\"name\":\"movablelimits\",\"namespace\":null},{\"name\":\"rspace\",\"namespace\":\
             null},{\"name\":\"separator\",\"namespace\":null},{\"name\":\"stretchy\",\
             \"namespace\":null},{\"name\":\"symmetric\",\"namespace\":null}]}",
            "[{\"name\":\"a\",\"namespace\":\"http://www.w3.org/1999/xhtml\",\"attributes\":\
             [{\"name\":\"href\",\"namespace\":null},{\"name\":\"hreflang\",\"namespace\":null},\
             {\"name\":\"type\",\"namespace\":null}]},{\"name\":\"abbr\",\"namespace\":\
             \"http://www.w3.org/1999/xhtml\",\"attributes\":[]}]",
            "[\"base\",\"embed\",\"frame\",\"iframe\",\"object\",\"script\",\"script\",\"use\"]",
            "139onabortonwheel",
            "constructor,get,allowElement,removeElement,replaceElementWithChildren,\
             allowAttribute,removeAttribute,setComments,setDataAttributes,removeUnsafe,\
             allowProcessingInstruction,removeProcessingInstruction",
        ]
    );
}

#[test]
fn a_configuration_is_made_canonical_and_an_invalid_one_refused() {
    let invalid = "TypeError: Failed to construct 'Sanitizer': Invalid Sanitizer configuration.";
    let said = answers(&[
        "new Sanitizer({elements: ['a'], removeElements: ['b']})",
        "new Sanitizer({attributes: ['a'], removeAttributes: ['b']})",
        "new Sanitizer({elements: ['a', 'a']})",
        "new Sanitizer({elements: [{name: 'a', attributes: ['x'], removeAttributes: ['y']}]})",
        "new Sanitizer({removeAttributes: ['x'], dataAttributes: true})",
        "new Sanitizer({elements: ['a'], replaceWithChildrenElements: ['a']})",
        "new Sanitizer({replaceWithChildrenElements: ['html']})",
        "new Sanitizer({elements: [{name: 'a', attributes: ['id']}], attributes: ['id']})",
        "new Sanitizer({elements: [{name: 'a', removeAttributes: ['x']}], attributes: ['id']})",
        "new Sanitizer({elements: [{name: 'a', attributes: ['data-x']}], attributes: [], \
           dataAttributes: true})",
        "new Sanitizer({processingInstructions: ['x'], removeProcessingInstructions: ['y']})",
        "J(new Sanitizer({elements: ['z', 'a', 'B', {name: 'c', namespace: \
           'http://www.w3.org/2000/svg'}, {name: 'c', namespace: ''}], attributes: ['z', 'a', \
           {name: 'm', namespace: 'urn:x'}]}).get())",
        "J(new Sanitizer({removeElements: ['script'], replaceWithChildrenElements: ['b'], \
           processingInstructions: ['x', {target: 'y'}]}).get())",
        "J(new Sanitizer(null).get())",
        "new Sanitizer('bogus')",
        "new Sanitizer(5)",
        "new Sanitizer({elements: 5})",
        "new Sanitizer({elements: [{}]})",
    ]);
    assert_eq!(said[..11], vec![invalid; 11]);
    assert_eq!(
        said[11..],
        [
            "{\"attributes\":[{\"name\":\"a\",\"namespace\":null},{\"name\":\"z\",\"namespace\":\
             null},{\"name\":\"m\",\"namespace\":\"urn:x\"}],\"comments\":true,\
             \"dataAttributes\":true,\"elements\":[{\"name\":\"c\",\"namespace\":null,\
             \"removeAttributes\":[]},{\"name\":\"B\",\"namespace\":\
             \"http://www.w3.org/1999/xhtml\",\"removeAttributes\":[]},{\"name\":\"a\",\
             \"namespace\":\"http://www.w3.org/1999/xhtml\",\"removeAttributes\":[]},\
             {\"name\":\"z\",\"namespace\":\"http://www.w3.org/1999/xhtml\",\
             \"removeAttributes\":[]},{\"name\":\"c\",\"namespace\":\
             \"http://www.w3.org/2000/svg\",\"removeAttributes\":[]}],\
             \"removeProcessingInstructions\":[]}",
            "{\"comments\":true,\"processingInstructions\":[{\"target\":\"x\"},{\"target\":\
             \"y\"}],\"removeAttributes\":[],\"removeElements\":[{\"name\":\"script\",\
             \"namespace\":\"http://www.w3.org/1999/xhtml\"}],\"replaceWithChildrenElements\":\
             [{\"name\":\"b\",\"namespace\":\"http://www.w3.org/1999/xhtml\"}]}",
            "{\"comments\":true,\"removeAttributes\":[],\"removeElements\":[],\
             \"removeProcessingInstructions\":[]}",
            "TypeError: Failed to construct 'Sanitizer': The provided value 'bogus' is not a \
             valid enum value of type SanitizerPresets.",
            "TypeError: Failed to construct 'Sanitizer': The provided value '5' is not a valid \
             enum value of type SanitizerPresets.",
            "TypeError: Failed to construct 'Sanitizer': Failed to read the 'elements' property \
             from 'SanitizerConfig': The provided value cannot be converted to a sequence.",
            "TypeError: Failed to construct 'Sanitizer': Failed to read the 'elements' property \
             from 'SanitizerConfig': Failed to read the 'name' property from \
             'SanitizerElementNamespace': Required member is undefined.",
        ]
    );
}

#[test]
fn the_methods_change_the_configuration_and_say_whether_they_did() {
    let said = answers(&[
        "(() => { const s = new Sanitizer({}); return s.allowElement('div') + ' ' + \
           J(s.get().removeElements); })()",
        "(() => { const s = new Sanitizer({elements: []}); return s.allowElement('div') + ' ' + \
           s.allowElement('div'); })()",
        "(() => { const s = new Sanitizer({elements: ['div', 'p']}); \
           return s.removeElement('div') + ' ' + s.removeElement('x'); })()",
        "(() => { const s = new Sanitizer({}); return s.removeElement('div') + ' ' + \
           s.removeElement('div'); })()",
        "(() => { const s = new Sanitizer({elements: ['div']}); \
           return s.replaceElementWithChildren('div') + ' ' + J(s.get().elements) + ' ' + \
           s.replaceElementWithChildren('html'); })()",
        "(() => { const s = new Sanitizer({replaceWithChildrenElements: ['b']}); \
           return s.removeElement('b') + ' ' + J(s.get().replaceWithChildrenElements); })()",
        "(() => { const s = new Sanitizer({removeAttributes: ['x']}); \
           return s.allowAttribute('x') + ' ' + s.allowAttribute('x'); })()",
        "(() => { const s = new Sanitizer({elements: [{name: 'a', removeAttributes: []}, \
           {name: 'b', attributes: ['x']}], attributes: []}); return s.allowAttribute('x') + \
           ' ' + J(s.get().elements[1]); })()",
        "(() => { const s = new Sanitizer({elements: [{name: 'a', removeAttributes: ['x']}], \
           attributes: ['x']}); return s.removeAttribute('x') + ' ' + J(s.get()); })()",
        "(() => { const s = new Sanitizer(); return s.allowElement('script') + ' ' + \
           s.allowElement('div') + ' ' + s.removeAttribute('title') + ' ' + \
           s.removeAttribute('title'); })()",
        "(() => { const s = new Sanitizer({}); return s.allowElement({name: 'a', \
           attributes: ['x']}); })()",
        "(() => { const s = new Sanitizer({}); return s.setComments(false) + ' ' + \
           s.setDataAttributes(true) + ' ' + s.removeUnsafe() + ' ' + J(s.get()).length; })()",
        "(() => { const s = new Sanitizer({elements: ['script', {name: 'a', attributes: \
           ['onclick', 'x']}], attributes: ['onfocus', 'id']}); s.removeUnsafe(); \
           return J(s.get()); })()",
        "(() => { const s = new Sanitizer({}); return s.allowProcessingInstruction('x') + ' ' + \
           s.removeProcessingInstruction('x') + ' ' + s.removeProcessingInstruction('x'); })()",
        "new Sanitizer().allowElement()",
        "Sanitizer.prototype.get.call({})",
    ]);
    assert_eq!(
        said,
        [
            "false []",
            "true false",
            "true false",
            "true false",
            "true [] false",
            "true []",
            "true false",
            "true {\"name\":\"b\",\"namespace\":\"http://www.w3.org/1999/xhtml\",\
             \"attributes\":[]}",
            "true {\"attributes\":[],\"comments\":true,\"dataAttributes\":true,\"elements\":\
             [{\"name\":\"a\",\"namespace\":\"http://www.w3.org/1999/xhtml\",\
             \"removeAttributes\":[]}],\"removeProcessingInstructions\":[]}",
            "true true true false",
            "false",
            "undefined undefined undefined 6261",
            "{\"attributes\":[{\"name\":\"id\",\"namespace\":null}],\"comments\":true,\
             \"dataAttributes\":true,\"elements\":[{\"name\":\"a\",\"namespace\":\
             \"http://www.w3.org/1999/xhtml\",\"attributes\":[{\"name\":\"x\",\"namespace\":\
             null}]}],\"removeProcessingInstructions\":[]}",
            "false true false",
            "TypeError: Failed to execute 'allowElement' on 'Sanitizer': 1 argument required, \
             but only 0 present.",
            "TypeError: Illegal invocation",
        ]
    );
}

#[test]
fn set_html_keeps_what_the_sanitizer_allows_and_never_what_is_unsafe() {
    let said = answers(&[
        "h(\"<p onclick=x class=c id=i data-x=1 title=t>a<!--c--><script>alert(1)</script>\
           <b>b</b><a href='javascript:x' target=_blank>l</a><img src=x><iframe></iframe>\
           <style>s</style><template><i>t</i></template><svg><a href='javascript:1'>s</a>\
           <use href='#x'/></svg><math><mi>x</mi></math></p>\")",
        "h(\"<p onclick=x>a<!--c--><script>s</script><a href='javascript:x'>l</a>\
           <a href=' JAVAscript:y'>m</a><form action='javascript:z'></form><base href=x>\
           <object></object><svg><animate attributeName=href to=x /><set \
           attributeName='xlink:href'/></svg></p>\", {sanitizer: {}})",
        "h('<p><b>b</b><i>i</i></p>', {sanitizer: new Sanitizer({elements: ['p', 'b']})})",
        "h('<p><b id=1>x<i onclick=2>y</i></b></p>', {sanitizer: \
           {replaceWithChildrenElements: ['b']}})",
        "h('<p data-x=1 data-Y=2>x</p>', {sanitizer: {attributes: ['id'], \
           dataAttributes: true}})",
        "h('<my-el>x</my-el><blink>y</blink><p>z</p>')",
        "h('<!--x--><p>a</p>', {sanitizer: {comments: true}})",
        "h('<p>a<!--c-->b</p>', {sanitizer: {}})",
        "h('<template><script>1</script><b onclick=1>x</b></template>', {sanitizer: {}})",
        "h(\"<a href='\\tjava\\nscript:x'>a</a><a href='java%73cript:x'>c</a>\
           <button formaction='javascript:1'>e</button>\", {sanitizer: {}})",
        "h(\"<svg><a xlink:href='javascript:1' href=x>t</a></svg><math><mi \
           href='javascript:1'>x</mi></math>\", {sanitizer: {}})",
        "h('<p id=1 title=2>x</p><b id=3>y</b>', {sanitizer: {elements: [{name: 'p', \
           removeAttributes: ['id']}, 'b'], attributes: ['id', 'title']}})",
        "h('<b onclick=1>x</b>', {sanitizer: 'bogus'})",
        "h('<b>x</b>', {sanitizer: {elements: ['a'], removeElements: ['b']}})",
        "d.setHTML('x', 5)",
        "d.setHTML()",
        "(() => { const s = document.createElement('script'); s.setHTML('x'); \
           return J(s.textContent); })()",
        "(() => { const t = document.createElement('table'); t.setHTML('<tr><td>x</td></tr>'); \
           return t.innerHTML; })()",
        "(() => { const t = document.createElement('template'); t.setHTML('<b>x</b>'); \
           return t.innerHTML + t.childNodes.length; })()",
        "(() => { const e = document.createElementNS('http://www.w3.org/2000/svg', 'g'); \
           e.setHTML('<circle onclick=1 r=1></circle><script>1</script>', {sanitizer: {}}); \
           return e.innerHTML; })()",
        "(() => { const r = []; const mo = new MutationObserver(() => {}); d.innerHTML = \
           '<i>old</i>'; mo.observe(d, {childList: true, subtree: true}); \
           d.setHTML('<b onclick=1>x</b><script>s</script>'); \
           return mo.takeRecords().map(r => r.addedNodes.length + '/' + \
           r.removedNodes.length).join(); })()",
        "Element.prototype.setHTML.length + ' ' + Element.prototype.setHTMLUnsafe.length",
    ]);
    assert_eq!(
        said,
        [
            "<p title=\"t\">a<b>b</b><a>l</a><svg><a>s</a></svg><math><mi>x</mi></math></p>",
            "<p>a<a>l</a><a>m</a></p><form></form><svg><animate to=\"x\"></animate><set>\
             </set></svg><p></p>",
            "<p><b>b</b></p>",
            "<p>x<i>y</i></p>",
            "<p data-x=\"1\" data-y=\"2\">x</p>",
            "<p>z</p>",
            "<!--x--><p>a</p>",
            "<p>ab</p>",
            "<template><b>x</b></template>",
            "<a>a</a><a href=\"java%73cript:x\">c</a><button>e</button>",
            "<svg><a href=\"x\">t</a></svg><math><mi>x</mi></math>",
            "<p title=\"2\">x</p><b id=\"3\">y</b>",
            "TypeError: Failed to execute 'setHTML' on 'Element': Failed to read the \
             'sanitizer' property from 'SetHTMLOptions': The provided value 'bogus' is not a \
             valid enum value of type SanitizerPresets.",
            "TypeError: Failed to execute 'setHTML' on 'Element': Invalid Sanitizer \
             configuration.",
            "TypeError: Failed to execute 'setHTML' on 'Element': The provided value is not of \
             type 'SetHTMLOptions'.",
            "TypeError: Failed to execute 'setHTML' on 'Element': 1 argument required, but only \
             0 present.",
            "\"\"",
            "<tbody><tr><td>x</td></tr></tbody>",
            "<b>x</b>0",
            "<circle r=\"1\"></circle>",
            "1/1",
            "1 1",
        ]
    );
}

#[test]
fn the_unsafe_methods_remove_only_what_their_sanitizer_says() {
    let said = answers(&[
        "u(\"<p onclick=x><script>1</script><a href='javascript:1'>a</a><!--c--></p>\", \
           {sanitizer: {}})",
        "u('<p onclick=x><b>b</b></p>', {sanitizer: {removeElements: ['b']}})",
        "u('<p onclick=x><script>1</script></p>')",
        "u('<template><b>x</b><i>y</i></template>', {sanitizer: {removeElements: ['b']}})",
        "u('<b onclick=1>x</b><script>1</script>', {sanitizer: 'default'})",
        "(() => { u('<script>window.ran = 1</script>'); return typeof window.ran; })()",
    ]);
    assert_eq!(
        said,
        [
            "<p onclick=\"x\"><script>1</script><a href=\"javascript:1\">a</a><!--c--></p>",
            "<p onclick=\"x\"></p>",
            "<p onclick=\"x\"><script>1</script></p>",
            "<template><i>y</i></template>",
            "<b>x</b>",
            "undefined",
        ]
    );
}

#[test]
fn parse_html_makes_a_sanitized_document_of_its_own() {
    let said = answers(&[
        "(() => { const doc = Document.parseHTML('<!doctype html><title>T</title><p \
           onclick=1>x<script>1</script></p>'); return doc.constructor.name + ' ' + \
           doc.documentElement.outerHTML + ' ' + doc.doctype.name + ' ' + doc.URL + ' ' + \
           doc.contentType; })()",
        "Document.parseHTML('<p><b>x</b></p>', {sanitizer: {elements: ['p']}}).documentElement",
        "Document.parseHTMLUnsafe('<p><b onclick=1>x</b><script>1</script></p>', \
           {sanitizer: {removeElements: ['b']}}).documentElement.outerHTML",
        "Document.parseHTML('<p>x').compatMode",
        "Document.parseHTMLUnsafe('<!--a--><p>x', {sanitizer: {comments: false}}).childNodes\
           .length",
        "Document.parseHTML('x', {sanitizer: {elements: ['a'], removeElements: ['b']}})",
        "Document.parseHTML.length + ' ' + Document.parseHTMLUnsafe.length",
        "Document.parseHTML('').domain === document.domain",
    ]);
    assert_eq!(
        said,
        [
            "HTMLDocument <html><head><title>T</title></head><body><p>x</p></body></html> html \
             about:blank text/html",
            "null",
            "<html><head></head><body><p><script>1</script></p></body></html>",
            "BackCompat",
            "2",
            "TypeError: Failed to execute 'parseHTML' on 'Document': Invalid Sanitizer \
             configuration.",
            "1 1",
            "true",
        ]
    );
}
