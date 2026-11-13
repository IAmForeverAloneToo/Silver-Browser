//! The site sweep's capture of one site (the scoreboard, 2026-11-13):
//! `silver-browser --sweep-site <URL> --sweep-out <dir>` loads the site
//! as the browser does, in a tab of renderer processes, sandboxed, on
//! the network, with no window and a context of its own that keeps
//! nothing, and writes what it saw to `<dir>`: `silver.json`, and the
//! page at rest and three screens down as `silver-top.png` and
//! `silver-scrolled.png`. `tools/sweep/sweep.mjs` runs it beside the
//! reference browser.
//!
//! Every permission a page asks for is denied and every dialog
//! cancelled; nothing is typed and no link is followed. A site has
//! [`CAP`] in all; it has settled when no layout has run for
//! [`SETTLE`]; its renderer has hung when it has answered nothing for
//! [`HUNG`], a renderer's thread being what answers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use devtools::Json;
use engine::service::RemoteTab;
use engine::{ConsoleLevel, ConsoleSource, FrameTime, LoadError, LoadState, Url, View};

use crate::platform;

/// The viewport the sweep loads at, the reference browser's alike.
pub(crate) const SIZE: (u32, u32) = (1280, 800);
/// A site's time in all.
pub(crate) const CAP: Duration = Duration::from_secs(30);
/// No layout for this long: the page has settled.
pub(crate) const SETTLE: Duration = Duration::from_secs(2);
/// Nothing answered for this long: the renderer has hung.
pub(crate) const HUNG: Duration = Duration::from_secs(10);
/// How far the second screenshot is, in screens.
const SCREENS_DOWN: f32 = 3.0;
/// The most console entries kept, and characters of each.
const MAX_ENTRIES: usize = 500;
const MAX_TEXT: usize = 300;

/// What the command line asked of the sweep.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Asked {
    pub(crate) url: Url,
    pub(crate) out: PathBuf,
    /// The self-test's: renderers that crash as they start.
    pub(crate) crash_renderers: bool,
}

/// `--sweep-site <URL> --sweep-out <dir> [--sweep-crash-renderers]`, or
/// `None` when the command line is not the sweep's.
pub(crate) fn asked(args: &[String]) -> Option<Result<Asked, String>> {
    let at = args.iter().position(|a| a == "--sweep-site")?;
    let url = match args.get(at + 1).map(|u| Url::parse(u)) {
        Some(Ok(url)) => url,
        _ => return Some(Err("--sweep-site takes a URL".to_owned())),
    };
    let out = match args.iter().position(|a| a == "--sweep-out") {
        Some(o) => match args.get(o + 1) {
            Some(dir) => PathBuf::from(dir),
            None => return Some(Err("--sweep-out takes a folder".to_owned())),
        },
        None => return Some(Err("--sweep-site needs --sweep-out <folder>".to_owned())),
    };
    let crash_renderers = args.iter().any(|a| a == "--sweep-crash-renderers");
    Some(Ok(Asked {
        url,
        out,
        crash_renderers,
    }))
}

/// What became of the site.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Ok,
    Crash(Option<String>),
    Hang,
    LoadError(String),
}

/// Loads `asked.url` and writes what came of it; the process's exit
/// code.
pub(crate) fn run(asked: &Asked) -> i32 {
    match capture(asked) {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("silver-browser: the sweep: {why}");
            let record = Json::object([
                ("outcome", Json::from("harness error")),
                ("error", Json::from(why)),
            ]);
            let _ = std::fs::create_dir_all(&asked.out);
            let _ = std::fs::write(asked.out.join("silver.json"), record.write());
            1
        }
    }
}

fn capture(asked: &Asked) -> Result<(), String> {
    std::fs::create_dir_all(&asked.out).map_err(|e| format!("no folder: {e}"))?;
    let crashes = asked.out.join("crashes");
    let mut launcher = platform::ProcessLauncher::new()
        .map_err(|e| format!("no renderer can be started: {e}"))?
        .with_crashes(Some(crashes.clone()));
    if asked.crash_renderers {
        launcher = launcher.with_arguments(vec!["--crash-now".into(), "panic".into()]);
    }
    text::FontCollection::scan_system();
    let fonts = text::FontCollection::system().map_err(|e| format!("no fonts: {e}"))?;
    // A context of its own, which keeps nothing: no profile is read or
    // written, and the browser's own folder is not touched.
    let context = engine::Context::new();
    let mut tab = RemoteTab::in_context(Arc::new(launcher), fonts, &context);
    let (wake, woken) = mpsc::channel();
    tab.set_waker(Arc::new(move || {
        let _ = wake.send(());
    }));
    let began = Instant::now();
    tab.set_clock(Arc::new(move || began.elapsed()));
    tab.set_scripting(true);
    tab.set_scale_factor(1.0);
    tab.set_viewport(SIZE.0, SIZE.1);
    // The raster's size, which the renderers draw at, is told by a frame.
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    tab.navigate(asked.url.clone());

    let mut console = Vec::new();
    let mut first_paint = None;
    let mut load = None;
    let mut settled = None;
    let mut counters = Json::Null;
    // The layouts the renderer last said it had run, and since when.
    let mut layouts: Option<(f64, Instant)> = None;
    // When a renderer last answered, or was first seen to run.
    let mut answered: Option<Instant> = None;
    let mut last_asked = began;
    let mut next_id = 1u64;
    let mut outcome = Outcome::Ok;
    let mut peaks = Peaks::default();
    while began.elapsed() < CAP {
        let _ = woken.recv_timeout(Duration::from_millis(50));
        step(&mut tab, began, &mut console);
        // The page's own frame, not the blank one the tab began with.
        let page = tab.url().is_some_and(|url| url.scheme() != "about");
        if first_paint.is_none() && page && tab.has_page_frame() {
            first_paint = Some(began.elapsed());
        }
        match tab.load_state() {
            LoadState::Loaded if load.is_none() => load = Some(began.elapsed()),
            LoadState::Failed(LoadError::Lost) => {
                outcome = Outcome::Crash(stopped(&mut tab, &crashes));
                break;
            }
            LoadState::Failed(error) => {
                outcome = Outcome::LoadError(error.to_string());
                break;
            }
            _ => {}
        }
        peaks.sample(&tab);
        // The renderer's counts, asked for every quarter second: an
        // answer says its thread runs, and its layouts whether the page
        // has settled.
        for message in tab.take_inspected() {
            let Ok(answer) = Json::parse(&message) else {
                continue;
            };
            if let Some(result) = answer.get("result") {
                answered = Some(Instant::now());
                let count = result.get("layouts").and_then(Json::as_f64).unwrap_or(0.0);
                layouts = match layouts {
                    Some((last, since)) if last == count => Some((last, since)),
                    _ => Some((count, Instant::now())),
                };
                counters = result.clone();
            }
        }
        if tab.renderer_process().is_some() {
            answered.get_or_insert_with(Instant::now);
            if last_asked.elapsed() >= Duration::from_millis(250) {
                tab.inspect(format!("{{\"id\":{next_id},\"method\":\"page.sweep\"}}"));
                next_id += 1;
                last_asked = Instant::now();
            }
        }
        if answered.is_some_and(|at| at.elapsed() > HUNG) {
            outcome = Outcome::Hang;
            break;
        }
        if let (Some(_), Some(_), Some((_, since))) = (first_paint, load, layouts)
            && since.elapsed() >= SETTLE
        {
            // Settled once loaded: layouts that stopped before the load
            // (the counts of the view the tab showed before) settle it at
            // the load.
            let load = load.unwrap_or_default();
            settled = Some(since.duration_since(began).max(load));
            break;
        }
    }
    if outcome == Outcome::Ok && first_paint.is_none() {
        outcome = Outcome::Hang;
    }
    let mut shots = Vec::new();
    if matches!(outcome, Outcome::Ok | Outcome::Hang) {
        shot(&mut tab, began, &asked.out.join("silver-top.png"))?;
        shots.push("top");
    }
    if outcome == Outcome::Ok {
        tab.scroll_by(SCREENS_DOWN * SIZE.1 as f32);
        let until = Instant::now() + Duration::from_secs(6);
        while Instant::now() < until {
            let _ = woken.recv_timeout(Duration::from_millis(50));
            step(&mut tab, began, &mut console);
        }
        shot(&mut tab, began, &asked.out.join("silver-scrolled.png"))?;
        shots.push("scrolled");
    }
    peaks.sample(&tab);
    let network = tab
        .network_log()
        .map_or(Json::Null, |log| log.sweep_summary());
    let (name, error, dump) = match &outcome {
        Outcome::Ok => ("ok", None, None),
        Outcome::Crash(dump) => ("renderer crash", None, dump.clone()),
        Outcome::Hang => ("hang", None, None),
        Outcome::LoadError(why) => ("load error", Some(why.clone()), None),
    };
    let millis =
        |at: Option<Duration>| at.map_or(Json::Null, |at| Json::from(at.as_millis() as f64));
    let record = Json::object([
        ("outcome", Json::from(name)),
        ("error", Json::from(error)),
        ("dump", Json::from(dump)),
        ("url", Json::from(asked.url.to_string())),
        // Where the site sent the tab, its redirects followed: the
        // reference browser may be sent elsewhere (another country's).
        ("finalUrl", Json::from(tab.url().map(Url::to_string))),
        (
            "timings",
            Json::object([
                ("firstPaint", millis(first_paint)),
                ("load", millis(load)),
                ("settled", millis(settled)),
            ]),
        ),
        ("console", Json::Array(console)),
        ("counters", counters),
        ("network", network),
        ("memory", peaks.json()),
        (
            "screenshots",
            Json::Array(shots.into_iter().map(Json::from).collect()),
        ),
    ]);
    std::fs::write(asked.out.join("silver.json"), record.write())
        .map_err(|e| format!("not written: {e}"))?;
    // The renderers end with the tab, and the context keeps nothing.
    drop(tab);
    Ok(())
}

/// Takes what the renderers said, runs what is due, refuses what the
/// page asks for, draws a frame when one is wanted, and keeps the
/// console.
fn step(tab: &mut RemoteTab, began: Instant, console: &mut Vec<Json>) {
    tab.poll_network();
    tab.run_tasks();
    refuse_prompts(tab);
    if tab.needs_frame() {
        tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(began.elapsed()));
    }
    take_console(tab, console);
}

/// Denies every permission a page asks for and cancels every dialog.
fn refuse_prompts(tab: &mut RemoteTab) {
    if let Some(prompt) = tab.permission_prompt() {
        tab.answer_prompt(prompt.id, Some(false));
    }
    if let Some(dialog) = tab.dialog() {
        tab.answer_dialog(dialog.id, None);
    }
}

/// The console's entries since last taken, as records.
fn take_console(tab: &mut RemoteTab, into: &mut Vec<Json>) {
    for entry in tab.take_console() {
        if into.len() >= MAX_ENTRIES {
            continue;
        }
        let level = match entry.level {
            ConsoleLevel::Error => "error",
            ConsoleLevel::Warn => "warn",
            ConsoleLevel::Info => "info",
            ConsoleLevel::Debug => "debug",
            ConsoleLevel::Log => "log",
        };
        let source = match entry.source {
            ConsoleSource::Console => "console",
            ConsoleSource::Exception => "exception",
            ConsoleSource::Network => "network",
            ConsoleSource::Policy => "policy",
            ConsoleSource::Worker => "worker",
            ConsoleSource::Engine => "engine",
        };
        let text: String = entry.text.chars().take(MAX_TEXT).collect();
        into.push(Json::object([
            ("level", Json::from(level)),
            ("source", Json::from(source)),
            ("text", Json::from(text)),
        ]));
    }
}

/// The dump of the renderer that stopped, if one was written.
fn stopped(tab: &mut RemoteTab, crashes: &Path) -> Option<String> {
    let _ = tab.take_notice();
    std::fs::read_dir(crashes)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|e| e == "dmp"))
        .map(|path| path.display().to_string())
}

/// The frame drawn now, written as a PNG.
fn shot(tab: &mut RemoteTab, began: Instant, path: &Path) -> Result<(), String> {
    let pixels = tab
        .frame_at(SIZE.0, SIZE.1, FrameTime::at(began.elapsed()))
        .to_vec();
    let png = canvas::encode_png(&pixels, SIZE.0, SIZE.1).ok_or("the frame would not encode")?;
    std::fs::write(path, png).map_err(|e| format!("not written: {e}"))
}

/// The peaks of the browser's process and the tab's renderers' (the
/// largest of each), their working sets and commit, as the system says.
#[derive(Default)]
struct Peaks {
    renderer_working_set: u64,
    renderer_commit: u64,
    browser_working_set: u64,
    browser_commit: u64,
}

impl Peaks {
    fn sample(&mut self, tab: &RemoteTab) {
        for pid in tab.renderer_processes() {
            if let Some(memory) = platform::process_memory(pid) {
                self.renderer_working_set = self.renderer_working_set.max(memory.peak_working_set);
                self.renderer_commit = self.renderer_commit.max(memory.peak_private.unwrap_or(0));
            }
        }
        if let Some(memory) = platform::process_memory(std::process::id()) {
            self.browser_working_set = self.browser_working_set.max(memory.peak_working_set);
            self.browser_commit = self.browser_commit.max(memory.peak_private.unwrap_or(0));
        }
    }

    fn json(&self) -> Json {
        let bytes = |n: u64| {
            if n == 0 {
                Json::Null
            } else {
                Json::from(n as f64)
            }
        };
        Json::object([
            ("rendererPeakWorkingSet", bytes(self.renderer_working_set)),
            ("rendererPeakCommit", bytes(self.renderer_commit)),
            ("browserPeakWorkingSet", bytes(self.browser_working_set)),
            ("browserPeakCommit", bytes(self.browser_commit)),
        ])
    }
}
