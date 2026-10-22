//! A worker renderer's service workers (M39, D35): the versions the
//! browser starts in it, each a thread of [`Versions`], their fetches
//! and caches the browser's through the link as one of its frames, their
//! registry the browser's.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::ipc::{Bytes, ToBrowser, ToRenderer};
use crate::renderer::link::{Link, RemoteAreas, RemoteFetcher};
use crate::scripts::service_workers::{Launch, Upstream, Versions};
use crate::service_worker::page::Client;
use crate::service_worker::{Evaluated, Start};
use crate::storage::Areas;

/// The browser, as a version's registry.
struct Browser {
    link: Arc<Link>,
    frame: u32,
}

impl Upstream for Browser {
    fn skip_waiting(&self, version: u64) {
        let _ = self
            .link
            .send(&ToBrowser::ServiceWorkerSkipWaiting { version });
    }

    fn serve(&self, client: &Client, request: &str) -> String {
        RemoteAreas::new(self.link.clone(), self.frame).service_worker(client, request)
    }
}

/// The renderer's versions, and the frames the browser numbered them.
pub(crate) struct ServiceWorkerSide {
    link: Arc<Link>,
    versions: Versions,
    frames: Arc<Mutex<HashMap<u32, u64>>>,
}

impl ServiceWorkerSide {
    pub(crate) fn new(link: Arc<Link>) -> ServiceWorkerSide {
        ServiceWorkerSide {
            link,
            versions: Versions::default(),
            frames: Arc::default(),
        }
    }

    /// Takes `message` if it is the service workers'; gives it back
    /// otherwise.
    pub(crate) fn handle(&mut self, message: ToRenderer) -> Option<ToRenderer> {
        match message {
            ToRenderer::StartServiceWorker {
                version,
                frame,
                registration,
                key,
                scope,
                script,
                source,
                stored,
                own,
                partition,
                evaluate,
            } => {
                let link = self.link.clone();
                let evaluated = evaluate.then(|| {
                    let link = link.clone();
                    Box::new(move |result: Option<Evaluated>| {
                        let (listened, imports) = match result {
                            Some(e) => (
                                Some(e.events),
                                e.imports
                                    .into_iter()
                                    .map(|(url, body)| (url, Bytes(body.to_vec())))
                                    .collect(),
                            ),
                            None => (None, Vec::new()),
                        };
                        let _ = link.send(&ToBrowser::ServiceWorkerEvaluated {
                            version,
                            listened,
                            imports,
                        });
                    }) as crate::service_worker::Done<Option<Evaluated>>
                });
                let frames = self.frames.clone();
                let ended = {
                    let link = link.clone();
                    Box::new(move || {
                        frames
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .retain(|_, v| *v != version);
                        let _ = link.send(&ToBrowser::ServiceWorkerStopped { version });
                    }) as Box<dyn FnOnce() + Send>
                };
                self.frames
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(frame, version);
                let launched = self.versions.launch(Launch {
                    start: Start {
                        version,
                        registration,
                        key,
                        scope: *scope,
                        script: *script,
                        source: Arc::from(source.0),
                        partition,
                    },
                    stored: stored
                        .into_iter()
                        .map(|(url, body)| (url, Arc::from(body.0)))
                        .collect(),
                    own,
                    client: Arc::new(RemoteFetcher::new(link.clone(), frame)),
                    areas: Arc::new(RemoteAreas::new(link.clone(), frame)),
                    upstream: Arc::new(Browser { link, frame }),
                    evaluated,
                    ended: Some(ended),
                });
                if !launched {
                    let _ = self.link.send(&ToBrowser::ServiceWorkerStopped { version });
                }
                None
            }
            ToRenderer::ServiceWorkerEvent {
                version,
                event,
                kind,
                data,
            } => {
                let link = self.link.clone();
                let done = Box::new(move |ok| {
                    let _ = link.send(&ToBrowser::ServiceWorkerEventDone {
                        version,
                        event,
                        ok,
                        ran: true,
                    });
                });
                if self.versions.give(version, &kind, data, done).is_err() {
                    let _ = self.link.send(&ToBrowser::ServiceWorkerEventDone {
                        version,
                        event,
                        ok: false,
                        ran: false,
                    });
                }
                None
            }
            ToRenderer::ServiceWorkerNotice { key, task } => {
                self.versions.tell(&key, &task);
                None
            }
            ToRenderer::StopServiceWorker { version } => {
                self.versions.terminate(version);
                None
            }
            // What its own registration's methods settled, for a version.
            ToRenderer::ServiceWorkerTask { frame, page, task } => {
                let version = self
                    .frames
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&frame)
                    .copied();
                match version {
                    Some(version) => {
                        self.versions.tell_version(version, &task);
                        None
                    }
                    None => Some(ToRenderer::ServiceWorkerTask { frame, page, task }),
                }
            }
            other => Some(other),
        }
    }
}
