//! What a client's `navigator.serviceWorker` asks of the registry (M39):
//! the operations of `ServiceWorkerContainer` and
//! `ServiceWorkerRegistration`, as JSON, the client's checks done here,
//! jobs scheduled through the context's driver and their promises
//! settled by a task of the client's; and the registry's views and
//! notices as the client's bindings read them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use devtools::Json;
use net::{Origin, Url};

use super::{Job, JobKind, Notice, Outcome, ServiceWorkers, Settled, Shown, UpdateViaCache, View};

/// The client asking: its storage key and partition, its origin, the
/// URL its own are parsed against and its own URL, and how a task
/// reaches it.
pub(crate) struct Client {
    pub(crate) key: String,
    pub(crate) partition: Option<net::Partition>,
    pub(crate) origin: Origin,
    pub(crate) base: Url,
    pub(crate) url: Url,
    /// The document's number in its engine, which a renderer's tasks
    /// name.
    pub(crate) page: u64,
    /// Queues a task of the text given in the client; false once the
    /// client is gone.
    pub(crate) tell: Arc<dyn Fn(String) -> bool + Send + Sync>,
    /// Whether it hears the registry's notices yet.
    pub(crate) listening: Arc<AtomicBool>,
}

fn error(name: &str, message: &str) -> String {
    Json::object([(
        "error",
        Json::object([("name", Json::from(name)), ("message", Json::from(message))]),
    )])
    .write()
}

fn result(value: Json) -> String {
    Json::object([("result", value)]).write()
}

fn shown_json(shown: Option<&Shown>) -> Json {
    match shown {
        Some(s) => Json::object([
            ("id", Json::from(s.id)),
            ("scriptURL", Json::from(s.script.as_str())),
            ("state", Json::from(s.state.name())),
        ]),
        None => Json::Null,
    }
}

/// A registration's view as the bindings read it.
pub(crate) fn view_json(view: &View) -> Json {
    Json::object([
        ("id", Json::from(view.id)),
        ("scope", Json::from(view.scope.as_str())),
        ("updateViaCache", Json::from(view.update_via_cache.name())),
        ("installing", shown_json(view.installing.as_ref())),
        ("waiting", shown_json(view.waiting.as_ref())),
        ("active", shown_json(view.active.as_ref())),
    ])
}

/// A notice as the bindings read it, with its registration's view.
pub(crate) fn notice_json(notice: &Notice, view: &View) -> String {
    let mut fields = match notice {
        Notice::Slot {
            registration, slot, ..
        } => vec![
            ("type", Json::from("slot")),
            ("registration", Json::from(*registration)),
            ("slot", Json::from(slot.name())),
        ],
        Notice::State { version, state } => vec![
            ("type", Json::from("state")),
            ("version", Json::from(*version)),
            ("state", Json::from(state.name())),
        ],
        Notice::UpdateFound { registration } => vec![
            ("type", Json::from("updatefound")),
            ("registration", Json::from(*registration)),
        ],
    };
    fields.push(("view", view_json(view)));
    Json::object(fields).write()
}

/// How the promise `promise` settled, as the bindings read it.
fn settle_json(promise: f64, settled: &Settled) -> String {
    let mut fields = vec![
        ("type", Json::from("settle")),
        ("promise", Json::Number(promise)),
    ];
    match (&settled.outcome, &settled.view) {
        (Outcome::Registration(_), Some(view)) => fields.push(("view", view_json(view))),
        (Outcome::Unregistered(was), _) => fields.push(("result", Json::from(*was))),
        (Outcome::Failed(failure), _) => fields.push((
            "error",
            Json::object([
                ("name", Json::from(failure.name)),
                ("message", Json::from(failure.message.as_str())),
            ]),
        )),
        // A registration gone before it settled.
        (Outcome::Registration(_), None) => fields.push((
            "error",
            Json::object([
                ("name", Json::from("InvalidStateError")),
                ("message", Json::from("The registration is gone.")),
            ]),
        )),
    }
    Json::object(fields).write()
}

/// Answers the client's `request`: an operation of its
/// `navigator.serviceWorker` or of a registration's. The client hears
/// the registry's notices of its storage key from its first.
pub(crate) fn serve(workers: &ServiceWorkers, client: &Client, request: &str) -> String {
    let Ok(request) = Json::parse(request) else {
        return error("TypeError", "Invalid request.");
    };
    listen(workers, client);
    let text = |name: &str| request.get(name).and_then(Json::as_str).map(str::to_owned);
    let number = |name: &str| request.get(name).and_then(Json::as_u64);
    match text("op").as_deref() {
        Some("check") => result(Json::Null),
        Some("register") => register(workers, client, &request),
        Some("getRegistration") => {
            let url = match text("url") {
                None => client.url.clone(),
                Some(raw) => match client.base.join(&raw) {
                    Ok(url) => url,
                    Err(_) => {
                        return error(
                            "TypeError",
                            &format!(
                                "Failed to get a ServiceWorkerRegistration: The URL provided ('{raw}') is invalid."
                            ),
                        );
                    }
                },
            };
            if url.origin() != client.origin {
                return error(
                    "SecurityError",
                    &format!(
                        "Failed to get a ServiceWorkerRegistration: The origin of the provided documentURL ('{}') does not match the current origin ('{}').",
                        url.origin().serialization(),
                        client.origin.serialization()
                    ),
                );
            }
            let view = workers.query(&client.key, |registry| {
                registry
                    .matching(&client.key, &url)
                    .and_then(|id| registry.view(id))
            });
            result(view.as_ref().map_or(Json::Null, view_json))
        }
        Some("getRegistrations") => {
            let views = workers.query(&client.key, |registry| {
                registry
                    .registrations(&client.key)
                    .into_iter()
                    .filter_map(|id| registry.view(id))
                    .collect::<Vec<View>>()
            });
            result(Json::Array(views.iter().map(view_json).collect()))
        }
        Some(op @ ("update" | "unregister")) => {
            let id = number("registration").unwrap_or(0);
            let found = workers.query(&client.key, |registry| {
                let reg = registry.registration(id).filter(|r| r.key == client.key)?;
                let newest = reg
                    .newest()
                    .and_then(|v| registry.version(v))
                    .map(|v| v.script.clone());
                Some((reg.scope.clone(), reg.update_via_cache, newest))
            });
            // A registration cleared still has its object, and its scope.
            let given = text("scope")
                .and_then(|scope| Url::parse(&scope).ok())
                .filter(|scope| scope.origin() == client.origin);
            let Some((scope, update_via_cache, newest)) =
                found.or_else(|| given.map(|scope| (scope, UpdateViaCache::default(), None)))
            else {
                return error("InvalidStateError", "The registration is gone.");
            };
            let (kind, script) = if op == "update" {
                let Some(script) = newest else {
                    return error(
                        "InvalidStateError",
                        &format!(
                            "Failed to update a ServiceWorker for scope ('{scope}') with script ('Unknown'): The object is in an invalid state."
                        ),
                    );
                };
                (JobKind::Update, script)
            } else {
                (JobKind::Unregister, scope.clone())
            };
            schedule(
                workers,
                client,
                &request,
                Job {
                    kind,
                    key: client.key.clone(),
                    scope,
                    script,
                    update_via_cache,
                    force_bypass: false,
                    partition: client.partition.clone(),
                },
            )
        }
        _ => error("TypeError", "Invalid request."),
    }
}

/// Has the client hear the notices of its storage key, once.
fn listen(workers: &ServiceWorkers, client: &Client) {
    if client.listening.swap(true, Ordering::SeqCst) {
        return;
    }
    let key = client.key.clone();
    let tell = client.tell.clone();
    workers.listen(Arc::new(move |of: &str, notice: &Notice, view: &View| {
        if of != key {
            return true;
        }
        tell(notice_json(notice, view))
    }));
}

/// `register()`: the URLs as the client parses and checks them, then the
/// job.
fn register(workers: &ServiceWorkers, client: &Client, request: &Json) -> String {
    let text = |name: &str| request.get(name).and_then(Json::as_str).map(str::to_owned);
    let raw = text("script").unwrap_or_default();
    let Ok(script) = client.base.join(&raw) else {
        return error(
            "TypeError",
            &format!(
                "Failed to register a ServiceWorker: The URL protocol of the script ('{raw}') is not supported."
            ),
        );
    };
    let scope = match text("scope") {
        None => None,
        Some(raw) => match client.base.join(&raw) {
            Ok(scope) => Some(scope),
            Err(_) => {
                return error(
                    "TypeError",
                    &format!(
                        "Failed to register a ServiceWorker: The URL protocol of the scope ('{raw}') is not supported."
                    ),
                );
            }
        },
    };
    let (script, scope) = match super::rules::register_urls(&script, scope.as_ref(), &client.origin)
    {
        Ok(urls) => urls,
        Err((name, message)) => return error(name, &message),
    };
    // Module scripts wait for G41: a module worker does not evaluate.
    if text("type").as_deref() == Some("module") {
        return error(
            "TypeError",
            &format!(
                "Failed to register a ServiceWorker for scope ('{scope}') with script ('{script}'): ServiceWorker script evaluation failed"
            ),
        );
    }
    let update_via_cache = text("updateViaCache")
        .as_deref()
        .and_then(UpdateViaCache::parse)
        .unwrap_or_default();
    schedule(
        workers,
        client,
        request,
        Job {
            kind: JobKind::Register,
            key: client.key.clone(),
            scope,
            script,
            update_via_cache,
            force_bypass: false,
            partition: client.partition.clone(),
        },
    )
}

/// Schedules `job`, its promise the request's, settled by a task of the
/// client's.
fn schedule(workers: &ServiceWorkers, client: &Client, request: &Json, job: Job) -> String {
    let promise = request.get("promise").and_then(Json::as_f64).unwrap_or(0.0);
    let tell = client.tell.clone();
    workers.schedule(
        job,
        Box::new(move |settled: Settled| {
            tell(settle_json(promise, &settled));
        }),
    );
    result(Json::Null)
}
