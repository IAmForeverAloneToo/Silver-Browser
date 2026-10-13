//! Pages of frames (M33.6): a page whose `iframe`s the input makes,
//! `srcdoc` documents nested in each other with sandbox tokens, `allow`
//! and names, and scripts in each that post messages between frames,
//! navigate them, name them, add frames and remove them; loaded from a
//! `data:` URL with scripts on, its tasks run on a hand's clock and its
//! frames drawn. Nothing panics, every document stays valid, and the tab
//! keeps to its limits: [`engine::MAX_FRAMES`] frames,
//! [`engine::MAX_FRAME_DEPTH`] deep.

#![no_main]
#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use engine::differential::{frames_of, settle_network};
use engine::{Engine, FrameTime, LoadState, MAX_FRAME_DEPTH, MAX_FRAMES};
use libfuzzer_sys::fuzz_target;
use net::url::Url;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

const SANDBOX: &[&str] = &[
    "",
    "allow-scripts",
    "allow-scripts allow-same-origin",
    "allow-forms allow-top-navigation",
    "allow-popups allow-modals",
    "bogus",
];

const ALLOW: &[&str] = &[
    "",
    "local-network",
    "local-network-access 'none'",
    "camera *",
];

/// What a frame's script does, `{}` the input's number where one is.
const CALLS: &[&str] = &[
    "parent.postMessage('up', '*')",
    "top.postMessage({ n: 1 }, '*')",
    "frames.length && frames[0].postMessage('down', '*')",
    "addEventListener('message', e => document.body.append(String(e.data)))",
    "frames.length && (frames[0].location = 'about:blank')",
    "frames.length && (frames[frames.length - 1].location = 'about:blank#x')",
    "window.name = 'f' + frames.length",
    "document.querySelector('iframe') && document.querySelector('iframe').remove()",
    "var f = document.createElement('iframe'); f.srcdoc = '<p>made'; document.body.append(f)",
    "for (var i = 0; i < 12; i++) document.body.append(document.createElement('iframe'))",
    "document.body.innerHTML += '<iframe srcdoc=\"<iframe></iframe>\"></iframe>'",
    "var g = document.querySelector('iframe'); g && g.setAttribute('sandbox', '')",
    "var k = document.querySelector('iframe'); k && (k.srcdoc = '<b>again')",
    "location.href = '#here'",
    "setTimeout(() => parent.postMessage('later', '*'), 5)",
    "top.frames.length",
    "document.body.style.width = '20px'",
];

struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let Some((&first, rest)) = self.0.split_first() else {
            return 0;
        };
        self.0 = rest;
        first
    }

    fn pick<'t>(&mut self, table: &[&'t str]) -> &'t str {
        table[usize::from(self.byte()) % table.len()]
    }

    fn done(&self) -> bool {
        self.0.is_empty()
    }
}

/// `text` as an attribute's value in double quotes.
fn quoted(text: &str) -> String {
    text.replace('&', "&amp;").replace('"', "&quot;")
}

/// A document's markup: some text, up to three frames of documents of
/// their own while the nesting allows, and a script of a few calls.
fn document(input: &mut Input, depth: usize) -> String {
    let mut markup = String::from("<p>d</p>");
    let frames = if depth < 5 { input.byte() % 4 } else { 0 };
    for n in 0..frames {
        if input.done() {
            break;
        }
        let inner = document(input, depth + 1);
        let _ = write!(
            markup,
            "<iframe name=\"f{depth}{n}\" sandbox=\"{}\" allow=\"{}\" srcdoc=\"{}\"></iframe>",
            input.pick(SANDBOX),
            input.pick(ALLOW),
            quoted(&inner)
        );
    }
    if input.byte() % 2 == 0 {
        markup.push_str("<script>");
        for _ in 0..input.byte() % 4 {
            let _ = write!(markup, "try {{ {} }} catch (e) {{}}\n", input.pick(CALLS));
        }
        markup.push_str("</script>");
    }
    markup
}

static FONTS: OnceLock<text::FontCollection> = OnceLock::new();
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

fn held(engine: &Engine) {
    if let Err(problem) = engine.document().validate() {
        panic!("the document is not valid: {problem}");
    }
    let (frames, depth) = frames_of(engine);
    assert!(frames <= MAX_FRAMES, "{frames} frames");
    assert!(depth <= MAX_FRAME_DEPTH, "{depth} deep");
}

fuzz_target!(|data: &[u8]| {
    NOW.store(0, Ordering::Relaxed);
    let mut input = Input(data);
    let page = document(&mut input, 0);
    let mut url = String::from("data:text/html,");
    for byte in page.bytes() {
        let _ = write!(url, "%{byte:02X}");
    }
    let url = Url::parse(&url).expect("a data: URL");
    let mut engine = Engine::with_fonts(fonts().clone());
    engine.set_scripting(true);
    engine.set_clock(Arc::new(now));
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
    for _ in 0..4 {
        settle_network(&mut engine);
        held(&engine);
        engine.frame_at(WIDTH, HEIGHT, FrameTime::at(now()));
        NOW.fetch_add(u64::from(input.byte()) * 4, Ordering::Relaxed);
        engine.run_tasks();
    }
    settle_network(&mut engine);
    held(&engine);
});
