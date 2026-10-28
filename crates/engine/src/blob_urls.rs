//! `blob:` URLs (gap batch A, D41; `docs/design/scripting-essentials.md`):
//! the process's store of the blobs `URL.createObjectURL()` made URLs
//! for, and the fetcher that answers those URLs from it before a request
//! leaves the process.
//!
//! The store is the process's: the engine's in one process, each
//! renderer's own. A URL is kept with its blob's bytes and type, the
//! origin of the global that made it, which alone may fetch it, and that
//! global's [`BlobOwner`], whose end revokes it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use net::{Delivery, NetworkError, Origin, Request, Response, Url};

use crate::loader::Fetcher;

/// The most URLs the store keeps; past it a URL is still made, which
/// then resolves to a network error.
pub(crate) const MAX_BLOB_URLS: usize = 10_000;

/// The most bytes the store's blobs come to, counted as for
/// [`MAX_BLOB_URLS`].
pub(crate) const MAX_BLOB_URL_BYTES: usize = 512 << 20;

struct Entry {
    bytes: Arc<[u8]>,
    kind: String,
    origin: Origin,
    owner: u64,
}

#[derive(Default)]
struct Store {
    entries: HashMap<String, Entry>,
    bytes: usize,
}

impl Store {
    /// Keeps `entry` for `url` if the bounds allow it.
    fn insert(&mut self, url: String, entry: Entry) {
        let len = entry.bytes.len();
        if self.entries.len() < MAX_BLOB_URLS && self.bytes + len <= MAX_BLOB_URL_BYTES {
            self.bytes += len;
            self.entries.insert(url, entry);
        }
    }

    fn remove(&mut self, url: &str) {
        if let Some(entry) = self.entries.remove(url) {
            self.bytes -= entry.bytes.len();
        }
    }
}

fn store() -> MutexGuard<'static, Store> {
    static STORE: LazyLock<Mutex<Store>> = LazyLock::new(Mutex::default);
    STORE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A global's hold on the URLs it made: they are revoked when it is
/// dropped, with the global.
pub(crate) struct BlobOwner(u64);

impl Default for BlobOwner {
    fn default() -> BlobOwner {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        BlobOwner(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl Drop for BlobOwner {
    fn drop(&mut self) {
        let mut store = store();
        let made: Vec<String> = store
            .entries
            .iter()
            .filter(|(_, entry)| entry.owner == self.0)
            .map(|(url, _)| url.clone())
            .collect();
        for url in made {
            store.remove(&url);
        }
    }
}

impl BlobOwner {
    /// `URL.createObjectURL()`: a new URL for a blob of `bytes` and
    /// `kind`, made by a global of `origin`.
    pub(crate) fn create(&self, bytes: Vec<u8>, kind: String, origin: &Origin) -> String {
        let url = format!(
            "blob:{}/{}",
            origin.serialization(),
            crate::service_worker::new_client_id()
        );
        let entry = Entry {
            bytes: bytes.into(),
            kind,
            origin: origin.clone(),
            owner: self.0,
        };
        store().insert(url.clone(), entry);
        url
    }

    /// `URL.revokeObjectURL()` by a global of `origin`: a URL of its
    /// origin's no longer resolves.
    pub(crate) fn revoke(&self, url: &str, origin: &Origin) {
        let Ok(url) = Url::parse(url) else {
            return;
        };
        let mut store = store();
        let key = url.without_fragment();
        if store
            .entries
            .get(key)
            .is_some_and(|entry| entry.origin == *origin)
        {
            store.remove(key);
        }
    }
}

/// The blob of `request`'s URL, its bytes and type, if the store holds
/// one for the request's origin.
fn resolve(request: &Request) -> Option<(Arc<[u8]>, String)> {
    let store = store();
    let entry = store.entries.get(request.url.without_fragment())?;
    (request.origin.as_ref() == Some(&entry.origin))
        .then(|| (entry.bytes.clone(), entry.kind.clone()))
}

/// A fetcher that answers `blob:` URLs from the store, Fetch's scheme
/// fetch, and gives every other request to `inner`.
pub(crate) struct Resolving {
    pub(crate) inner: Arc<dyn Fetcher>,
}

impl Resolving {
    fn answer(request: &Request) -> Result<Response, NetworkError> {
        let blob = resolve(request);
        net::blob(
            request,
            blob.as_ref()
                .map(|(bytes, kind)| (&bytes[..], kind.as_str())),
        )
    }
}

impl Fetcher for Resolving {
    fn fetch(&self, request: &Request) -> Result<Response, NetworkError> {
        match request.url.scheme() {
            "blob" => Resolving::answer(request),
            _ => self.inner.fetch(request),
        }
    }

    fn fetch_streaming(
        &self,
        request: &Request,
        deliver: &mut dyn FnMut(Delivery<'_>),
    ) -> Result<Response, NetworkError> {
        if request.url.scheme() != "blob" {
            return self.inner.fetch_streaming(request, deliver);
        }
        let mut response = Resolving::answer(request)?;
        let body = std::mem::take(&mut response.body);
        deliver(Delivery::Head(&response));
        if !body.is_empty() {
            deliver(Delivery::Body(&body));
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(url: &str) -> Origin {
        Url::parse(url).expect("a URL").origin()
    }

    fn get(url: &str, from: &Origin) -> Result<Response, NetworkError> {
        let mut request = Request::get(Url::parse(url).expect("a URL"), net::Destination::Empty);
        request.origin = Some(from.clone());
        Resolving::answer(&request)
    }

    #[test]
    fn a_blob_url_is_its_origins_alone_and_goes_with_its_owner() {
        let (a, b) = (origin("http://a.test/"), origin("http://b.test/"));
        let owner = BlobOwner::default();
        let url = owner.create(b"bytes".to_vec(), "text/plain".to_owned(), &a);
        assert!(url.starts_with("blob:http://a.test/"), "{url}");
        assert_eq!(get(&url, &a).map(|r| r.body), Ok(b"bytes".to_vec()));
        assert_eq!(get(&url, &b).map(|r| r.body), Err(NetworkError::BlobUrl));
        // Another origin's revoking leaves it.
        owner.revoke(&url, &b);
        assert!(get(&url, &a).is_ok());
        let other = owner.create(Vec::new(), String::new(), &a);
        owner.revoke(&format!("{url}#f"), &a);
        assert_eq!(get(&url, &a).map(|r| r.body), Err(NetworkError::BlobUrl));
        assert!(get(&other, &a).is_ok());
        drop(owner);
        assert_eq!(get(&other, &a).map(|r| r.body), Err(NetworkError::BlobUrl));
        // An opaque origin's URL says `null`.
        let owner = BlobOwner::default();
        let opaque = Origin::new_opaque();
        let url = owner.create(Vec::new(), String::new(), &opaque);
        assert!(url.starts_with("blob:null/"), "{url}");
        assert!(get(&url, &opaque).is_ok());
    }

    #[test]
    fn the_store_keeps_no_more_than_its_bounds() {
        let entry = |len: usize| Entry {
            bytes: vec![0; len].into(),
            kind: String::new(),
            origin: Origin::new_opaque(),
            owner: 0,
        };
        let mut store = Store::default();
        store.insert("big".to_owned(), entry(MAX_BLOB_URL_BYTES));
        store.insert("one more byte".to_owned(), entry(1));
        assert_eq!(store.entries.len(), 1);
        store.remove("big");
        assert_eq!(store.bytes, 0);
        for i in 0..=MAX_BLOB_URLS {
            store.insert(i.to_string(), entry(0));
        }
        assert_eq!(store.entries.len(), MAX_BLOB_URLS);
    }
}
