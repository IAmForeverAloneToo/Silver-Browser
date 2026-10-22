//! The context's service workers (M39): jobs run by its driver, their
//! scripts fetched from a server, their versions run by a host given
//! it, the notices heard, and the registrations kept in the profile.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use net::Url;
use net::test_server::{Reply, TestServer};

use super::*;
use crate::Context;
use crate::testing::serve;

/// A host that runs nothing but says, of each version, that its script
/// listened for `install` and `activate` and both went well; and what it
/// was asked.
#[derive(Default)]
struct Host {
    asked: Mutex<Vec<String>>,
}

impl ServiceWorkerHost for Host {
    fn evaluate(&self, start: Start, done: Done<Option<Evaluated>>) {
        let text = String::from_utf8_lossy(&start.source).into_owned();
        self.asked.lock().unwrap().push(format!("evaluate {text}"));
        done((text != "throw").then(|| Evaluated {
            events: vec!["install".to_owned(), "activate".to_owned()],
            imports: Vec::new(),
        }));
    }

    fn install(&self, _version: u64, done: Done<bool>) {
        self.asked.lock().unwrap().push("install".to_owned());
        done(true);
    }

    fn activate(&self, _version: u64, done: Done<()>) {
        self.asked.lock().unwrap().push("activate".to_owned());
        done(());
    }

    fn terminate(&self, _version: u64) {
        self.asked.lock().unwrap().push("terminate".to_owned());
    }
}

fn server() -> TestServer {
    serve(|path| match path {
        "/sw.js" => Some(Reply::with_fields(
            200,
            &[("Content-Type", "text/javascript")],
            b"one",
        )),
        "/throws.js" => Some(Reply::with_fields(
            200,
            &[("Content-Type", "text/javascript")],
            b"throw",
        )),
        "/moved.js" => Some(Reply::with_fields(302, &[("Location", "/sw.js")], b"")),
        _ => None,
    })
}

fn register(context: &Context, server: &TestServer, script: &str) -> Receiver<Outcome> {
    let (sender, receiver) = channel();
    let key = server.url("/").origin().serialization();
    context.service_workers().schedule(
        Job {
            kind: JobKind::Register,
            key,
            scope: server.url("/"),
            script: server.url(script),
            update_via_cache: UpdateViaCache::Imports,
            force_bypass: false,
            partition: None,
        },
        Box::new(move |settled: crate::service_worker::Settled| {
            let _ = sender.send(settled.outcome);
        }),
    );
    receiver
}

/// The scope's active version's state, once it is `activated`.
fn wait_activated(context: &Context, server: &TestServer) -> bool {
    let key = server.url("/").origin().serialization();
    let scope = server.url("/");
    for _ in 0..500 {
        let state = context.service_workers().query(&key, |registry| {
            let reg = registry.registration(registry.get(&key, scope.as_str())?)?;
            registry.version(reg.active?).map(|v| v.state)
        });
        if state == Some(State::Activated) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn a_job_fetches_its_script_and_the_host_runs_its_version() {
    let server = server();
    let context = Context::new();
    let host = Arc::new(Host::default());
    context.service_workers().set_host(host.clone());
    let heard = Arc::new(Mutex::new(Vec::new()));
    let hearing = heard.clone();
    context
        .service_workers()
        .listen(Arc::new(move |key: &str, notice: &Notice, _: &View| {
            hearing
                .lock()
                .unwrap()
                .push((key.to_owned(), notice.clone()));
            true
        }));
    let outcome = register(&context, &server, "/sw.js")
        .recv_timeout(Duration::from_secs(20))
        .expect("settled");
    assert!(matches!(outcome, Outcome::Registration(_)), "{outcome:?}");
    assert!(wait_activated(&context, &server));
    assert_eq!(
        *host.asked.lock().unwrap(),
        ["evaluate one", "install", "activate"]
    );
    // The script's request: a service worker's, marked so.
    let requests = server.requests();
    let request = requests
        .iter()
        .find(|r| r.target == "/sw.js")
        .expect("fetched");
    assert_eq!(request.header("Service-Worker"), Some("script"));
    assert_eq!(request.header("Sec-Fetch-Dest"), Some("serviceworker"));
    let key = server.url("/").origin().serialization();
    let heard = heard.lock().unwrap();
    assert!(heard.iter().all(|(k, _)| *k == key));
    let states: Vec<State> = heard
        .iter()
        .filter_map(|(_, notice)| match notice {
            Notice::State { state, .. } => Some(*state),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        [
            State::Installing,
            State::Installed,
            State::Activating,
            State::Activated
        ]
    );
}

#[test]
fn a_script_that_throws_or_is_redirected_fails_its_job() {
    let server = server();
    let context = Context::new();
    context
        .service_workers()
        .set_host(Arc::new(Host::default()));
    let outcome = register(&context, &server, "/throws.js")
        .recv_timeout(Duration::from_secs(20))
        .expect("settled");
    let Outcome::Failed(error) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(error.name, "TypeError");
    assert!(
        error
            .message
            .ends_with("ServiceWorker script evaluation failed")
    );
    let outcome = register(&context, &server, "/moved.js")
        .recv_timeout(Duration::from_secs(20))
        .expect("settled");
    let Outcome::Failed(error) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(error.name, "SecurityError");
    assert!(
        error
            .message
            .ends_with("The script resource is behind a redirect, which is disallowed.")
    );
    // With no host, nothing evaluates.
    let alone = Context::new();
    let outcome = register(&alone, &server, "/sw.js")
        .recv_timeout(Duration::from_secs(20))
        .expect("settled");
    assert!(matches!(outcome, Outcome::Failed(_)));
}

#[test]
fn a_registration_outlives_its_context_in_the_profile_until_its_site_is_cleared() {
    let server = server();
    let memory = profile::Memory::new();
    let key = server.url("/").origin().serialization();
    let scope = server.url("/");
    {
        let context = Context::new();
        context
            .set_profile(profile::Profile::in_memory(&memory).unwrap())
            .unwrap();
        context
            .service_workers()
            .set_host(Arc::new(Host::default()));
        register(&context, &server, "/sw.js")
            .recv_timeout(Duration::from_secs(20))
            .expect("settled");
        assert!(wait_activated(&context, &server));
        context.flush_profile(Duration::from_secs(10)).unwrap();
    }
    let context = Context::new();
    context
        .set_profile(profile::Profile::in_memory(&memory).unwrap())
        .unwrap();
    let host = Arc::new(Host::default());
    context.service_workers().set_host(host.clone());
    let script = context.service_workers().query(&key, |registry| {
        let reg = registry.registration(registry.get(&key, scope.as_str())?)?;
        let active = registry.version(reg.active?)?;
        Some((active.script.clone(), active.resources[0].1.to_vec()))
    });
    assert_eq!(script, Some((server.url("/sw.js"), b"one".to_vec())));
    // Clearing the site (as DevTools' storage panel does) unregisters it;
    // the driver hears of it and stops its version.
    {
        let mut storage = context.shared_storage().lock().unwrap();
        assert!(storage.sites().contains(&key));
        storage.service_workers(&key).clear_key(&key);
        storage.wake_registry();
    }
    for _ in 0..500 {
        if host.asked.lock().unwrap().contains(&"terminate".to_owned()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*host.asked.lock().unwrap(), ["terminate"]);
    let gone = context
        .service_workers()
        .query(&key, |registry| registry.get(&key, scope.as_str()));
    assert_eq!(gone, None);
    let _: Url = scope;
}
