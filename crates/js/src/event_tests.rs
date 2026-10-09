//! The bindings' events (M26, `docs/design/events.md`) against the
//! tests' small tree: the event classes and their dictionaries, the
//! DOM's dispatch, listeners' options, aborting, errors reported as
//! events, `click()` and the activation it runs, the dialogs, and a
//! dispatch the engine makes, with its checkpoints. What the reference
//! browser answers is in the comments where it decided a value.

use crate::tests::{Recorder, realm, script};
use crate::{Completion, ErrorKind, EventInit, Field, Handle, Realm, Target, Task};

/// The tree: an `html` element with a `body` holding a `div` with a
/// `button` in it, then an `a` with a `span` in it. The prelude names
/// them.
fn host() -> Recorder {
    let mut host = Recorder::default();
    let html = host.tree.element(0, "html");
    let body = host.tree.element(html, "body");
    let div = host.tree.element(body, "div");
    host.tree.element(div, "button");
    let a = host.tree.element(body, "a");
    host.tree.element(a, "span");
    host
}

const PRELUDE: &str = "var d = document.body.firstChild, b = d.firstChild, \
                       a = d.nextSibling, s = a.firstChild; ";

/// What `source` ends with, a string, run after the prelude; and the
/// host.
fn run(source: &str) -> (String, Recorder, Box<dyn Realm<Recorder>>) {
    let mut realm = realm();
    let mut host = host();
    let text = format!("{PRELUDE}{source}");
    let done = realm.run(&script(&text), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    match done {
        Completion::String(text) => (text, host, realm),
        other => panic!("{source} ended with {other:?}"),
    }
}

fn string(source: &str) -> String {
    run(source).0
}

#[test]
fn event_classes_read_their_dictionaries_as_the_reference_browser_does() {
    for (source, expected) in [
        ("String(new ErrorEvent('e').error)", "undefined"),
        (
            "var e = new ErrorEvent('e', {message: 'm', lineno: 3.7, colno: -1}); \
             [e.message, e.filename, e.lineno, e.colno].join()",
            "m,,3,4294967295",
        ),
        // Coordinates are truncated, as the reference browser's are; a
        // mouse event's `which` is its button and one.
        (
            "var e = new MouseEvent('x', {clientX: 1.5, screenX: 2.5, button: 1.7}); \
             [e.clientX, e.screenX, e.pageX, e.x, e.button, e.buttons, e.which, e.detail].join()",
            "1,2,1,1,1,0,2,0",
        ),
        (
            "var e = new MouseEvent('x'); [e.bubbles, e.cancelable, e.composed, e.view, \
             e.relatedTarget, e.movementX, e.getModifierState('Shift')].join()",
            "false,false,false,,,0,false",
        ),
        (
            "try { new UIEvent('x', {view: {}}); } catch (e) { e.name + ': ' + e.message }",
            "TypeError: Failed to construct 'UIEvent': Failed to read the 'view' property \
             from 'UIEventInit': Failed to convert value to 'Window'.",
        ),
        (
            "String(new UIEvent('x', {view: window}).view === window)",
            "true",
        ),
        (
            "try { new Event(); } catch (e) { e.message }",
            "Failed to construct 'Event': 1 argument required, but only 0 present.",
        ),
        (
            "try { new MouseEvent('x', {clientX: NaN}); } catch (e) { e.message }",
            "Failed to construct 'MouseEvent': Failed to read the 'clientX' property from \
             'MouseEventInit': The provided double value is non-finite.",
        ),
        (
            "var e = new Event('x'); [e.type, e.target, e.currentTarget, e.eventPhase, \
             e.bubbles, e.cancelable, e.defaultPrevented, e.composed, e.isTrusted, \
             typeof e.timeStamp, e.returnValue, e.cancelBubble, e.srcElement, \
             e.composedPath().length].join()",
            "x,,,0,false,false,false,false,false,number,true,false,,0",
        ),
        (
            "String(new Event('x')) + ' ' + String(new CustomEvent('x')) + ' ' + \
             Object.prototype.toString.call(Event.prototype)",
            "[object Event] [object CustomEvent] [object Event]",
        ),
        (
            "var d1 = Object.getOwnPropertyDescriptor(new Event('x'), 'isTrusted'); \
             [typeof d1.get, d1.set, d1.enumerable, d1.configurable, \
             Object.getOwnPropertyDescriptor(new Event('y'), 'isTrusted').get === d1.get].join()",
            "function,,true,false,true",
        ),
        (
            "[Event.NONE, Event.CAPTURING_PHASE, Event.AT_TARGET, Event.BUBBLING_PHASE, \
             Event.prototype.AT_TARGET, WheelEvent.DOM_DELTA_PAGE, \
             KeyboardEvent.DOM_KEY_LOCATION_NUMPAD].join()",
            "0,1,2,3,2,2,3",
        ),
        (
            "[new CustomEvent('x').detail, new CustomEvent('x', {detail: 5}).detail].join()",
            ",5",
        ),
        // A keyboard event's `which` is its key code.
        (
            "var e = new KeyboardEvent('keydown', {key: 'a', code: 'KeyA', keyCode: 65, \
             charCode: 97, which: 9, repeat: true}); [e.key, e.code, e.keyCode, e.charCode, \
             e.which, e.location, e.repeat, e.isComposing].join()",
            "a,KeyA,65,97,65,0,true,false",
        ),
        (
            "var e = new WheelEvent('wheel', {deltaY: 3.5, deltaMode: 1}); [e.deltaX, \
             e.deltaY, e.deltaMode, e.button, e instanceof MouseEvent].join()",
            "0,3.5,1,0,true",
        ),
        (
            "var e = new PointerEvent('pointerdown'); [e.pointerId, e.width, e.height, \
             e.pressure, e.pointerType, e.isPrimary, e.tiltX, e.twist, e.altitudeAngle, \
             e.azimuthAngle, e.getCoalescedEvents().length].join()",
            "0,1,1,0,,false,0,0,1.5707963267948966,0,0",
        ),
        (
            "var e = new InputEvent('input', {data: 'x', inputType: 'insertText'}); \
             [e.data, e.inputType, e.isComposing, e.dataTransfer, new InputEvent('i').data, \
             e.getTargetRanges().length].join()",
            "x,insertText,false,,,0",
        ),
        ("String(new FocusEvent('focus').relatedTarget)", "null"),
        (
            "try { new SubmitEvent('submit', {submitter: {}}); } catch (e) { e.message }",
            "Failed to construct 'SubmitEvent': Failed to read the 'submitter' property from \
             'SubmitEventInit': Failed to convert value to 'HTMLElement'.",
        ),
        (
            "String(new SubmitEvent('submit', {submitter: b}).submitter === b)",
            "true",
        ),
        (
            "[new PageTransitionEvent('pageshow').persisted, \
             new PageTransitionEvent('pageshow', {persisted: 1}).persisted].join()",
            "false,true",
        ),
        (
            "['Event', 'Events', 'HTMLEvents', 'MouseEvents', 'UIEvents', 'CustomEvent', \
             'KeyboardEvent', 'FocusEvent', 'svgevents', 'WheelEvent', 'ErrorEvent'] \
             .map(function (n) { try { var e = document.createEvent(n); \
             return e.constructor.name + '/' + JSON.stringify(e.type); } \
             catch (x) { return x.name; } }).join(' ')",
            "Event/\"\" Event/\"\" Event/\"\" MouseEvent/\"\" UIEvent/\"\" CustomEvent/\"\" \
             KeyboardEvent/\"\" FocusEvent/\"\" Event/\"\" NotSupportedError NotSupportedError",
        ),
        (
            "try { document.createEvent('Bogus'); } catch (e) { e.message }",
            "Failed to execute 'createEvent' on 'Document': The provided event type \
             ('Bogus') is invalid.",
        ),
        (
            "try { b.dispatchEvent(document.createEvent('Event')); } catch (e) { e.message }",
            "Failed to execute 'dispatchEvent' on 'EventTarget': The event provided is \
             uninitialized.",
        ),
        (
            "var e = document.createEvent('Event'); e.initEvent('foo', true, true); \
             var r = b.dispatchEvent(e); [e.type, e.bubbles, e.cancelable, r].join()",
            "foo,true,true,true",
        ),
        (
            "try { b.dispatchEvent(null); } catch (e) { e.message }",
            "Failed to execute 'dispatchEvent' on 'EventTarget': parameter 1 is not of type \
             'Event'.",
        ),
        (
            "var r; var e = new Event('x'); b.addEventListener('x', function () { \
             try { b.dispatchEvent(e); } catch (x) { r = x.name + ': ' + x.message; } }); \
             b.dispatchEvent(e); r",
            "InvalidStateError: Failed to execute 'dispatchEvent' on 'EventTarget': The \
             event is already being dispatched.",
        ),
        (
            "var e = new MouseEvent('x'); e.initMouseEvent('click', true, true, window, 2, \
             1, 2, 3, 4, true); [e.type, e.bubbles, e.detail, e.screenY, e.clientX, \
             e.ctrlKey, e.view === window].join()",
            "click,true,2,2,3,true,true",
        ),
    ] {
        assert_eq!(string(source), expected, "{source}");
    }
}

#[test]
fn a_dispatch_runs_capture_target_and_bubble_listeners_in_the_doms_order() {
    for (source, expected) in [
        // The reference browser's order: the target's capture listeners
        // before its others.
        (
            "var r = []; var e = new Event('z', {bubbles: true}); \
             window.addEventListener('z', () => r.push('wc'), true); \
             window.addEventListener('z', () => r.push('wb')); \
             document.addEventListener('z', () => r.push('dc'), true); \
             d.addEventListener('z', () => r.push('dib')); \
             b.addEventListener('z', () => r.push('tb')); \
             b.addEventListener('z', () => r.push('tc'), true); \
             b.dispatchEvent(e); \
             r.join(' ') + ' phase=' + e.eventPhase + ' ct=' + e.currentTarget + \
             ' path=' + e.composedPath().length + ' target=' + (e.target === b)",
            "wc dc tc tb dib wb phase=0 ct=null path=0 target=true",
        ),
        // An event that does not bubble reaches the capture listeners
        // above its target, not the others.
        (
            "var r = []; window.addEventListener('nb', () => r.push('wc'), true); \
             d.addEventListener('nb', () => r.push('db')); \
             b.addEventListener('nb', () => r.push('t')); \
             b.dispatchEvent(new Event('nb')); r.join()",
            "wc,t",
        ),
        (
            "var r; b.addEventListener('p', function (e) { r = e.composedPath().map(function \
             (n) { return n === window ? 'window' : n.nodeName; }).join(); }); \
             b.dispatchEvent(new Event('p')); r",
            "BUTTON,DIV,BODY,HTML,#document,window",
        ),
        // A load event does not reach the window; a detached node's path
        // ends at its root.
        (
            "var r = 'none'; window.addEventListener('load', function () { r = 'window'; }); \
             b.addEventListener('load', function (e) { r = e.composedPath().length; }); \
             b.dispatchEvent(new Event('load', {bubbles: true})); String(r)",
            "5",
        ),
        (
            "var r; var p = document.createElement('p'); var q = document.createElement('span'); \
             p.appendChild(q); q.addEventListener('q', function (e) { r = e.composedPath() \
             .map(function (n) { return n.nodeName; }).join(); }); \
             q.dispatchEvent(new Event('q')); r",
            "SPAN,P",
        ),
        (
            "var r = []; d.addEventListener('s', function (e) { r.push('d'); }); \
             b.addEventListener('s', function (e) { r.push('b1'); e.stopPropagation(); }); \
             b.addEventListener('s', function (e) { r.push('b2'); }); \
             b.dispatchEvent(new Event('s', {bubbles: true})); \
             b.addEventListener('t', function (e) { r.push('t1'); \
             e.stopImmediatePropagation(); }); \
             b.addEventListener('t', function (e) { r.push('t2'); }); \
             b.dispatchEvent(new Event('t')); r.join()",
            "b1,b2,t1",
        ),
        // A passive listener's preventDefault is ignored.
        (
            "var r; b.addEventListener('pz', function (e) { e.preventDefault(); \
             r = e.defaultPrevented; }, {passive: true}); \
             var ok = b.dispatchEvent(new Event('pz', {cancelable: true})); r + ' ' + ok",
            "false true",
        ),
        (
            "var e = new Event('x', {cancelable: true}); e.returnValue = false; \
             var f = new Event('x'); f.preventDefault(); \
             [e.defaultPrevented, e.returnValue, f.defaultPrevented].join()",
            "true,false,false",
        ),
        (
            "var e = new Event('x'); e.cancelBubble = true; var c = e.cancelBubble; \
             e.cancelBubble = false; c + ' ' + e.cancelBubble",
            "true true",
        ),
        (
            "var n = 0; b.addEventListener('o', function () { n++; }, {once: true}); \
             b.dispatchEvent(new Event('o')); b.dispatchEvent(new Event('o')); String(n)",
            "1",
        ),
        (
            "var r; var o = { handleEvent: function (e) { r = (this === o) + ' ' + e.type; } }; \
             b.addEventListener('h', o); b.dispatchEvent(new Event('h')); r",
            "true h",
        ),
        // One listener removes the next during the dispatch: it is not
        // called; one added is not either.
        (
            "var r = []; function two() { r.push('two'); } \
             b.addEventListener('m', function () { r.push('one'); \
             b.removeEventListener('m', two); \
             b.addEventListener('m', function () { r.push('late'); }); }); \
             b.addEventListener('m', two); b.dispatchEvent(new Event('m')); r.join()",
            "one",
        ),
        // The same listener is added once; `capture` tells two apart.
        (
            "var n = 0; function f() { n++; } b.addEventListener('u', f); \
             b.addEventListener('u', f, {once: true}); b.addEventListener('u', f, true); \
             b.dispatchEvent(new Event('u')); String(n)",
            "2",
        ),
        (
            "try { b.addEventListener('x', 5); } catch (e) { e.message }",
            "Failed to execute 'addEventListener' on 'EventTarget': parameter 2 is not of \
             type 'Object'.",
        ),
        (
            "var r = []; var e = new Event('w'); \
             window.addEventListener('w', function (x) { r.push(window.event === x); }); \
             window.dispatchEvent(e); r.push(window.event); r.join()",
            "true,",
        ),
        (
            "var t = new EventTarget(); var r; t.addEventListener('x', function (e) { \
             r = (e.target === t) + ' ' + e.composedPath().length; }); \
             t.dispatchEvent(new Event('x')); r",
            "true 1",
        ),
    ] {
        assert_eq!(string(source), expected, "{source}");
    }
}

#[test]
fn a_wheel_listener_on_the_window_document_or_body_is_passive_unless_it_says() {
    let source = "var r = []; \
        function prevent(e) { e.preventDefault(); r.push(e.defaultPrevented); } \
        [window, document, document.documentElement, document.body, d].forEach(function (t) { \
          var e = new WheelEvent('wheel', {cancelable: true}); \
          t.addEventListener('wheel', prevent); t.dispatchEvent(e); \
          t.removeEventListener('wheel', prevent); }); \
        var e = new WheelEvent('wheel', {cancelable: true}); \
        window.addEventListener('wheel', prevent, {passive: false}); \
        window.dispatchEvent(e); r.join()";
    assert_eq!(string(source), "false,false,false,false,true,true");
}

#[test]
fn an_aborted_signal_removes_its_listeners_and_fires_abort() {
    for (source, expected) in [
        (
            "var n = 0; var c = new AbortController(); \
             b.addEventListener('s', function () { n++; }, {signal: c.signal}); \
             b.dispatchEvent(new Event('s')); c.abort(); b.dispatchEvent(new Event('s')); \
             n + ' ' + c.signal.aborted + ' ' + c.signal.reason.name + ': ' + \
             c.signal.reason.message",
            "1 true AbortError: signal is aborted without reason",
        ),
        (
            "var c = new AbortController(); var r = []; c.signal.onabort = function (e) { \
             r.push('on ' + e.isTrusted + ' ' + e.type + ' ' + (this === c.signal)); }; \
             c.signal.addEventListener('abort', function () { r.push('listener'); }); \
             c.abort('why'); c.abort('again'); r.join() + ' ' + c.signal.reason",
            "on true abort true,listener why",
        ),
        (
            "var s = AbortSignal.abort(); var n = 0; \
             b.addEventListener('x', function () { n++; }, {signal: s}); \
             b.dispatchEvent(new Event('x')); [s.aborted, s.reason.name, n].join()",
            "true,AbortError,0",
        ),
        (
            "try { AbortSignal.abort(42).throwIfAborted(); } catch (e) { 'threw ' + e }",
            "threw 42",
        ),
        (
            "try { new AbortSignal(); } catch (e) { e.message }",
            "Failed to construct 'AbortSignal': Illegal constructor",
        ),
        (
            "var x = new AbortController(), y = new AbortController(); \
             var s = AbortSignal.any([x.signal, y.signal]); y.abort('B'); \
             var t = AbortSignal.any([AbortSignal.abort('C')]); \
             [s.aborted, s.reason, t.aborted, t.reason].join()",
            "true,B,true,C",
        ),
        (
            "try { b.addEventListener('x', function () {}, {signal: null}); } \
             catch (e) { e.message }",
            "Failed to execute 'addEventListener' on 'EventTarget': Failed to read the \
             'signal' property from 'AddEventListenerOptions': Failed to convert value to \
             'AbortSignal'.",
        ),
    ] {
        assert_eq!(string(source), expected, "{source}");
    }
    // A timeout aborts when its timer runs.
    let (_, mut host, mut realm) =
        run("var s = AbortSignal.timeout(10); s.onabort = function () {}; 'set'");
    assert_eq!(host.timers, [(1, 10)]);
    realm.enter(&Task::Timer(1), &mut host).expect("it runs");
    let reason = realm
        .run(&script("s.aborted + ' ' + s.reason.name"), &mut host)
        .expect("it runs");
    assert_eq!(reason, Completion::String("true TimeoutError".to_owned()));
}

#[test]
fn what_a_listener_throws_is_reported_as_an_error_event_that_can_be_canceled() {
    // The next listener runs; the error event names the exception.
    let (text, host, _) = run("var r = []; var errs = []; \
         window.addEventListener('error', function (e) { errs.push(e.message + '|' + \
           e.error.message + '|' + typeof e.filename + '|' + e.isTrusted + '|' + \
           e.cancelable); }); \
         b.addEventListener('k', function () { throw new Error('first'); }); \
         b.addEventListener('k', function () { r.push('second'); }); \
         b.dispatchEvent(new Event('k')); r.join() + ' ' + errs.join()");
    assert_eq!(text, "second Uncaught Error: first|first|string|true|true");
    assert_eq!(host.reported.len(), 1);
    assert_eq!(host.reported[0].message, "Error: first");
    // The window's `onerror` gets five arguments, and returning true
    // cancels the report; a listener of an error event that throws is
    // not given its own.
    let (text, host, _) = run("var r; window.onerror = function (m, src, l, c, e) { \
           r = [m, typeof src, e && e.message, arguments.length].join('|'); return true; }; \
         b.addEventListener('w', function () { throw new Error('werr'); }); \
         b.dispatchEvent(new Event('w')); window.onerror = null; \
         var n = 0; window.addEventListener('error', function () { n++; \
           throw new Error('inner'); }); \
         reportError(new Error('outer')); r + ' ' + n");
    assert_eq!(text, "Uncaught Error: werr|string|werr|5 1");
    let messages: Vec<&str> = host.reported.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(messages, ["Error: inner", "Error: outer"]);
    let (text, _, _) = run("var r; window.addEventListener('error', function (e) { \
           r = e.message + ' ' + (e.error === 7); }); reportError(7); r");
    assert_eq!(text, "Uncaught 7 true");
}

#[test]
fn a_scripts_uncaught_exception_fires_the_error_event_first() {
    let mut realm = realm();
    let mut host = host();
    realm
        .run(
            &script(
                "var seen = []; window.addEventListener('error', function (e) { \
                 seen.push(e.message + '@' + e.lineno); \
                 if (e.error instanceof TypeError) e.preventDefault(); });",
            ),
            &mut host,
        )
        .expect("it runs");
    let thrown = realm
        .run(&script("\n\nthrow new Error('plain')"), &mut host)
        .expect_err("it throws");
    assert_eq!(thrown.kind, ErrorKind::Exception);
    let handled = realm
        .run(&script("null.x"), &mut host)
        .expect_err("it throws");
    assert_eq!(handled.kind, ErrorKind::Handled);
    let mut muted = script("throw new Error('secret')");
    muted.muted = true;
    realm.run(&muted, &mut host).expect_err("it throws");
    let seen = realm
        .run(&script("seen.join(' ')"), &mut host)
        .expect("it runs");
    assert_eq!(
        seen,
        Completion::String(
            "Uncaught Error: plain@3 Uncaught TypeError: cannot read property 'x' of null@1 \
             Script error.@0"
                .to_owned()
        )
    );
}

#[test]
fn click_dispatches_a_synthetic_click_that_activates_the_nearest_link() {
    let (text, host, _) = run(
        "var r; b.addEventListener('click', function (e) { r = [e.constructor.name, \
           e.isTrusted, e.bubbles, e.cancelable, e.composed, e.detail, e.button, e.pointerId, \
           e.pointerType, e.view === window].join(); }); b.click(); \
         var n = 0; d.addEventListener('click', function () { n++; d.click(); }); d.click(); \
         r + ' ' + n",
    );
    assert_eq!(text, "PointerEvent,false,true,true,true,0,0,-1,,true 1");
    assert!(host.activated.is_empty());
    // A click at the link, or bubbling from inside it, activates it,
    // unless canceled; a plain Event does not, nor a click that does not
    // bubble from inside.
    let (_, host, _) = run("a.click(); s.click(); \
         s.dispatchEvent(new MouseEvent('click')); \
         a.dispatchEvent(new Event('click')); \
         a.addEventListener('click', function (e) { e.preventDefault(); }, {once: true}); \
         a.click(); a.dispatchEvent(new MouseEvent('click', {cancelable: true})); ''");
    let link = 5;
    assert_eq!(host.activated, [link, link, link]);
}

#[test]
fn dialogs_block_nothing_and_answer_as_when_suppressed() {
    let (text, host, _) = run(
        "alert('hi'); alert(); [confirm('sure?'), prompt('name', 'x'), alert.length, \
         confirm.length, prompt.length].join()",
    );
    assert_eq!(text, "false,,0,0,0");
    let dialogs: Vec<(&str, &str)> = host
        .dialogs
        .iter()
        .map(|(kind, message)| (kind.as_str(), message.as_str()))
        .collect();
    assert_eq!(
        dialogs,
        [
            ("alert", "hi"),
            ("alert", ""),
            ("confirm", "sure?"),
            ("prompt", "name")
        ]
    );
}

#[test]
fn an_engine_dispatch_is_trusted_and_checkpoints_between_listeners() {
    let mut realm = realm();
    let mut host = host();
    realm
        .run(
            &script(
                &(PRELUDE.to_owned()
                    + "var log = []; Object.defineProperty(Object.prototype, 'clientY', \
                 { get: function () { return 99; }, configurable: true }); \
                 b.addEventListener('click', function (e) { log.push('L1 ' + e.isTrusted + \
                   ' ' + e.clientX + ' ' + e.clientY + ' ' + (e.relatedTarget === d) + ' ' + \
                   (e.view === window)); \
                   Promise.resolve().then(function () { log.push('M1'); }); }); \
                 b.addEventListener('click', function (e) { log.push('L2'); \
                   e.preventDefault(); }); \
                 window.addEventListener('ping', function (e) { log.push('window ' + \
                   e.target + ' ' + e.eventPhase); });"),
            ),
            &mut host,
        )
        .expect("it runs");
    // The button is node 4 and the div node 3 of the tests' tree.
    let click = EventInit::new("MouseEvent", "click")
        .with("bubbles", Field::Bool(true))
        .with("cancelable", Field::Bool(true))
        .with("clientX", Field::Number(12.0))
        .with("relatedTarget", Field::Node(Some(Handle(3))))
        .with("view", Field::Window);
    let canceled = realm
        .enter(
            &Task::Dispatch {
                target: Target::Node(Handle(4)),
                event: click,
            },
            &mut host,
        )
        .expect("it runs");
    assert!(canceled);
    let ping = Task::Dispatch {
        target: Target::Window,
        event: EventInit::new("Event", "ping"),
    };
    assert!(!realm.enter(&ping, &mut host).expect("it runs"));
    let log = realm
        .run(&script("log.join(' | ')"), &mut host)
        .expect("it runs");
    assert_eq!(
        log,
        Completion::String(
            "L1 true 12 0 true true | M1 | L2 | window [object Window] 2".to_owned()
        )
    );
    // A script's click() makes no checkpoint between its listeners.
    let order = realm
        .run(
            &script(
                &(PRELUDE.to_owned() + "log = []; b.click(); log.push('after'); log.join(' ')"),
            ),
            &mut host,
        )
        .expect("it runs");
    assert_eq!(
        order,
        Completion::String("L1 false 0 0 false true L2 after".to_owned())
    );
}

#[test]
fn a_job_that_throws_between_listeners_is_reported_at_the_checkpoint() {
    let mut realm = realm();
    let mut host = host();
    realm
        .run(
            &script(
                "window.addEventListener('ping', function () { \
                 queueMicrotask(function () { throw new Error('job'); }); });",
            ),
            &mut host,
        )
        .expect("it runs");
    let ping = Task::Dispatch {
        target: Target::Window,
        event: EventInit::new("Event", "ping"),
    };
    assert!(!realm.enter(&ping, &mut host).expect("it runs"));
    let errors = realm.checkpoint(&mut host);
    let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(messages, ["Error: job"]);
}

#[test]
fn a_run_stopped_inside_the_bootstrap_leaves_it_working() {
    let mut realm = crate::tests::realm_with(crate::Limits {
        steps: 200,
        ..crate::Limits::default()
    });
    let mut host = host();
    realm
        .run(
            &script(
                "var spin = true; window.addEventListener('error', function () { \
                 while (spin) {} }); window.seen = 0; \
                 window.addEventListener('error', function () { window.seen++; });",
            ),
            &mut host,
        )
        .expect("it runs");
    // The error event's first listener spins until the budget stops the
    // run, inside the bootstrap's report.
    let stopped = realm
        .run(&script("throw new Error('one')"), &mut host)
        .expect_err("it is stopped");
    assert_eq!(stopped.kind, ErrorKind::Budget);
    // The next error fires its event again.
    realm
        .run(&script("spin = false; throw new Error('two')"), &mut host)
        .expect_err("it throws");
    let seen = realm
        .run(&script("String(window.seen)"), &mut host)
        .expect("it runs");
    assert_eq!(seen, Completion::String("1".to_owned()));
}
