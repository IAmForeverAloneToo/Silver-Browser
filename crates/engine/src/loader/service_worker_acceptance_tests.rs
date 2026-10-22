//! M39's acceptance (docs/design/service-workers.md, "The acceptance"):
//! a site whose worker keeps its pages (its document, a sheet, a script,
//! an image) in a cache at install, loaded once; then, its server down,
//! loaded again in another tab, and again after the profile is closed and
//! opened: shown whole each time, from the worker, the server asked for
//! nothing of the site. Its update: the server up again with a changed
//! worker, which installs, waits, and takes over on `skipWaiting`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use net::test_server::{Reply, TestServer};

use crate::testing::*;

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&vec![200; (width * height * 4) as usize])
        .unwrap();
    writer.finish().unwrap();
    out
}

/// The site's document: its sheet colours its paragraph, its script says
/// what loaded, and it registers its worker.
const INDEX: &str = "<!doctype html><title>kept</title>\
    <link rel=stylesheet href=/site/style.css>\
    <p id=p>a page kept by its worker</p><img id=i src=/site/img.png>\
    <script src=/site/app.js></script>\
    <script>navigator.serviceWorker.register('/site/sw.js', {scope: '/site/'})\
      .then(() => navigator.serviceWorker.ready).then(() => console.log('ready'));</script>";

const APP: &str = "addEventListener('load', () => console.log('shown ' + \
    getComputedStyle(document.getElementById('p')).color + ' image ' + \
    document.getElementById('i').naturalWidth + ' controlled ' + \
    !!navigator.serviceWorker.controller));";

/// The worker: the site's files kept at install, every request answered
/// from them first; version `v`, the second skipping waiting when asked
/// and saying so.
fn worker(v: usize) -> String {
    format!(
        "const V = {v};
         const FILES = ['/site/index.html', '/site/style.css', '/site/app.js', '/site/img.png'];
         self.oninstall = (e) => e.waitUntil(caches.open('site-' + V).then(c => c.addAll(FILES)));
         self.onfetch = (e) => e.respondWith(caches.match(e.request).then(r => r || fetch(e.request)));
         self.onmessage = (e) => {{ if (e.data === 'skip') skipWaiting(); e.source.postMessage('version ' + V); }};"
    )
}

/// A site that can be taken down: then every connection is closed
/// without a word, as with no server; what it was asked of the site
/// while down is counted.
struct Site {
    server: TestServer,
    down: Arc<AtomicBool>,
    asked_down: Arc<AtomicUsize>,
    version: Arc<AtomicUsize>,
}

fn site() -> Site {
    let down = Arc::new(AtomicBool::new(false));
    let asked_down = Arc::new(AtomicUsize::new(0));
    let version = Arc::new(AtomicUsize::new(1));
    let (is_down, asked, which) = (down.clone(), asked_down.clone(), version.clone());
    let server = TestServer::plain(move |request| {
        if is_down.load(Ordering::SeqCst) {
            if request.target.starts_with("/site/") {
                asked.fetch_add(1, Ordering::SeqCst);
            }
            return Reply::Close;
        }
        let ok = |kind: &str, body: &[u8]| {
            Reply::with_fields(
                200,
                &[("Content-Type", kind), ("Cache-Control", "no-store")],
                body,
            )
        };
        match request.target.as_str() {
            "/site/index.html" => ok("text/html", INDEX.as_bytes()),
            "/site/style.css" => ok("text/css", b"#p { color: rgb(0, 128, 0) }"),
            "/site/app.js" => ok("text/javascript", APP.as_bytes()),
            "/site/img.png" => ok("image/png", &png_bytes(3, 2)),
            "/site/sw.js" => ok(
                "text/javascript",
                worker(which.load(Ordering::SeqCst)).as_bytes(),
            ),
            // Not kept by the worker: the update's page.
            "/site/update.html" => ok(
                "text/html",
                b"<script>
                  (async () => {
                    const reg = await navigator.serviceWorker.getRegistration();
                    await reg.update();
                    const w = reg.installing || reg.waiting;
                    while (w.state !== 'installed') await new Promise(r => setTimeout(r, 10));
                    console.log('waiting ' + (reg.waiting === w) + ' active ' + reg.active.state);
                    const changed = new Promise(r => navigator.serviceWorker.oncontrollerchange = r);
                    navigator.serviceWorker.onmessage = (e) => console.log('told ' + e.data);
                    w.postMessage('skip');
                    await new Promise(r => setTimeout(r, 50));
                    console.log('took over ' + (reg.active === w) + ' ' + w.state);
                  })().catch(e => console.log('failed ' + e.name + ': ' + e.message));
                  </script>",
            ),
            _ => Reply::with_fields(404, &[], b""),
        }
    });
    Site {
        server,
        down,
        asked_down,
        version,
    }
}

fn tab_in(context: &crate::Context) -> Tab {
    let mut tab = Tab::new();
    tab.engine.join(context);
    tab.engine.set_scripting(true);
    tab
}

/// Loads the site's document in `tab` until it says it was shown.
fn shown(tab: &mut Tab, site: &Site) -> Vec<String> {
    tab.engine.navigate(site.server.url("/site/index.html"));
    tab.run_until(|lines| lines.iter().any(|l| l.starts_with("shown")))
}

#[test]
fn a_site_kept_by_its_worker_is_shown_whole_with_its_server_down_and_after_the_profile_reopens() {
    let site = site();
    let memory = profile::Memory::new();
    {
        let context = crate::Context::new();
        context
            .set_profile(profile::Profile::in_memory(&memory).unwrap())
            .unwrap();
        // Online, once: the worker installs, keeping the site.
        let mut first = tab_in(&context);
        first.engine.navigate(site.server.url("/site/index.html"));
        let lines = first.run_until(|lines| lines.iter().any(|l| l == "ready"));
        assert!(
            lines.contains(&"shown rgb(0, 128, 0) image 3 controlled false".to_owned()),
            "{lines:?}"
        );
        // Down: another tab of the context has it whole, from the worker.
        site.down.store(true, Ordering::SeqCst);
        let mut second = tab_in(&context);
        let lines = shown(&mut second, &site);
        assert!(
            lines.contains(&"shown rgb(0, 128, 0) image 3 controlled true".to_owned()),
            "{lines:?}"
        );
        assert!(text(&second.engine).contains("a page kept by its worker"));
        assert_eq!(site.asked_down.load(Ordering::SeqCst), 0);
        context
            .flush_profile(std::time::Duration::from_secs(10))
            .unwrap();
    }
    // The profile closed and opened again, the server still down: the
    // registration and its caches are read from it, its version started
    // from the scripts it kept.
    let context = crate::Context::new();
    context
        .set_profile(profile::Profile::in_memory(&memory).unwrap())
        .unwrap();
    let mut third = tab_in(&context);
    let lines = shown(&mut third, &site);
    assert!(
        lines.contains(&"shown rgb(0, 128, 0) image 3 controlled true".to_owned()),
        "{lines:?}"
    );
    assert_eq!(site.asked_down.load(Ordering::SeqCst), 0);
    // Up again, with a changed worker: it installs, waits while the old
    // one controls the tab, and takes over when it skips waiting.
    site.down.store(false, Ordering::SeqCst);
    site.version.store(2, Ordering::SeqCst);
    third.engine.navigate(site.server.url("/site/update.html"));
    let lines = third.run_until(|lines| {
        lines
            .iter()
            .any(|l| l.starts_with("took over") || l.starts_with("failed"))
    });
    assert!(
        lines.contains(&"waiting true active activated".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"told version 2".to_owned()), "{lines:?}");
    assert!(
        lines.contains(&"took over true activated".to_owned()),
        "{lines:?}"
    );
}
