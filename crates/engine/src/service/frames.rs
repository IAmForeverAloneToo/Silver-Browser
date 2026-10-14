//! The tab's frames as the browser knows them (M34.1, D33,
//! docs/design/processes.md, "Frames across processes"): the browser
//! numbers every frame of the tab, knows which renderer's engine has it
//! and which renderer shows its documents when another does, and carries
//! what one frame posts to a frame of another renderer, the sender's
//! frame and origin checked against the renderer that sent it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};

use net::{Origin, Url};

use super::network::{Grants, Lock};
use crate::ipc::{Answer, Sender, ToBrowser, ToRenderer, WireTask};

/// What the browser knows of a frame it numbered.
#[derive(Debug, Clone)]
pub(crate) struct FrameInfo {
    /// The renderer whose engine has the frame: its parent's, or, for a
    /// tab's top-level frame, its own.
    pub(crate) renderer: u32,
    pub(crate) parent: Option<u32>,
    /// Its document's origin and URL, as the browser last saw one commit.
    pub(crate) origin: Option<Origin>,
    pub(crate) url: Option<Url>,
    /// The sandboxing flags its parent says its next document is made
    /// with.
    pub(crate) sandbox: u32,
    /// The renderer and view showing its documents, when another
    /// renderer's do.
    pub(crate) root: Option<(u32, u32)>,
}

/// What the frames' service tells the browser's tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameEvent {
    /// Frame `key`, shown by another renderer, left its parent's page.
    Dropped(u32),
}

/// The tab's frames, by the browser's numbers, and where to send what is
/// posted to each.
#[derive(Default)]
pub(crate) struct Registry {
    pub(crate) frames: HashMap<u32, FrameInfo>,
    next: u32,
    senders: HashMap<u32, Sender<ToRenderer>>,
}

impl Registry {
    /// A new frame's number.
    pub(crate) fn number(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    /// Renderer `renderer` is reached through `sender` from now on.
    pub(crate) fn reach(&mut self, renderer: u32, sender: Sender<ToRenderer>) {
        self.senders.insert(renderer, sender);
    }

    /// Forgets renderer `renderer`'s way in.
    pub(crate) fn unreach(&mut self, renderer: u32) {
        self.senders.remove(&renderer);
    }

    /// The renderer showing frame `key`'s documents.
    pub(crate) fn host(&self, key: u32) -> Option<u32> {
        let info = self.frames.get(&key)?;
        Some(info.root.map_or(info.renderer, |(renderer, _)| renderer))
    }

    /// Frame `key`'s ancestors, its parent first: the browser's number,
    /// origin and URL of each.
    pub(crate) fn ancestors(&self, key: u32) -> Vec<(u32, Origin, Option<Url>)> {
        let mut out = Vec::new();
        let mut at = self.frames.get(&key).and_then(|info| info.parent);
        while let Some(ancestor) = at {
            let info = self.frames.get(&ancestor);
            out.push((
                ancestor,
                info.and_then(|info| info.origin.clone())
                    .unwrap_or_else(Origin::new_opaque),
                info.and_then(|info| info.url.clone()),
            ));
            at = info.and_then(|info| info.parent);
            if out.len() > crate::MAX_FRAME_DEPTH + 1 {
                break;
            }
        }
        out
    }

    /// Frame `key` and the frames under it.
    pub(crate) fn subtree(&self, key: u32) -> Vec<u32> {
        let mut out = vec![key];
        let mut at = 0;
        while at < out.len() {
            let parent = out[at];
            out.extend(
                self.frames
                    .iter()
                    .filter(|(_, info)| info.parent == Some(parent))
                    .map(|(child, _)| *child),
            );
            at += 1;
        }
        out
    }
}

pub(crate) type SharedRegistry = Arc<Mutex<Registry>>;

pub(crate) fn lock(registry: &SharedRegistry) -> MutexGuard<'_, Registry> {
    registry.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The frames' service: numbers for new frames, posts between renderers,
/// the frames dropped and their parents' word on their sandboxing.
pub(crate) struct FramesService {
    registry: SharedRegistry,
    grants: Arc<Grants>,
    events: Mutex<mpsc::Sender<FrameEvent>>,
    wake: crate::loader::Waker,
}

impl FramesService {
    pub(crate) fn new(
        registry: SharedRegistry,
        grants: Arc<Grants>,
        events: mpsc::Sender<FrameEvent>,
        wake: crate::loader::Waker,
    ) -> Arc<FramesService> {
        Arc::new(FramesService {
            registry,
            grants,
            events: Mutex::new(events),
            wake,
        })
    }

    fn tell(&self, event: FrameEvent) {
        let _ = self
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(event);
        (self.wake)();
    }

    /// Whether frame `key` is one renderer `renderer`'s engines have.
    fn has(&self, renderer: u32, key: u32) -> bool {
        lock(&self.registry)
            .frames
            .get(&key)
            .is_some_and(|info| info.renderer == renderer)
    }
}

impl super::Serve for FramesService {
    fn serve(
        &self,
        renderer: u32,
        message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        match message {
            ToBrowser::NewFrame { call, parent } => {
                let (_, grant) = self.grants.frame(renderer, parent)?;
                let key = {
                    let mut registry = lock(&self.registry);
                    let key = registry.number();
                    registry.frames.insert(
                        key,
                        FrameInfo {
                            renderer,
                            parent: Some(parent),
                            origin: None,
                            url: None,
                            sandbox: 0,
                            root: None,
                        },
                    );
                    key
                };
                // A frame of its parent's renderer is of its parent's site:
                // its place in the tab is its parent's.
                self.grants.grant_frame(renderer, key, grant);
                let _ = reply.send(&ToRenderer::Answer {
                    call,
                    answer: Answer::Frame { key: Some(key) },
                });
                Ok(None)
            }
            ToBrowser::PostFrame { from, to, task } => {
                if let Some(from) = from {
                    self.grants.frame(renderer, from)?;
                }
                let lock_held = self.grants.lock_of(renderer);
                let admitted =
                    |origin: &Origin| lock_held.as_ref().is_some_and(|held| held.admits(origin));
                match &task {
                    WireTask::Message { origin, .. } => {
                        let own = origin == "null"
                            || Url::parse(origin).is_ok_and(|url| {
                                admitted(&url.origin()) && url.origin().serialization() == *origin
                            });
                        if !own {
                            return Err(format!("a message from {origin}"));
                        }
                    }
                    WireTask::Navigate { origin, .. } => {
                        if !admitted(origin) {
                            return Err(format!("a navigation by {}", origin.serialization()));
                        }
                    }
                }
                let registry = lock(&self.registry);
                let Some(host) = registry.host(to) else {
                    return Ok(None);
                };
                if let Some(sender) = registry.senders.get(&host) {
                    let _ = sender.send(&ToRenderer::FrameTask { from, to, task });
                }
                Ok(None)
            }
            ToBrowser::FrameDropped { key } => {
                if !self.has(renderer, key) {
                    return Err(format!("frame {key}, not of the renderer's pages"));
                }
                self.tell(FrameEvent::Dropped(key));
                Ok(None)
            }
            ToBrowser::FramePolicy { key, sandbox } => {
                if !self.has(renderer, key) {
                    return Err(format!("frame {key}, not of the renderer's pages"));
                }
                if let Some(info) = lock(&self.registry).frames.get_mut(&key) {
                    info.sandbox = sandbox;
                }
                Ok(None)
            }
            other => Ok(Some(other)),
        }
    }
}

/// Whether renderer `renderer` shows frame `info`'s documents: its
/// root's, or else its parent's.
pub(crate) fn lock_matches(info: &FrameInfo, renderer: u32) -> bool {
    info.root.map_or(info.renderer, |(root, _)| root) == renderer
}

/// Whether a document of `lock`'s site goes to another renderer than
/// `held`'s: one of a site, not its renderer's.
pub(crate) fn elsewhere(lock: &Lock, held: &Lock) -> bool {
    *lock != Lock::Opaque && lock != held
}
