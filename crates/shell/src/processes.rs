//! The shell's processes (M34, docs/design/processes.md): a renderer's
//! life when the executable is started with `--renderer <number>`, its
//! channel the process's standard input and output; and
//! `--check-processes`, which starts renderers the way the browser does
//! and says on standard output what became of them, for the shell's tests
//! and for a user who wants to know.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use engine::service::{Event, Gone, HELLO_TIMEOUT, RemoteTab, Renderer};
use engine::{FrameTime, LoadState, Url, View};

use crate::platform::ProcessLauncher;

/// Runs this process as a renderer; its exit code.
pub(crate) fn renderer() -> i32 {
    match engine::renderer::run(std::io::stdin(), std::io::stdout()) {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("{why}");
            1
        }
    }
}

/// How long the check waits for what it waits for.
const WAIT: Duration = Duration::from_secs(20);

/// Waits for renderer `number`'s end on `events`, and its exit code.
fn ended(
    renderer: &Renderer,
    events: &mpsc::Receiver<(u32, Event)>,
) -> Result<(Gone, i64), String> {
    let number = renderer.number();
    let gone = loop {
        match events.recv_timeout(WAIT) {
            Ok((_, Event::Gone(why))) => break why,
            Ok(_) => {}
            Err(_) => return Err(format!("renderer {number} did not end")),
        }
    };
    let began = Instant::now();
    while began.elapsed() < WAIT {
        if let Some(code) = renderer.exit_code() {
            return Ok((gone, code));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(format!("renderer {number}'s process did not end"))
}

/// Starts a renderer and ends it when told, then another and kills it;
/// prints what each did and returns 0 when both did as they should. With
/// `orphan`, starts one, prints its process's number and exits at once,
/// without ending it: its job object must. With `page`, shows
/// `about:silver` in a tab of a renderer and says what it drew.
pub(crate) fn check(what: Option<&str>) -> i32 {
    let checked = match what {
        Some("orphan") => check_orphan(),
        Some("page") => check_page(),
        _ => check_all(),
    };
    match checked {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("silver-browser: {why}");
            1
        }
    }
}

fn check_all() -> Result<(), String> {
    let launcher = ProcessLauncher::new().map_err(|error| format!("no executable: {error}"))?;
    let start = |number: u32| {
        let (events, received) = mpsc::channel();
        let began = Instant::now();
        let renderer = Renderer::start(&launcher, number, events, Arc::new(|| {}), None)
            .map_err(|error| format!("renderer {number} did not start: {error}"))?;
        if !renderer.wait_ready(HELLO_TIMEOUT + Duration::from_secs(1)) {
            return Err(format!(
                "renderer {number} did not answer: {:?}",
                renderer.gone()
            ));
        }
        println!(
            "renderer {number}: process {}, ready in {} ms",
            renderer.process_id(),
            began.elapsed().as_millis()
        );
        Ok((renderer, received))
    };

    let (first, events) = start(1)?;
    first.shutdown();
    let (gone, code) = ended(&first, &events)?;
    println!("renderer 1: {gone:?}, exit code {code}");
    if code != 0 {
        return Err(format!("renderer 1 ended with {code}"));
    }

    let (second, events) = start(2)?;
    second.bad_message("the check's");
    let (gone, code) = ended(&second, &events)?;
    println!("renderer 2: {gone:?}, exit code {code}");
    if code == 0 {
        return Err("renderer 2 was not killed".into());
    }
    Ok(())
}

fn check_orphan() -> Result<(), String> {
    let launcher = ProcessLauncher::new().map_err(|error| format!("no executable: {error}"))?;
    let (events, _received) = mpsc::channel();
    let renderer = Renderer::start(&launcher, 3, events, Arc::new(|| {}), None)
        .map_err(|error| format!("renderer 3 did not start: {error}"))?;
    if !renderer.wait_ready(HELLO_TIMEOUT + Duration::from_secs(1)) {
        return Err("renderer 3 did not answer".into());
    }
    println!("renderer 3: process {}", renderer.process_id());
    // No destructor runs: only the operating system ends the renderer.
    std::process::exit(0);
}

fn check_page() -> Result<(), String> {
    const SIZE: (u32, u32) = (400, 300);
    let launcher = ProcessLauncher::new().map_err(|error| format!("no executable: {error}"))?;
    let fonts = engine::Engine::new()
        .map_err(|error| format!("no fonts: {error}"))?
        .fonts()
        .clone();
    let mut tab = RemoteTab::new(Arc::new(launcher), fonts);
    tab.set_viewport(SIZE.0, SIZE.1);
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    let url = Url::parse("about:silver").map_err(|_| "about:silver")?;
    tab.navigate(url);
    let began = Instant::now();
    let mut drawn = 0;
    while began.elapsed() < WAIT {
        tab.poll_network();
        tab.run_tasks();
        if *tab.load_state() == LoadState::Loaded && tab.needs_frame() {
            let frame = tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
            drawn = frame
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[..3] != [255, 255, 255])
                .count();
            if drawn > 0 {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    println!(
        "page: about:silver {:?}, {} pixels drawn of {}, renderer process {:?}",
        tab.load_state(),
        drawn,
        SIZE.0 * SIZE.1,
        tab.renderer_process()
    );
    if drawn == 0 {
        return Err("the renderer drew nothing".into());
    }
    Ok(())
}
