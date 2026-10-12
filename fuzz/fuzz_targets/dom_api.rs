//! Sequences of DOM calls (M25, T2.4). The input is read as calls drawn
//! from M25's surface, each naming its nodes by index into those the
//! sequence has made or found so far, and its strings from tables or
//! from the input; they are written out as scripts, each call caught.
//! The first few are a script at the end of a page with scripts on,
//! loaded from a `data:` URL as any page is, run as it is parsed; the
//! rest are run in the page in runs of up to eight, a frame drawn
//! before each (`engine::differential::run_script`). Nothing panics; the
//! document stays valid (`Document::validate`); and the frame drawn
//! before every fourth run and at the end shows what a fresh engine
//! computes for the document as it then stands (M25.3): a script's
//! changes reach the frame through the invalidation every other change
//! goes through.
//!
//! M26's surface among the calls: listeners, handlers and the events
//! dispatched to them, `click()` and the focus, timers, animation frame
//! callbacks and microtasks, an `AbortController`, and the controls'
//! state, validity and forms. A listener's or a timer's body is a call
//! of its own. After each run the engine's clock, a hand's, moves on as
//! the input says, the tasks then due run, and a frame's rendering
//! steps with them, so that what a script left for later happens.
//!
//! M27's: `fetch()` and `XMLHttpRequest`, synchronous too, of `data:`
//! URLs and of URLs they refuse, their answers settled after each run
//! (`engine::differential::settle_network`) and run as tasks on the
//! hand's clock; `Headers`, `FormData`, `URLSearchParams`, `Blob` and
//! `Response` bodies, and `document.cookie`, which a `data:` document
//! does not have.
//!
//! M28's: `localStorage` and `sessionStorage`, which a `data:` document
//! is refused, and `StorageEvent`, made, initialized and dispatched.
//!
//! M29's: WebAssembly, a small module ([`MODULE`]) compiled and
//! instantiated with an import that changes the document, its exports
//! used, and `instantiate`'s promise settled with the run's microtasks.
//!
//! M30's: canvases in the document drawn on through their 2D contexts,
//! their paths, styles, text and pixels; an `OffscreenCanvas`, an
//! `ImageBitmap` and the exports; numbers from [`NUMBERS`].
//!
//! M31's: a worker started from a `data:` URL that echoes what it is
//! sent, values from [`MESSAGES`] posted to it and to a port of a
//! `MessageChannel`, the answers changing the document as they come
//! (from the worker's thread: whenever they do), and `terminate` and
//! `close`.
//!
//! M32's: SVG in the pages and the markup (a `<use>` of its own ancestor
//! among it), elements of SVG's made with `createElementNS`, its
//! presentation attributes set and removed, and the geometry
//! scripts ask of it (`getBBox()`, `getCTM()`, `isPointInFill()`,
//! `getTotalLength()`), the frame drawing each outer `<svg>` as a fresh
//! engine does.

#![no_main]

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use engine::differential::{compare_with_fresh, run_script, settle_network};
use engine::{Engine, FrameTime, LoadState};
use libfuzzer_sys::fuzz_target;
use net::url::Url;

/// The most calls one input makes: enough for any sequence the bindings
/// could get wrong, few enough to keep the fuzzer quick.
const MAX_CALLS: usize = 64;
/// The most calls a run after the page has loaded makes.
const RUN: usize = 8;
/// The frame: small, so the time goes into the engine.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

/// A module (M29) that imports `env.f` and exports a memory `mem` of one
/// to four pages, a table `t`, a mutable global `g` and `run`, which
/// stores its argument at 0, calls `f` with it, adds `g` and grows the
/// memory by a page.
const MODULE: &str = "0,97,115,109,1,0,0,0,1,6,1,96,1,127,1,127,2,9,1,3,101,110,118,1,102,0,0,3,2,1,0,4,4,1,112,0,2,5,4,1,1,1,4,6,6,1,127,1,65,7,11,7,21,4,3,109,101,109,2,0,1,116,1,0,1,103,3,0,3,114,117,110,0,1,9,7,1,0,65,0,11,1,1,10,23,1,21,0,65,0,32,0,54,2,0,32,0,16,0,35,0,106,65,1,64,0,26,11,0,16,4,110,97,109,101,1,9,2,0,1,102,1,3,114,117,110";

/// The pages a sequence starts in.
const PAGES: &[&str] = &[
    "<!DOCTYPE html><p id=a class=c>One <b>two</b></p><div><span>three</span></div>",
    "<!DOCTYPE html><style>.c { color: #f00; margin-left: 4px } p + p { display: none }</style>\
     <table><tr><td>1<td>2</table><ul><li>a<li class=c>b</ul><p>x</p><p>y</p>",
    "<!DOCTYPE html><div style='width: 30px; float: left'>f</div><pre>a\nb</pre>\
     <template><p>t</p></template><noscript><p>n</p></noscript>",
    "<p>quirks<table><tr><td>cell</td></tr></table>",
    "<!DOCTYPE html><style>:focus { outline: 2px solid #00f } :checked + label { color: #f00 }\
     :invalid { margin-left: 3px } :user-invalid { color: #f0f } :hover { color: #080 }\
     :focus-within { padding: 1px } :placeholder-shown { width: 30px }</style>\
     <form id=a><input id=b required pattern='[a-z]*' maxlength=8 placeholder=p>\
     <input type=checkbox id=c><label for=c>c</label><input type=radio name=r>\
     <input type=radio name=r checked><select><option>1<option selected>2</select>\
     <select multiple size=2><option>x<option>y</select><input type=number min=1 max=5>\
     <textarea>t</textarea><fieldset><input type=email value=e></fieldset><button>b</button>\
     </form><a href=#a>link</a>",
];

// What the calls are given: the strings of these tables, or the input's.
const TAGS: &[&str] = &[
    "div", "p", "span", "b", "table", "tbody", "tr", "td", "ul", "li", "a", "img", "br", "pre",
    "style", "script", "template", "noscript", "select", "option", "h1", "svg", "x-y", "input",
    "textarea", "button", "form", "label", "fieldset",
];
// SVG's (M32): its elements, its presentation and geometry attributes,
// and values they take or refuse.
const SVG_TAGS: &[&str] = &[
    "svg",
    "g",
    "rect",
    "circle",
    "path",
    "text",
    "tspan",
    "use",
    "linearGradient",
    "stop",
    "clipPath",
];
const SVG_ATTRIBUTES: &[&str] = &[
    "viewBox",
    "width",
    "height",
    "x",
    "r",
    "d",
    "fill",
    "stroke",
    "stroke-width",
    "transform",
    "opacity",
    "href",
    "clip-path",
    "offset",
    "stop-color",
    "points",
];
const SVG_VALUES: &[&str] = &[
    "0 0 10 10",
    "5",
    "50%",
    "-1",
    "red",
    "url(#c) blue",
    "none",
    "M0 0 L5 5 Z",
    "rotate(45 2 2)",
    "scale(2) foo",
    "0.5",
    "#c",
    "url(#k)",
    "currentColor",
    "1e9",
];
const ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "style",
    "title",
    "hidden",
    "dir",
    "lang",
    "data-x",
    "colspan",
    "width",
    "type",
    "value",
    "checked",
    "selected",
    "required",
    "disabled",
    "readonly",
    "pattern",
    "min",
    "max",
    "maxlength",
    "placeholder",
    "multiple",
    "size",
    "for",
    "form",
    "name",
    "tabindex",
    "autofocus",
    "novalidate",
];
const VALUES: &[&str] = &[
    "",
    "a",
    "c",
    "a b",
    "color: #00f",
    "display: none",
    "rtl",
    "2",
    "10px",
    "x",
    "float: right",
    "checkbox",
    "radio",
    "number",
    "email",
    "submit",
    "-1",
    "[a-z]+",
];
const MARKUP: &[&str] = &[
    "<svg viewBox='0 0 10 10' width=20><rect width=5 height=5 fill=red></rect>\
     <circle id=c cx=5 cy=5 r=3 stroke=blue></circle><text x=1 y=8>t</text>\
     <use href='#c' x=2></use></svg>",
    "<svg><g id=u><use href='#u'></use><path id=p d='M0 0 H9'></path></g></svg>",
    "<p>p</p>",
    "<table><tr><td>1</td></tr></table>",
    "<b><i>x</b></i>",
    "<li>a<li>b",
    "</p>",
    "<!-- c -->",
    "x &amp; y",
    "<style>p { color: #0f0 }</style>",
    "<script>document.title = 'w'</script>",
    "<template><p>t</p></template>",
    "<select><option>o</select>",
    "<div class=c style='margin: 3px'>d</div>",
];
const SELECTORS: &[&str] = &[
    "*",
    "p",
    "div > p",
    ":scope > *",
    "[id]",
    ".c",
    "td:nth-child(2)",
    "p + p",
    ":not(p)",
    "#a",
    "li:first-child",
    "[[",
];
const PROPERTIES: &[&str] = &[
    "color",
    "display",
    "margin-left",
    "width",
    "position",
    "float",
    "background-color",
    "font-size",
    "padding",
    "opacity",
    "transform",
    "visibility",
];
const PROPERTY_VALUES: &[&str] = &[
    "#f00",
    "none",
    "10px",
    "auto",
    "absolute",
    "left",
    "50%",
    "0.5",
    "rotate(10deg)",
    "inherit",
    "",
    "1em",
    "block",
    "inline-block",
    "hidden",
];
const POSITIONS: &[&str] = &[
    "beforebegin",
    "afterbegin",
    "beforeend",
    "afterend",
    "nowhere",
];
/// Scripts a sequence inserts, which run as they are connected.
const SCRIPTS: &[&str] = &[
    "document.body.appendChild(document.createElement('p'))",
    "document.write('<p>w</p>')",
    "throw new Error('e')",
    "document.currentScript.remove()",
    "document.querySelector('p') && (document.querySelector('p').innerHTML = '<i>i</i>')",
];
/// The events listened for and dispatched.
const EVENTS: &[&str] = &[
    "click",
    "input",
    "change",
    "focus",
    "blur",
    "focusin",
    "keydown",
    "submit",
    "invalid",
    "reset",
    "pointerdown",
    "mouseover",
    "x",
    "load",
];
/// What dispatched events are made as.
const EVENT_CLASSES: &[&str] = &[
    "Event",
    "CustomEvent",
    "UIEvent",
    "MouseEvent",
    "PointerEvent",
    "KeyboardEvent",
    "FocusEvent",
    "InputEvent",
    "SubmitEvent",
    "WheelEvent",
];
/// A listener's options.
const OPTIONS: &[&str] = &[
    "false",
    "true",
    "{once: true}",
    "{passive: true}",
    "{capture: true, once: true}",
    "{signal: ac.signal}",
];
/// What an event handler attribute holds.
const HANDLER_CODE: &[&str] = &[
    "return false",
    "event.preventDefault()",
    "this.remove()",
    "event.stopPropagation(); this.className = 'c'",
    "document.body.appendChild(document.createElement('p'))",
    "this.focus()",
    "x(",
];

/// What scripts fetch (M27): `data:` URLs of each kind a body comes
/// in, and URLs a fetch refuses or cannot parse.
/// Numbers a canvas's call takes: the edges of what it takes.
/// What the calls post and clone (M31): values the structured clone
/// makes again, and some it refuses.
const MESSAGES: &[&str] = &[
    "1",
    "'a'",
    "[1, { a: 2 }]",
    "new Map([[1, 2]])",
    "new Set(['x'])",
    "new Uint8Array([1, 2]).buffer",
    "new Date(0)",
    "/x/g",
    "{ a: { b: [null, undefined, -0, NaN, 1n] } }",
    "new Blob(['x'])",
    "new ImageData(1, 1)",
    "new TypeError('t')",
    "() => 0",
    "Symbol()",
    "document.body",
];

const NUMBERS: &[&str] = &[
    "0",
    "1",
    "-1",
    "0.5",
    "7",
    "300",
    "1e9",
    "-1e9",
    "NaN",
    "Infinity",
    "-Infinity",
    "2e-300",
];

const URLS: &[&str] = &[
    "data:text/plain,hello",
    "data:text/html,%3Cp%3Ex%3C/p%3E",
    "data:application/json,%7B%22a%22:[1,2]%7D",
    "data:,",
    "data:text/plain;base64,aGk=",
    "data:text/plain;charset=utf-16le;base64,aABpAA==",
    "data:image/png;base64,AA==",
    "data:text/plain,a%00b%FF",
    "about:blank",
    "http://[",
    "javascript:1",
    "x",
];
const METHODS: &[&str] = &[
    "GET", "POST", "PUT", "HEAD", "delete", "patch", "TRACE", "x y",
];
/// A fetch's init.
const INITS: &[&str] = &[
    "{}",
    "{method: 'POST', body: 'b'}",
    "{mode: 'no-cors'}",
    "{mode: 'same-origin'}",
    "{redirect: 'manual'}",
    "{redirect: 'error'}",
    "{credentials: 'include'}",
    "{headers: {'X-A': '1', 'Content-Type': 'text/plain'}}",
    "{signal: ac.signal}",
    "{method: 'POST', body: new URLSearchParams('a=1&b=2')}",
    "{method: 'POST', body: new Blob(['x'], {type: 'text/x'})}",
    "{method: 'POST', body: new FormData()}",
    "{cache: 'no-store', keepalive: true}",
];
/// What an XHR sends.
const BODIES: &[&str] = &[
    "null",
    "'b'",
    "new Blob(['x'])",
    "new URLSearchParams('a=1')",
    "new FormData()",
    "new Uint8Array([1, 2])",
    "new ArrayBuffer(3)",
];
const RESPONSE_TYPES: &[&str] = &["", "text", "json", "arraybuffer", "blob", "document", "x"];
/// What a `Headers` is made from.
const HEADER_INITS: &[&str] = &[
    "undefined",
    "[['a', '1'], ['B', '2'], ['a', '3']]",
    "{'Set-Cookie': 'x', c: 'd'}",
    "[['a']]",
    "new Headers([['c', '3']])",
    "[['a b', '1']]",
];

/// The input, read a byte at a time; past its end, zeros.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    /// Whether it is all read.
    fn done(&self) -> bool {
        self.0.is_empty()
    }

    /// One of `table`'s strings.
    fn pick(&mut self, table: &[&str]) -> String {
        js_string(table[usize::from(self.byte()) % table.len()])
    }

    /// A node: `h[i]`, one of those made or found so far.
    fn node(&mut self) -> String {
        format!("g({})", self.byte())
    }

    /// A string from a table, or of up to 15 code units from the input,
    /// any of them, lone surrogates among them.
    fn text(&mut self, table: &[&str]) -> String {
        let length = self.byte();
        if length < 192 {
            return js_string(table[usize::from(length) % table.len()]);
        }
        let mut text = String::from("'");
        for _ in 0..length % 16 {
            let unit = u16::from_le_bytes([self.byte(), self.byte()]);
            let _ = write!(text, "\\u{unit:04x}");
        }
        text.push('\'');
        text
    }
}

/// `text` as a JavaScript string literal, every character escaped.
fn js_string(text: &str) -> String {
    let mut literal = String::from("'");
    for unit in text.encode_utf16() {
        let _ = write!(literal, "\\u{unit:04x}");
    }
    literal.push('\'');
    literal
}

/// The body of a listener or a timer's callback: a call of its own, or,
/// in one already `nested`, a plain one.
fn body(input: &mut Input, nested: bool) -> String {
    if nested {
        String::from("p(this)")
    } else {
        call(input, true)
    }
}

/// One call, read from `input`, as a statement; `nested` in a
/// listener's or a callback's body.
fn call(input: &mut Input, nested: bool) -> String {
    let op = input.byte() % 75;
    let a = input.node();
    match op {
        0 => format!("p(document.createElement({}))", input.pick(TAGS)),
        1 => format!("p(document.createTextNode({}))", input.text(VALUES)),
        2 => format!("p(document.getElementById({}))", input.text(VALUES)),
        3 => format!(
            "{a}.replaceChildren({}, {})",
            input.node(),
            input.text(VALUES)
        ),
        4 => format!("{a}.appendChild({})", input.node()),
        5 => format!("{a}.insertBefore({}, {})", input.node(), input.node()),
        6 => format!("{a}.removeChild({})", input.node()),
        7 => format!("{a}.replaceChild({}, {})", input.node(), input.node()),
        8 => format!("{a}.remove()"),
        9 => format!(
            "{a}.setAttribute({}, {})",
            input.pick(ATTRIBUTES),
            input.text(VALUES)
        ),
        10 => format!("{a}.removeAttribute({})", input.pick(ATTRIBUTES)),
        11 => format!("{a}.innerHTML = {}", input.text(MARKUP)),
        12 => format!("{a}.outerHTML = {}", input.text(MARKUP)),
        13 => format!(
            "{a}.insertAdjacentHTML({}, {})",
            input.pick(POSITIONS),
            input.text(MARKUP)
        ),
        14 => format!("{a}.textContent = {}", input.text(VALUES)),
        15 => format!("p({a}.cloneNode({}))", input.byte() % 2 == 0),
        16 => format!("{a}.classList.toggle({})", input.text(VALUES)),
        17 => format!(
            "{a}.style.setProperty({}, {})",
            input.pick(PROPERTIES),
            input.pick(PROPERTY_VALUES)
        ),
        18 => format!("{a}.style.cssText = {}", input.text(VALUES)),
        19 => format!("p({a}.querySelector({}))", input.pick(SELECTORS)),
        20 => format!(
            "var l = {a}.querySelectorAll({}); p(l[l.length - 1])",
            input.pick(SELECTORS)
        ),
        21 => format!(
            "getComputedStyle({a}).getPropertyValue({})",
            input.pick(PROPERTIES)
        ),
        22 => {
            let position = input.pick(POSITIONS);
            format!(
                "{a}.insertAdjacentText({position}, {}); p({a}.insertAdjacentElement({position}, {}))",
                input.text(VALUES),
                input.node()
            )
        }
        23 => {
            let method = ["append", "prepend", "before", "after", "replaceWith"]
                [usize::from(input.byte()) % 5];
            format!("{a}.{method}({}, {})", input.node(), input.text(VALUES))
        }
        24 => format!("p({a}.splitText({}))", input.byte() % 8),
        25 => format!(
            "{a}.deleteData({}, {}); {a}.insertData({}, {})",
            input.byte() % 8,
            input.byte() % 8,
            input.byte() % 8,
            input.text(VALUES)
        ),
        26 => format!("{a}.dataset.x = {}", input.text(VALUES)),
        27 => format!("document.write({})", input.text(MARKUP)),
        28 => {
            let step = [
                "firstChild",
                "lastChild",
                "parentNode",
                "nextSibling",
                "previousSibling",
                "firstElementChild",
                "lastElementChild",
            ][usize::from(input.byte()) % 7];
            format!("p({a}.{step})")
        }
        29 => format!(
            "p({a}.childNodes[{}]); p({a}.children[0])",
            input.byte() % 4
        ),
        30 => format!(
            "{a}.id = {}; {a}.className = {}",
            input.text(VALUES),
            input.text(VALUES)
        ),
        31 => format!(
            "p({a}.closest({})); {a}.matches({})",
            input.pick(SELECTORS),
            input.pick(SELECTORS)
        ),
        32 => format!(
            "var c = {a}.getElementsByTagName({}); {}.appendChild(c[0]); p(c[c.length - 1])",
            input.pick(TAGS),
            input.node()
        ),
        33 => format!(
            "var s = document.createElement('script'); s.textContent = {}; {a}.appendChild(s)",
            input.pick(SCRIPTS)
        ),
        34 => {
            let event = input.pick(EVENTS);
            let options = OPTIONS[usize::from(input.byte()) % OPTIONS.len()];
            format!(
                "var k = function (e) {{ {} }}; l.push(k); {a}.addEventListener({event}, k, {options})",
                body(input, nested)
            )
        }
        35 => format!(
            "{a}.removeEventListener({}, l[{} % l.length], {})",
            input.pick(EVENTS),
            input.byte(),
            input.byte() % 2 == 0
        ),
        36 => {
            let class = EVENT_CLASSES[usize::from(input.byte()) % EVENT_CLASSES.len()];
            let flags = input.byte();
            format!(
                "{a}.dispatchEvent(new {class}({}, {{bubbles: {}, cancelable: {}}}))",
                input.pick(EVENTS),
                flags & 1 != 0,
                flags & 2 != 0
            )
        }
        37 => format!("{a}.click()"),
        38 => match input.byte() % 3 {
            0 => format!("{a}.focus(); p(document.activeElement)"),
            1 => format!("{a}.blur()"),
            _ => format!("{a}.focus({{preventScroll: true}}); {a}.tabIndex = -1"),
        },
        39 => {
            let event = EVENTS[usize::from(input.byte()) % EVENTS.len()];
            format!("{a}.on{event} = function (e) {{ {} }}", body(input, nested))
        }
        40 => {
            let event = EVENTS[usize::from(input.byte()) % EVENTS.len()];
            format!(
                "{a}.setAttribute('on{event}', {})",
                input.pick(HANDLER_CODE)
            )
        }
        41 => {
            let delay = input.byte() % 64;
            format!(
                "t.push(setTimeout(function () {{ {} }}, {delay}))",
                body(input, nested)
            )
        }
        42 => {
            let delay = input.byte() % 64;
            format!(
                "t.push(setInterval(function () {{ {} }}, {delay}))",
                body(input, nested)
            )
        }
        43 => format!("clearTimeout(t[{} % t.length])", input.byte()),
        44 => format!(
            "requestAnimationFrame(function () {{ {} }})",
            body(input, nested)
        ),
        45 => format!("queueMicrotask(function () {{ {} }})", body(input, nested)),
        46 => String::from("ac.abort(); ac = new AbortController()"),
        47 => format!("{a}.value = {}", input.text(VALUES)),
        48 => {
            let flags = input.byte();
            format!(
                "{a}.checked = {}; {a}.indeterminate = {}",
                flags & 1 != 0,
                flags & 2 != 0
            )
        }
        49 => format!(
            "{a}.selectedIndex = {}; p({a}.options[{}])",
            i32::from(input.byte() % 4) - 1,
            input.byte() % 4
        ),
        50 => match input.byte() % 3 {
            0 => format!(
                "{a}.setSelectionRange({}, {}, 'backward')",
                input.byte() % 8,
                input.byte() % 8
            ),
            1 => format!("{a}.select()"),
            _ => format!("{a}.selectionStart = {}", input.byte() % 8),
        },
        51 => format!(
            "{a}.setCustomValidity({}); {a}.reportValidity(); {a}.checkValidity()",
            input.text(VALUES)
        ),
        52 => match input.byte() % 3 {
            0 => format!("({a}.form || {a}).reset()"),
            1 => format!("({a}.form || {a}).requestSubmit()"),
            _ => format!("p({a}.elements[{}]); p({a}.labels[0])", input.byte() % 4),
        },
        54 => format!(
            "fetch({}, {}).then(r => r.text()).then(t => {{ {a}.textContent = t }}, e => {{ {a}.title = e.name }})",
            input.text(URLS),
            INITS[usize::from(input.byte()) % INITS.len()]
        ),
        55 => {
            let method = input.text(METHODS);
            let url = input.text(URLS);
            let response_type = input.pick(RESPONSE_TYPES);
            let timeout = input.byte() % 4 * 10;
            let body = BODIES[usize::from(input.byte()) % BODIES.len()];
            format!(
                "var x = new XMLHttpRequest(); x.open({method}, {url}); x.responseType = {response_type}; \
                 x.timeout = {timeout}; x.onloadend = function () {{ {a}.textContent = x.status + ' ' + \
                 (x.responseType === '' || x.responseType === 'text' ? x.responseText : typeof x.response) }}; \
                 x.send({body})"
            )
        }
        56 => format!(
            "var y = new XMLHttpRequest(); y.open('GET', {}, false); y.send(); \
             {a}.textContent = y.responseText + y.getAllResponseHeaders()",
            input.text(URLS)
        ),
        57 => format!(
            "var hd = new Headers({}); hd.append({}, {}); hd.delete({}); {a}.title = [...hd].join() + hd.get('a')",
            HEADER_INITS[usize::from(input.byte()) % HEADER_INITS.len()],
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES)
        ),
        58 => format!(
            "var fd = new FormData({a} instanceof HTMLFormElement ? {a} : undefined); fd.append({}, {}); \
             fd.set('f', new Blob(['x']), 'n'); {a}.title = [...fd.keys()].join()",
            input.text(VALUES),
            input.text(VALUES)
        ),
        59 => format!(
            "var u = new URLSearchParams({}); u.append({}, {}); u.sort(); {a}.title = u.toString() + u.size",
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES)
        ),
        60 => format!(
            "new Blob([{}, {}], {{type: {}}}).slice({}, {}).text().then(t => {{ {a}.title = t }}); \
             new Response({}).arrayBuffer().then(b => {{ {a}.dataset.n = b.byteLength }})",
            input.text(VALUES),
            input.text(MARKUP),
            input.text(VALUES),
            input.byte() % 8,
            input.byte() % 8,
            input.text(VALUES)
        ),
        61 => format!(
            "document.cookie = {}; {a}.title = document.cookie + navigator.cookieEnabled",
            input.text(VALUES)
        ),
        62 => format!(
            "{a}.title = [localStorage, sessionStorage][{}].getItem({})",
            input.byte() % 2,
            input.text(VALUES)
        ),
        63 => format!(
            "{a}.dispatchEvent(new StorageEvent({}, {{key: {}, newValue: {}, url: {}, \
             bubbles: true}})); const s = document.createEvent('StorageEvent'); \
             s.initStorageEvent({}, true, true, {}, null, null, {}, null); \
             {a}.title = s.key + s.url + s.type",
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES)
        ),
        64 => format!(
            "var wi = new WebAssembly.Instance(new WebAssembly.Module(new Uint8Array([{MODULE}])), \
             {{env: {{f: (x) => {{ {a}.title = x; return x; }}}}}}).exports; \
             {a}.title = wi.run({}) + new Int32Array(wi.mem.buffer)[0] + wi.t.length + wi.g.value",
            input.byte()
        ),
        65 => format!(
            "WebAssembly.instantiate(new Uint8Array([{MODULE}]), {{env: {{f: (x) => x}}}}).then((r) => \
             {{ {a}.dataset.w = r.instance.exports.run({}); r.instance.exports.mem.grow(1); }}); \
             {a}.title = WebAssembly.validate(new Uint8Array([{}, {}, {}]))",
            input.byte(),
            input.byte(),
            input.byte(),
            input.byte()
        ),
        66 => format!(
            "var cv = document.createElement('canvas'); cv.width = {}; cv.height = {}; {a}.append(cv); \
             p(cv); var cx = cv.getContext('2d'); cx.fillStyle = {}; cx.fillRect({}, {}, {}, {}); \
             cx.beginPath(); cx.arc({}, {}, {}, 0, {}); cx.roundRect(0, 0, {}, {}, [{}]); cx.fill(); \
             cx.lineWidth = {}; cx.setLineDash([{}, {}]); cx.stroke(new Path2D({}))",
            input.byte() % 64,
            input.byte() % 64,
            input.text(VALUES),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.text(VALUES)
        ),
        67 => format!(
            "var cy = ({a} instanceof HTMLCanvasElement ? {a} : document.createElement('canvas')).getContext('2d'); \
             cy.globalCompositeOperation = {}; cy.globalAlpha = {}; cy.shadowBlur = {}; cy.shadowColor = 'red'; \
             cy.shadowOffsetX = {}; var gr = cy.createLinearGradient(0, 0, {}, {}); gr.addColorStop(0.5, {}); \
             cy.fillStyle = gr; cy.translate({}, {}); cy.rotate({}); cy.fillRect(0, 0, 9, 9); \
             cy.drawImage(cy.canvas, {}, {}); cy.putImageData(cy.createImageData(3, 2), {}, {}); \
             {a}.title = cy.getImageData(0, 0, 2, 2).data.join() + cy.isPointInPath({}, {})",
            input.text(VALUES),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.text(VALUES),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.byte() % 8,
            input.byte() % 8,
            input.pick(NUMBERS),
            input.pick(NUMBERS)
        ),
        68 => format!(
            "var cz = document.createElement('canvas').getContext('2d'); cz.font = {}; cz.textAlign = {}; \
             cz.textBaseline = {}; cz.letterSpacing = {}; cz.fillText({}, {}, {}, {}); \
             cz.strokeText({}, 1, 20); {a}.title = cz.measureText({}).width + cz.font",
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES),
            input.text(VALUES),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.text(VALUES),
            input.text(VALUES)
        ),
        69 => format!(
            "var oc = new OffscreenCanvas({}, {}); var ox = oc.getContext('2d'); ox.fillRect(0, 0, {}, {}); \
             createImageBitmap(oc, {{resizeWidth: {}}}).then((b) => {{ {a}.title = b.width; b.close(); }}, () => 0); \
             oc.convertToBlob().then((b) => {{ {a}.dataset.c = b.size; }}, () => 0); \
             {a}.title = document.createElement('canvas').toDataURL().length + oc.transferToImageBitmap().width",
            input.byte() % 32,
            input.byte() % 32,
            input.pick(NUMBERS),
            input.pick(NUMBERS),
            input.byte() % 16 + 1
        ),
        70 => format!(
            "var w = new Worker('data:text/javascript,onmessage%3Dfunction(e)%7BpostMessage(e.data)%7D'); \
             w.onmessage = (e) => {{ {a}.title = typeof e.data; }}; \
             w.onerror = () => {{ {a}.title = 'error'; }}; \
             w.postMessage({}); if ({}) w.terminate()",
            input.pick(MESSAGES),
            input.byte() % 2 == 0
        ),
        71 => format!(
            "var mc = new MessageChannel(); \
             mc.port2.onmessage = (e) => {{ {a}.title = String(e.data); }}; \
             mc.port1.postMessage({}); if ({}) mc.port1.close(); \
             {a}.dataset.m = typeof structuredClone({}, {{ transfer: [] }})",
            input.pick(MESSAGES),
            input.byte() % 2 == 0,
            input.pick(MESSAGES)
        ),
        72 => format!(
            "p(document.createElementNS('http://www.w3.org/2000/svg', {}))",
            input.pick(SVG_TAGS)
        ),
        73 => {
            let name = input.pick(SVG_ATTRIBUTES);
            if input.byte() % 4 == 0 {
                format!("{a}.removeAttribute({name})")
            } else {
                format!("{a}.setAttribute({name}, {})", input.text(SVG_VALUES))
            }
        }
        74 => format!(
            "if ({a}.getBBox) {{ var b = {a}.getBBox(); {a}.dataset && ({a}.dataset.b = \
             [b.x, b.width, {a}.getCTM() && {a}.getCTM().e].join()); }} \
             if ({a}.getTotalLength) {{ {a}.isPointInFill({{x: 1, y: 1}}); \
             {a}.isPointInStroke(new DOMPoint(2, 2)); {a}.getPointAtLength({a}.getTotalLength() / 2); }}"
        ),
        _ => {
            let property = ["disabled", "required", "readOnly", "multiple", "noValidate"]
                [usize::from(input.byte()) % 5];
            format!("{a}.{property} = !{a}.{property}")
        }
    }
}

/// The page: one of `PAGES`, and a script at its end that keeps the
/// nodes the calls name, the listeners and the timers they make, and
/// makes up to fifteen calls, as the input's next byte says.
fn page(input: &mut Input) -> String {
    let mut page = PAGES[usize::from(input.byte()) % PAGES.len()].to_owned();
    let first = input.byte() % 16;
    page.push_str(
        "<script>var h = [document, document.documentElement, document.body];\
         var l = [], t = [], ac = new AbortController();\
         function p(x) { if (x !== undefined && h.length < 256) h.push(x); }\
         function g(i) { return h[i % h.length]; }\n",
    );
    page.push_str(&calls(input, usize::from(first)));
    page.push_str("</script>");
    page
}

/// Up to `count` calls, as long as the input lasts, each a statement
/// whose exception is caught.
fn calls(input: &mut Input, count: usize) -> String {
    let mut script = String::new();
    for _ in 0..count {
        if input.done() {
            break;
        }
        let _ = writeln!(script, "try {{ {} }} catch (e) {{}}", call(input, false));
    }
    script
}

static FONTS: OnceLock<text::FontCollection> = OnceLock::new();

/// The engines' clock, in milliseconds: moved by hand, so that an input
/// runs the same timers however fast the machine is.
static NOW: AtomicU64 = AtomicU64::new(0);

fn fonts() -> &'static text::FontCollection {
    FONTS.get_or_init(|| {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    })
}

fn now() -> Duration {
    Duration::from_millis(NOW.load(Ordering::Relaxed))
}

/// A new engine over the bundled font, with scripts on and the hand's
/// clock: the one under test, and the fresh one, whose `noscript` is
/// shown as the other's.
fn scripting() -> Engine {
    let mut engine = Engine::with_fonts(fonts().clone());
    engine.set_scripting(true);
    engine.set_clock(Arc::new(now));
    engine
}

fuzz_target!(|data: &[u8]| {
    NOW.store(0, Ordering::Relaxed);
    let mut input = Input(data);
    let page = page(&mut input);
    let mut url = String::from("data:text/html,");
    for byte in page.bytes() {
        let _ = write!(url, "%{byte:02X}");
    }
    let url = Url::parse(&url).expect("a data: URL");
    let mut engine = scripting();
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(url.clone());
    loop {
        engine.poll_network();
        if engine.url() == Some(&url) && *engine.load_state() == LoadState::Loaded {
            break;
        }
        woken
            .recv_timeout(Duration::from_secs(60))
            .expect("the page loads");
    }
    settle_network(&mut engine);
    let mut fresh = scripting();
    let mut made = 0;
    for run in 1.. {
        if let Err(problem) = engine.document().validate() {
            panic!("the document is not valid: {problem}");
        }
        if input.done() || made >= MAX_CALLS {
            break;
        }
        if run % 4 == 0 {
            if let Err(difference) = compare_with_fresh(&mut engine, &mut fresh, WIDTH, HEIGHT) {
                panic!("before run {run}: {difference}");
            }
        } else {
            engine.frame(WIDTH, HEIGHT);
        }
        run_script(&mut engine, &calls(&mut input, RUN));
        settle_network(&mut engine);
        made += RUN;
        // Time passes, up to a second: the timers then due run, then a
        // frame's rendering steps, its animation frame callbacks.
        NOW.fetch_add(u64::from(input.byte()) * 4, Ordering::Relaxed);
        engine.run_tasks();
        settle_network(&mut engine);
        engine.frame_at(WIDTH, HEIGHT, FrameTime::at(now()));
    }
    if let Err(difference) = compare_with_fresh(&mut engine, &mut fresh, WIDTH, HEIGHT) {
        panic!("at the end: {difference}");
    }
});
