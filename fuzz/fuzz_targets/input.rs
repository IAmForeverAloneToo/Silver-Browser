//! Pages of controls used through the engine's input (M26): the
//! pointer's moves, presses and releases, keys, text typed, pasted, cut
//! and copied, the wheel, the focus moved by Tab and the window's, the
//! modifiers held, and the engine's clock moved on between them, a
//! hand's, so that timers run. The input's first byte picks the page and
//! whether scripts run, whose listeners change the document as the
//! input reaches them; the rest is read as steps, each followed by a
//! frame as the shell draws one after an input. Nothing panics; the
//! document stays valid (`Document::validate`); and the frame drawn
//! every eighth step and at the end shows what a fresh engine computes
//! for the document as it then stands, with the state no markup holds
//! (the controls', the focus, the pointer's hover, an open list) given
//! to it: the input's changes reach the frame through the invalidation.

#![no_main]
#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use engine::differential::compare_with_fresh;
use engine::{Button, Engine, FrameTime, Key, LoadState, Modifiers};
use libfuzzer_sys::fuzz_target;
use net::url::Url;

/// The most steps one input takes.
const MAX_STEPS: usize = 96;
/// The frame: room for a few controls, small enough to keep the fuzzer
/// quick.
const WIDTH: u32 = 160;
const HEIGHT: u32 = 120;

/// The pages, each of controls and what styles them by their state; the
/// scripts run only with scripts on.
const PAGES: &[&str] = &[
    "<!DOCTYPE html><style>body { margin: 0; font-size: 10px } :focus-visible { outline: 2px solid #00f }\
     :checked + label { color: #f00 } :user-invalid { background: #fcc } :hover { color: #080 }\
     :active { color: #800 } :focus-within { padding-left: 1px }</style>\
     <form><input name=q required pattern='[a-z]*' maxlength=6 placeholder=find>\
     <input type=checkbox id=c name=c><label for=c>c</label>\
     <input type=radio name=r value=1><input type=radio name=r value=2 checked>\
     <select name=s><option>one<option selected>two<option disabled>three</select>\
     <button>go</button></form><a href=#t>t</a><p id=t>target</p>",
    "<!DOCTYPE html><style>body { margin: 0; font-size: 10px } textarea { width: 60px; height: 30px }\
     div { height: 40px; overflow: auto } :invalid { border-color: #f00 }</style>\
     <textarea maxlength=40>one\ntwo\nthree four five six</textarea>\
     <select multiple size=3><optgroup label=g><option>a<option>b</optgroup><option>c<option>d</select>\
     <div><input type=number min=1 max=9 step=2 value=3><br><input type=email multiple>\
     <br><input readonly value=r><br><input disabled value=d><br><button disabled>no</button></div>",
    "<!DOCTYPE html><style>body { margin: 0; font-size: 10px } li.done { text-decoration: line-through }\
     </style><input id=new autofocus><button id=add>add</button><ul id=list><li>first</ul>\
     <form id=f novalidate><input name=a required><input type=submit formnovalidate></form>\
     <script>\
     var list = document.getElementById('list'), field = document.getElementById('new');\
     function add() { var li = document.createElement('li'); li.textContent = field.value;\
       li.onclick = function () { li.classList.toggle('done'); }; list.appendChild(li); field.value = ''; }\
     document.getElementById('add').addEventListener('click', add);\
     field.addEventListener('keydown', function (e) { if (e.key === 'Enter') add();\
       if (e.key === 'x') e.preventDefault(); });\
     field.addEventListener('input', function () { document.title = field.value; });\
     list.addEventListener('dblclick', function (e) { e.target.remove(); });\
     document.getElementById('f').addEventListener('submit', function (e) { e.preventDefault();\
       setTimeout(function () { list.firstChild && list.firstChild.remove(); }, 10); });\
     document.addEventListener('focusin', function (e) { e.target.style.color = '#00f'; });\
     document.addEventListener('pointerdown', function (e) { if (e.button === 2) e.preventDefault(); });\
     requestAnimationFrame(function step() { list.style.paddingLeft = (list.children.length % 4) + 'px'; });\
     </script>",
    "<!DOCTYPE html><style>body { margin: 0; font-size: 10px } input { display: block }\
     .hide { display: none }</style>\
     <form id=g onsubmit='return false'><input id=a value=abc><input id=b type=checkbox>\
     <select id=s onchange='this.nextElementSibling.classList.toggle(\"hide\")'><option>1<option>2</select>\
     <input id=c><textarea id=d></textarea></form>\
     <script>\
     document.getElementById('a').addEventListener('input', function (e) {\
       if (e.target.value.length > 4) e.target.remove(); });\
     document.getElementById('b').addEventListener('click', function (e) {\
       document.getElementById('c').disabled = e.target.checked; });\
     document.getElementById('c').addEventListener('focus', function () {\
       document.getElementById('d').focus(); });\
     document.getElementById('d').addEventListener('beforeinput', function (e) {\
       if (e.data === 'a') e.preventDefault(); });\
     document.getElementById('g').addEventListener('keyup', function (e) {\
       if (e.key === 'Escape') document.getElementById('s').hidden = true; });\
     setInterval(function () { var s = document.getElementById('s');\
       s.selectedIndex = (s.selectedIndex + 1) % 2; }, 50);\
     </script>",
];

/// The keys pressed: name, code and the text each types.
const KEYS: &[(&str, &str, Option<&str>)] = &[
    ("Tab", "Tab", None),
    ("Enter", "Enter", None),
    (" ", "Space", Some(" ")),
    ("a", "KeyA", Some("a")),
    ("x", "KeyX", Some("x")),
    ("A", "KeyA", Some("A")),
    ("1", "Digit1", Some("1")),
    ("Backspace", "Backspace", None),
    ("Delete", "Delete", None),
    ("ArrowLeft", "ArrowLeft", None),
    ("ArrowRight", "ArrowRight", None),
    ("ArrowUp", "ArrowUp", None),
    ("ArrowDown", "ArrowDown", None),
    ("Home", "Home", None),
    ("End", "End", None),
    ("PageUp", "PageUp", None),
    ("PageDown", "PageDown", None),
    ("Escape", "Escape", None),
    ("Shift", "ShiftLeft", None),
    ("F5", "F5", None),
];

/// Text typed, pasted or committed by an input method.
const TEXTS: &[&str] = &[
    "a",
    "abc",
    "Z9",
    " ",
    "line\nbreak",
    "\u{e9}\u{301}",
    "\u{1f600}",
    "",
];

const BUTTONS: [Button; 5] = [
    Button::Primary,
    Button::Primary,
    Button::Secondary,
    Button::Auxiliary,
    Button::Back,
];

/// The input, read a byte at a time; past its end, zeros.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    fn done(&self) -> bool {
        self.0.is_empty()
    }

    /// A point in the viewport, or a little past its edges.
    fn point(&mut self) -> (f32, f32) {
        let x = f32::from(self.byte()) * (WIDTH as f32 + 16.0) / 256.0 - 8.0;
        let y = f32::from(self.byte()) * (HEIGHT as f32 + 16.0) / 256.0 - 8.0;
        (x, y)
    }

    fn button(&mut self) -> Button {
        BUTTONS[usize::from(self.byte()) % BUTTONS.len()]
    }

    fn key(&mut self) -> Key {
        let (key, code, text) = KEYS[usize::from(self.byte()) % KEYS.len()];
        let key = Key::new(key, code);
        match text {
            Some(text) => key.with_text(text),
            None => key,
        }
    }

    /// A string from `TEXTS`, or of up to 7 characters from the input.
    fn text(&mut self) -> String {
        let length = self.byte();
        if length < 224 {
            return TEXTS[usize::from(length) % TEXTS.len()].to_owned();
        }
        (0..length % 8)
            .filter_map(|_| {
                char::from_u32(u32::from(u16::from_le_bytes([self.byte(), self.byte()])))
            })
            .collect()
    }
}

/// One step, read from `input`.
fn step(engine: &mut Engine, input: &mut Input) {
    match input.byte() % 16 {
        0 | 1 => {
            let (x, y) = input.point();
            engine.pointer_move(x, y);
        }
        2 => {
            let (x, y) = input.point();
            let (button, count) = (input.button(), u32::from(input.byte() % 3) + 1);
            engine.pointer_down(x, y, button, count);
        }
        3 => {
            let (x, y) = input.point();
            let (button, count) = (input.button(), u32::from(input.byte() % 3) + 1);
            engine.pointer_up(x, y, button, count);
        }
        4 | 5 => {
            // A click: a move, a press and a release at one point.
            let (x, y) = input.point();
            let count = u32::from(input.byte() % 2) + 1;
            engine.pointer_move(x, y);
            engine.pointer_down(x, y, Button::Primary, count);
            engine.pointer_up(x, y, Button::Primary, count);
        }
        6 | 7 => {
            let key = input.key();
            engine.key_down(&key);
            engine.key_up(&key);
        }
        8 => {
            let mut key = input.key();
            key.repeat = input.byte() % 2 == 0;
            engine.key_down(&key);
        }
        9 => {
            let key = input.key();
            engine.key_up(&key);
        }
        10 => {
            let text = input.text();
            match input.byte() % 4 {
                0 => {
                    engine.insert_text(&text);
                }
                1 => {
                    engine.paste(&text);
                }
                2 => {
                    let _ = engine.cut();
                }
                _ => {
                    let _ = engine.copy();
                }
            }
        }
        11 => {
            let (x, y) = input.point();
            let dy = (f32::from(input.byte()) - 128.0) / 2.0;
            engine.wheel(x, y, dy);
        }
        12 => {
            let backward = input.byte() % 2 == 0;
            engine.focus_next(backward);
        }
        13 => {
            let bits = input.byte();
            engine.set_window_focus(bits & 16 == 0);
            engine.set_modifiers(Modifiers {
                shift: bits & 1 != 0,
                ctrl: bits & 2 != 0,
                alt: bits & 4 != 0,
                meta: bits & 8 != 0,
            });
        }
        14 => {
            // Time passes, up to a second: the timers then due run.
            NOW.fetch_add(u64::from(input.byte()) * 4, Ordering::Relaxed);
            engine.run_tasks();
        }
        _ => match input.byte() % 3 {
            0 => engine.pointer_leave(),
            1 => engine.scroll_by(f32::from(input.byte()) - 128.0),
            _ => {
                let _ = engine.cursor_at(f32::from(input.byte()), f32::from(input.byte()));
            }
        },
    }
}

static FONTS: OnceLock<text::FontCollection> = OnceLock::new();

/// The engines' clock, in milliseconds: moved by hand.
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

/// A new engine over the bundled font with the hand's clock, scripts on
/// or off: the one under test, and the fresh one, whose `noscript` is
/// shown as the other's.
fn new_engine(scripting: bool) -> Engine {
    let mut engine = Engine::with_fonts(fonts().clone());
    engine.set_scripting(scripting);
    engine.set_clock(Arc::new(now));
    engine
}

fuzz_target!(|data: &[u8]| {
    NOW.store(0, Ordering::Relaxed);
    let mut input = Input(data);
    let first = input.byte();
    let page = PAGES[usize::from(first >> 1) % PAGES.len()];
    let scripting = first & 1 == 0;
    let mut url = String::from("data:text/html,");
    for byte in page.bytes() {
        let _ = write!(url, "%{byte:02X}");
    }
    let url = Url::parse(&url).expect("a data: URL");
    let mut engine = new_engine(scripting);
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
    engine.frame_at(WIDTH, HEIGHT, FrameTime::at(now()));
    let mut fresh = new_engine(scripting);
    for n in 1..=MAX_STEPS {
        if input.done() {
            break;
        }
        step(&mut engine, &mut input);
        engine.frame_at(WIDTH, HEIGHT, FrameTime::at(now()));
        if let Err(problem) = engine.document().validate() {
            panic!("after step {n}: the document is not valid: {problem}");
        }
        if n % 8 == 0 {
            if let Err(difference) = compare_with_fresh(&mut engine, &mut fresh, WIDTH, HEIGHT) {
                panic!("after step {n}: {difference}");
            }
        }
    }
    if let Err(difference) = compare_with_fresh(&mut engine, &mut fresh, WIDTH, HEIGHT) {
        panic!("at the end: {difference}");
    }
});
