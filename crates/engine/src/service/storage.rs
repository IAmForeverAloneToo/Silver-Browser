//! The storage service (M34.1, D31, docs/design/processes.md): Web
//! Storage kept by the browser. Each area is the Storage Standard's, by
//! its storage key: the origin and, for a frame in a context of another
//! site than the top, the top-level site, as M33 keyed it. A renderer
//! names the origin; the browser checks it against the renderer's lock
//! and decides the partition from what it knows of the frame. The
//! quota is the browser's, and so is the profile the `localStorage`
//! areas are written to.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use net::{Origin, Partition};

use super::network::Grants;
use crate::ipc::{Answer, Sender, StorageOp, ToBrowser, ToRenderer};
use crate::storage::{AreaKey, Change, Scope, Source, Storage};

/// A document asking the service workers (M39): its number, whether it
/// hears their notices, whether it is still its frame's, and its id as
/// their client.
struct Asker {
    page: u64,
    listening: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    id: String,
}

impl Asker {
    fn new(page: u64) -> Asker {
        Asker {
            page,
            listening: Arc::default(),
            alive: Arc::new(AtomicBool::new(true)),
            id: crate::service_worker::new_client_id(),
        }
    }
}

/// The browser's Web Storage, for a tab's renderers.
pub struct StorageService {
    storage: Arc<Mutex<Storage>>,
    grants: Arc<Grants>,
    /// The tab's number in its context (M37), whose `sessionStorage`
    /// areas its renderers use.
    tab: u64,
    /// Each renderer's frame's document that asked the service workers
    /// (M39): its number, whether it hears their notices, and whether it
    /// is still the frame's.
    clients: Mutex<HashMap<(u32, u32), Asker>>,
    /// The versions worker renderers run, by their renderer and frame:
    /// a worker renderers' service's (M39).
    versions: Option<Versions>,
}

/// Service workers' versions by their renderer and frame (M39).
pub(crate) type Versions = Arc<Mutex<HashMap<(u32, u32), u64>>>;

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
            clients: Mutex::default(),
            versions: None,
        })
    }

    /// The worker renderers' service, whose frames are the versions of
    /// `versions` (M39).
    pub(crate) fn of_versions(
        storage: Arc<Mutex<Storage>>,
        grants: Arc<Grants>,
        versions: Versions,
    ) -> Arc<StorageService> {
        Arc::new(StorageService {
            storage,
            grants,
            tab: 0,
            clients: Mutex::default(),
            versions: Some(versions),
        })
    }

    /// A request of the service workers' of document `page` of frame
    /// `frame` of renderer `renderer` (M39), of `area`, its URL `url`
    /// the area's origin's; what it hears later goes to the renderer.
    #[allow(clippy::too_many_arguments)]
    fn service_worker(
        &self,
        renderer: u32,
        frame: u32,
        area: AreaKey,
        request: &str,
        url: net::Url,
        base: net::Url,
        page: u64,
        reply: &Sender<ToRenderer>,
    ) -> Result<String, String> {
        if url.origin() != area.origin {
            return Err(format!(
                "a document of {} for {}",
                url,
                area.origin.serialization()
            ));
        }
        // A version asks as itself: a client of none.
        let version = self.versions.as_ref().and_then(|versions| {
            versions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&(renderer, frame))
                .copied()
        });
        let (listening, alive, id) = {
            let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = clients
                .entry((renderer, frame))
                .or_insert_with(|| Asker::new(page));
            if entry.page != page {
                // The frame's document before it hears nothing more.
                entry.alive.store(false, Ordering::SeqCst);
                *entry = Asker::new(page);
            }
            if version.is_some() {
                // Its host tells it the notices already.
                entry.listening.store(true, Ordering::SeqCst);
                entry.id.clear();
            }
            (
                entry.listening.clone(),
                entry.alive.clone(),
                entry.id.clone(),
            )
        };
        // A frame in a context of another site is taken for nested; one of
        // its own site, the browser cannot tell from its grant.
        let nested = self
            .grants
            .frame(renderer, frame)
            .is_ok_and(|(_, grant)| grant.cross_site);
        let watched = alive.clone();
        let sender = reply.clone();
        let client = crate::service_worker::page::Client {
            key: area.serialization(),
            partition: area.partition.clone(),
            origin: area.origin,
            base,
            url,
            page,
            id,
            nested,
            version,
            alive: Arc::new(move || watched.load(Ordering::SeqCst)),
            tell: Arc::new(move |task| {
                alive.load(Ordering::SeqCst)
                    && sender
                        .send(&ToRenderer::ServiceWorkerTask { frame, page, task })
                        .is_ok()
            }),
            listening,
        };
        let driver = self
            .storage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .service_worker_driver();
        Ok(match driver {
            Some(driver) => crate::service_worker::page::serve(&driver, &client, request),
            None => crate::storage::NO_SERVICE_WORKERS.to_owned(),
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
        if let ToBrowser::Broadcast {
            frame,
            origin,
            partition,
            name,
            data,
            sender,
        } = message
        {
            // Held to the storage key the frame may use (gap batch A).
            let key = self.key(renderer, frame, origin, partition)?;
            self.storage
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .post(crate::storage::Broadcast {
                    key,
                    name,
                    data,
                    sender,
                    source: Source::Renderer(renderer),
                });
            return Ok(None);
        }
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
        let area = self.key(renderer, frame, origin, partition)?;
        if let StorageOp::ServiceWorker {
            request,
            url,
            base,
            page,
        } = op
        {
            let answer =
                self.service_worker(renderer, frame, area, &request, *url, *base, page, reply)?;
            let _ = reply.send(&ToRenderer::Answer {
                call,
                answer: Answer::ServiceWorker { answer },
            });
            return Ok(None);
        }
        let key = area.serialization();
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
            StorageOp::ServiceWorker { .. } => return Ok(None),
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
