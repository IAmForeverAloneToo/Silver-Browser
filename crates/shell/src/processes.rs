//! The shell's processes (M34, docs/design/processes.md): a renderer's
//! life when the executable is started with `--renderer <number>`, its
//! channel the process's standard input and output; and
//! `--check-processes`, which starts renderers the way the browser does
//! and says on standard output what became of them, for the shell's tests
//! and for a user who wants to know; among them the sandbox's probes
//! (M34.3), run in a renderer started as any other.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use engine::service::{
    Event, Gone, HELLO_TIMEOUT, Launcher, MAX_RENDERER_MEMORY, RemoteTab, Renderer,
};
use engine::{FrameTime, LoadState, Url, View};

use crate::platform::ProcessLauncher;

/// Runs this process as a renderer; its exit code. After its number,
/// `--crash` and three handles' values set it up to report its crash to
/// the browser (M34.4); then `--probe` and a file of the user's run the
/// sandbox's probes instead and say what became of each, and
/// `--crash-now` with `panic` or `fault` crashes it, for the check of
/// the dumps. A panic that ends it is reported, its exit code 101's.
pub(crate) fn renderer() -> i32 {
    let mut arguments: Vec<OsString> = std::env::args_os().skip(3).collect();
    if arguments.first().is_some_and(|first| first == "--crash") {
        let handles: Vec<usize> = arguments
            .iter()
            .skip(1)
            .take(3)
            .filter_map(|value| value.to_str()?.parse().ok())
            .collect();
        if let Ok(handles) = <[usize; 3]>::try_from(handles) {
            crate::platform::report_crashes(handles);
        }
        arguments.drain(..arguments.len().min(4));
    }
    if arguments.first().is_some_and(|first| first == "--probe") {
        let file = arguments.get(1).map(PathBuf::from).unwrap_or_default();
        for (probe, outcome) in probe(&file) {
            println!("{probe}: {outcome}");
        }
        return 0;
    }
    let crash = arguments
        .first()
        .is_some_and(|first| first == "--crash-now")
        .then(|| arguments.get(1).and_then(|how| how.to_str()))
        .flatten();
    let ran = std::panic::catch_unwind(|| {
        match crash {
            Some("panic") => panic!("the check's panic"),
            Some("fault") => crate::platform::fault(),
            _ => {}
        }
        engine::renderer::run(std::io::stdin(), std::io::stdout())
    });
    match ran {
        Ok(Ok(())) => 0,
        Ok(Err(why)) => {
            eprintln!("{why}");
            1
        }
        Err(_) => {
            crate::platform::crashed_by_panic();
            101
        }
    }
}

/// How long the check waits for what it waits for.
const WAIT: Duration = Duration::from_secs(20);

/// What the sandbox must refuse a renderer (M34.3, D2), by its probes'
/// names.
const REFUSED: [&str; 9] = [
    "the user's file, read",
    "the user's folder, listed",
    "the temporary folder, written",
    "the folder of the profile, written",
    "a folder of the users, written at low integrity",
    "the executable, written",
    "a child process",
    "more memory than a renderer may have",
    "the desktop's global state, changed",
];

/// What it must allow one.
const FONTS: &str = "the system's fonts, read";

/// A probe's outcome: `refused` when the system denied it access.
fn outcome<T>(result: std::io::Result<T>) -> String {
    match result {
        Ok(_) => "allowed".into(),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => "refused".into(),
        Err(error) => format!("failed ({error})"),
    }
}

/// Makes a new file in `folder`, gone again when closed: a probe the
/// sandbox wrongly allows leaves nothing behind, even where the file,
/// once made, could not be removed.
fn made_in(folder: &Path) -> std::io::Result<()> {
    let path = folder.join(format!("silver-sandbox-probe-{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // GENERIC_WRITE and DELETE, and FILE_FLAG_DELETE_ON_CLOSE.
        options
            .access_mode(0x4000_0000 | 0x0001_0000)
            .custom_flags(0x0400_0000);
    }
    options.open(&path)?;
    #[cfg(not(windows))]
    std::fs::remove_file(path)?;
    Ok(())
}

/// What a renderer may do where it runs (M34.3, D2): each probe by its
/// name, with `refused`, `allowed`, or how it failed otherwise. `file` is
/// a file of the user's the browser made for the probes.
fn probe(file: &Path) -> Vec<(&'static str, String)> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default();
    // The local application data, on Windows, where the profile lives.
    let profile = crate::platform::profile_directory()
        .and_then(|profile| profile.ancestors().nth(2).map(Path::to_path_buf))
        .unwrap_or_default();
    let users = std::env::var_os("ProgramData")
        .map_or_else(|| PathBuf::from("C:\\ProgramData"), PathBuf::from);
    let executable = std::env::current_exe().unwrap_or_default();
    let child = match std::process::Command::new(&executable)
        .args(["--renderer", "0"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
            "allowed".to_owned()
        }
        Err(error) => format!("refused ({error})"),
    };
    let more = usize::try_from(MAX_RENDERER_MEMORY + (512 << 20)).unwrap_or(usize::MAX);
    let memory = match Vec::<u8>::new().try_reserve_exact(more) {
        Ok(()) => "allowed".to_owned(),
        Err(_) => "refused".to_owned(),
    };
    let fonts = match engine::Engine::new() {
        Ok(_) => "allowed".to_owned(),
        Err(error) => format!("failed ({error})"),
    };
    vec![
        (REFUSED[0], outcome(std::fs::read(file))),
        (REFUSED[1], outcome(std::fs::read_dir(&home))),
        (REFUSED[2], outcome(made_in(&std::env::temp_dir()))),
        (REFUSED[3], outcome(made_in(&profile))),
        (REFUSED[4], outcome(made_in(&users))),
        (
            REFUSED[5],
            outcome(std::fs::OpenOptions::new().write(true).open(&executable)),
        ),
        (REFUSED[6], child),
        (REFUSED[7], memory),
        (
            REFUSED[8],
            if crate::platform::desktop_closed() {
                "refused (by its job)".to_owned()
            } else {
                "allowed".to_owned()
            },
        ),
        (FONTS, fonts),
    ]
}

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
/// `about:silver` in a tab of a renderer and says what it drew. With
/// `sandbox`, runs the sandbox's probes in a renderer and says what
/// became of each. With `crash`, crashes a renderer by a panic and
/// another by a fault and says what the browser kept of each. With
/// `kill`, kills a renderer in the middle of an animation, sees the page
/// say so and come back on a reload (M34.7's acceptance). With `cap` and
/// a port, shows twenty sites of `localhost` at that port one after
/// another and says how many renderers live and what each holds.
pub(crate) fn check(what: Option<&str>) -> i32 {
    let checked = match what {
        Some("orphan") => check_orphan(),
        Some("page") => check_page(),
        Some("sandbox") => check_sandbox(),
        Some("crash") => check_crash(),
        Some("kill") => check_kill(),
        Some("cap") => check_cap(std::env::args().nth(3)),
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

fn check_sandbox() -> Result<(), String> {
    let file = std::env::temp_dir().join(format!("silver-sandbox-file-{}", std::process::id()));
    std::fs::write(&file, "the user's").map_err(|error| format!("no file: {error}"))?;
    let said = probed(&file);
    let _ = std::fs::remove_file(&file);
    let said = said?;
    print!("{said}");
    let said_of = |probe: &str| {
        said.lines()
            .find_map(|line| line.strip_prefix(probe)?.strip_prefix(": "))
            .unwrap_or("not said")
            .to_owned()
    };
    for probe in REFUSED {
        if !said_of(probe).starts_with("refused") {
            return Err(format!("{probe}: {}", said_of(probe)));
        }
    }
    if said_of(FONTS) != "allowed" {
        return Err(format!("{FONTS}: {}", said_of(FONTS)));
    }
    Ok(())
}

fn check_crash() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("silver-crashes-{}", std::process::id()));
    let checked = crashed(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    checked
}

/// Crashes two renderers, their dumps going to `dir`, where as many
/// older dumps as are kept wait to be removed.
fn crashed(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("no folder: {error}"))?;
    let older: Vec<PathBuf> = (0..crate::crashes::MAX_CRASH_DUMPS)
        .map(|n| dir.join(crate::crashes::dump_name(n as u128, 1)))
        .collect();
    for dump in &older {
        std::fs::write(dump, b"older").map_err(|error| format!("not written: {error}"))?;
    }
    for (number, how) in [(8, "panic"), (9, "fault")] {
        let launcher = ProcessLauncher::new()
            .map_err(|error| format!("no executable: {error}"))?
            .with_crashes(Some(dir.to_path_buf()))
            .with_arguments(vec!["--crash-now".into(), how.into()]);
        let (events, received) = mpsc::channel();
        let renderer = Renderer::start(&launcher, number, events, Arc::new(|| {}), None)
            .map_err(|error| format!("renderer {number} did not start: {error}"))?;
        let (gone, code) = ended(&renderer, &received)?;
        let dump = renderer
            .dump()
            .ok_or_else(|| format!("renderer {number}: no dump"))?;
        let bytes = std::fs::read(&dump).unwrap_or_default();
        let streams = streams(&bytes);
        let size = bytes.len();
        let message = std::fs::read_to_string(dump.with_extension("txt")).unwrap_or_default();
        println!(
            "renderer {number} ({how}): {gone:?}, exit code {code:#x}, dump of {size} bytes, \
             {} streams, message {:?}",
            streams.len(),
            message.lines().take(2).collect::<Vec<_>>().join(" ")
        );
        if streams.is_empty() || dump.parent() != Some(dir) {
            return Err(format!("renderer {number}: the dump {}", dump.display()));
        }
        if how == "fault" && !streams.contains(&EXCEPTION_STREAM) {
            return Err(format!(
                "renderer {number}: no exception in the dump: {streams:?}"
            ));
        }
        if how == "panic" && !message.contains("the check's panic") {
            return Err(format!("renderer {number}: the message {message:?}"));
        }
    }
    let kept = std::fs::read_dir(dir)
        .map_err(|error| format!("not listed: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|x| x == "dmp"))
        .count();
    println!("dumps kept: {kept}");
    if kept != crate::crashes::MAX_CRASH_DUMPS || older[..2].iter().any(|dump| dump.exists()) {
        return Err(format!("{kept} dumps kept, the oldest among them"));
    }
    Ok(())
}

/// A minidump's stream of the exception that crashed its process.
const EXCEPTION_STREAM: u32 = 6;

/// The types of a minidump's streams, read from its header and its
/// directory: none for what is not one.
fn streams(dump: &[u8]) -> Vec<u32> {
    let word = |at: usize| {
        dump.get(at..at + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    if dump.get(..4) != Some(b"MDMP") {
        return Vec::new();
    }
    let (Some(count), Some(directory)) = (word(8), word(12)) else {
        return Vec::new();
    };
    (0..count.min(1024) as usize)
        .map_while(|entry| word(directory as usize + entry * 12))
        .collect()
}

/// What a renderer started with the probes' arguments says.
fn probed(file: &Path) -> Result<String, String> {
    let launcher = ProcessLauncher::new()
        .map_err(|error| format!("no executable: {error}"))?
        .with_arguments(vec!["--probe".into(), file.into()]);
    let launched = launcher
        .launch(7)
        .map_err(|error| format!("renderer 7 did not start: {error}"))?;
    let mut from_renderer = launched.from_renderer;
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        let mut said = String::new();
        let _ = from_renderer.read_to_string(&mut said);
        let _ = sender.send(said);
    });
    let said = received
        .recv_timeout(WAIT)
        .map_err(|_| "the probes did not end".to_owned());
    drop((launched.to_renderer, launched.process));
    said
}

/// A tab of renderer processes the size of `size`, its pages animated.
fn process_tab(size: (u32, u32)) -> Result<RemoteTab, String> {
    let launcher = ProcessLauncher::new().map_err(|error| format!("no executable: {error}"))?;
    let fonts = engine::Engine::new()
        .map_err(|error| format!("no fonts: {error}"))?
        .fonts()
        .clone();
    let mut tab = RemoteTab::new(Arc::new(launcher), fonts);
    tab.set_viewport(size.0, size.1);
    tab.frame_at(size.0, size.1, FrameTime::at(Duration::ZERO));
    Ok(tab)
}

/// Takes what the tab's renderers say until `done` holds, at most
/// `WAIT`; whether it did.
fn wait_for(tab: &mut RemoteTab, mut done: impl FnMut(&mut RemoteTab) -> bool) -> bool {
    let began = Instant::now();
    while began.elapsed() < WAIT {
        tab.poll_network();
        tab.run_tasks();
        if done(tab) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

/// The colours of the page's top left pixel over `span`, each once.
fn colours(tab: &mut RemoteTab, size: (u32, u32), span: Duration) -> Vec<[u8; 4]> {
    let mut seen = Vec::new();
    let began = Instant::now();
    while began.elapsed() < span {
        tab.poll_network();
        if tab.needs_frame() {
            let frame = tab.frame_at(size.0, size.1, FrameTime::at(Duration::ZERO));
            if let Ok(pixel) = <[u8; 4]>::try_from(&frame[..4])
                && !seen.contains(&pixel)
            {
                seen.push(pixel);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    seen
}

/// A page whose colour never stops changing.
const ANIMATED: &str = "<!DOCTYPE html><style>\
    @keyframes shift { from { background: rgb(0, 0, 255) } to { background: rgb(255, 0, 0) } }\
    html, body { margin: 0; height: 100% }\
    body { animation: shift 400ms linear infinite alternate }</style>";

/// The background of the browser's own pages: the page saying a
/// renderer stopped.
const STOPPED: [u8; 4] = [246, 246, 244, 255];

fn check_kill() -> Result<(), String> {
    const SIZE: (u32, u32) = (200, 100);
    let file = std::env::temp_dir().join(format!("silver-animated-{}.html", std::process::id()));
    std::fs::write(&file, ANIMATED).map_err(|error| format!("no page: {error}"))?;
    let killed = kill_animated(&file, SIZE);
    let _ = std::fs::remove_file(&file);
    killed
}

fn kill_animated(file: &Path, size: (u32, u32)) -> Result<(), String> {
    let url = Url::parse(&format!(
        "file:///{}",
        file.display().to_string().replace('\\', "/")
    ))
    .map_err(|_| "no URL for the page")?;
    let mut tab = process_tab(size)?;
    tab.navigate(url);
    if !wait_for(&mut tab, |tab| *tab.load_state() == LoadState::Loaded) {
        return Err(format!("the page did not load: {:?}", tab.load_state()));
    }
    let before = colours(&mut tab, size, Duration::from_millis(800));
    let first = tab.renderer_process();
    println!(
        "animated: {} colours, renderer process {first:?}",
        before.len()
    );
    if before.len() < 3 {
        return Err("the page did not animate".into());
    }
    tab.kill_renderer();
    let stopped = wait_for(&mut tab, |tab| {
        matches!(tab.load_state(), LoadState::Failed(_))
            && tab.needs_frame()
            && tab.frame_at(size.0, size.1, FrameTime::at(Duration::ZERO))[..4] == STOPPED
    });
    let notice = tab.take_notice();
    println!(
        "killed: {:?}, the page saying so: {stopped}, {notice:?}",
        tab.load_state()
    );
    if !stopped {
        return Err("no page saying the renderer stopped".into());
    }
    tab.reload();
    if !wait_for(&mut tab, |tab| *tab.load_state() == LoadState::Loaded) {
        return Err(format!("no reload: {:?}", tab.load_state()));
    }
    let after = colours(&mut tab, size, Duration::from_millis(800));
    let second = tab.renderer_process();
    println!(
        "reloaded: {} colours, renderer process {second:?}",
        after.len()
    );
    if after.len() < 3 || second == first {
        return Err("the page did not come back in a new renderer".into());
    }
    Ok(())
}

/// What `tasklist` says process `id` holds, where there is one.
fn held(id: u32) -> String {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {id}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()
        .and_then(|out| {
            let line = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            line.rsplit("\",\"")
                .next()
                .map(|held| held.trim_end_matches('"').to_owned())
        })
        .unwrap_or_default()
}

fn check_cap(port: Option<String>) -> Result<(), String> {
    const SIZE: (u32, u32) = (200, 100);
    let port: u16 = port
        .and_then(|port| port.parse().ok())
        .ok_or("cap: the port of a server of localhost's names")?;
    let mut tab = process_tab(SIZE)?;
    for n in 0..20 {
        let url = Url::parse(&format!("http://site{n}.localhost:{port}/")).map_err(|_| "no URL")?;
        tab.navigate(url.clone());
        if !wait_for(&mut tab, |tab| {
            tab.url() == Some(&url) && *tab.load_state() == LoadState::Loaded
        }) {
            return Err(format!("site {n} did not load: {:?}", tab.load_state()));
        }
        let alive = tab.renderers().len();
        println!("site {n}: {alive} renderers");
        if alive > engine::service::MAX_RENDERERS {
            return Err(format!("{alive} renderers past the cap"));
        }
    }
    let ids = tab.renderer_processes();
    println!("renderers: {}", ids.len());
    for id in &ids {
        println!("renderer process {id}: {}", held(*id));
    }
    println!(
        "browser process {}: {}",
        std::process::id(),
        held(std::process::id())
    );
    if ids.len() != engine::service::MAX_RENDERERS {
        return Err(format!("{} renderers, not the cap", ids.len()));
    }
    Ok(())
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
