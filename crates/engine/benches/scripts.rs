//! M24's: what scripts cost (docs/design/scripting.md, "Order of work",
//! step 11). A realm made, as a document's first script makes it; a
//! script of two million additions run in one, and a thousand console
//! calls; and a page of 1,000 paragraphs with 100 inline scripts and 10
//! external ones, served by `net`'s test server on 127.0.0.1, from
//! `Engine::navigate` to its first frame, with scripts on and off, the
//! page streamed and parsed as it arrives (M24.8) either way.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench scripts
//! ```

use std::hint::black_box;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{Engine, FrameTime, LoadState};
use js::{ConsoleLevel, Limits, QuickJs, Script, ScriptEngine, ScriptingHost};
use net::test_server::{Reply, TestServer};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const PARAGRAPHS: usize = 1000;
const INLINE: usize = 100;
const EXTERNAL: usize = 10;
const RUNS: usize = 20;

/// A host that counts what it is asked, as the engine's would answer.
#[derive(Default)]
struct Host {
    lines: usize,
    title: String,
}

impl ScriptingHost for Host {
    fn console(&mut self, _: ConsoleLevel, _: String) {
        self.lines += 1;
    }

    fn title(&mut self) -> String {
        self.title.clone()
    }

    fn set_title(&mut self, title: String) {
        self.title = title;
    }
}

fn script(source: &str) -> Script<'_> {
    Script {
        source,
        url: "https://example.com/bench.js",
        line: 1,
        muted: false,
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The median and range of `times`, in milliseconds.
fn spread(times: &mut [f64]) -> String {
    times.sort_by(f64::total_cmp);
    format!(
        "median {:.2} ms, {:.2}-{:.2} ms",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    )
}

/// The page: every tenth paragraph followed by an inline script that
/// sets the title, and an external script after every hundredth.
fn page() -> String {
    let mut source = String::from("<!DOCTYPE html><html><head><title>Scripts</title></head><body>");
    for i in 0..PARAGRAPHS {
        source.push_str(&format!(
            "<p>Paragraph {i}. The quick brown fox jumps over the lazy dog.</p>"
        ));
        if i % (PARAGRAPHS / INLINE) == 0 {
            source.push_str(&format!(
                "<script>document.title = 'Paragraph {i}';</script>"
            ));
        }
        if i % (PARAGRAPHS / EXTERNAL) == 0 {
            source.push_str(&format!(
                "<script src=/s{}.js></script>",
                i / (PARAGRAPHS / EXTERNAL)
            ));
        }
    }
    source.push_str("</body></html>");
    source
}

/// A new engine over the bundled font, and the channel its waker sends
/// on.
fn engine(scripting: bool) -> (Engine, Receiver<()>) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(scripting);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    (engine, woken)
}

/// One load: from `navigate` to the first frame once loaded.
fn load(server: &TestServer, scripting: bool) -> Duration {
    let (mut engine, woken) = engine(scripting);
    let start = Instant::now();
    engine.navigate(server.url("/page.html"));
    while *engine.load_state() != LoadState::Loaded {
        woken
            .recv_timeout(Duration::from_secs(30))
            .expect("the page loads");
        engine.poll_network();
    }
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(Duration::ZERO)));
    let took = start.elapsed();
    let title = if scripting {
        format!("Paragraph {}", PARAGRAPHS - PARAGRAPHS / INLINE)
    } else {
        "Scripts".to_owned()
    };
    assert_eq!(engine.title(), title);
    took
}

fn main() {
    let mut made = Vec::new();
    for _ in 0..RUNS {
        let start = Instant::now();
        let realm = QuickJs
            .realm::<Host>(Limits::default())
            .expect("a realm is made");
        made.push(ms(start.elapsed()));
        black_box(realm.heap());
    }
    println!("a realm made: {}", spread(&mut made));

    let mut realm = QuickJs
        .realm::<Host>(Limits::default())
        .expect("a realm is made");
    let mut host = Host::default();
    for (what, source) in [
        (
            "two million additions",
            "{ let s = 0; for (let i = 0; i < 2000000; i++) s += i; s }",
        ),
        (
            "a thousand console calls",
            "for (let i = 0; i < 1000; i++) console.log('line', i, { at: i });",
        ),
    ] {
        let mut ran = Vec::new();
        for _ in 0..RUNS {
            let start = Instant::now();
            black_box(realm.run(&script(source), &mut host).expect("it runs"));
            assert!(realm.checkpoint(&mut host).is_empty());
            ran.push(ms(start.elapsed()));
        }
        println!("{what}: {}", spread(&mut ran));
    }
    assert_eq!(host.lines, 1000 * RUNS);

    let html = page();
    let server = {
        let html = html.clone();
        TestServer::plain(move |request| match request.target.as_str() {
            "/page.html" => Reply::ok("text/html; charset=utf-8", html.as_bytes()),
            external if external.starts_with("/s") => {
                Reply::ok("text/javascript", b"console.log('external');")
            }
            _ => Reply::with_fields(404, &[], b""),
        })
    };
    println!(
        "a page of {PARAGRAPHS} paragraphs, {} bytes, with {INLINE} inline scripts and {EXTERNAL} \
         external ones, to its first frame:",
        html.len()
    );
    for scripting in [false, true] {
        let mut loads: Vec<f64> = (0..RUNS / 2)
            .map(|_| ms(load(&server, scripting)))
            .collect();
        let on = if scripting { "on" } else { "off" };
        println!("  scripts {on}: {}", spread(&mut loads));
    }
}
