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
