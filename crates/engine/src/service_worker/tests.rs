use std::sync::Arc;

use net::{Origin, Url};

use super::*;

const KEY: &str = "https://a.test";

fn url(text: &str) -> Url {
    Url::parse(text).expect("a URL")
}

fn job(kind: JobKind, scope: &str, script: &str) -> Job {
    Job {
        kind,
        key: KEY.to_owned(),
        scope: url(scope),
        script: url(script),
        update_via_cache: UpdateViaCache::Imports,
        force_bypass: false,
        partition: None,
    }
}

fn script(body: &[u8]) -> Fetched {
    Fetched {
        status: 200,
        mime: Some("text/javascript".to_owned()),
        javascript: true,
        allowed: None,
        body: Arc::from(body),
        from_cache: false,
    }
}

fn listening(events: &[&str]) -> Evaluated {
    Evaluated {
        events: events.iter().map(|e| (*e).to_owned()).collect(),
        imports: Vec::new(),
    }
}

/// Answers every effect as all going well, `body` the script's; the
/// effects it saw.
fn run(registry: &mut Registry, body: &[u8], events: &[&str]) -> Vec<Effect> {
    let mut seen = Vec::new();
    loop {
        let effects = registry.take_effects();
        if effects.is_empty() {
            return seen;
        }
        for effect in effects {
            match &effect {
                Effect::Fetch { job, .. } => registry.fetched(*job, Ok(script(body))),
                Effect::Evaluate { job, .. } => registry.evaluated(*job, Some(listening(events))),
                Effect::Install { version } => registry.installed(*version, true),
                Effect::Activate { version } => registry.activated(*version),
                _ => {}
            }
            seen.push(effect);
        }
    }
}

fn settled(effects: &[Effect], ticket: u64) -> Option<&Outcome> {
    effects.iter().find_map(|effect| match effect {
        Effect::Settle { ticket: t, outcome } if *t == ticket => Some(outcome),
        _ => None,
    })
}

fn notices(effects: &[Effect]) -> Vec<Notice> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Notice { notice, .. } => Some(notice.clone()),
            _ => None,
        })
        .collect()
}

fn active<'a>(registry: &'a Registry, scope: &str) -> Option<&'a Version> {
    let reg = registry.registration(registry.get(KEY, scope)?)?;
    registry.version(reg.active?)
}

#[test]
fn a_registration_installs_and_activates_as_the_standard_orders_it() {
    let mut registry = Registry::default();
    let ticket = registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let first = registry.take_effects();
    // The script is fetched past the HTTP cache: `updateViaCache`
    // "imports" keeps the script itself out of it.
    assert!(matches!(
        &first[..],
        [Effect::Fetch { main: true, bypass_cache: true, url, .. }] if url.as_str() == "https://a.test/sw.js"
    ));
    let Effect::Fetch { job: id, .. } = first[0] else {
        unreachable!()
    };
    registry.fetched(id, Ok(script(b"self.oninstall = () => {};")));
    let evaluate = registry.take_effects();
    let [Effect::Evaluate { version, .. }] = evaluate[..] else {
        panic!("evaluated: {evaluate:?}");
    };
    registry.evaluated(id, Some(listening(&["install", "activate"])));
    let effects = registry.take_effects();
    let reg = registry.get(KEY, "https://a.test/").expect("in the map");
    // The promise resolves as the version begins to install, with
    // `installing` set; `updatefound` follows.
    assert_eq!(
        notices(&effects),
        [
            Notice::Slot {
                registration: reg,
                slot: Slot::Installing,
                version: Some(version)
            },
            Notice::State {
                version,
                state: State::Installing
            },
            Notice::UpdateFound { registration: reg },
        ]
    );
    let settle_at = effects
        .iter()
        .position(|e| matches!(e, Effect::Settle { .. }))
        .expect("settled");
    assert!(matches!(
        effects[settle_at + 1],
        Effect::Notice {
            notice: Notice::UpdateFound { .. },
            ..
        }
    ));
    assert_eq!(settled(&effects, ticket), Some(&Outcome::Registration(reg)));
    assert!(matches!(effects.last(), Some(Effect::Install { version: v }) if *v == version));
    registry.installed(version, true);
    let effects = registry.take_effects();
    assert_eq!(
        notices(&effects),
        [
            Notice::Slot {
                registration: reg,
                slot: Slot::Waiting,
                version: Some(version)
            },
            Notice::Slot {
                registration: reg,
                slot: Slot::Installing,
                version: None
            },
            Notice::State {
                version,
                state: State::Installed
            },
            Notice::Slot {
                registration: reg,
                slot: Slot::Active,
                version: Some(version)
            },
            Notice::Slot {
                registration: reg,
                slot: Slot::Waiting,
                version: None
            },
            Notice::State {
                version,
                state: State::Activating
            },
        ]
    );
    assert!(matches!(effects.last(), Some(Effect::Activate { version: v }) if *v == version));
    registry.activated(version);
    assert_eq!(
        notices(&registry.take_effects()),
        [Notice::State {
            version,
            state: State::Activated
        }]
    );
    assert_eq!(
        active(&registry, "https://a.test/").map(|v| v.state),
        Some(State::Activated)
    );
}

#[test]
fn events_it_does_not_listen_for_are_passed_over() {
    let mut registry = Registry::default();
    registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let effects = run(&mut registry, b"1", &[]);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Install { .. } | Effect::Activate { .. }))
    );
    assert_eq!(
        active(&registry, "https://a.test/").map(|v| v.state),
        Some(State::Activated)
    );
}

#[test]
fn registering_the_same_script_again_resolves_with_the_registration_unfetched() {
    let mut registry = Registry::default();
    registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    run(&mut registry, b"1", &[]);
    let reg = registry.get(KEY, "https://a.test/");
    let ticket = registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let effects = run(&mut registry, b"1", &[]);
    assert_eq!(effects.len(), 1);
    assert_eq!(
        settled(&effects, ticket),
        Some(&Outcome::Registration(reg.expect("a registration")))
    );
    // Another `updateViaCache` is an update: the script is fetched again,
    // by the HTTP cache this time, and is the same.
    let mut all = job(JobKind::Register, "https://a.test/", "https://a.test/sw.js");
    all.update_via_cache = UpdateViaCache::All;
    registry.schedule(all);
    let effects = run(&mut registry, b"1", &[]);
    assert!(matches!(
        effects[0],
        Effect::Fetch {
            bypass_cache: false,
            ..
        }
    ));
    assert!(!effects.iter().any(|e| matches!(e, Effect::Evaluate { .. })));
    let reg = registry
        .registration(reg.expect("a registration"))
        .expect("kept");
    assert_eq!(reg.update_via_cache, UpdateViaCache::All);
}

#[test]
fn an_update_makes_a_version_only_when_a_script_changed() {
    let mut registry = Registry::default();
    registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let mut fetched = 0;
    loop {
        let effects = registry.take_effects();
        if effects.is_empty() {
            break;
        }
        for effect in effects {
            match effect {
                Effect::Fetch { job, .. } => {
                    fetched += 1;
                    registry.fetched(job, Ok(script(b"importScripts('lib.js')")));
                }
                Effect::Evaluate { job, .. } => registry.evaluated(
                    job,
                    Some(Evaluated {
                        events: Vec::new(),
                        imports: vec![(url("https://a.test/lib.js"), Arc::from(&b"A"[..]))],
                    }),
                ),
                _ => {}
            }
        }
    }
    assert_eq!(fetched, 1);
    let first = active(&registry, "https://a.test/").map(|v| v.id);
    // The script and its import unchanged: no version.
    let ticket = registry.schedule(job(
        JobKind::Update,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let effects = registry.take_effects();
    let Effect::Fetch { job: id, .. } = effects[0] else {
        panic!("{effects:?}")
    };
    registry.fetched(id, Ok(script(b"importScripts('lib.js')")));
    let effects = registry.take_effects();
    let [
        Effect::Fetch {
            job: id,
            main: false,
            ref url,
            ..
        },
    ] = effects[..]
    else {
        panic!("the import is fetched: {effects:?}");
    };
    assert_eq!(url.as_str(), "https://a.test/lib.js");
    registry.fetched(id, Ok(script(b"A")));
    let effects = registry.take_effects();
    assert!(matches!(
        settled(&effects, ticket),
        Some(Outcome::Registration(_))
    ));
    assert_eq!(active(&registry, "https://a.test/").map(|v| v.id), first);
    // The import changed: a new version, which installs and activates.
    registry.schedule(job(
        JobKind::Update,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let effects = registry.take_effects();
    let Effect::Fetch { job: id, .. } = effects[0] else {
        panic!()
    };
    registry.fetched(id, Ok(script(b"importScripts('lib.js')")));
    let Effect::Fetch { job: id, .. } = registry.take_effects()[0] else {
        panic!()
    };
    registry.fetched(id, Ok(script(b"B")));
    let effects = registry.take_effects();
    let [Effect::Evaluate { job: id, .. }] = effects[..] else {
        panic!("a new version: {effects:?}");
    };
    registry.evaluated(id, Some(listening(&[])));
    run(&mut registry, b"importScripts('lib.js')", &[]);
    assert_ne!(active(&registry, "https://a.test/").map(|v| v.id), first);
}

#[test]
fn a_job_fails_with_the_reference_browsers_words() {
    let cases: [(Result<Fetched, FetchFailure>, &str, &str); 6] = [
        (
            Ok(Fetched {
                status: 404,
                ..script(b"")
            }),
            "TypeError",
            "A bad HTTP response code (404) was received when fetching the script.",
        ),
        (
            Ok(Fetched {
                mime: Some("text/plain".to_owned()),
                javascript: false,
                ..script(b"")
            }),
            "SecurityError",
            "The script has an unsupported MIME type ('text/plain').",
        ),
        (
            Ok(Fetched {
                mime: None,
                javascript: false,
                ..script(b"")
            }),
            "SecurityError",
            "The script does not have a MIME type.",
        ),
        (
            Err(FetchFailure::Network),
            "TypeError",
            "An unknown error occurred when fetching the script.",
        ),
        (
            Err(FetchFailure::Redirect),
            "SecurityError",
            "The script resource is behind a redirect, which is disallowed.",
        ),
        (
            Ok(script(b"")),
            "SecurityError",
            "The path of the provided scope ('/p4/') is not under the max scope allowed ('/p4/sub/'). Adjust the scope, move the Service Worker script, or use the Service-Worker-Allowed HTTP header to allow the scope.",
        ),
    ];
    for (result, name, words) in cases {
        let mut registry = Registry::default();
        let ticket = registry.schedule(job(
            JobKind::Register,
            "https://a.test/p4/",
            "https://a.test/p4/sub/sw.js",
        ));
        let Effect::Fetch { job: id, .. } = registry.take_effects()[0] else {
            panic!()
        };
        registry.fetched(id, result);
        let effects = registry.take_effects();
        assert_eq!(
            settled(&effects, ticket),
            Some(&Outcome::Failed(JobError {
                name,
                message: format!(
                    "Failed to register a ServiceWorker for scope ('https://a.test/p4/') with script ('https://a.test/p4/sub/sw.js'): {words}"
                ),
            }))
        );
        // Its first version failed: the registration is gone.
        assert_eq!(registry.get(KEY, "https://a.test/p4/"), None);
        assert!(registry.registrations.is_empty());
    }
}

#[test]
fn a_scope_the_script_allows_may_be_above_it() {
    let mut registry = Registry::default();
    let ticket = registry.schedule(job(
        JobKind::Register,
        "https://a.test/p4/",
        "https://a.test/p4/sub/sw.js",
    ));
    let Effect::Fetch { job: id, .. } = registry.take_effects()[0] else {
        panic!()
    };
    registry.fetched(
        id,
        Ok(Fetched {
            allowed: Some("/p4/".to_owned()),
            ..script(b"")
        }),
    );
    let effects = run(&mut registry, b"", &[]);
    assert!(matches!(
        settled(&effects, ticket),
        Some(Outcome::Registration(_))
    ));
    for allowed in ["https://b.test/", "http://[", "/p4/sub/x/"] {
        let mut registry = Registry::default();
        let ticket = registry.schedule(job(
            JobKind::Register,
            "https://a.test/p4/",
            "https://a.test/p4/sub/sw.js",
        ));
        let Effect::Fetch { job: id, .. } = registry.take_effects()[0] else {
            panic!()
        };
        registry.fetched(
            id,
            Ok(Fetched {
                allowed: Some(allowed.to_owned()),
                ..script(b"")
            }),
        );
        let effects = registry.take_effects();
        let Some(Outcome::Failed(error)) = settled(&effects, ticket) else {
            panic!("{allowed} refused");
        };
        assert_eq!(error.name, "SecurityError");
    }
}

#[test]
fn a_failed_evaluation_or_install_leaves_the_version_before_it() {
    let mut registry = Registry::default();
    registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    run(&mut registry, b"1", &["install"]);
    let old = active(&registry, "https://a.test/").map(|v| v.id);
    // A script that throws.
    let ticket = registry.schedule(job(
        JobKind::Update,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let Effect::Fetch { job: id, .. } = registry.take_effects()[0] else {
        panic!()
    };
    registry.fetched(id, Ok(script(b"2")));
    registry.take_effects();
    registry.evaluated(id, None);
    let effects = registry.take_effects();
    let Some(Outcome::Failed(error)) = settled(&effects, ticket) else {
        panic!("{effects:?}");
    };
    assert_eq!(
        error.message,
        "Failed to update a ServiceWorker for scope ('https://a.test/') with script ('https://a.test/sw.js'): ServiceWorker script evaluation failed"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Terminate { .. }))
    );
    // An install whose `waitUntil` rejects: the promise had resolved; the
    // version is redundant.
    let ticket = registry.schedule(job(
        JobKind::Update,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let Effect::Fetch { job: id, .. } = registry.take_effects()[0] else {
        panic!()
    };
    registry.fetched(id, Ok(script(b"3")));
    registry.take_effects();
    registry.evaluated(id, Some(listening(&["install"])));
    let effects = registry.take_effects();
    assert!(matches!(
        settled(&effects, ticket),
        Some(Outcome::Registration(_))
    ));
    let Some(Effect::Install { version }) = effects.last() else {
        panic!()
    };
    let version = *version;
    registry.installed(version, false);
    let effects = registry.take_effects();
    assert_eq!(
        notices(&effects)[0],
        Notice::State {
            version,
            state: State::Redundant
        }
    );
    assert_eq!(active(&registry, "https://a.test/").map(|v| v.id), old);
    assert!(registry.version(version).is_none());
}

#[test]
fn a_version_waits_while_its_registration_controls_clients() {
    let mut registry = Registry::default();
    registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    run(&mut registry, b"1", &[]);
    let reg = registry.get(KEY, "https://a.test/").expect("listed");
    registry.use_registration(reg, true);
    registry.schedule(job(
        JobKind::Update,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    run(&mut registry, b"2", &[]);
    let waiting = registry
        .registration(reg)
        .and_then(|r| r.waiting)
        .expect("waiting");
    assert_eq!(
        registry.version(waiting).map(|v| v.state),
        Some(State::Installed)
    );
    // skipWaiting() activates it at once.
    registry.skip_waiting(waiting);
    run(&mut registry, b"", &[]);
    assert_eq!(
        registry.registration(reg).and_then(|r| r.active),
        Some(waiting)
    );
    // Another waits again, and activates once the client goes.
    registry.schedule(job(
        JobKind::Update,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    run(&mut registry, b"3", &[]);
    let third = registry
        .registration(reg)
        .and_then(|r| r.waiting)
        .expect("waiting");
    registry.use_registration(reg, false);
    run(&mut registry, b"", &[]);
    assert_eq!(
        registry.registration(reg).and_then(|r| r.active),
        Some(third)
    );
    assert!(
        registry.version(waiting).is_none(),
        "the old one is redundant"
    );
}

#[test]
fn unregistering_takes_the_registration_out_of_the_map_and_clears_it_once_unused() {
    let mut registry = Registry::default();
    registry.set_writing(true);
    registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    run(&mut registry, b"1", &[]);
    let reg = registry.get(KEY, "https://a.test/").expect("listed");
    registry.use_registration(reg, true);
    let ticket = registry.schedule(job(
        JobKind::Unregister,
        "https://a.test/",
        "https://a.test/",
    ));
    let effects = run(&mut registry, b"", &[]);
    assert_eq!(
        settled(&effects, ticket),
        Some(&Outcome::Unregistered(true))
    );
    assert_eq!(registry.get(KEY, "https://a.test/"), None);
    // Gone from the profile at once, though its client keeps it.
    let written = registry.take_written();
    assert!(!written.is_empty() && written.iter().all(|w| w.record.is_none()));
    // Its client still uses it.
    let version = registry
        .registration(reg)
        .and_then(|r| r.active)
        .expect("kept");
    let ticket = registry.schedule(job(
        JobKind::Unregister,
        "https://a.test/",
        "https://a.test/",
    ));
    let effects = run(&mut registry, b"", &[]);
    assert_eq!(
        settled(&effects, ticket),
        Some(&Outcome::Unregistered(false))
    );
    registry.use_registration(reg, false);
    let effects = registry.take_effects();
    assert!(matches!(effects[0], Effect::Terminate { version: v } if v == version));
    assert!(registry.registration(reg).is_none());
}

#[test]
fn the_longest_scope_that_is_a_prefix_matches() {
    let mut registry = Registry::default();
    for (scope, script) in [
        ("https://a.test/app/", "https://a.test/app/sw.js"),
        ("https://a.test/", "https://a.test/sw.js"),
    ] {
        registry.schedule(job(JobKind::Register, scope, script));
        run(&mut registry, b"1", &[]);
    }
    let scope_of = |url_text: &str, key: &str| {
        registry
            .matching(key, &url(url_text))
            .and_then(|id| registry.registration(id))
            .map(|reg| reg.scope.to_string())
    };
    assert_eq!(
        scope_of("https://a.test/app/page", KEY).as_deref(),
        Some("https://a.test/app/")
    );
    assert_eq!(
        scope_of("https://a.test/apps", KEY).as_deref(),
        Some("https://a.test/")
    );
    assert_eq!(
        scope_of("https://a.test/app/#x", KEY).as_deref(),
        Some("https://a.test/app/")
    );
    assert_eq!(
        scope_of("https://a.test/app/", "https://a.test^https://b.test"),
        None
    );
    // Listed in their scopes' order, as the reference lists them.
    let scopes: Vec<String> = registry
        .registrations(KEY)
        .into_iter()
        .map(|id| registry.registration(id).expect("listed").scope.to_string())
        .collect();
    assert_eq!(scopes, ["https://a.test/", "https://a.test/app/"]);
}

#[test]
fn equivalent_jobs_share_one_and_a_scopes_jobs_run_in_turn() {
    let mut registry = Registry::default();
    let one = registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let two = registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    let three = registry.schedule(job(
        JobKind::Unregister,
        "https://a.test/",
        "https://a.test/",
    ));
    // One fetch: the second job joined the first, the third waits.
    let effects = registry.take_effects();
    assert_eq!(effects.len(), 1);
    let Effect::Fetch { job: id, .. } = effects[0] else {
        panic!()
    };
    registry.fetched(id, Ok(script(b"1")));
    let effects = run(&mut registry, b"1", &[]);
    let reg = settled(&effects, one).cloned();
    assert!(matches!(reg, Some(Outcome::Registration(_))));
    assert_eq!(settled(&effects, two).cloned(), reg);
    assert_eq!(settled(&effects, three), Some(&Outcome::Unregistered(true)));
}

#[test]
fn the_profile_keeps_a_registration_with_its_active_version() {
    let mut registry = Registry::default();
    registry.set_writing(true);
    registry.set_now(1000);
    let mut register = job(JobKind::Register, "https://a.test/", "https://a.test/sw.js");
    register.update_via_cache = UpdateViaCache::All;
    registry.schedule(register);
    run(&mut registry, b"self.onfetch = f", &["fetch"]);
    let reg = registry.get(KEY, "https://a.test/").expect("listed");
    registry.use_registration(reg, true);
    registry.schedule(job(
        JobKind::Update,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    run(&mut registry, b"waits", &[]);
    assert!(registry.has_written());
    let written = registry.take_written();
    assert_eq!(written.len(), 1);
    let record = written[0].record.clone().expect("kept");
    let mut again = Registry::default();
    again.load(KEY, vec![(reg, record), (reg + 1, b"\x09damaged".to_vec())]);
    assert!(again.knows(KEY));
    assert_eq!(again.registrations(KEY), [reg]);
    let r = again.registration(reg).expect("read");
    assert_eq!(r.update_via_cache, UpdateViaCache::All);
    assert_eq!(r.last_check, Some(1000));
    assert!(r.waiting.is_none(), "a waiting version is not kept");
    let version = again.version(r.active.expect("active")).expect("read");
    assert_eq!(&*version.resources[0].1, b"self.onfetch = f");
    assert!(version.handles("fetch"));
    assert_eq!(version.state, State::Activated);
    // A day later, an update goes past the cache whatever its mode.
    again.set_now(1000 + STALE_AFTER + 1);
    again.schedule(job(
        JobKind::Update,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    assert!(matches!(
        again.take_effects()[0],
        Effect::Fetch {
            bypass_cache: true,
            ..
        }
    ));
    // Unregistered, it goes from the profile.
    let mut registry = again;
    registry.set_writing(true);
    registry.clear_key(KEY);
    let written = registry.take_written();
    assert_eq!(written.len(), 1);
    assert!(written[0].record.is_none());
}

#[test]
fn clearing_a_site_rejects_its_jobs_and_clears_its_registrations() {
    let mut registry = Registry::default();
    registry.schedule(job(
        JobKind::Register,
        "https://a.test/",
        "https://a.test/sw.js",
    ));
    run(&mut registry, b"1", &[]);
    let ticket = registry.schedule(job(
        JobKind::Register,
        "https://a.test/x/",
        "https://a.test/x/sw.js",
    ));
    registry.take_effects();
    registry.clear_key(KEY);
    let effects = registry.take_effects();
    let Some(Outcome::Failed(error)) = settled(&effects, ticket) else {
        panic!("{effects:?}");
    };
    assert_eq!(error.name, "AbortError");
    assert!(registry.registrations(KEY).is_empty());
    assert!(registry.registrations.is_empty() && registry.versions.is_empty());
    assert!(registry.jobs.is_empty() && registry.queues.is_empty());
}

#[test]
fn a_registers_urls_are_checked_before_its_job_in_the_reference_browsers_words() {
    let origin = url("https://a.test/page").origin();
    let ok =
        rules::register_urls(&url("https://a.test/app/sw.js#f"), None, &origin).expect("allowed");
    assert_eq!(ok.0.as_str(), "https://a.test/app/sw.js");
    assert_eq!(ok.1.as_str(), "https://a.test/app/");
    let refused = |script: &str, scope: Option<&str>, origin: &Origin| {
        rules::register_urls(&url(script), scope.map(url).as_ref(), origin).expect_err("refused")
    };
    assert_eq!(
        refused("data:text/javascript,1", None, &origin),
        (
            "TypeError",
            "Failed to register a ServiceWorker: The URL protocol of the script ('data:text/javascript,1') is not supported.".to_owned()
        )
    );
    assert_eq!(
        refused("https://b.test/sw.js", None, &origin),
        (
            "SecurityError",
            "Failed to register a ServiceWorker: The origin of the provided scriptURL ('https://b.test') does not match the current origin ('https://a.test').".to_owned()
        )
    );
    assert_eq!(
        refused("https://a.test/sw.js", Some("https://b.test/"), &origin),
        (
            "SecurityError",
            "Failed to register a ServiceWorker: The origin of the provided scope ('https://b.test') does not match the current origin ('https://a.test').".to_owned()
        )
    );
    assert_eq!(
        refused("https://a.test/sw.js", Some("https://a.test/a%2Fb/"), &origin),
        (
            "TypeError",
            "Failed to register a ServiceWorker: The provided scope ('https://a.test/a%2Fb/') or scriptURL ('https://a.test/sw.js') includes a disallowed escape character.".to_owned()
        )
    );
    assert_eq!(
        refused("https://a.test/a%5cb/sw.js", None, &origin).0,
        "TypeError"
    );
}

#[test]
fn the_fuzz_driver_runs_over_many_sequences() {
    let mut seed = 0x9e37_79b9_u32;
    for length in 0..3000 {
        let data: Vec<u8> = (0..(length % 200))
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed.to_le_bytes()[0]
            })
            .collect();
        fuzz_registry(&data);
    }
}
