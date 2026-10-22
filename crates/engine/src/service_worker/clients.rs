//! The service workers' clients (M39, docs/design/service-workers.md
//! "Clients and messages"): each document that reached the service
//! workers, by the id it was given, with its URL, its place, how a task
//! reaches it, and its controller, the registration whose active version
//! it is controlled by. A controller counts as a use of its registration
//! (a waiting version waits while it has one), and a registration's new
//! active version becomes its clients' controller.

use std::sync::Arc;

use devtools::Json;
use net::Url;

use super::{ServiceWorkers, Shown};

/// A client as the service workers know it.
#[derive(Clone)]
pub(crate) struct ClientEntry {
    pub(crate) id: String,
    pub(crate) key: String,
    pub(crate) url: Url,
    /// Whether it is a frame's document rather than a tab's.
    pub(crate) nested: bool,
    pub(crate) controller: Option<u64>,
    /// Queues a task of the text given in it; false once it is gone.
    pub(crate) tell: Arc<dyn Fn(String) -> bool + Send + Sync>,
    /// Whether it is still there.
    pub(crate) alive: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// A new client's id: a UUID's form, from the process's random state.
pub(crate) fn new_client_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let state = std::collections::hash_map::RandomState::new();
    let half = |salt: u64| {
        let mut hasher = state.build_hasher();
        hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
        hasher.write_u64(salt);
        hasher.finish()
    };
    let (a, b) = (half(1), half(2));
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        0x8000 | ((b >> 48) & 0x3fff),
        b & 0xffff_ffff_ffff
    )
}

impl ClientEntry {
    /// The client as a `WindowClient` reads it.
    pub(crate) fn json(&self) -> Json {
        Json::object([
            ("id", Json::from(self.id.as_str())),
            ("url", Json::from(self.url.as_str())),
            ("type", Json::from("window")),
            (
                "frameType",
                Json::from(if self.nested { "nested" } else { "top-level" }),
            ),
            ("visibilityState", Json::from("visible")),
            ("focused", Json::from(!self.nested)),
        ])
    }
}

/// A version as a client's bindings read it.
pub(crate) fn shown_json(shown: &Shown) -> Json {
    Json::object([
        ("id", Json::from(shown.id)),
        ("scriptURL", Json::from(shown.script.as_str())),
        ("state", Json::from(shown.state.name())),
    ])
}

impl ServiceWorkers {
    /// Takes in `entry`, unless a client of its id is there: its
    /// controller the registration matching its URL with an active
    /// version. The controller's active version, if any.
    pub(crate) fn add_client(&self, mut entry: ClientEntry) -> Option<Shown> {
        self.sweep();
        if let Some(known) = self.client(&entry.id) {
            return self.controller_of(&known);
        }
        let controller = self.query(&entry.key, |registry| {
            let id = registry.matching(&entry.key, &entry.url)?;
            registry.registration(id)?.active.map(|_| id)
        });
        entry.controller = controller;
        if let Some(registration) = controller {
            self.use_registration(registration, true);
        }
        let shown = self.controller_of(&entry);
        self.clients().insert(entry.id.clone(), entry);
        shown
    }

    /// The client of `id`, if it is there.
    pub(crate) fn client(&self, id: &str) -> Option<ClientEntry> {
        self.clients().get(id).cloned()
    }

    /// The active version of `entry`'s controller.
    pub(crate) fn controller_of(&self, entry: &ClientEntry) -> Option<Shown> {
        let registration = entry.controller?;
        self.query_any(|registry| registry.view(registration)?.active)
    }

    /// Client `id` is gone: its controller is used no more.
    pub(crate) fn client_gone(&self, id: &str) {
        let gone = self.clients().remove(id);
        if let Some(registration) = gone.and_then(|entry| entry.controller) {
            self.use_registration(registration, false);
        }
    }

    /// Forgets the clients gone, their controllers used no more.
    pub(crate) fn sweep(&self) {
        let gone: Vec<String> = self
            .clients()
            .values()
            .filter(|entry| !(entry.alive)())
            .map(|entry| entry.id.clone())
            .collect();
        for id in gone {
            self.client_gone(&id);
        }
    }

    /// The clients of storage key `key`, in the order they came.
    pub(crate) fn clients_of(&self, key: &str) -> Vec<ClientEntry> {
        self.sweep();
        let mut all: Vec<ClientEntry> = self
            .clients()
            .values()
            .filter(|entry| entry.key == key)
            .cloned()
            .collect();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        all
    }

    /// Makes `registration` the controller of client `id`, telling it of
    /// its new controller `shown`.
    pub(crate) fn control(&self, id: &str, registration: u64, shown: &Shown) {
        let old = {
            let mut clients = self.clients();
            let Some(entry) = clients.get_mut(id) else {
                return;
            };
            entry.controller.replace(registration)
        };
        if old != Some(registration) {
            self.use_registration(registration, true);
            if let Some(old) = old {
                self.use_registration(old, false);
            }
        }
        self.tell_client(id, &controller_task(shown));
    }

    /// Tells client `id` the task `text`; a client gone is forgotten.
    pub(crate) fn tell_client(&self, id: &str, text: &str) -> bool {
        let Some(entry) = self.client(id) else {
            return false;
        };
        if (entry.tell)(text.to_owned()) {
            return true;
        }
        self.client_gone(id);
        false
    }

    /// A registration's new active version `shown` controls the clients
    /// its previous one did (Activate).
    pub(crate) fn new_active(&self, registration: u64, shown: &Shown) {
        let ids: Vec<String> = self
            .clients()
            .values()
            .filter(|entry| entry.controller == Some(registration))
            .map(|entry| entry.id.clone())
            .collect();
        let task = controller_task(shown);
        for id in ids {
            self.tell_client(&id, &task);
        }
    }
}

/// The task telling a client its controller is `shown` now.
fn controller_task(shown: &Shown) -> String {
    Json::object([
        ("type", Json::from("controller")),
        ("version", shown_json(shown)),
    ])
    .write()
}
