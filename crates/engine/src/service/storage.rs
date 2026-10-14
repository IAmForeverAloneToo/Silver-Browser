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
use crate::storage::{AreaKey, Storage};

/// The browser's Web Storage, for every renderer.
pub struct StorageService {
    storage: Arc<Mutex<Storage>>,
    grants: Arc<Grants>,
}

impl StorageService {
    /// The service of `storage`, the browser's, checking renderers against
    /// `grants`. The browser's tab makes it from step 5 (M34.5).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new(storage: Arc<Mutex<Storage>>, grants: Arc<Grants>) -> Arc<StorageService> {
        Arc::new(StorageService { storage, grants })
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
        let mut storage = self.storage.lock().unwrap_or_else(PoisonError::into_inner);
        let answer = match op {
            StorageOp::Length {} => Answer::StorageLength {
                length: storage.area(local, &key).len() as u64,
            },
            StorageOp::Key { index } => Answer::StorageValue {
                value: usize::try_from(index)
                    .ok()
                    .and_then(|index| storage.area(local, &key).key(index)),
            },
            StorageOp::Keys {} => Answer::StorageKeys {
                keys: storage.area(local, &key).keys().to_vec(),
            },
            StorageOp::Get { item } => Answer::StorageValue {
                value: storage.area(local, &key).get(&item).map(str::to_owned),
            },
            StorageOp::Set { item, value } => match storage.set_item(local, &key, &item, &value) {
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
                value: storage.remove_item(local, &key, &item),
            },
            StorageOp::Clear {} => Answer::StorageCleared {
                cleared: storage.clear_area(local, &key),
            },
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
