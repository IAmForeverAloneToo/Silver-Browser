//! The browser's renderers as processes of its executable (M34.2,
//! docs/design/processes.md): started, answering, ended and killed.

#![forbid(unsafe_code)]

use std::io::Read;
use std::process::{Command, Stdio};

use engine::ipc::{self, IPC_VERSION, ToBrowser, ToRenderer};

const BROWSER: &str = env!("CARGO_BIN_EXE_silver-browser");

#[test]
fn the_browser_starts_renderers_and_ends_them() {
    let out = Command::new(BROWSER)
        .arg("--check-processes")
        .output()
        .expect("the check ran");
    let said = String::from_utf8_lossy(&out.stdout);
    let complained = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{said}{complained}");
    assert!(said.contains("renderer 1: Shutdown, exit code 0"), "{said}");
    // Killed: 1 on Windows, where the browser ends it so; a signal's end,
    // without a code, elsewhere. The check fails on a code of zero.
    assert!(
        said.contains("renderer 2: BadMessage(\"the check's\"), exit code "),
        "{said}"
    );
    assert!(!said.contains("renderer 2: BadMessage(\"the check's\"), exit code 0"));
}

#[test]
fn a_renderer_answers_the_hello_and_ends_with_its_input() {
    let mut renderer = Command::new(BROWSER)
        .args(["--renderer", "4"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("started");
    let to_renderer = ipc::Sender::<ToRenderer>::new(renderer.stdin.take().expect("a pipe"));
    let mut from_renderer = renderer.stdout.take().expect("a pipe");
    to_renderer
        .send(&ToRenderer::Hello {
            version: IPC_VERSION,
            process: 4,
        })
        .expect("sent");
    let answer = ipc::read_message::<ToBrowser>(&mut from_renderer).expect("read");
    assert_eq!(
        answer,
        Some(ToBrowser::Hello {
            version: IPC_VERSION
        })
    );
    to_renderer.close();
    assert!(matches!(
        ipc::read_message::<ToBrowser>(&mut from_renderer),
        Ok(None)
    ));
    assert!(renderer.wait().expect("ended").success());
}

#[test]
fn a_renderer_refuses_a_hello_of_another_version() {
    let mut renderer = Command::new(BROWSER)
        .args(["--renderer", "5"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("started");
    let to_renderer = ipc::Sender::<ToRenderer>::new(renderer.stdin.take().expect("a pipe"));
    to_renderer
        .send(&ToRenderer::Hello {
            version: IPC_VERSION + 1,
            process: 5,
        })
        .expect("sent");
    let status = renderer.wait().expect("ended");
    let mut said = String::new();
    renderer
        .stderr
        .take()
        .expect("a pipe")
        .read_to_string(&mut said)
        .expect("read");
    assert!(!status.success());
    assert!(said.contains("a hello of version"), "{said}");
}

/// Whether a process of number `id` runs.
#[cfg(windows)]
fn runs(id: u32) -> bool {
    let list = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {id}"), "/NH"])
        .output()
        .expect("tasklist ran");
    String::from_utf8_lossy(&list.stdout).contains(&format!(" {id} "))
}

#[cfg(windows)]
#[test]
fn a_renderer_ends_with_the_browser_that_started_it() {
    let out = Command::new(BROWSER)
        .args(["--check-processes", "orphan"])
        .output()
        .expect("the check ran");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{said}");
    let id: u32 = said
        .trim()
        .strip_prefix("renderer 3: process ")
        .and_then(|id| id.parse().ok())
        .expect("its process's number");
    let began = std::time::Instant::now();
    while runs(id) && began.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!runs(id), "renderer process {id} outlived its browser");
}

#[cfg(windows)]
#[test]
fn a_renderer_runs_in_the_sandbox() {
    let out = Command::new(BROWSER)
        .args(["--check-processes", "sandbox"])
        .output()
        .expect("the check ran");
    let said = String::from_utf8_lossy(&out.stdout);
    let complained = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{said}{complained}");
    for refused in [
        "the user's file, read: refused",
        "the user's folder, listed: refused",
        "the temporary folder, written: refused",
        "the folder of the profile, written: refused",
        "a folder of the users, written at low integrity: refused",
        "the executable, written: refused",
        "a child process: refused",
        "more memory than a renderer may have: refused",
        "the desktop's global state, changed: refused",
    ] {
        assert!(said.contains(refused), "{refused}: {said}");
    }
    assert!(said.contains("the system's fonts, read: allowed"), "{said}");
}

#[cfg(windows)]
#[test]
fn a_renderers_crash_leaves_a_dump_the_browser_wrote() {
    let out = Command::new(BROWSER)
        .args(["--check-processes", "crash"])
        .output()
        .expect("the check ran");
    let said = String::from_utf8_lossy(&out.stdout);
    let complained = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{said}{complained}");
    assert!(
        said.contains("renderer 8 (panic): Ended, exit code 0x65, dump of "),
        "{said}"
    );
    assert!(said.contains("the check's panic"), "{said}");
    assert!(
        said.contains("renderer 9 (fault): Ended, exit code 0xc0000005, dump of "),
        "{said}"
    );
}

#[test]
fn a_renderer_killed_mid_animation_leaves_a_page_that_a_reload_brings_back() {
    let out = Command::new(BROWSER)
        .args(["--check-processes", "kill"])
        .output()
        .expect("the check ran");
    let said = String::from_utf8_lossy(&out.stdout);
    let complained = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{said}{complained}");
    assert!(said.contains("the page saying so: true"), "{said}");
}

#[test]
fn a_tab_keeps_at_most_the_cap_of_renderer_processes() {
    let server = net::test_server::TestServer::plain(|_| {
        net::test_server::Reply::ok("text/html", b"<p>a site</p>")
    });
    let out = Command::new(BROWSER)
        .args(["--check-processes", "cap", &server.port().to_string()])
        .output()
        .expect("the check ran");
    let said = String::from_utf8_lossy(&out.stdout);
    let complained = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{said}{complained}");
    assert!(said.contains("site 19: 16 renderers"), "{said}");
}

#[test]
fn a_renderer_process_shows_a_page_for_the_browser() {
    let out = Command::new(BROWSER)
        .args(["--check-processes", "page"])
        .output()
        .expect("the check ran");
    let said = String::from_utf8_lossy(&out.stdout);
    let complained = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{said}{complained}");
    assert!(said.starts_with("page: about:silver Loaded, "), "{said}");
}
