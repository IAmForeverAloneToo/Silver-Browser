//! M25.3: what a script's change to the document costs the next frame
//! (docs/design/dom-scripting.md, "Mutations reach the frame"). A page
//! served by `net`'s test server on 127.0.0.1, with scripts on, whose
//! links run its functions as a click on them would (`javascript:` URLs,
//! M24): one builds a table of rows of four cells, one toggles a class
//! on the table that changes its cells' padding and colour, and one
//! edits one cell's text, in its middle row. Each is followed by a frame
//! at 1280 x 800, whose milliseconds are reported whole and stage by
//! stage (`Engine::frame_stats`, M13.4), with the frames over the budget
//! and what the incremental engine did (M13.1, M13.2); the script's own
//! run, from the click to its changes taken, is reported apart. Two
//! tables: of 100 rows and of 1,000.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench dom
//! ```

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime, LoadState};
use net::test_server::{Reply, TestServer};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const ROWS: [usize; 2] = [100, 1000];
const BUILDS: usize = 5;
const CHANGES: usize = 40;

/// The page: three links, one for each change, side by side at the top
/// left, and the script whose functions they call; `ROWS` the rows the
/// table is built with.
const PAGE: &str = "<!DOCTYPE html><html><head><title>DOM</title><style>\
    body { margin: 0; font: 14px sans-serif; }\
    #links a { display: inline-block; width: 80px; height: 20px; }\
    td { padding: 1px 2px; border: 1px solid #ccc; }\
    table.wide td { padding: 3px 8px; background-color: #eef; }\
    </style></head><body><div id=links><a href='javascript:build()'>build</a><a \
    href='javascript:toggle()'>toggle</a><a href='javascript:edit()'>edit</a></div>\
    <script>\
    var rows = ROWS;\
    function build() {\
      var table = document.createElement('table');\
      var body = table.appendChild(document.createElement('tbody'));\
      for (var i = 0; i < rows; i++) {\
        var row = body.appendChild(document.createElement('tr'));\
        for (var j = 0; j < 4; j++) {\
          var cell = row.appendChild(document.createElement('td'));\
          cell.textContent = 'Row ' + i + ', cell ' + j;\
        }\
      }\
      document.body.appendChild(table);\
    }\
    function toggle() { document.querySelector('table').classList.toggle('wide'); }\
    var edits = 0;\
    function edit() {\
      edits++;\
      var row = 'tr:nth-child(' + (rows >> 1) + ') td';\
      document.querySelector(row).textContent = 'Edited ' + edits;\
    }\
    </script></body></html>";

/// Where each link is: the middle of its 80 x 20 box.
const BUILD: (f32, f32) = (40.0, 10.0);
const TOGGLE: (f32, f32) = (120.0, 10.0);
const EDIT: (f32, f32) = (200.0, 10.0);

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The engine's counters of incremental work.
#[derive(Clone, Copy)]
struct Work {
    restyled: u64,
    laid_out: u64,
    reused: u64,
    layouts: u64,
    cascades: u64,
}

impl Work {
    fn of(engine: &Engine) -> Self {
        Work {
            restyled: engine.restyled_elements(),
            laid_out: engine.boxes_laid_out(),
            reused: engine.boxes_reused(),
            layouts: engine.layout_runs(),
            cascades: engine.style_runs(),
        }
    }

    /// What was done between `self` and `later`, as one line, per
    /// change over `changes`.
    fn since(self, later: Work, changes: usize) -> String {
        let per = |a: u64, b: u64| (b - a) as usize / changes;
        format!(
            "{} layouts, {} whole cascades, {} element styles restyled, \
             {} boxes laid out, {} taken from the cache, a change",
            per(self.layouts, later.layouts),
            per(self.cascades, later.cascades),
            per(self.restyled, later.restyled),
            per(self.laid_out, later.laid_out),
            per(self.reused, later.reused)
        )
    }
}

/// Prints the median and worst of `times`, in milliseconds.
fn spread(name: &str, times: &mut [f64]) {
    times.sort_by(f64::total_cmp);
    println!(
        "  {name}: median {:.2} ms, worst {:.2} ms",
        times[times.len() / 2],
        times[times.len() - 1]
    );
}

/// Prints the median and worst of each stage over `stats`, and the
/// frames over budget.
fn report(stats: &[FrameStats]) {
    let stage = |name: &str, of: fn(&FrameStats) -> Duration| {
        let mut times: Vec<f64> = stats.iter().map(|s| ms(of(s))).collect();
        spread(name, &mut times);
    };
    stage("style", |s| s.style);
    stage("layout", |s| s.layout);
    stage("paint", |s| s.paint);
    stage("raster", |s| s.raster);
    stage("frame", |s| s.total);
    println!(
        "  over the {:.1} ms budget: {} of {}",
        ms(FRAME_BUDGET),
        stats.iter().filter(|s| s.overran).count(),
        stats.len()
    );
}

/// The page of `rows` loaded, scripts on, and its first frame drawn.
fn loaded(server: &TestServer, rows: usize) -> (Engine, Receiver<()>) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(true);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(server.url(&format!("/{rows}.html")));
    while *engine.load_state() != LoadState::Loaded {
        woken
            .recv_timeout(Duration::from_secs(30))
            .expect("the page loads");
        engine.poll_network();
    }
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(Duration::ZERO)));
    (engine, woken)
}

/// A click on the link at `at`, timed from the click to the script's
/// changes taken.
fn click(engine: &mut Engine, at: (f32, f32)) -> Duration {
    let start = Instant::now();
    let link = engine.link_at(at.0, at.1).expect("a link is there");
    engine.follow_link(&link);
    start.elapsed()
}

fn main() {
    let server = TestServer::plain(|request| {
        match ROWS
            .iter()
            .find(|rows| request.target == format!("/{rows}.html"))
        {
            Some(rows) => Reply::ok(
                "text/html; charset=utf-8",
                PAGE.replace("ROWS", &rows.to_string()).as_bytes(),
            ),
            None => Reply::with_fields(404, &[], b""),
        }
    });
    let mut frame = 0u32;
    let mut draw = |engine: &mut Engine| {
        frame += 1;
        black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * frame)));
        engine.frame_stats()
    };

    for rows in ROWS {
        println!("a table of {rows} rows of four cells built by a script, {BUILDS} times:");
        let mut runs = Vec::new();
        let mut stats = Vec::new();
        let mut work = Vec::new();
        let mut engine = None;
        for _ in 0..BUILDS {
            let (mut fresh, _woken) = loaded(&server, rows);
            let before = Work::of(&fresh);
            runs.push(ms(click(&mut fresh, BUILD)));
            stats.push(draw(&mut fresh));
            work.push(before.since(Work::of(&fresh), 1));
            assert_eq!(
                fresh
                    .document()
                    .subtree(fresh.document().root())
                    .filter(|&node| fresh.document().tag(node) == Some("td"))
                    .count(),
                rows * 4
            );
            engine = Some(fresh);
        }
        spread("the script", &mut runs);
        report(&stats);
        println!("  {}", work[0]);
        let mut engine = engine.expect("built");

        for (what, at) in [
            ("the table's class toggled", TOGGLE),
            ("one cell's text edited", EDIT),
        ] {
            println!("{what}, {CHANGES} times, a frame after each:");
            let mut runs = Vec::new();
            let mut stats = Vec::new();
            let before = Work::of(&engine);
            for _ in 0..CHANGES {
                runs.push(ms(click(&mut engine, at)));
                stats.push(draw(&mut engine));
            }
            spread("the script", &mut runs);
            report(&stats);
            println!("  {}", before.since(Work::of(&engine), CHANGES));
        }
    }
}
