//! The browser left alone uses next to no CPU (the beta's daily use): a
//! window showing a page with nothing running, or a page whose only
//! work is a clock's text changed once a second, settled, then measured
//! for ten seconds. Every process of the browser's (the browser's, its
//! renderers', its GPU process's) is to use less than 2% of a core.
//!
//! A real window, which the maintainer's gate shows and a CI runner may
//! not: it runs when `SILVER_WINDOWED_TESTS` is set, and says it was
//! left out otherwise. The process times are read through PowerShell,
//! as nothing in the standard library gives another process's.

#![forbid(unsafe_code)]
#![cfg(windows)]

use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use net::test_server::{Reply, TestServer};

const BROWSER: &str = env!("CARGO_BIN_EXE_silver-browser");

/// How long a page is given to load and settle before it is measured.
const SETTLE: Duration = Duration::from_secs(6);

/// How long it is measured.
const MEASURED: Duration = Duration::from_secs(10);

/// The most of a core a process may use meanwhile.
const MOST: f64 = 0.02;

fn windowed() -> bool {
    std::env::var_os("SILVER_WINDOWED_TESTS").is_some()
}

/// The CPU time each process of the browser `root` (it and every process
/// under it) has used, in milliseconds, by process id.
fn cpu_times(root: u32) -> Vec<(u32, String, f64)> {
    let script = format!(
        "$all = Get-CimInstance Win32_Process; \
         $tree = @({root}); $grew = $true; \
         while ($grew) {{ $grew = $false; foreach ($p in $all) {{ \
           if ($tree -contains $p.ParentProcessId -and -not ($tree -contains $p.ProcessId)) \
           {{ $tree += $p.ProcessId; $grew = $true }} }} }}; \
         foreach ($p in $all) {{ if ($tree -contains $p.ProcessId) {{ \
           $role = 'browser'; \
           if ($p.CommandLine -match '--renderer\\s+(\\d+)') {{ $role = 'renderer' + $Matches[1] }} \
           elseif ($p.CommandLine -match '--gpu') {{ $role = 'gpu' }}; \
           '{{0}} {{1}} {{2}}' -f $p.ProcessId, $role, (($p.KernelModeTime + $p.UserModeTime) / 10000) }} }}"
    );
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .expect("PowerShell ran");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let role = parts.next()?.to_owned();
            let ms = parts.next()?.parse().ok()?;
            Some((pid, role, ms))
        })
        .collect()
}

/// Ends the browser `child` and every process under it.
fn end(child: &mut Child) {
    let _ = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .output();
    let _ = child.wait();
}

/// The share of a core each process of the browser showing `url` used
/// over [`MEASURED`], once settled, by its role.
fn idle_shares(url: &str) -> Vec<(String, f64)> {
    let mut child = Command::new(BROWSER)
        .args(["--temporary-profile", "--enable-scripts", "--software", url])
        .spawn()
        .expect("the browser started");
    thread::sleep(SETTLE);
    assert!(
        child.try_wait().ok().flatten().is_none(),
        "the browser ended before it was measured"
    );
    let before = cpu_times(child.id());
    thread::sleep(MEASURED);
    let after = cpu_times(child.id());
    end(&mut child);
    after
        .iter()
        .map(|(pid, role, ms)| {
            let was = before
                .iter()
                .find(|(p, ..)| p == pid)
                .map_or(0.0, |(.., ms)| *ms);
            (role.clone(), (ms - was) / MEASURED.as_millis() as f64)
        })
        .collect()
}

fn server() -> TestServer {
    TestServer::plain(|request| match request.target.as_str() {
        "/static.html" => Reply::ok(
            "text/html",
            b"<!DOCTYPE html><title>static</title><style>body { font: 16px serif }</style>\
              <h1>Nothing runs here</h1><p>A page left alone.</p>",
        ),
        "/clock.html" => Reply::ok(
            "text/html",
            b"<!DOCTYPE html><title>clock</title><p id=t>0</p><script>\
              var n = 0, t = document.getElementById('t');\
              setInterval(function () { t.textContent = String(++n); }, 1000);\
              </script>",
        ),
        _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"no"),
    })
}

/// One browser measured at a time: another's work is not this one's.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn assert_idle(path: &str) {
    if !windowed() {
        eprintln!("left out: set SILVER_WINDOWED_TESTS to show a window");
        return;
    }
    let _one = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let server = server();
    let shares = idle_shares(server.url(path).as_str());
    eprintln!("{path}: {shares:?}");
    assert!(
        shares.iter().any(|(role, _)| role.starts_with("renderer")),
        "no renderer measured: {shares:?}"
    );
    for (role, share) in &shares {
        assert!(
            *share < MOST,
            "{path}: the {role} used {share:.3} of a core: {shares:?}"
        );
    }
}

#[test]
fn a_window_with_a_static_page_left_alone_uses_next_to_no_cpu() {
    assert_idle("/static.html");
}

#[test]
fn a_window_with_a_clock_ticking_each_second_uses_next_to_no_cpu() {
    assert_idle("/clock.html");
}
