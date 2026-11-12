//! Custom elements as pages use them (gap batch H, D70): definitions,
//! construction by `createElement()`, `new` and the parser, upgrades,
//! the reactions the tree's changes queue and when they run, `:defined`
//! and `:state()`, and form-associated custom elements.

use super::script_tests::scripted;

/// The console of `body` loaded with scripts on.
fn said(body: &'static str) -> Vec<String> {
    scripted(body, true).1
}

#[test]
fn a_definition_constructs_its_elements_and_upgrades_those_waiting() {
    let console = said(
        "<x-a id=early></x-a><script>\
         const log = [];\
         class A extends HTMLElement {\
           constructor() { super(); log.push('construct ' + this.id); }\
           connectedCallback() { log.push('connected ' + this.id); }\
         }\
         log.push(early instanceof HTMLElement, early instanceof A, early.matches(':defined'));\
         customElements.define('x-a', A);\
         log.push(early instanceof A, early.matches(':defined'), customElements.get('x-a') === A);\
         const made = document.createElement('x-a');\
         log.push(made instanceof A, made.matches(':defined'));\
         made.id = 'made';\
         document.body.append(made);\
         console.log(log.join(' '));\
         </script>",
    );
    assert_eq!(
        console,
        [
            "true false false construct early connected early true true true construct  true true \
          connected made"
        ]
    );
}

#[test]
fn the_parser_constructs_a_defined_element_before_its_attributes_and_children() {
    let console = said(
        "<script>\
         const log = [];\
         class B extends HTMLElement {\
           static get observedAttributes() { return ['title']; }\
           constructor() { super(); log.push('ctor ' + this.attributes.length + ' ' + \
             this.childNodes.length + ' ' + (this.parentNode === null)); }\
           attributeChangedCallback(n, o, v) {\
             log.push('attr ' + n + ' ' + o + ' ' + v + ' ' + this.isConnected);\
           }\
           connectedCallback() { log.push('connected ' + this.childNodes.length); }\
         }\
         customElements.define('x-b', B);\
         </script><x-b title=t lang=en>text <i>i</i></x-b><script>\
         const b = document.querySelector('x-b');\
         log.push(b instanceof B, b.childNodes.length);\
         console.log(log.join(', '));\
         </script>",
    );
    assert_eq!(
        console,
        ["ctor 0 0 true, attr title null t false, connected 0, true, 2"]
    );
}

#[test]
fn reactions_run_when_the_call_returns_each_elements_in_order() {
    let console = said(
        "<div id=host></div><script>\
         const log = [];\
         class C extends HTMLElement {\
           static observedAttributes = ['a'];\
           attributeChangedCallback(n, o, v) { log.push(this.id + ':' + n + '=' + v); }\
           connectedCallback() { log.push(this.id + ':connected'); }\
           disconnectedCallback() { log.push(this.id + ':disconnected'); }\
           adoptedCallback(o, n) { log.push(this.id + ':adopted ' + (n === doc)); }\
         }\
         customElements.define('x-c', C);\
         const doc = document.implementation.createHTMLDocument('');\
         const c = new C(); c.id = 'c';\
         c.setAttribute('a', '1');\
         c.setAttribute('b', '2');\
         c.setAttribute('a', '1');\
         host.append(c);\
         log.push('appended');\
         c.remove();\
         doc.body.append(c);\
         c.removeAttribute('a');\
         console.log(log.join(' '));\
         </script>",
    );
    assert_eq!(
        console,
        ["c:a=1 c:a=1 c:connected appended c:disconnected c:adopted true c:connected c:a=null"]
    );
}

#[test]
fn a_failed_construction_leaves_an_unknown_element_and_reports_it() {
    let console = said(
        "<script>\
         customElements.define('x-d', class extends HTMLElement {\
           constructor() { super(); this.setAttribute('x', '1'); } });\
         window.onerror = (m) => { console.log('error: ' + m); return true; };\
         const d = document.createElement('x-d');\
         console.log(d.constructor.name, d.matches(':defined'));\
         </script>",
    );
    assert_eq!(
        console,
        [
            "error: Uncaught NotSupportedError: Failed to execute 'createElement' on 'Document': \
             The result must not have attributes",
            "HTMLUnknownElement false"
        ]
    );
}

#[test]
fn states_match_state_and_internals_hold_a_form_value() {
    let console = said(
        "<style>x-e:state(on) { color: rgb(1, 2, 3) }</style>\
         <form id=f><x-e name=n></x-e></form><script>\
         class E extends HTMLElement {\
           static formAssociated = true;\
           constructor() { super(); this.internals = this.attachInternals(); }\
           formResetCallback() { console.log('reset'); }\
         }\
         customElements.define('x-e', E);\
         const e = document.querySelector('x-e');\
         e.internals.states.add('on');\
         e.internals.setFormValue('v');\
         console.log(getComputedStyle(e).color, e.matches(':state(on)'), \
           [...new FormData(f)].join(';'), e.internals.form === f, f.elements.length);\
         e.internals.setValidity({valueMissing: true}, 'missing');\
         console.log(e.matches(':invalid'), e.internals.validationMessage, f.checkValidity());\
         f.reset();\
         </script>",
    );
    assert_eq!(
        console,
        [
            "rgb(1, 2, 3) true n,v true 1",
            "true missing false",
            "reset"
        ]
    );
}

#[test]
fn a_written_element_is_constructed_and_write_throws_while_it_is() {
    let console = said(
        "<script>\
         const log = [];\
         class W extends HTMLElement {\
           constructor() {\
             super();\
             try { document.write('x'); } catch (e) { log.push(e.name); }\
             log.push('ctor ' + this.attributes.length);\
           }\
           connectedCallback() { log.push('connected'); }\
         }\
         customElements.define('x-w', W);\
         document.write('<x-w id=w></x-w>');\
         log.push(document.getElementById('w') instanceof W);\
         console.log(log.join(' '));\
         </script>",
    );
    assert_eq!(console, ["InvalidStateError ctor 0 connected true"]);
}

#[test]
fn a_move_is_heard_by_connected_move_callback_or_as_a_removal_and_insertion() {
    let console = said(
        "<div id=a></div><div id=b></div><script>\
         const log = [];\
         class M extends HTMLElement {\
           connectedCallback() { log.push(this.id + ':c'); }\
           disconnectedCallback() { log.push(this.id + ':d'); }\
           connectedMoveCallback() { log.push(this.id + ':m'); }\
         }\
         class N extends HTMLElement {\
           connectedCallback() { log.push(this.id + ':c'); }\
           disconnectedCallback() { log.push(this.id + ':d'); }\
         }\
         customElements.define('x-m', M);\
         customElements.define('x-n', N);\
         const m = new M(); m.id = 'm';\
         const n = new N(); n.id = 'n';\
         a.append(m, n);\
         b.moveBefore(m, null);\
         b.moveBefore(n, null);\
         console.log(log.join(' '));\
         </script>",
    );
    assert_eq!(console, ["m:c n:c m:m n:d n:c"]);
}

#[test]
fn a_scoped_registry_defines_the_elements_of_its_trees_alone() {
    let console = said(
        "<div id=host></div><script>\
         class Global extends HTMLElement {}\
         class Scoped extends HTMLElement {}\
         customElements.define('x-s', Global);\
         const registry = new CustomElementRegistry();\
         registry.define('x-s', Scoped);\
         const root = host.attachShadow({mode: 'open', customElementRegistry: registry});\
         root.innerHTML = '<x-s></x-s>';\
         const made = document.createElement('x-s', {customElementRegistry: registry});\
         const plain = document.createElement('x-s');\
         console.log(root.firstChild instanceof Scoped, root.customElementRegistry === registry,\
           root.firstChild.customElementRegistry === registry, made instanceof Scoped,\
           plain instanceof Global, document.customElementRegistry === customElements);\
         try { new Scoped(); } catch (e) { console.log(e.message); }\
         </script>",
    );
    assert_eq!(
        console,
        [
            "true true true true true true",
            "Failed to construct 'HTMLElement': Illegal constructor"
        ]
    );
}

#[test]
fn a_shadow_trees_keyframes_are_the_ones_get_keyframes_reads() {
    let console = said(
        "<div id=host></div><script>\
         const root = host.attachShadow({mode: 'open'});\
         root.innerHTML = '<style>@keyframes spin { from { opacity: 0.25 } to { opacity: 0.75 } }\
           p { animation: spin 10s }</style><p>p</p>';\
         const p = root.querySelector('p');\
         const frames = p.getAnimations()[0].effect.getKeyframes();\
         console.log(frames.length, frames[0].opacity, frames[1].opacity);\
         </script>",
    );
    assert_eq!(console, ["2 0.25 0.75"]);
}

#[test]
fn a_constructor_that_takes_its_intended_parent_in_leaves_itself_out() {
    // HTML's "insert an element at the adjusted insertion location": the
    // constructed element goes in only where it may, never under itself.
    let console = said(
        "<script>\
         let made;\
         customElements.define('x-r', class extends HTMLElement {\
           constructor() {\
             super();\
             made = this;\
             this.attachShadow({mode: 'open'}).append(document.getElementById('outer'));\
           }\
         });\
         </script><div id=outer><x-r></x-r></div><script>\
         console.log(made.parentNode === null, made.shadowRoot.getElementById('outer') !== null,\
           document.querySelectorAll('x-r').length);\
         </script>",
    );
    assert_eq!(console, ["true true 0"]);
}

#[test]
fn templates_hold_their_custom_elements_undefined() {
    let console = said(
        "<script>\
         customElements.define('x-t', class extends HTMLElement {});\
         </script><template id=t><x-t></x-t></template><script>\
         const inside = t.content.firstChild;\
         const imported = document.importNode(t.content, true).firstChild;\
         console.log(inside.matches(':defined'), inside.constructor.name,\
           imported.matches(':defined'), String(inside.customElementRegistry));\
         </script>",
    );
    assert_eq!(console, ["false HTMLElement true null"]);
}

#[test]
fn slots_a_constructor_or_a_callback_makes_signal_their_change() {
    let console = said(
        "<x-c id=constructor-upgrade><div></div></x-c><x-k id=callback-upgrade><div></div></x-k>\
         <script>\
         var calls = [];\
         const slotted = (host) => {\
           host.attachShadow({mode: 'open'}).innerHTML = '<slot></slot>';\
           host.shadowRoot.children[0].addEventListener('slotchange', () => calls.push(host.id));\
         };\
         customElements.define('x-c', class extends HTMLElement {\
           constructor() { super(); slotted(this); } });\
         customElements.define('x-k', class extends HTMLElement {\
           connectedCallback() { slotted(this); } });\
         </script><x-c id=constructor-parser><div></div></x-c>\
         <x-k id=callback-parser><div></div></x-k><script>\
         console.log(calls.sort().join());\
         </script>",
    );
    assert_eq!(
        console,
        ["callback-parser,callback-upgrade,constructor-parser,constructor-upgrade"]
    );
}

#[test]
fn an_is_value_is_serialized_first_when_no_attribute_holds_it() {
    let console = said(
        "<script>\
         class P extends HTMLParagraphElement {}\
         customElements.define('x-p', P, {extends: 'p'});\
         const made = document.createElement('p', {is: 'x-p'});\
         made.className = 'c';\
         const copy = made.cloneNode();\
         console.log(made.outerHTML, made instanceof P, copy instanceof P);\
         const built = new P();\
         console.log(built.outerHTML, built.cloneNode() instanceof P);\
         </script>",
    );
    assert_eq!(
        console,
        [
            "<p is=\"x-p\" class=\"c\"></p> true true",
            "<p is=\"x-p\"></p> true"
        ]
    );
}

#[test]
fn one_calls_reactions_run_once_all_its_steps_are_done() {
    // `replaceWith()` takes the element out and puts the other in as one
    // `[CEReactions]` method: the reactions run after both. A reset
    // button's activation is no such method's: the reset callback it
    // queues waits in the backup element queue, not run as `click()`
    // returns, and runs with the element's next reactions, here
    // `form.reset()`'s, which run before it returns.
    let console = said(
        "<div id=host><x-d id=old></x-d></div>\
         <form id=f><x-r></x-r><input type=reset id=button></form><script>\
         const log = [];\
         class D extends HTMLElement {\
           connectedCallback() { log.push(this.id + ' in, other ' + other.isConnected); }\
           disconnectedCallback() { log.push(this.id + ' out, other ' + other.isConnected); }\
         }\
         class R extends HTMLElement {\
           static formAssociated = true;\
           formResetCallback() { log.push('reset'); }\
         }\
         const other = document.createElement('x-d');\
         other.id = 'new';\
         customElements.define('x-d', D);\
         customElements.define('x-r', R);\
         log.length = 0;\
         old.replaceWith(other);\
         button.click();\
         log.push('clicked');\
         f.reset();\
         log.push('reset called');\
         Promise.resolve().then(() => console.log(log.join(', ')));\
         </script>",
    );
    assert_eq!(
        console,
        ["old out, other true, new in, other true, clicked, reset, reset, reset called"]
    );
}

#[test]
fn a_custom_elements_object_lives_while_its_tree_is_reached() {
    // Out of the document, a custom element's object (what its
    // constructor set on it) stays the node's while a script reaches the
    // node, here through the element it is in, and is collected once
    // nothing does, as a sweep after a task finds.
    let (mut tab, _) = scripted(
        "<script>\
         class E extends HTMLElement {\
           constructor() { super(); this.made = 'by its constructor'; }\
         }\
         customElements.define('x-e', E);\
         const gone = new WeakRef(new E());\
         const holder = document.createElement('div');\
         holder.append(new E());\
         for (let i = 0; i < 100; i++) new E();\
         let rounds = 0;\
         function round() {\
           if (++rounds < 20) { setTimeout(round, 0); return; }\
           const kept = holder.firstChild;\
           console.log(gone.deref() === undefined, kept instanceof E, kept.made);\
         }\
         setTimeout(round, 0);\
         </script>",
        true,
    );
    let console = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(console, ["true true by its constructor"]);
}

#[test]
fn a_custom_element_alone_out_of_the_document_keeps_its_object_while_held() {
    // G247: a custom element's object alone in its tree, made by its
    // constructor or upgraded after, has its tree's group made and held,
    // though no other object joins it: the host holds the node, for a
    // range here, and what the constructor set stays.
    let (mut tab, console) = scripted(
        "<p>x</p><script>\
         class E extends HTMLElement {\
           constructor() { super(); this.made = 'made'; }\
         }\
         customElements.define('x-e', E);\
         var made = document.createRange();\
         (function () { made.setStart(new E(), 0); })();\
         var up = document.createRange();\
         (function () {\
           var e = document.createElement('x-up');\
           customElements.define('x-up', class extends HTMLElement {\
             constructor() { super(); this.made = 'upgraded'; }\
           });\
           customElements.upgrade(e);\
           up.setStart(e, 0);\
         })();\
         </script>",
        true,
    );
    assert!(console.is_empty(), "{console:?}");
    for _ in 0..3 {
        tab.engine.shown.scripts.collect();
        tab.engine.evaluate_script("var tick = 1");
        tab.engine.run_tasks();
    }
    tab.engine.take_console();
    tab.engine
        .evaluate_script("console.log(made.startContainer.made, up.startContainer.made)");
    tab.engine.run_tasks();
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["made upgraded"]);
}

#[test]
fn a_custom_element_a_mutation_record_holds_keeps_its_object() {
    // Taken out of the document with nothing of a script's holding it, a
    // custom element's object stays the node's while the host holds the
    // node, here for a mutation record: its tree's group is held (D70).
    let console = said(
        "<script>\
         class E extends HTMLElement {\
           constructor() { super(); this.made = 'by its constructor'; }\
         }\
         customElements.define('x-e', E);\
         const box = document.createElement('div');\
         document.documentElement.append(box);\
         new MutationObserver((records) => {\
           const gone = records[records.length - 1].removedNodes[0];\
           console.log(gone instanceof E, gone.made);\
         }).observe(box, {childList: true});\
         (function () { const e = new E(); box.append(e); box.removeChild(e); })();\
         </script>",
    );
    assert_eq!(console, ["true by its constructor"]);
}

#[test]
fn a_sweep_keeps_a_custom_elements_tree_the_host_holds_a_node_of() {
    // A sweep comes after this task (as many trees held as it takes);
    // the script in the custom element's tree has its `error` event yet
    // to fire then, which the host holds it for: the tree stays held,
    // and the event reaches the listener on its object.
    let console = said(
        "<script>\
         class E extends HTMLElement {}\
         customElements.define('x-e', E);\
         for (let i = 0; i < 100; i++) new E();\
         (function () {\
           const e = new E();\
           const s = document.createElement('script');\
           s.addEventListener('error', () => console.log('error heard'));\
           s.src = '';\
           document.documentElement.append(s);\
           e.append(s);\
         })();\
         </script>",
    );
    assert_eq!(console, ["error heard"]);
}

#[test]
fn the_collection_the_documents_growth_asks_for_sweeps_too() {
    // One custom element's tree held is no cause for a sweep yet; the
    // collection the host asks for as its document's nodes double sweeps
    // the trees held for custom elements with it, after that task.
    let (mut tab, _) = scripted(
        "<script>\
         class E extends HTMLElement {}\
         customElements.define('x-e', E);\
         const gone = new WeakRef(new E());\
         const kept = document.createDocumentFragment();\
         for (let i = 0; i < 17000; i++) kept.appendChild(document.createElement('i'));\
         setTimeout(() => console.log(gone.deref() === undefined, kept.childNodes.length), 0);\
         </script>",
        true,
    );
    let console = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(console, ["true 17000"]);
}

#[test]
fn clones_and_imports_keep_their_registries_or_take_the_fallback() {
    // An element with no registry imported with a scoped one takes it;
    // one a script gave none keeps none, cloned and inserted alike; a
    // template's contents hold none, and take the document's when they
    // leave; `upgrade()` of a registry upgrades its own elements alone;
    // and `createElementNS()` makes an HTML element in a document whose
    // `createElement()` makes elements of no namespace.
    let console = said(
        "<script>\
         const scoped = new CustomElementRegistry();\
         class S extends HTMLElement {}\
         class G extends HTMLElement {}\
         scoped.define('x-s', S);\
         customElements.define('x-s', G);\
         const r = [];\
         const other = document.implementation.createHTMLDocument('');\
         const imported = document.importNode(other.createElement('x-s'),\
           {customElementRegistry: scoped});\
         r.push(imported instanceof S, imported.customElementRegistry === scoped);\
         const none = document.createElement('x-s', {customElementRegistry: null});\
         const copy = none.cloneNode();\
         document.documentElement.append(copy);\
         r.push(copy.customElementRegistry, copy instanceof G);\
         const template = document.createElement('template');\
         template.innerHTML = '<x-s></x-s>';\
         r.push(template.content.firstChild.customElementRegistry);\
         const alone = document.createElement('template');\
         alone.innerHTML = '<x-s></x-s>';\
         document.documentElement.append(alone.content.firstChild);\
         r.push(document.documentElement.lastChild instanceof G);\
         document.documentElement.append(template.content);\
         r.push(document.documentElement.lastChild instanceof G);\
         const own = document.createElement('x-u', {customElementRegistry: scoped});\
         class U extends HTMLElement {}\
         scoped.define('x-u', U);\
         customElements.define('x-u', class extends HTMLElement {});\
         customElements.upgrade(own);\
         r.push(own instanceof U);\
         scoped.upgrade(own);\
         r.push(own instanceof U);\
         const xml = new Document();\
         r.push(xml.createElementNS('http://www.w3.org/1999/xhtml', 'x-v').namespaceURI);\
         r.push(xml.createElement('x-v').namespaceURI);\
         console.log(r.map(String).join(' '));\
         </script>",
    );
    assert_eq!(
        console,
        ["true true null false null true true false true http://www.w3.org/1999/xhtml null"]
    );
}

#[test]
fn a_prefixed_custom_element_takes_its_prefix_once_constructed() {
    let console = said(
        "<script>\
         let seen;\
         class P extends HTMLElement {\
           constructor() { super(); seen = this.prefix + ' ' + this.tagName; }\
         }\
         customElements.define('x-pre', P);\
         const made = document.createElementNS('http://www.w3.org/1999/xhtml', 'p:x-pre');\
         console.log(seen, made.prefix, made.tagName, made.localName, made instanceof P,\
           made.outerHTML);\
         </script>",
    );
    assert_eq!(
        console,
        ["null X-PRE p P:X-PRE x-pre true <p:x-pre></p:x-pre>"]
    );
}

#[test]
fn reflected_members_the_reactions_tests_set_write_their_attributes() {
    // Each setter writes its content attribute, as the reactions hear,
    // and each getter reads it by its own rules.
    let console = said(
        "<script>\
         const m = document.createElement('meter');\
         m.max = 10; m.value = 12; m.low = -1;\
         const b = document.createElement('button');\
         b.formMethod = 'DIALOG'; b.formEnctype = 'bogus';\
         const e = document.createElement('div');\
         e.contentEditable = 'PLAINTEXT-ONLY';\
         let thrown = '';\
         try { e.contentEditable = 'maybe'; } catch (error) { thrown = error.name; }\
         const col = document.createElement('col');\
         col.setAttribute('span', '5000');\
         console.log(m.getAttribute('value'), m.value, m.low, b.formMethod, b.formEnctype,\
           e.getAttribute('contenteditable'), thrown, col.span, e.translate, e.draggable);\
         </script>",
    );
    assert_eq!(
        console,
        [
            "12 10 0 dialog application/x-www-form-urlencoded plaintext-only SyntaxError 1000 true false"
        ]
    );
}

#[test]
fn outputs_reset_and_the_parsers_form_pointer_owns_controls_outside_the_form() {
    // An output's value set by a script goes back to its default on a
    // reset; and a form the parser opened in a table owns the controls
    // the rows hold, though it is no ancestor of them.
    let console = said(
        "<form id=f><output id=o>default</output></form>\
         <table><form id=t><tr><td><input id=i></td></tr></form></table><script>\
         o.value = 'changed';\
         const before = o.value + ' ' + o.defaultValue;\
         f.reset();\
         console.log(before, o.value, i.form === t, t.elements.length, t.contains(i));\
         </script>",
    );
    assert_eq!(console, ["changed default default true 1 false"]);
}

#[test]
fn a_shorthand_set_replaces_its_longhands_and_aliases_write_their_properties() {
    // The style attribute, which the reactions tests observe, is written
    // once: a shorthand in the place its longhands had, and a `-webkit-`
    // alias as the property it stands for.
    let console = said(
        "<script>\
         const d = document.createElement('div');\
         d.setAttribute('style', 'border-width: 2px; color: red');\
         d.style.borderWidth = '4px';\
         d.style.webkitFilter = 'grayscale(20%)';\
         d.style.setProperty('-webkit-box-shadow', 'none');\
         console.log(d.getAttribute('style'));\
         </script>",
    );
    assert_eq!(
        console,
        ["border-width: 4px; color: red; filter: grayscale(20%); box-shadow: none;"]
    );
}
