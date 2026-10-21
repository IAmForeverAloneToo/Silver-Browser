//! What the tabs of one profile share (M37, docs/design/chrome.md, "The
//! context"): the cookie jar, the HTTP cache, the HSTS hosts, the
//! permissions, the storage and its writing to the profile, and the
//! numbers of tabs and renderers, so that no two tabs' renderers share
//! one. A tab is made in a context ([`crate::Engine::join`],
//! [`crate::service::RemoteTab::in_context`]); a tab made alone has a
//! context of its own.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use net::cookies::Jar;
use profile::Profile;

use crate::permissions::Permissions;
use crate::storage::Storage;

/// What the tabs of one profile share; clones are the same context.
#[derive(Clone)]
pub struct Context {
    shared: Arc<Shared>,
}

struct Shared {
    jar: Arc<Mutex<Jar>>,
    cache: Arc<net::Cache>,
    hsts: Arc<Mutex<net::Hsts>>,
    permissions: Arc<Permissions>,
    storage: Arc<Mutex<Storage>>,
    tabs: AtomicU64,
    renderers: AtomicU32,
}

impl Default for Context {
    fn default() -> Self {
        Context::new()
    }
}

impl Context {
    /// A context keeping nothing until it is given a profile.
    pub fn new() -> Context {
        let hsts: Arc<Mutex<net::Hsts>> = Arc::default();
        let permissions: Arc<Permissions> = Arc::default();
        let mut storage = Storage::new();
        storage.set_hsts(hsts.clone());
        storage.set_permissions(permissions.clone());
        Context {
            shared: Arc::new(Shared {
                jar: Arc::default(),
                cache: Arc::default(),
                hsts,
                permissions,
                storage: Arc::new(Mutex::new(storage)),
                tabs: AtomicU64::new(1),
                renderers: AtomicU32::new(1),
            }),
        }
    }

    /// Keeps what the context's tabs keep in `profile` from now on, as
    /// [`crate::Engine::set_profile`] says; fails, and does not use it,
    /// when it cannot be read.
    pub fn set_profile(&self, profile: Profile) -> Result<(), profile::Error> {
        self.storage().set_profile(profile, &self.shared.jar)
    }

    /// Writes to the profile what changed since it was last written, at
    /// `now` on the tabs' clock.
    pub fn flush_profile(&self, now: Duration) -> Result<(), profile::Error> {
        self.storage().flush(&self.shared.jar, now)
    }

    /// Whether `other` is this context.
    pub fn same(&self, other: &Context) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    /// Forgets what tab `tab` kept of its own: its `sessionStorage`.
    pub fn close_tab(&self, tab: u64) {
        self.storage().forget_tab(tab);
    }

    /// A new tab's number.
    pub(crate) fn new_tab(&self) -> u64 {
        self.shared.tabs.fetch_add(1, Ordering::Relaxed)
    }

    /// A new renderer's number, never one another tab's renderer had.
    pub(crate) fn new_renderer(&self) -> u32 {
        self.shared.renderers.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn jar(&self) -> &Arc<Mutex<Jar>> {
        &self.shared.jar
    }

    pub(crate) fn cache(&self) -> &Arc<net::Cache> {
        &self.shared.cache
    }

    pub(crate) fn hsts(&self) -> &Arc<Mutex<net::Hsts>> {
        &self.shared.hsts
    }

    pub(crate) fn permissions(&self) -> &Arc<Permissions> {
        &self.shared.permissions
    }

    pub(crate) fn shared_storage(&self) -> &Arc<Mutex<Storage>> {
        &self.shared.storage
    }

    fn storage(&self) -> std::sync::MutexGuard<'_, Storage> {
        self.shared
            .storage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
