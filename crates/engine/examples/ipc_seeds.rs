//! Seeds of the `ipc_decode` fuzz target from real traffic (M34,
//! docs/design/processes.md): a session in a tab of renderers (threads
//! here, the protocol the same) whose page fetches, keeps storage and a
//! cookie, frames another site's page that posts it a message, is
//! clicked and typed in (the click opening a tab, saving a file and
//! asking to close the tab, M37), its storage change heard by another
//! tab of the same context, then goes to another site and back; its
//! messages each way, as they crossed the pipes, a few of each kind, one
//! frame to a file.
//!
//! ```text
//! cargo run -p engine --example ipc_seeds -- fuzz/seeds/ipc_decode
//! ```

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use engine::ipc::{ToBrowser, ToRenderer, Wire, read_message};
use engine::service::{Launched, Launcher, RemoteTab, ThreadLauncher};
use engine::{Button, FrameTime, LoadState, Url, View};
use net::test_server::{Reply, TestServer};

/// The bytes each pipe carried: by renderer, and whether to it.
type Logs = Arc<Mutex<HashMap<(u32, bool), Vec<u8>>>>;

/// A pipe's end whose bytes are copied into the log as they pass.
struct Tee<T> {
    inner: T,
    logs: Logs,
    key: (u32, bool),
}

impl<T> Tee<T> {
    fn log(&self, bytes: &[u8]) {
        self.logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(self.key)
            .or_default()
            .extend_from_slice(bytes);
    }
}

impl<R: Read> Read for Tee<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.log(&buf[..n]);
        Ok(n)
    }
}

impl<W: Write> Write for Tee<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.log(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Renderers on threads whose pipes are recorded.
struct Recording {
    inner: ThreadLauncher,
    logs: Logs,
}

impl Launcher for Recording {
    fn launch(&self, number: u32) -> io::Result<Launched> {
        let launched = self.inner.launch(number)?;
        Ok(Launched {
            from_renderer: Box::new(Tee {
                inner: launched.from_renderer,
                logs: self.logs.clone(),
                key: (number, false),
            }),
            to_renderer: Box::new(Tee {
                inner: launched.to_renderer,
                logs: self.logs.clone(),
                key: (number, true),
            }),
            log: launched.log,
            process: launched.process,
        })
    }
}

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

fn server() -> TestServer {
    TestServer::plain(|request| {
        let host = request.header("Host").unwrap_or_default().to_owned();
        let port = host.rsplit(':').next().unwrap_or_default().to_owned();
        let body = match request.target.as_str() {
            "/" => format!(
                "<title>seeds</title><input id=i>\
                 <iframe src='http://b.localhost:{port}/child'></iframe>\
                 <script>localStorage.setItem('k', 'v'); document.cookie = 'c=1';\
                 fetch('/data').then(r => r.text()).then(t => console.log('data', t));\
                 addEventListener('message', e => console.log('got', e.data));\
                 addEventListener('click', () => {{ window.open('/other'); \
                 const a = document.createElement('a'); a.href = '/data'; a.download = 'x.txt'; document.body.append(a); \
                 a.click(); localStorage.setItem('k2', 'w'); window.close(); }});</script>"
            ),
            "/child" => "<p>child</p><script>parent.postMessage('hi', '*');</script>".to_owned(),
            "/data" => "{\"a\": 1}".to_owned(),
            _ => "<p>other</p>".to_owned(),
        };
        Reply::ok("text/html", body.as_bytes())
    })
}

/// Takes what the renderers say for a while, drawing as they ask.
fn run(tab: &mut RemoteTab, for_: Duration) {
    let began = Instant::now();
    while began.elapsed() < for_ {
        tab.poll_network();
        tab.run_tasks();
        let _ = tab.take_console();
        if tab.needs_frame() {
            tab.frame_at(400, 300, FrameTime::at(Duration::ZERO));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// The frames of `bytes`, each with its message's tag, as `M` reads them.
fn frames<M: Wire>(bytes: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut reader = Cursor::new(bytes);
    while let Ok(Some(_)) = read_message::<M>(&mut reader) {
        let end = reader.position() as usize;
        let framed = bytes[at..end].to_vec();
        let tag = u16::from_le_bytes([framed[4], framed[5]]);
        out.push((tag, framed));
        at = end;
    }
    out
}

/// A name for a seed: its bytes' FNV-1a hash.
fn name(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("the seeds' folder"));
    let server = server();
    let logs: Logs = Arc::default();
    let launcher = Arc::new(Recording {
        inner: ThreadLauncher::with_fonts(fonts()),
        logs: logs.clone(),
    });
    let context = engine::Context::new();
    let launcher: Arc<dyn Launcher> = launcher;
    let mut tab = RemoteTab::in_context_with_cap(launcher.clone(), fonts(), 16, false, &context);
    tab.set_viewport(400, 300);
    tab.set_scripting(true);
    let url = |host: &str, path: &str| {
        Url::parse(&format!("http://{host}:{}{path}", server.port())).expect("a URL")
    };
    tab.navigate(url("a.localhost", "/"));
    run(&mut tab, Duration::from_secs(2));
    assert_eq!(*tab.load_state(), LoadState::Loaded);
    // Another tab of the context on the same site, which hears the
    // first's storage change.
    let mut other = RemoteTab::in_context_with_cap(launcher, fonts(), 16, false, &context);
    other.set_viewport(400, 300);
    other.set_scripting(true);
    other.navigate(url("a.localhost", "/other"));
    run(&mut other, Duration::from_secs(1));
    tab.pointer_move(20.0, 10.0);
    tab.pointer_down(20.0, 10.0, Button::Primary, 1);
    tab.pointer_up(20.0, 10.0, Button::Primary, 1);
    tab.insert_text("typed");
    run(&mut tab, Duration::from_millis(500));
    run(&mut other, Duration::from_millis(500));
    drop(other);
    tab.navigate(url("c.localhost", "/other"));
    run(&mut tab, Duration::from_secs(1));
    tab.go_back();
    run(&mut tab, Duration::from_secs(1));
    drop(tab);

    std::fs::create_dir_all(&dir).expect("the folder");
    let logs = logs.lock().unwrap_or_else(PoisonError::into_inner);
    let mut kept: BTreeMap<(bool, u16), usize> = BTreeMap::new();
    let mut written = 0;
    for ((_, to_renderer), bytes) in logs.iter() {
        let framed = match to_renderer {
            true => frames::<ToRenderer>(bytes),
            false => frames::<ToBrowser>(bytes),
        };
        for (tag, seed) in framed {
            let count = kept.entry((*to_renderer, tag)).or_default();
            if *count >= 3 || seed.len() > 16 << 10 {
                continue;
            }
            *count += 1;
            std::fs::write(dir.join(name(&seed)), &seed).expect("written");
            written += 1;
        }
    }
    println!("{written} seeds of {} kinds of message", kept.len());
}
