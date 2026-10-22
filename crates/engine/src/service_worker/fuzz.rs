//! For fuzzing (M39): the registry driven by any sequence of jobs, each
//! effect answered as the input says, compared after each job with a
//! model of the jobs written apart from it; written to a profile of
//! records and read back by a new registry, which must hold the same.

use std::collections::HashMap;
use std::sync::Arc;

use net::Url;

use super::{
    Effect, Evaluated, FetchFailure, Fetched, Job, JobKind, Outcome, Registry, Slot, State,
    UpdateViaCache, record,
};

const KEY: &str = "https://a.test";
const SCOPES: [&str; 3] = [
    "https://a.test/",
    "https://a.test/app/",
    "https://a.test/app/x/",
];
const SCRIPTS: [&str; 4] = [
    "https://a.test/sw.js",
    "https://a.test/app/sw.js",
    "https://a.test/app/x/sw.js",
    "https://a.test/other/sw.js",
];
const IMPORT: &str = "https://a.test/lib.js";
const URLS: [&str; 5] = [
    "https://a.test/",
    "https://a.test/app/",
    "https://a.test/app/x/y#z",
    "https://a.test/apps",
    "https://b.test/app/",
];
const MODES: [UpdateViaCache; 3] = [
    UpdateViaCache::Imports,
    UpdateViaCache::All,
    UpdateViaCache::None,
];

struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    fn pick<T: Copy>(&mut self, table: &[T]) -> T {
        table[usize::from(self.byte()) % table.len()]
    }
}

fn url(text: &str) -> Url {
    Url::parse(text).expect("a URL")
}

fn body(n: u8) -> Arc<[u8]> {
    Arc::from(vec![n])
}

/// How the input answered the main script's fetch.
#[derive(Clone, Copy)]
enum Main {
    Failed(FetchFailure),
    NotFound,
    Plain,
    NoType,
    Ok { body: u8, everywhere: bool },
}

/// What one job's effects were answered with.
#[derive(Default)]
struct Trace {
    main: Vec<Main>,
    imports: Vec<Option<u8>>,
    evaluations: Vec<Option<(bool, Option<u8>)>>,
    installs: Vec<bool>,
}

/// The model's registration: its active version's script, body and
/// imported script's body, and its `updateViaCache`.
#[derive(Clone, PartialEq, Debug)]
struct Model {
    script: &'static str,
    body: u8,
    import: Option<u8>,
    mode: UpdateViaCache,
}

/// Answers the registry's effects until it has none, as the input says;
/// what it answered, and how the tickets settled.
fn drive(
    registry: &mut Registry,
    input: &mut Input<'_>,
    trace: &mut Trace,
) -> HashMap<u64, Outcome> {
    let mut settled = HashMap::new();
    loop {
        let effects = registry.take_effects();
        if effects.is_empty() {
            return settled;
        }
        for effect in effects {
            match effect {
                Effect::Fetch {
                    job, main: true, ..
                } => {
                    let answer = match input.byte() % 8 {
                        0 => Main::Failed(FetchFailure::Network),
                        1 => Main::Failed(FetchFailure::Redirect),
                        2 => Main::NotFound,
                        3 => Main::Plain,
                        4 => Main::NoType,
                        n => Main::Ok {
                            body: input.byte() % 3,
                            everywhere: n == 5,
                        },
                    };
                    trace.main.push(answer);
                    let fetched = |status, mime: Option<&str>, b, allowed: Option<&str>| Fetched {
                        status,
                        javascript: mime == Some("text/javascript"),
                        mime: mime.map(str::to_owned),
                        allowed: allowed.map(str::to_owned),
                        body: body(b),
                        from_cache: false,
                    };
                    let result = match answer {
                        Main::Failed(failure) => Err(failure),
                        Main::NotFound => Ok(fetched(404, Some("text/javascript"), 0, None)),
                        Main::Plain => Ok(fetched(200, Some("text/plain"), 0, None)),
                        Main::NoType => Ok(fetched(200, None, 0, None)),
                        Main::Ok { body, everywhere } => Ok(fetched(
                            200,
                            Some("text/javascript"),
                            body,
                            everywhere.then_some("/"),
                        )),
                    };
                    registry.fetched(job, result);
                }
                Effect::Fetch { job, .. } => {
                    let n = input.byte() % 3;
                    let answer = (n < 2).then_some(n);
                    trace.imports.push(answer);
                    registry.fetched(
                        job,
                        Ok(Fetched {
                            status: if answer.is_some() { 200 } else { 500 },
                            mime: Some("text/javascript".to_owned()),
                            javascript: true,
                            allowed: None,
                            body: body(n),
                            from_cache: true,
                        }),
                    );
                }
                Effect::Evaluate { job, .. } => {
                    let flags = input.byte();
                    let answer = (!flags.is_multiple_of(5)).then(|| {
                        let import = (flags & 8 != 0).then_some((flags >> 4) % 2);
                        (flags & 2 != 0, import)
                    });
                    trace.evaluations.push(answer);
                    registry.evaluated(
                        job,
                        answer.map(|(install, import)| Evaluated {
                            events: [
                                install.then_some("install"),
                                (flags & 4 != 0).then_some("activate"),
                            ]
                            .into_iter()
                            .flatten()
                            .map(str::to_owned)
                            .collect(),
                            imports: import.map(|n| (url(IMPORT), body(n))).into_iter().collect(),
                        }),
                    );
                }
                Effect::Install { version } => {
                    let ok = !input.byte().is_multiple_of(4);
                    trace.installs.push(ok);
                    registry.installed(version, ok);
                }
                Effect::Activate { version } => registry.activated(version),
                Effect::Settle {
                    ticket, outcome, ..
                } => {
                    assert!(
                        settled.insert(ticket, outcome).is_none(),
                        "a promise settles once"
                    );
                }
                Effect::Terminate { .. } | Effect::Notice { .. } => {}
            }
        }
    }
}

/// What the model says a register or update job does, from what its
/// effects were answered with: how it settles (the error's name, if it
/// fails), and the scope's registration after it.
fn model_job(
    model: &mut HashMap<&'static str, Model>,
    kind: JobKind,
    scope: &'static str,
    script: &'static str,
    mode: UpdateViaCache,
    trace: &Trace,
) -> Result<(), &'static str> {
    let had = model.get(scope).cloned();
    if kind == JobKind::Register {
        if had
            .as_ref()
            .is_some_and(|m| m.script == script && m.mode == mode)
        {
            assert!(trace.main.is_empty(), "no fetch for the same script");
            return Ok(());
        }
        if let Some(m) = model.get_mut(scope) {
            m.mode = mode;
        }
    } else if had.is_none() {
        return Err("TypeError");
    }
    let Some(&main) = trace.main.first() else {
        panic!("the script is fetched");
    };
    let failure = match main {
        Main::Failed(FetchFailure::Network) | Main::NotFound => Some("TypeError"),
        Main::Failed(FetchFailure::Redirect) | Main::Plain | Main::NoType => Some("SecurityError"),
        Main::Ok { everywhere, .. } => {
            let dir = &script[..=script.rfind('/').expect("a path")];
            (!everywhere && !scope.starts_with(dir)).then_some("SecurityError")
        }
    };
    if let Some(name) = failure {
        assert!(trace.evaluations.is_empty());
        return Err(name);
    }
    let Main::Ok { body, .. } = main else {
        unreachable!()
    };
    if let Some(old) = had.as_ref().filter(|old| old.body == body) {
        let changed = match old.import {
            Some(stored) => {
                let fetched = trace.imports.first().copied().expect("the import fetched");
                fetched.is_some_and(|n| n != stored)
            }
            None => false,
        };
        if !changed {
            assert!(trace.evaluations.is_empty(), "no new version");
            return Ok(());
        }
    }
    let evaluation = trace.evaluations.first().copied().expect("evaluated");
    let Some((install, import)) = evaluation else {
        return Err("TypeError");
    };
    if install && !trace.installs.first().copied().expect("installed") {
        // The promise resolved when it began to install.
        return Ok(());
    }
    model.insert(
        scope,
        Model {
            script,
            body,
            import,
            mode: had.map_or(mode, |m| {
                if kind == JobKind::Register {
                    mode
                } else {
                    m.mode
                }
            }),
        },
    );
    Ok(())
}

/// The registry against the model: the scopes, each active version's
/// scripts, and nothing left running.
fn compare(registry: &Registry, model: &HashMap<&'static str, Model>) {
    let mut scopes: Vec<String> = registry
        .registrations(KEY)
        .into_iter()
        .map(|id| registry.registration(id).expect("listed").scope.to_string())
        .collect();
    scopes.sort();
    let mut wanted: Vec<String> = model.keys().map(|s| (*s).to_owned()).collect();
    wanted.sort();
    assert_eq!(scopes, wanted, "the scopes");
    for (scope, m) in model {
        let id = registry.get(KEY, scope).expect("the scope's");
        let reg = registry.registration(id).expect("listed");
        assert_eq!(reg.update_via_cache, m.mode, "updateViaCache");
        assert!(reg.installing.is_none() && reg.waiting.is_none());
        let active = registry
            .version(reg.active.expect("an active version"))
            .expect("its version");
        assert_eq!(active.state, State::Activated);
        assert_eq!(active.script.as_str(), m.script);
        assert_eq!(*active.resources[0].1, [m.body]);
        let import = active.resources.get(1).map(|(_, b)| b[0]);
        assert_eq!(import, m.import, "the imported script");
    }
    assert!(registry.jobs.is_empty() && registry.queues.is_empty());
    for (id, reg) in &registry.registrations {
        let slots = [reg.installing, reg.waiting, reg.active];
        for (slot, version) in [Slot::Installing, Slot::Waiting, Slot::Active]
            .into_iter()
            .zip(slots)
        {
            let Some(version) = version else { continue };
            let v = registry.version(version).expect("a slot's version");
            assert_eq!(v.registration, *id);
            let fits = match slot {
                Slot::Installing => v.state == State::Installing,
                Slot::Waiting => v.state == State::Installed,
                Slot::Active => matches!(v.state, State::Activating | State::Activated),
            };
            assert!(fits, "a slot's state");
        }
    }
    for url_text in URLS {
        let found = registry
            .matching(KEY, &url(url_text))
            .and_then(|id| registry.registration(id))
            .map(|reg| reg.scope.to_string());
        let wanted = model
            .keys()
            .filter(|scope| url_text.starts_with(**scope))
            .max_by_key(|scope| scope.len())
            .map(|scope| (*scope).to_owned());
        assert_eq!(found, wanted, "the match of {url_text}");
    }
}

/// A new registry, its writing on, given what `kept` holds.
fn reread(kept: &HashMap<u64, Vec<u8>>) -> Registry {
    let mut registry = Registry::default();
    registry.set_writing(true);
    let mut records: Vec<(u64, Vec<u8>)> = kept.iter().map(|(k, v)| (*k, v.clone())).collect();
    records.sort_by_key(|(id, _)| *id);
    registry.load(KEY, records);
    registry
}

/// Runs the jobs `data` says, checking the registry against the model
/// after each; panics where they differ.
#[doc(hidden)]
pub fn fuzz_registry(data: &[u8]) {
    if let Some(read) = record::decode(data) {
        // A record that reads writes back to one that reads the same.
        let reg = super::Registration {
            id: 1,
            row: 1,
            key: KEY.to_owned(),
            scope: read.scope.clone(),
            update_via_cache: read.update_via_cache,
            installing: None,
            waiting: None,
            active: Some(1),
            last_check: read.last_check,
            listed: true,
            partition: None,
        };
        let version = super::Version {
            id: 1,
            registration: 1,
            script: read.script.clone(),
            resources: read.resources.clone(),
            events: read.events.clone(),
            state: State::Activated,
            skip_waiting: false,
        };
        let again = record::decode(&record::encode(&reg, &version)).expect("written, it reads");
        assert_eq!(again.scope, read.scope);
        assert_eq!(again.resources, read.resources);
        assert_eq!(again.events, read.events);
        assert_eq!(again.last_check, read.last_check);
    }
    let mut input = Input(data);
    let mut kept: HashMap<u64, Vec<u8>> = HashMap::new();
    let mut registry = reread(&kept);
    let mut model: HashMap<&'static str, Model> = HashMap::new();
    let mut steps: u32 = 0;
    while !input.0.is_empty() && steps < 48 {
        steps += 1;
        registry.set_now(u64::from(steps) * 3600 * 7);
        let scope = input.pick(&SCOPES);
        match input.byte() % 6 {
            op @ (0 | 3) => {
                let script = input.pick(&SCRIPTS);
                let mode = input.pick(&MODES);
                let job = Job {
                    kind: JobKind::Register,
                    key: KEY.to_owned(),
                    scope: url(scope),
                    script: url(script),
                    update_via_cache: mode,
                    force_bypass: false,
                    partition: None,
                };
                let first = registry.schedule(job.clone());
                let second = (op == 3).then(|| registry.schedule(job));
                let mut trace = Trace::default();
                let settled = drive(&mut registry, &mut input, &mut trace);
                let wanted = model_job(&mut model, JobKind::Register, scope, script, mode, &trace);
                let outcome = &settled[&first];
                if let Some(second) = second {
                    assert_eq!(&settled[&second], outcome, "equivalent jobs settle alike");
                }
                check(&registry, outcome, wanted, scope);
            }
            1 => {
                let had = model.get(scope).map(|m| (m.script, m.mode));
                let script = had.map_or_else(|| input.pick(&SCRIPTS), |(s, _)| s);
                let mode = had.map_or(UpdateViaCache::Imports, |(_, m)| m);
                let ticket = registry.schedule(Job {
                    kind: JobKind::Update,
                    key: KEY.to_owned(),
                    scope: url(scope),
                    script: url(script),
                    update_via_cache: mode,
                    force_bypass: false,
                    partition: None,
                });
                let mut trace = Trace::default();
                let settled = drive(&mut registry, &mut input, &mut trace);
                let wanted = model_job(&mut model, JobKind::Update, scope, script, mode, &trace);
                check(&registry, &settled[&ticket], wanted, scope);
            }
            2 => {
                let ticket = registry.schedule(Job {
                    kind: JobKind::Unregister,
                    key: KEY.to_owned(),
                    scope: url(scope),
                    script: url(scope),
                    update_via_cache: UpdateViaCache::Imports,
                    force_bypass: false,
                    partition: None,
                });
                let settled = drive(&mut registry, &mut input, &mut Trace::default());
                let had = model.remove(scope).is_some();
                assert_eq!(settled[&ticket], Outcome::Unregistered(had));
            }
            _ => {
                for written in registry.take_written() {
                    match written.record {
                        Some(record) => kept.insert(written.id, record),
                        None => kept.remove(&written.id),
                    };
                }
                registry = reread(&kept);
            }
        }
        compare(&registry, &model);
    }
}

/// A job's outcome against the model's.
fn check(registry: &Registry, outcome: &Outcome, wanted: Result<(), &str>, scope: &str) {
    match (outcome, wanted) {
        (Outcome::Registration(id), Ok(())) => {
            let reg = registry.registration(*id);
            assert!(reg.is_none_or(|reg| reg.scope.as_str() == scope));
        }
        (Outcome::Failed(error), Err(name)) => assert_eq!(error.name, name, "{}", error.message),
        (outcome, wanted) => panic!("settled {outcome:?}, the model says {wanted:?}"),
    }
}
