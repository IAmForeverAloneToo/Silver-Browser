//! Service workers' versions in one process (M39): a version stops once
//! idle and starts again from its kept scripts for its next event, its
//! imports taken from them and not fetched again; an event past its
//! limit fails and stops it. The limits are a second and three seconds
//! here.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use net::test_server::Reply;

use super::service_workers::in_process_with_host;
use crate::service_worker::{Job, JobKind, ServiceWorkerHost, Settled, State, UpdateViaCache};
use crate::testing::{serve, wait_for};

fn js(body: &str) -> Reply {
    Reply::with_fields(
        200,
        &[
            ("Content-Type", "text/javascript"),
            ("Cache-Control", "no-store"),
        ],
        body.as_bytes(),
    )
}

#[test]
fn an_idle_version_stops_and_starts_again_from_its_scripts_for_an_event() {
    let imports = Arc::new(AtomicUsize::new(0));
    let counted = imports.clone();
    let server = serve(move |path| match path {
        "/sw.js" => Some(js("importScripts('lib.js');\
             self.onactivate = (e) => e.waitUntil(caches.open('c').then(c => c.put('/x', new Response(String(lib)))));")),
        "/lib.js" => {
            counted.fetch_add(1, Ordering::SeqCst);
            Some(js("var lib = 'from lib';"))
        }
        _ => None,
    });
    let storage = Arc::new(Mutex::new(crate::storage::Storage::new()));
    let (workers, host) = in_process_with_host(storage, Arc::new(net::Client::default()));
    let key = server.url("/").origin().serialization();
    let (sender, receiver) = channel();
    workers.schedule(
        Job {
            kind: JobKind::Register,
            key: key.clone(),
            scope: server.url("/"),
            script: server.url("/sw.js"),
            update_via_cache: UpdateViaCache::Imports,
            force_bypass: false,
            partition: None,
        },
        Box::new(move |settled: Settled| {
            let _ = sender.send(settled.outcome);
        }),
    );
    receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("settled");
    let active = || {
        workers.query(&key, |registry| {
            let reg = registry.registration(registry.get(&key, server.url("/").as_str())?)?;
            let v = registry.version(reg.active?)?;
            Some((v.id, v.state, v.resources.len()))
        })
    };
    wait_for(|| active().is_some_and(|(_, state, _)| state == State::Activated));
    let (version, _, scripts) = active().expect("active");
    // Its import kept with it.
    assert_eq!(scripts, 2);
    assert_eq!(imports.load(Ordering::SeqCst), 1);
    // Idle, it stops.
    wait_for(|| !host.runs(version));
    // An event starts it again; its import comes from what it kept.
    let (sender, receiver) = channel();
    host.install(
        version,
        Box::new(move |ok| {
            let _ = sender.send(ok);
        }),
    );
    let ok = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("dispatched");
    assert!(ok, "dispatched to the version started again");
    assert!(host.runs(version));
    assert_eq!(imports.load(Ordering::SeqCst), 1);
}

#[test]
fn an_event_past_its_limit_fails_and_stops_its_version() {
    let server = serve(|path| match path {
        "/sw.js" => Some(js(
            "self.oninstall = (e) => e.waitUntil(new Promise(() => {}));",
        )),
        _ => None,
    });
    let storage = Arc::new(Mutex::new(crate::storage::Storage::new()));
    let (workers, host) = in_process_with_host(storage, Arc::new(net::Client::default()));
    let key = server.url("/").origin().serialization();
    let (sender, receiver) = channel();
    workers.schedule(
        Job {
            kind: JobKind::Register,
            key: key.clone(),
            scope: server.url("/"),
            script: server.url("/sw.js"),
            update_via_cache: UpdateViaCache::Imports,
            force_bypass: false,
            partition: None,
        },
        Box::new(move |settled: Settled| {
            let _ = sender.send(settled);
        }),
    );
    let settled = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("settled");
    let version = settled
        .view
        .and_then(|view| view.installing)
        .map(|shown| shown.id)
        .expect("installing");
    assert!(host.runs(version));
    // The install never ends: past the limit it fails, the version
    // stops, and its first version gone, so is the registration.
    wait_for(|| {
        workers
            .query(&key, |registry| {
                registry.get(&key, server.url("/").as_str())
            })
            .is_none()
    });
    wait_for(|| !host.runs(version));
}
