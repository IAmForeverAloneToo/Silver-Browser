//! The storage service (M34.1, D31, docs/design/processes.md): Web
//! Storage kept by the browser. Each area is the Storage Standard's, by
//! its storage key: the origin and, for a frame in a context of another
//! site than the top, the top-level site, as M33 keyed it. A renderer
//! names the origin; the browser checks it against the renderer's lock
//! and decides the partition from what it knows of the frame. The
//! quota is the browser's, and so is the profile the `localStorage`
//! areas are written to.

use std::sync::{Arc, Mutex, PoisonError};

use net::{Origin, Partition};

use super::network::Grants;
use crate::ipc::{Answer, Sender, StorageOp, ToBrowser, ToRenderer};
use crate::storage::{AreaKey, Change, Scope, Source, Storage};

/// The browser's Web Storage, for a tab's renderers.
pub struct StorageService {
    storage: Arc<Mutex<Storage>>,
    grants: Arc<Grants>,
    /// The tab's number in its context (M37), whose `sessionStorage`
    /// areas its renderers use.
    tab: u64,
}

impl StorageService {
    /// The service of `storage`, the browser's, for tab `tab`'s
    /// renderers, checking them against `grants`.
    pub(crate) fn new(
        storage: Arc<Mutex<Storage>>,
        grants: Arc<Grants>,
        tab: u64,
    ) -> Arc<StorageService> {
        Arc::new(StorageService {
            storage,
            grants,
            tab,
        })
    }

    /// A change frame `frame` of renderer `renderer` says its document
    /// made (M37), checked as an operation is, for the other renderers
    /// of the context to hear of.
    #[allow(clippy::too_many_arguments)]
    fn changed(
        &self,
        renderer: u32,
        frame: u32,
        local: bool,
        origin: net::Origin,
        partition: Option<Partition>,
        key: Option<String>,
        old: Option<String>,
        new: Option<String>,
        url: String,
    ) -> Result<(), String> {
        let area = self.key(renderer, frame, origin, partition)?;
        // A URL of another origin is the renderer's word alone: left out.
        let url = net::Url::parse(&url)
            .ok()
            .filter(|url| url.origin() == area.origin)
            .map_or_else(String::new, |url| url.as_str().to_owned());
        let change = Change {
            scope: Scope::of(local, self.tab),
            origin: area.serialization(),
            key,
            old,
            new,
            url,
            page: 0,
            source: Source::Renderer(renderer),
            owner: area.origin,
            partition: area.partition,
        };
        self.storage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .broadcast(change);
        Ok(())
    }

    /// The area key a frame of renderer `renderer` may use for `origin`
    /// with the partition it claims; Err for one it may not.
    fn key(
        &self,
        renderer: u32,
        frame: u32,
        origin: Origin,
        partition: Option<Partition>,
    ) -> Result<AreaKey, String> {
        let (lock, grant) = self.grants.frame(renderer, frame)?;
        if origin.is_opaque() || !lock.admits(&origin) {
            return Err(format!("the storage of {}", origin.serialization()));
        }
        let partition = super::network::partition(partition, &grant)?;
        Ok(AreaKey { origin, partition })
    }

    /// Serves `message` of renderer `renderer` if it is Web Storage's.
    pub fn serve(
        &self,
        renderer: u32,
        message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        if let ToBrowser::StorageChanged {
            frame,
            local,
            origin,
            partition,
            key,
            old,
            new,
            url,
        } = message
        {
            self.changed(
                renderer, frame, local, origin, partition, key, old, new, url,
            )?;
            return Ok(None);
        }
        let ToBrowser::Storage {
            call,
            frame,
            local,
            origin,
            partition,
            op,
        } = message
        else {
            return Ok(Some(message));
        };
        let key = self.key(renderer, frame, origin, partition)?;
        let key = key.serialization();
        let scope = Scope::of(local, self.tab);
        let mut storage = self.storage.lock().unwrap_or_else(PoisonError::into_inner);
        let answer = match op {
            StorageOp::Length {} => Answer::StorageLength {
                length: storage.area(scope, &key).len() as u64,
            },
            StorageOp::Key { index } => Answer::StorageValue {
                value: usize::try_from(index)
                    .ok()
                    .and_then(|index| storage.area(scope, &key).key(index)),
            },
            StorageOp::Keys {} => Answer::StorageKeys {
                keys: storage.area(scope, &key).keys().to_vec(),
            },
            StorageOp::Get { item } => Answer::StorageValue {
                value: storage.area(scope, &key).get(&item).map(str::to_owned),
            },
            StorageOp::Set { item, value } => match storage.set_item(scope, &key, &item, &value) {
                Err(_) => Answer::StorageSet {
                    stored: false,
                    old: None,
                    changed: false,
                },
                Ok(None) => Answer::StorageSet {
                    stored: true,
                    old: None,
                    changed: false,
                },
                Ok(Some(old)) => Answer::StorageSet {
                    stored: true,
                    old,
                    changed: true,
                },
            },
            StorageOp::Remove { item } => Answer::StorageValue {
                value: storage.remove_item(scope, &key, &item),
            },
            StorageOp::Clear {} => Answer::StorageCleared {
                cleared: storage.clear_area(scope, &key),
            },
            StorageOp::Usage {} => Answer::StorageLength {
                length: crate::storage::devtools::usage(&mut storage, &key),
            },
            StorageOp::Idb { request } => Answer::Idb {
                answer: crate::idb::serve(storage.idb(&key), &key, &request),
            },
            StorageOp::Caches { request, bytes } => {
                let (answer, bytes) = storage.serve_caches(&key, &request, &bytes.0);
                Answer::Caches {
                    answer,
                    bytes: crate::ipc::Bytes(bytes),
                }
            }
        };
        drop(storage);
        let _ = reply.send(&ToRenderer::Answer { call, answer });
        Ok(None)
    }
}

impl super::Serve for StorageService {
    fn serve(
        &self,
        renderer: u32,
        message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        StorageService::serve(self, renderer, message, reply)
    }
}
