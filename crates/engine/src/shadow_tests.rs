//! Shadow trees as scripts see them (gap batch H, D68): their objects,
//! events along the flat tree retargeted at each boundary, slots'
//! `slotchange`, focus delegated to a host's tree, and the sheets a
//! shadow root adopts.

use crate::{Engine, differential};

fn test_engine() -> Engine {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.set_scripting(true);
    engine
}

/// What `script` logs, run in a page of `markup`, its microtasks and
/// the tasks due after it run.
fn said(markup: &str, script: &str) -> Vec<String> {
    let mut engine = test_engine();
    engine.set_viewport(800, 600);
    engine.load_document(markup);
    engine.take_console();
    differential::run_script(&mut engine, script);
    engine.run_tasks();
    engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

#[test]
fn a_shadow_roots_object_lives_as_long_as_its_hosts() {
    // What a script set on a shadow root stays with it while its host's
    // object lives, in the document or out of it.
    let lines = said(
        "<div id=host></div>",
        "const sheet = new CSSStyleSheet(); \
         const h = document.getElementById('host'); \
         h.attachShadow({mode: 'open'}).expando = 1; \
         h.shadowRoot.adoptedStyleSheets = [sheet]; \
         h.remove(); \
         console.log(h.shadowRoot.expando, h.shadowRoot.adoptedStyleSheets.length, \
           h.shadowRoot === h.shadowRoot, h.shadowRoot.host === h);",
    );
    assert_eq!(lines, ["1 1 true true"]);
}

#[test]
fn events_are_retargeted_at_each_boundary_and_closed_trees_hidden() {
    let lines = said(
        "<div id=host><span id=light>l</span></div>",
        "const root = host.attachShadow({mode: 'closed'}); \
         root.innerHTML = '<p><slot></slot><b id=inner>i</b></p>'; \
         const inner = root.getElementById('inner'); \
         const seen = []; \
         document.addEventListener('ping', e => seen.push('doc:' + e.target.id + ':' + \
           e.composedPath().map(n => n.id || n.nodeName || 'window').join('/'))); \
         root.addEventListener('ping', e => seen.push('root:' + e.target.id)); \
         inner.dispatchEvent(new Event('ping', {bubbles: true, composed: true})); \
         inner.dispatchEvent(new Event('ping', {bubbles: true})); \
         light.dispatchEvent(new Event('ping', {bubbles: true})); \
         console.log(seen.join(' '));",
    );
    assert_eq!(
        lines,
        [
            "root:inner doc:host:host/BODY/HTML/#document/window root:inner root:light \
             doc:light:light/host/BODY/HTML/#document/window"
        ]
    );
}

#[test]
fn slotchange_fires_once_a_microtask_for_each_slot_changed() {
    let lines = said(
        "<div id=host></div>",
        "const root = host.attachShadow({mode: 'open'}); \
         root.innerHTML = '<slot name=a></slot><slot></slot>'; \
         const [a, rest] = root.querySelectorAll('slot'); \
         for (const s of [a, rest]) s.addEventListener('slotchange', \
           () => console.log('change', s.name || 'default', s.assignedNodes().length)); \
         const x = document.createElement('i'); \
         x.slot = 'a'; \
         host.append(x, 'text', document.createElement('u')); \
         console.log('sync', x.assignedSlot === a);",
    );
    assert_eq!(lines, ["sync true", "change a 1", "change default 2"]);
}

#[test]
fn a_slot_out_of_the_document_is_held_until_its_slotchange() {
    // Nothing holds the slots once the function returns but their
    // signals, which keep their objects, and so their listeners, until
    // the events fire (DOM's signal slots).
    let lines = said(
        "<body>",
        "(function () { \
           const host = document.createElement('div'); \
           const root = host.attachShadow({mode: 'open', slotAssignment: 'manual'}); \
           const slot = document.createElement('slot'); \
           const named = document.createElement('slot'); \
           slot.addEventListener('slotchange', () => console.log('slot', slot.assignedNodes().length)); \
           named.addEventListener('slotchange', () => console.log('named')); \
           root.append(slot, named); \
           const child = document.createElement('i'); \
           host.append(child); \
           slot.assign(child); \
         })(); \
         let kept = []; \
         for (let i = 0; i < 200000; i++) kept.push({i}); \
         kept = null; \
         console.log('sync');",
    );
    assert_eq!(lines, ["sync", "slot 1"]);
}

#[test]
fn a_nodes_listeners_live_as_long_as_its_tree_out_of_the_document() {
    // Only the host is held; the slot's object, with its listener, lives
    // as long as the tree it is in, and hears a change made later.
    let mut engine = test_engine();
    engine.set_viewport(800, 600);
    engine.load_document("<body>");
    engine.take_console();
    differential::run_script(
        &mut engine,
        "window.host = document.createElement('div'); \
         (function () { \
           const slot = document.createElement('slot'); \
           slot.addEventListener('slotchange', () => console.log('heard', slot.assignedNodes().length)); \
           host.attachShadow({mode: 'open'}).append(slot); \
         })(); \
         let kept = []; \
         for (let i = 0; i < 200000; i++) kept.push({i}); \
         kept = null;",
    );
    engine.run_tasks();
    differential::run_script(&mut engine, "host.append(document.createElement('i'));");
    engine.run_tasks();
    let lines: Vec<String> = engine.take_console().into_iter().map(|e| e.text).collect();
    assert_eq!(lines, ["heard 1"]);
}

#[test]
fn a_host_that_delegates_focus_gives_it_to_its_first_area() {
    let lines = said(
        "<div id=host></div><input id=after>",
        "const root = host.attachShadow({mode: 'open', delegatesFocus: true}); \
         root.innerHTML = '<span>s</span><input id=first><input id=second>'; \
         host.focus(); \
         console.log(document.activeElement.id, root.activeElement.id, \
           host.matches(':focus'), host.matches(':focus-within')); \
         after.focus(); \
         console.log(document.activeElement.id, root.activeElement, host.matches(':focus'));",
    );
    assert_eq!(lines, ["host first true true", "after null false"]);
}

#[test]
fn a_scroll_container_in_a_shadow_tree_scrolls_its_slotted_content() {
    let lines = said(
        "<style>x-box { display: block; &::part(box) { height: 100px; overflow: auto } \
           > .content { height: 500px } } .content { height: 500px }</style>\
         <x-box id=host><div class=content></div></x-box>\
         <div id=light style='overflow: auto; height: 100px'><div class=content></div></div>",
        "const root = document.getElementById('host').attachShadow({mode: 'open'}); \
         root.innerHTML = '<div part=box><slot></slot></div>'; \
         const box = root.firstChild; \
         const light = document.getElementById('light'); \
         box.scrollTop = 100; light.scrollTop = 100; \
         console.log(box.scrollTop, box.scrollHeight, light.scrollTop, light.scrollHeight);",
    );
    assert_eq!(lines, ["100 500 100 500"]);
}

#[test]
fn a_shadow_roots_markup_is_set_safely_or_not_in_its_hosts_context() {
    // HTML section 8.5.1 and the Sanitizer API: setHTML() sanitizes, a
    // script and an event handler attribute gone; setHTMLUnsafe() keeps
    // them and takes a declarative shadow root; both parse in the host's
    // context, where a `tr` makes no row.
    let lines = said(
        "<div id=host></div>",
        "const root = document.getElementById('host').attachShadow({mode: 'open'}); \
         root.setHTML('<p onclick=\"x()\">t</p><script>bad()</script>'); \
         console.log(root.innerHTML); \
         root.setHTMLUnsafe('<p onclick=\"x()\">u</p><div><template shadowrootmode=open>i\
           </template></div>'); \
         console.log(root.innerHTML, !!root.querySelector('div').shadowRoot); \
         root.setHTMLUnsafe('<tr><td>c</td></tr>'); \
         console.log(root.innerHTML); \
         try { root.setHTML(); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        lines,
        [
            "<p>t</p>",
            "<p onclick=\"x()\">u</p><div></div> true",
            "c",
            "TypeError"
        ]
    );
}
