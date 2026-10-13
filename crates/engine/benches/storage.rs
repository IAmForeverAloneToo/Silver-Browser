//! M28's: what keeping costs (docs/design/storage.md, "Acceptance"). A
//! page that sets 1,000 `localStorage` items and reads them back, from
//! `Engine::navigate` to its title, beside the same page doing nothing;
//! the profile's write of 1,000 items changed, in memory and on disk;
//! an engine given a profile on disk that holds 3,000 cookies and 100
//! origins' items, and its first read of one of them; and a page of 100
//! stylesheets that may be kept for ten minutes, loaded, then loaded
//! again from the HTTP cache.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench storage
//! ```

#![forbid(unsafe_code)]

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use engine::{Engine, LoadState, Profile, StorageDriver};
use net::Url;
use net::cookies::{Api, Context, Jar, MAX_COOKIES};
use net::test_server::{Reply, TestServer};
use profile::{Changes, Memory};

const ITEMS: usize = 1000;
const SHEETS: usize = 100;
const ORIGINS: usize = 100;
const RUNS: usize = 10;

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

fn page(script: &str) -> String {
    format!("<!DOCTYPE html><title>Keeping</title><script>{script}</script>")
}

fn server() -> TestServer {
    let items = page(&format!(
        "localStorage.clear(); for (let i = 0; i < {ITEMS}; i++) localStorage.setItem('k' + i, 'v' + i);\
         let n = 0; for (let i = 0; i < {ITEMS}; i++) if (localStorage.getItem('k' + i) === 'v' + i) n++;\
         if (n === {ITEMS}) document.title = 'done';"
    ));
    let none = page("document.title = 'done';");
    let mut links = String::from("<!DOCTYPE html><title>done</title>");
    for i in 0..SHEETS {
        links.push_str(&format!("<link rel=stylesheet href=/s{i}.css>"));
    }
    links.push_str("<p class=c0>sheets</p>");
    TestServer::plain(move |request| match request.target.as_str() {
        "/items.html" => Reply::ok("text/html", items.as_bytes()),
        "/none.html" => Reply::ok("text/html", none.as_bytes()),
        "/sheets.html" => Reply::ok("text/html", links.as_bytes()),
        target if target.starts_with("/s") => Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/css"),
                ("Cache-Control", "max-age=600"),
            ],
            format!(".c{} {{ color: red }}", &target[2..target.len() - 4]).as_bytes(),
        ),
        _ => Reply::with_fields(404, &[], b""),
    })
}

/// A new engine over the bundled font with scripts on, and the channel
/// its waker sends on.
fn engine() -> (Engine, Receiver<()>) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(true);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    (engine, woken)
}

/// From `navigate` to `url` to its title saying "done" and its load
/// ended.
fn load(engine: &mut Engine, woken: &Receiver<()>, url: Url) -> Duration {
    let start = Instant::now();
    engine.navigate(url);
    while !(engine.title() == "done" && *engine.load_state() == LoadState::Loaded) {
        if engine.next_task_at().is_none() {
            woken
                .recv_timeout(Duration::from_secs(30))
                .expect("the page finishes");
        }
        engine.poll_network();
        engine.run_tasks();
    }
    start.elapsed()
}

/// The write of [`ITEMS`] items set in one origin's `localStorage`.
fn flush(profile: Profile) -> Duration {
    let mut driver = StorageDriver::new(Some(profile)).expect("the profile reads");
    for i in 0..ITEMS {
        driver.set(
            true,
            "https://bench.test",
            &format!("k{i}"),
            &format!("v{i}"),
        );
    }
    let start = Instant::now();
    driver.flush().expect("the profile writes");
    start.elapsed()
}

/// A directory of the system's temporary one, empty.
fn directory(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("silver-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A profile on disk with [`MAX_COOKIES`] cookies and [`ORIGINS`]
/// origins' ten items each: the time to open it and take its cookies,
/// as an engine given it does, then to read one origin's area.
fn opened() -> (Duration, Duration) {
    let dir = directory("open");
    {
        let profile = Profile::open(&dir).expect("a profile opens");
        let now = SystemTime::now();
        let mut jar = Jar::new();
        for n in 0..MAX_COOKIES {
            let url = Url::parse(&format!("https://d{}.example.com/", n % 20)).expect("a URL");
            let line = format!("c{n}=v{n}; Max-Age=86400");
            jar.store(line.as_bytes(), &url, Api::Http, Context::SAME_SITE, now);
        }
        let mut changes = Changes::new();
        changes.cookies(jar.export());
        for o in 0..ORIGINS {
            for i in 0..10u64 {
                changes.set_item(&format!("https://o{o}.test"), &format!("k{i}"), "value", i);
            }
        }
        profile.commit(&changes).expect("the profile writes");
    }
    let start = Instant::now();
    let profile = Profile::open(&dir).expect("a profile opens");
    let mut driver = StorageDriver::new(Some(profile)).expect("the profile reads");
    let given = start.elapsed();
    let start = Instant::now();
    let items = driver.items(true, "https://o7.test");
    let read = start.elapsed();
    assert_eq!(items.len(), 10);
    drop(driver);
    std::fs::remove_dir_all(&dir).ok();
    (given, read)
}

fn main() {
    let server = server();
    for (path, what) in [
        ("/none.html", "the page alone"),
        ("/items.html", "1,000 setItems and getItems"),
    ] {
        let (mut warm, woken) = engine();
        load(&mut warm, &woken, server.url(path));
        let mut times: Vec<f64> = (0..RUNS)
            .map(|_| {
                let (mut engine, woken) = engine();
                ms(load(&mut engine, &woken, server.url(path)))
            })
            .collect();
        println!("{what}, navigate to its title: {}", spread(&mut times));
    }
    let mut memory: Vec<f64> = (0..RUNS)
        .map(|_| ms(flush(Profile::in_memory(&Memory::new()).expect("opens"))))
        .collect();
    println!(
        "a write of {ITEMS} items, in memory: {}",
        spread(&mut memory)
    );
    let mut disk: Vec<f64> = (0..RUNS)
        .map(|run| {
            let dir = directory(&format!("flush{run}"));
            let time = ms(flush(Profile::open(&dir).expect("opens")));
            std::fs::remove_dir_all(&dir).ok();
            time
        })
        .collect();
    println!("a write of {ITEMS} items, on disk: {}", spread(&mut disk));
    let (mut given, mut read): (Vec<f64>, Vec<f64>) = (0..RUNS)
        .map(|_| {
            let (given, read) = opened();
            (ms(given), ms(read))
        })
        .unzip();
    println!(
        "a profile of {MAX_COOKIES} cookies and {ORIGINS} origins opened: {}",
        spread(&mut given)
    );
    println!("then an origin's items read: {}", spread(&mut read));
    let mut first = Vec::new();
    let mut again = Vec::new();
    for _ in 0..RUNS {
        let (mut engine, woken) = engine();
        first.push(ms(load(&mut engine, &woken, server.url("/sheets.html"))));
        again.push(ms(load(&mut engine, &woken, server.url("/sheets.html"))));
    }
    println!("a page of {SHEETS} stylesheets: {}", spread(&mut first));
    println!(
        "the page again, its sheets from the cache: {}",
        spread(&mut again)
    );
}
