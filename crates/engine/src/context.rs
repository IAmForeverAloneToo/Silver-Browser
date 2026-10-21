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
    /// The connections (M38): every tab of the profile shares them, the
    /// partition in their key keeping sites apart.
    transport: Arc<net::transport::Transport>,
    hsts: Arc<Mutex<net::Hsts>>,
    permissions: Arc<Permissions>,
    storage: Arc<Mutex<Storage>>,
    downloads: crate::downloads::Downloads,
    /// The settings (M37), and how many times they changed; the
    /// `Accept-Language` the context's clients send, from them.
    settings: Mutex<crate::settings::Settings>,
    settings_changes: AtomicU64,
    accept_language: Arc<Mutex<String>>,
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
                transport: Arc::new(net::transport::Transport::new(
                    net::transport::Config::default(),
                )),
                hsts,
                permissions,
                storage: Arc::new(Mutex::new(storage)),
                downloads: crate::downloads::Downloads::new(),
                settings: Mutex::new(crate::settings::Settings::default()),
                settings_changes: AtomicU64::new(0),
                accept_language: Arc::new(Mutex::new(
                    crate::settings::Settings::default().accept_language(),
                )),
                tabs: AtomicU64::new(1),
                renderers: AtomicU32::new(1),
            }),
        }
    }

    /// Keeps what the context's tabs keep in `profile` from now on, as
    /// [`crate::Engine::set_profile`] says; fails, and does not use it,
    /// when it cannot be read.
    pub fn set_profile(&self, profile: Profile) -> Result<(), profile::Error> {
        self.storage().set_profile(profile, &self.shared.jar)?;
        self.read_settings();
        Ok(())
    }

    /// Takes as its settings those the profile just given holds, if it
    /// holds any.
    pub(crate) fn read_settings(&self) {
        let read = self.storage().take_read_settings();
        if let Some(bytes) = read {
            *self.settings_lock() = crate::settings::Settings::from_bytes(&bytes);
            self.settings_changed();
        }
    }

    /// The settings (M37).
    pub fn settings(&self) -> crate::settings::Settings {
        self.settings_lock().clone()
    }

    /// How many times the settings changed: a reader of them reads again
    /// when it does.
    pub fn settings_changes(&self) -> u64 {
        self.shared.settings_changes.load(Ordering::Relaxed)
    }

    /// Changes the settings by `change`, keeping them in the profile.
    pub fn update_settings(&self, change: impl FnOnce(&mut crate::settings::Settings)) {
        let bytes = {
            let mut settings = self.settings_lock();
            change(&mut settings);
            settings.to_bytes()
        };
        self.storage().keep_settings(bytes);
        self.settings_changed();
    }

    /// Keeps the tabs' URLs `urls` for the session restored (M37): the
    /// settings' own, but no change of them that readers act on.
    pub fn keep_session(&self, urls: Vec<String>) {
        let bytes = {
            let mut settings = self.settings_lock();
            let value = devtools::Json::Array(
                urls.iter()
                    .map(|u| devtools::Json::from(u.as_str()))
                    .collect(),
            );
            let _ = settings.set("session", &value);
            settings.to_bytes()
        };
        self.storage().keep_settings(bytes);
    }

    /// What follows from the settings in the context itself: the jar's
    /// third-party cookies, the clients' languages, the downloads'
    /// folder; and their count of changes.
    fn settings_changed(&self) {
        let settings = self.settings();
        self.shared
            .jar
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_block_third_party(settings.block_third_party_cookies);
        *self
            .shared
            .accept_language
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = settings.accept_language();
        if let Some(folder) = &settings.downloads_folder {
            self.shared
                .downloads
                .set_folder(std::path::PathBuf::from(folder));
        }
        self.shared.settings_changes.fetch_add(1, Ordering::Relaxed);
    }

    fn settings_lock(&self) -> std::sync::MutexGuard<'_, crate::settings::Settings> {
        self.shared
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The `Accept-Language` the context's clients send.
    pub(crate) fn accept_language(&self) -> &Arc<Mutex<String>> {
        &self.shared.accept_language
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

    /// The downloads of the context's tabs (M37).
    pub fn downloads(&self) -> &crate::downloads::Downloads {
        &self.shared.downloads
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

    pub(crate) fn transport(&self) -> &Arc<net::transport::Transport> {
        &self.shared.transport
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
