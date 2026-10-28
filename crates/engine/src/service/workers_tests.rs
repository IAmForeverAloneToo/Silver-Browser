//! Service workers in worker renderers (M39, D35): a renderer's page
//! registers, its version runs in a renderer of the site, reaching its
//! caches through the browser as one of the renderer's frames; the
//! renderer shuts once nothing runs in it, and starts again for the
//! version's next event. The idle times are a second and two here.

use std::sync::Arc;
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::{RemoteTab, ThreadLauncher};
use crate::service_worker::{ServiceWorkerHost, State};
use crate::view::View;

const WAIT: Duration = Duration::from_secs(30);

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

fn settle(tab: &mut RemoteTab, mut done: impl FnMut(&mut RemoteTab) -> bool) {
    let began = Instant::now();
    loop {
        tab.poll_network();
        tab.run_tasks();
        if done(tab) {
            return;
        }
        assert!(began.elapsed() < WAIT, "the tab gets there in time");
        thread::sleep(Duration::from_millis(2));
    }
}

fn wait(mut done: impl FnMut() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(began.elapsed() < WAIT, "it happens in time");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_version_runs_in_a_renderer_of_its_site_and_reaches_its_caches_through_the_browser() {
    let server = TestServer::plain(|request| {
        match request.target.as_str() {
        "/sw.js" => Reply::with_fields(
            200,
            &[("Content-Type", "text/javascript")],
            b"self.oninstall = (e) => e.waitUntil(caches.open('c').then(c => c.put('/k', new Response('kept by ' + self.constructor.name))));",
        ),
        _ => Reply::ok(
            "text/html",
            b"<script>
             (async () => {
               const reg = await navigator.serviceWorker.register('sw.js');
               await navigator.serviceWorker.ready;
               while (reg.active.state !== 'activated') await new Promise(r => setTimeout(r, 20));
               const r = await caches.match('/k');
               console.log('page reads ' + (r && await r.text()));
             })().catch(e => console.log('failed ' + e.name + ': ' + e.message));
             </script>",
        ),
    }
    });
    let context = crate::Context::new();
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let host = super::workers::install(&context, launcher.clone()).expect("the first");
    let mut tab = RemoteTab::in_context(launcher, fonts(), &context);
    tab.set_scripting(true);
    tab.navigate(server.url("/"));
    let mut lines = Vec::new();
    settle(&mut tab, |tab| {
        lines.extend(tab.take_console().into_iter().map(|entry| entry.text));
        lines
            .iter()
            .any(|l| l.starts_with("page reads") || l.starts_with("failed"))
    });
    assert!(
        lines.contains(&"page reads kept by ServiceWorkerGlobalScope".to_owned()),
        "{lines:?}"
    );
    assert_eq!(host.renderers(), 1);
    // Idle, the version stops, and then its renderer.
    wait(|| host.renderers() == 0);
    // Its next event starts both again.
    let key = server.url("/").origin().serialization();
    let version = context.service_workers().query(&key, |registry| {
        let reg = registry.registration(registry.get(&key, server.url("/").as_str())?)?;
        let v = registry.version(reg.active?)?;
        (v.state == State::Activated).then_some(v.id)
    });
    let (sender, receiver) = channel();
    host.install(
        version.expect("activated"),
        Box::new(move |ok| {
            let _ = sender.send(ok);
        }),
    );
    assert_eq!(receiver.recv_timeout(WAIT), Ok(true));
    assert_eq!(host.renderers(), 1);
    // A remote tab's context has its own host already.
    let other = crate::Context::new();
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let _tab = RemoteTab::in_context(launcher.clone(), fonts(), &other);
    assert!(super::workers::install(&other, launcher).is_none());
}

/// A worker renderer that answers its version's evaluation as another
/// version's, then watches what the browser does: whether it was cut
/// off.
struct Liar {
    cut: Arc<std::sync::atomic::AtomicBool>,
}

struct Pipes([crate::ipc::PipeBreaker; 2]);

impl super::RendererProcess for Pipes {
    fn kill(&mut self) {
        for breaker in &self.0 {
            breaker.break_pipe();
        }
    }
    fn id(&self) -> u32 {
        0
    }
    fn exit_code(&mut self) -> Option<i64> {
        None
    }
}

impl super::Launcher for Liar {
    fn launch(&self, _number: u32) -> std::io::Result<super::Launched> {
        use std::io::Write;

        use crate::ipc::{self, ToBrowser, ToRenderer};
        let ((browser_in, browser_out), (mut renderer_in, mut renderer_out)) = ipc::duplex();
        let breakers = [browser_out.breaker(), renderer_out.breaker()];
        let cut = self.cut.clone();
        thread::spawn(move || {
            let Ok(Some(ToRenderer::Hello { .. })) =
                ipc::read_message::<ToRenderer>(&mut renderer_in)
            else {
                return;
            };
            let hello = ipc::frame(&ToBrowser::Hello {
                version: ipc::IPC_VERSION,
            })
            .expect("a hello");
            let _ = renderer_out.write_all(&hello);
            loop {
                match ipc::read_message::<ToRenderer>(&mut renderer_in) {
                    Ok(Some(ToRenderer::StartServiceWorker { version, .. })) => {
                        let lie = ipc::frame(&ToBrowser::ServiceWorkerEvaluated {
                            version: version + 1000,
                            listened: Some(Vec::new()),
                            imports: Vec::new(),
                        })
                        .expect("framed");
                        let _ = renderer_out.write_all(&lie);
                    }
                    Ok(Some(_)) => {}
                    _ => {
                        cut.store(true, std::sync::atomic::Ordering::SeqCst);
                        return;
                    }
                }
            }
        });
        Ok(super::Launched {
            from_renderer: Box::new(browser_in),
            to_renderer: Box::new(browser_out),
            log: None,
            process: Box::new(Pipes(breakers)),
        })
    }
}

/// A worker renderer whose version stops, idle, as its install event
/// comes: it says it stopped, then that the event did not run, as a
/// real one does; started again, it runs the event.
struct Crossing {
    starts: Arc<std::sync::atomic::AtomicUsize>,
}

impl super::Launcher for Crossing {
    fn launch(&self, _number: u32) -> std::io::Result<super::Launched> {
        use std::io::Write;

        use crate::ipc::{self, ToBrowser, ToRenderer};
        let ((browser_in, browser_out), (mut renderer_in, mut renderer_out)) = ipc::duplex();
        let breakers = [browser_out.breaker(), renderer_out.breaker()];
        let starts = self.starts.clone();
        thread::spawn(move || {
            let Ok(Some(ToRenderer::Hello { .. })) =
                ipc::read_message::<ToRenderer>(&mut renderer_in)
            else {
                return;
            };
            let mut say = |message: &ToBrowser| {
                let framed = ipc::frame(message).expect("framed");
                let _ = renderer_out.write_all(&framed);
            };
            say(&ToBrowser::Hello {
                version: ipc::IPC_VERSION,
            });
            let mut crossed = false;
            loop {
                match ipc::read_message::<ToRenderer>(&mut renderer_in) {
                    Ok(Some(ToRenderer::StartServiceWorker { version, .. })) => {
                        let first = starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
                        if first {
                            say(&ToBrowser::ServiceWorkerEvaluated {
                                version,
                                listened: Some(vec!["install".to_owned()]),
                                imports: Vec::new(),
                            });
                        }
                    }
                    Ok(Some(ToRenderer::ServiceWorkerEvent { version, event, .. })) => {
                        if crossed {
                            say(&ToBrowser::ServiceWorkerEventDone {
                                version,
                                event,
                                ok: true,
                                ran: true,
                            });
                        } else {
                            crossed = true;
                            say(&ToBrowser::ServiceWorkerStopped { version });
                            say(&ToBrowser::ServiceWorkerEventDone {
                                version,
                                event,
                                ok: false,
                                ran: false,
                            });
                        }
                    }
                    Ok(Some(_)) => {}
                    _ => return,
                }
            }
        });
        Ok(super::Launched {
            from_renderer: Box::new(browser_in),
            to_renderer: Box::new(browser_out),
            log: None,
            process: Box::new(Pipes(breakers)),
        })
    }
}

#[test]
fn an_install_event_crossing_its_versions_idle_stop_is_sent_again() {
    // A version stopped idle as its install event came: the browser heard
    // the stop first and failed the event, its "did not run" dropped, so
    // the registration never activated (CI's macOS run of
    // tab_tests::a_renderers_page_registers_a_service_worker_through_the_browser,
    // whose page waited for it).
    let server = TestServer::plain(|_| {
        Reply::with_fields(200, &[("Content-Type", "text/javascript")], b"1")
    });
    let context = crate::Context::new();
    let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let _host = super::workers::install(
        &context,
        Arc::new(Crossing {
            starts: starts.clone(),
        }),
    )
    .expect("the first");
    let (sender, receiver) = channel();
    context.service_workers().schedule(
        crate::service_worker::Job {
            kind: crate::service_worker::JobKind::Register,
            key: server.url("/").origin().serialization(),
            scope: server.url("/"),
            script: server.url("/sw.js"),
            update_via_cache: crate::service_worker::UpdateViaCache::Imports,
            force_bypass: false,
            partition: None,
        },
        Box::new(move |settled: crate::service_worker::Settled| {
            let _ = sender.send(settled.outcome);
        }),
    );
    let outcome = receiver.recv_timeout(WAIT).expect("settled");
    assert!(
        matches!(outcome, crate::service_worker::Outcome::Registration(_)),
        "{outcome:?}"
    );
    let key = server.url("/").origin().serialization();
    let state = || {
        context.service_workers().query(&key, |registry| {
            let reg = registry.registration(registry.get(&key, server.url("/").as_str())?)?;
            let version = reg.active.or(reg.waiting).or(reg.installing)?;
            Some(registry.version(version)?.state)
        })
    };
    wait(|| matches!(state(), Some(State::Activated | State::Redundant) | None));
    assert_eq!(state(), Some(State::Activated));
    // Started again for the event sent again.
    assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn a_worker_renderer_that_answers_for_another_version_is_cut_off() {
    let server = TestServer::plain(|_| {
        Reply::with_fields(200, &[("Content-Type", "text/javascript")], b"1")
    });
    let context = crate::Context::new();
    let cut = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let host =
        super::workers::install(&context, Arc::new(Liar { cut: cut.clone() })).expect("the first");
    let (sender, receiver) = channel();
    context.service_workers().schedule(
        crate::service_worker::Job {
            kind: crate::service_worker::JobKind::Register,
            key: server.url("/").origin().serialization(),
            scope: server.url("/"),
            script: server.url("/sw.js"),
            update_via_cache: crate::service_worker::UpdateViaCache::Imports,
            force_bypass: false,
            partition: None,
        },
        Box::new(move |settled: crate::service_worker::Settled| {
            let _ = sender.send(settled.outcome);
        }),
    );
    // Its evaluation fails with it.
    let outcome = receiver.recv_timeout(WAIT).expect("settled");
    assert!(
        matches!(&outcome, crate::service_worker::Outcome::Failed(e) if e.message.ends_with("ServiceWorker script evaluation failed")),
        "{outcome:?}"
    );
    wait(|| cut.load(std::sync::atomic::Ordering::SeqCst));
    wait(|| host.renderers() == 0);
}

#[test]
fn a_version_in_a_worker_renderer_and_a_renderers_page_message_each_other() {
    // M39 step 4: the page a client of the browser's, the version asking
    // as itself; messages both ways through the browser.
    let server = TestServer::plain(|request| {
        match request.target.as_str() {
        "/sw.js" => Reply::with_fields(
            200,
            &[("Content-Type", "text/javascript")],
            b"self.onactivate = (e) => e.waitUntil(clients.claim());
              self.onmessage = async (e) => {
                const all = await clients.matchAll();
                e.source.postMessage('got ' + e.data.n + ' from ' + e.source.type + ' ' + all.length + ' ' + (all[0].id === e.source.id));
              };",
        ),
        _ => Reply::ok(
            "text/html",
            b"<script>
             (async () => {
               await navigator.serviceWorker.register('sw.js');
               await new Promise(r => { navigator.serviceWorker.oncontrollerchange = r; });
               navigator.serviceWorker.onmessage = (e) => console.log('heard ' + e.data + ' ' + (e.source === navigator.serviceWorker.controller));
               navigator.serviceWorker.controller.postMessage({n: 7});
             })().catch(e => console.log('failed ' + e.name + ': ' + e.message));
             </script>",
        ),
    }
    });
    let context = crate::Context::new();
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let host = super::workers::install(&context, launcher.clone()).expect("the first");
    let mut tab = RemoteTab::in_context(launcher, fonts(), &context);
    tab.set_scripting(true);
    tab.navigate(server.url("/"));
    let mut lines = Vec::new();
    settle(&mut tab, |tab| {
        lines.extend(tab.take_console().into_iter().map(|entry| entry.text));
        lines
            .iter()
            .any(|l| l.starts_with("heard") || l.starts_with("failed"))
    });
    assert!(
        lines.contains(&"heard got 7 from window 1 true true".to_owned()),
        "{lines:?}"
    );
    assert_eq!(host.renderers(), 1);
}

#[test]
fn a_renderers_controlled_page_has_its_requests_answered_by_a_worker_renderer() {
    // M39 step 5: the page's requests reach the browser, which gives them
    // to the version in its worker renderer; what it does not answer goes
    // to the network.
    let server = TestServer::plain(|request| {
        match request.target.as_str() {
        "/sw.js" => Reply::with_fields(
            200,
            &[("Content-Type", "text/javascript")],
            b"self.onfetch = (e) => {
                if (e.request.url.endsWith('/made')) e.respondWith(new Response('made by the worker', {headers: {'x-sw': 'yes'}}));
                else if (e.request.url.endsWith('/kept')) e.respondWith(caches.match('/kept'));
              };
              self.oninstall = (e) => e.waitUntil(caches.open('c').then(c => c.put('/kept', new Response('kept at install'))));",
        ),
        "/real" => Reply::ok("text/plain", b"from the network"),
        _ => Reply::ok(
            "text/html",
            b"<script>
             (async () => {
               if (!navigator.serviceWorker.controller) {
                 await navigator.serviceWorker.register('/sw.js');
                 await navigator.serviceWorker.ready;
                 location.reload();
                 return;
               }
               const read = async (path) => { const r = await fetch(path); return (await r.text()) + ' ' + r.headers.get('x-sw'); };
               console.log('answered ' + await read('/made') + ', ' + await read('/kept') + ', ' + await read('/real'));
             })().catch(e => console.log('failed ' + e.name + ': ' + e.message));
             </script>",
        ),
    }
    });
    let context = crate::Context::new();
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let host = super::workers::install(&context, launcher.clone()).expect("the first");
    let mut tab = RemoteTab::in_context(launcher, fonts(), &context);
    tab.set_scripting(true);
    tab.navigate(server.url("/page"));
    let mut lines = Vec::new();
    settle(&mut tab, |tab| {
        lines.extend(tab.take_console().into_iter().map(|entry| entry.text));
        lines
            .iter()
            .any(|l| l.starts_with("answered") || l.starts_with("failed"))
    });
    assert!(
        lines.contains(
            &"answered made by the worker yes, kept at install null, from the network null"
                .to_owned()
        ),
        "{lines:?}"
    );
    assert_eq!(host.renderers(), 1);
}

#[test]
fn a_renderers_navigation_is_answered_by_a_worker_renderer() {
    // M39 step 6: the browser gives the navigation to the version whose
    // scope it is in; the document it makes is controlled.
    let server = TestServer::plain(|request| {
        match request.target.as_str() {
        "/sw.js" => Reply::with_fields(
            200,
            &[("Content-Type", "text/javascript")],
            b"self.onfetch = (e) => {
                if (e.request.mode === 'navigate' && e.request.url.endsWith('/made'))
                  e.respondWith(new Response('<p>made</p><script>console.log(\"made page controlled \" + !!navigator.serviceWorker.controller + \" \" + document.querySelector(\"p\").textContent)</scr' + 'ipt>', {headers: {'content-type': 'text/html'}}));
              };",
        ),
        _ => Reply::ok(
            "text/html",
            b"<script>
             (async () => {
               await navigator.serviceWorker.register('/sw.js');
               await navigator.serviceWorker.ready;
               location.href = '/made';
             })().catch(e => console.log('failed ' + e.name + ': ' + e.message));
             </script>",
        ),
    }
    });
    let context = crate::Context::new();
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let host = super::workers::install(&context, launcher.clone()).expect("the first");
    let mut tab = RemoteTab::in_context(launcher, fonts(), &context);
    tab.set_scripting(true);
    tab.navigate(server.url("/page"));
    let mut lines = Vec::new();
    settle(&mut tab, |tab| {
        lines.extend(tab.take_console().into_iter().map(|entry| entry.text));
        lines
            .iter()
            .any(|l| l.starts_with("made page") || l.starts_with("failed"))
    });
    assert!(
        lines.contains(&"made page controlled true made".to_owned()),
        "{lines:?}"
    );
    // The server never saw it.
    let asked: Vec<String> = server.requests().iter().map(|r| r.target.clone()).collect();
    assert!(!asked.contains(&"/made".to_owned()), "{asked:?}");
    assert_eq!(host.renderers(), 1);
}
