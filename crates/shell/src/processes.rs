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

/// Runs this process as a renderer; its exit code. With `--probe` and a
/// file of the user's after its number, it runs the sandbox's probes
/// instead and says what became of each.
pub(crate) fn renderer() -> i32 {
    let arguments: Vec<OsString> = std::env::args_os().skip(3).collect();
    if arguments.first().is_some_and(|first| first == "--probe") {
        let file = arguments.get(1).map(PathBuf::from).unwrap_or_default();
        for (probe, outcome) in probe(&file) {
            println!("{probe}: {outcome}");
        }
        return 0;
    }
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
/// became of each.
pub(crate) fn check(what: Option<&str>) -> i32 {
    let checked = match what {
        Some("orphan") => check_orphan(),
        Some("page") => check_page(),
        Some("sandbox") => check_sandbox(),
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
