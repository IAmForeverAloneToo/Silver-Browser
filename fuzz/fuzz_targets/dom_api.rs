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

#![no_main]

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::sync::mpsc::channel;
use std::time::Duration;

use engine::differential::{compare_with_fresh, run_script};
use engine::{Engine, LoadState};
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

/// The pages a sequence starts in.
const PAGES: &[&str] = &[
    "<!DOCTYPE html><p id=a class=c>One <b>two</b></p><div><span>three</span></div>",
    "<!DOCTYPE html><style>.c { color: #f00; margin-left: 4px } p + p { display: none }</style>\
     <table><tr><td>1<td>2</table><ul><li>a<li class=c>b</ul><p>x</p><p>y</p>",
    "<!DOCTYPE html><div style='width: 30px; float: left'>f</div><pre>a\nb</pre>\
     <template><p>t</p></template><noscript><p>n</p></noscript>",
    "<p>quirks<table><tr><td>cell</td></tr></table>",
];

// What the calls are given: the strings of these tables, or the input's.
const TAGS: &[&str] = &[
    "div", "p", "span", "b", "table", "tbody", "tr", "td", "ul", "li", "a", "img", "br", "pre",
    "style", "script", "template", "noscript", "select", "option", "h1", "svg", "x-y",
];
const ATTRIBUTES: &[&str] = &[
    "id", "class", "style", "title", "hidden", "dir", "lang", "data-x", "colspan", "width",
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
];
const MARKUP: &[&str] = &[
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

/// One call, read from `input`, as a statement.
fn call(input: &mut Input) -> String {
    let op = input.byte() % 34;
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
        _ => format!(
            "var s = document.createElement('script'); s.textContent = {}; {a}.appendChild(s)",
            input.pick(SCRIPTS)
        ),
    }
}

/// The page: one of `PAGES`, and the calls in a script at its end.
/// The page: one of `PAGES`, and a script at its end that keeps the
/// nodes the calls name and makes up to fifteen of them, as the input's
/// next byte says.
fn page(input: &mut Input) -> String {
    let mut page = PAGES[usize::from(input.byte()) % PAGES.len()].to_owned();
    let first = input.byte() % 16;
    page.push_str(
        "<script>var h = [document, document.documentElement, document.body];\
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
        let _ = writeln!(script, "try {{ {} }} catch (e) {{}}", call(input));
    }
    script
}

static FONTS: OnceLock<text::FontCollection> = OnceLock::new();

fn fonts() -> &'static text::FontCollection {
    FONTS.get_or_init(|| {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    })
}

/// A new engine over the bundled font, with scripts on: the one under
/// test, and the fresh one, whose `noscript` is shown as the other's.
fn scripting() -> Engine {
    let mut engine = Engine::with_fonts(fonts().clone());
    engine.set_scripting(true);
    engine
}

fuzz_target!(|data: &[u8]| {
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
        made += RUN;
    }
    if let Err(difference) = compare_with_fresh(&mut engine, &mut fresh, WIDTH, HEIGHT) {
        panic!("at the end: {difference}");
    }
});
