//! A renderer's link to the browser (M34.2, docs/design/processes.md):
//! its sender, the questions its threads wait on, and its fetches in
//! flight, to which the reader thread hands what the browser answers.
//! Through it, the engines of a renderer fetch ([`RemoteFetcher`]) and
//! reach their cookies ([`RemoteCookies`]) in the browser: nothing of
//! the network, the jar or the cache is in the renderer.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

use net::cookies::Context;
use net::csp::Violation;
use net::{Delivery, NetworkError, Request, Response, Url};

use crate::ipc::{Answer, Bytes, Calls, Closed, Sender, StorageOp, ToBrowser, ToRenderer};
use crate::loader::{Cookies, Fetcher};
use crate::storage::{AreaKey, Areas, OverQuota};

/// How often a fetch waiting for the browser looks whether it was
/// aborted.
const ABORT_CHECK: Duration = Duration::from_millis(20);

/// What the browser said of a fetch.
enum Part {
    Head(Response),
    Body(Vec<u8>),
    Done(Result<Response, NetworkError>, Vec<Violation>),
}

/// The renderer's link to the browser.
pub(crate) struct Link {
    sender: Sender<ToBrowser>,
    calls: Calls<Answer>,
    fetches: Mutex<Option<HashMap<u64, mpsc::Sender<Part>>>>,
    next_fetch: AtomicU64,
}

impl Link {
    pub(crate) fn new(sender: Sender<ToBrowser>) -> Arc<Link> {
        Arc::new(Link {
            sender,
            calls: Calls::new(),
            fetches: Mutex::new(Some(HashMap::new())),
            next_fetch: AtomicU64::new(1),
        })
    }

    fn fetches(&self) -> MutexGuard<'_, Option<HashMap<u64, mpsc::Sender<Part>>>> {
        self.fetches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends `message` to the browser.
    pub(crate) fn send(&self, message: &ToBrowser) -> Result<(), Closed> {
        self.sender.send(message)
    }

    /// Asks the browser the message `ask` makes of the question's number,
    /// and waits for its answer.
    pub(crate) fn call(&self, ask: impl FnOnce(u64) -> ToBrowser) -> Result<Answer, Closed> {
        self.calls.call(&self.sender, ask)
    }

    /// Tells the browser of `change`, which frame `frame`'s document made
    /// (M37), for the other renderers.
    pub(crate) fn storage_changed(&self, frame: u32, change: &crate::storage::Change) {
        let _ = self.send(&ToBrowser::StorageChanged {
            frame,
            local: change.scope.is_local(),
            origin: change.owner.clone(),
            partition: change.partition.clone(),
            key: change.key.clone(),
            old: change.old.clone(),
            new: change.new.clone(),
            url: change.url.clone(),
        });
    }

    /// The browser's number for a new frame under its frame `parent`
    /// (M34); none when it refuses one.
    pub(crate) fn new_frame(&self, parent: u32) -> Option<u32> {
        match self.call(|call| ToBrowser::NewFrame { call, parent }) {
            Ok(Answer::Frame { key }) => key,
            _ => None,
        }
    }

    /// Takes `message` if it is for one who waits on the link (an answer,
    /// a piece of a fetch); otherwise gives it back, for the renderer's
    /// main thread. Err when it answers nothing that was asked: the
    /// browser does not do that.
    pub(crate) fn route(&self, message: ToRenderer) -> Result<Option<ToRenderer>, String> {
        let (id, part) = match message {
            ToRenderer::Answer { call, answer } => {
                if !self.calls.answer(call, answer) {
                    return Err(format!("an answer to no question ({call})"));
                }
                return Ok(None);
            }
            ToRenderer::FetchHead { id, response } => (id, Part::Head(response)),
            ToRenderer::FetchBody { id, bytes } => (id, Part::Body(bytes.0)),
            ToRenderer::FetchDone {
                id,
                result,
                violations,
            } => (id, Part::Done(result, violations)),
            other => return Ok(Some(other)),
        };
        let done = matches!(part, Part::Done(..));
        let mut fetches = self.fetches();
        let Some(fetches) = fetches.as_mut() else {
            return Ok(None);
        };
        // A fetch given up on may still hear from the browser.
        if let Some(waiting) = fetches.get(&id) {
            let _ = waiting.send(part);
        }
        if done {
            fetches.remove(&id);
        }
        Ok(None)
    }

    /// The browser is gone: everyone waiting stops.
    pub(crate) fn close(&self) {
        self.calls.close();
        *self.fetches() = None;
        self.sender.close();
    }

    /// Fetches `request` for frame `frame` through the browser, handing
    /// a streamed fetch's head and body to `deliver` as they come; the
    /// response the browser holds as `handover`, if one.
    fn fetch(
        &self,
        frame: u32,
        request: &Request,
        streamed: bool,
        handover: Option<u64>,
        deliver: &mut dyn FnMut(Delivery<'_>),
    ) -> Result<Response, NetworkError> {
        let gone = || NetworkError::Io("the browser is gone".to_owned());
        let id = self.next_fetch.fetch_add(1, Ordering::Relaxed);
        let (parts, waiting) = mpsc::channel();
        self.fetches().as_mut().ok_or_else(gone)?.insert(id, parts);
        let mut asked = request.clone();
        // The policies' violations come back from the browser; its copy
        // keeps none.
        asked.csp = request.csp.as_ref().map(net::csp::Csp::inherited);
        let ask = match handover {
            Some(handover) => ToBrowser::FetchHandedOver {
                id,
                frame,
                handover,
                request: Box::new(asked),
            },
            None => ToBrowser::Fetch {
                id,
                frame,
                streamed,
                request: Box::new(asked),
            },
        };
        if self.send(&ask).is_err() {
            return Err(gone());
        }
        let mut body = Vec::new();
        loop {
            match waiting.recv_timeout(ABORT_CHECK) {
                Ok(Part::Head(head)) => {
                    if streamed {
                        deliver(Delivery::Head(&head));
                    }
                }
                Ok(Part::Body(bytes)) => match streamed {
                    true => deliver(Delivery::Body(&bytes)),
                    false => body.extend_from_slice(&bytes),
                },
                Ok(Part::Done(result, violations)) => {
                    if let Some(csp) = &request.csp {
                        csp.record(&violations);
                    }
                    return result.map(|mut response| {
                        if !streamed {
                            response.body = body;
                        }
                        response
                    });
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if request.abort.is_aborted() {
                        if let Some(fetches) = self.fetches().as_mut() {
                            fetches.remove(&id);
                        }
                        let _ = self.send(&ToBrowser::CancelFetch { id });
                        return Err(NetworkError::Aborted);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(gone()),
            }
        }
    }
}

/// The fetches of a renderer's frame, made by the browser.
pub(crate) struct RemoteFetcher {
    link: Arc<Link>,
    frame: u32,
    /// The response the browser holds for the next fetch of a document of
    /// this URL (M34.1).
    handover: Mutex<Option<(Url, u64)>>,
}

impl RemoteFetcher {
    pub(crate) fn new(link: Arc<Link>, frame: u32) -> RemoteFetcher {
        RemoteFetcher {
            link,
            frame,
            handover: Mutex::new(None),
        }
    }

    /// The next fetch of a document of `url` takes response `handover`.
    pub(crate) fn hand_over(&self, url: Url, handover: u64) {
        *self.handover.lock().unwrap_or_else(PoisonError::into_inner) = Some((url, handover));
    }

    /// The response held for `request`, if it is the document's fetch the
    /// browser handed one over for.
    fn handed_over(&self, request: &Request) -> Option<u64> {
        let mut held = self.handover.lock().unwrap_or_else(PoisonError::into_inner);
        let matches = held.as_ref().is_some_and(|(url, _)| {
            matches!(
                request.destination,
                net::Destination::Document | net::Destination::Iframe
            ) && url.as_str() == request.url.as_str()
        });
        if !matches {
            return None;
        }
        held.take().map(|(_, handover)| handover)
    }
}

impl Fetcher for RemoteFetcher {
    fn fetch(&self, request: &Request) -> Result<Response, NetworkError> {
        self.link
            .fetch(self.frame, request, false, None, &mut |_| {})
    }

    fn fetch_streaming(
        &self,
        request: &Request,
        deliver: &mut dyn FnMut(Delivery<'_>),
    ) -> Result<Response, NetworkError> {
        let handover = self.handed_over(request);
        self.link
            .fetch(self.frame, request, true, handover, deliver)
    }
}

/// A renderer's frame's `document.cookie`, kept by the browser.
pub(crate) struct RemoteCookies {
    link: Arc<Link>,
    frame: u32,
}

impl RemoteCookies {
    pub(crate) fn new(link: Arc<Link>, frame: u32) -> RemoteCookies {
        RemoteCookies { link, frame }
    }
}

impl Cookies for RemoteCookies {
    fn script_cookies(&self, url: &Url, context: Context) -> Option<Vec<u8>> {
        let answer = self.link.call(|call| ToBrowser::Cookies {
            call,
            frame: self.frame,
            url: url.clone(),
            context,
        });
        match answer {
            Ok(Answer::Cookies { cookies }) => cookies.map(|cookies| cookies.0),
            _ => None,
        }
    }

    fn set_script_cookie(&self, value: &[u8], url: &Url, context: Context) {
        let _ = self.link.send(&ToBrowser::SetCookie {
            frame: self.frame,
            url: url.clone(),
            value: Bytes(value.to_vec()),
            context,
        });
    }
}

/// A renderer's frame's Web Storage areas, kept by the browser.
pub(crate) struct RemoteAreas {
    link: Arc<Link>,
    frame: u32,
}

impl RemoteAreas {
    pub(crate) fn new(link: Arc<Link>, frame: u32) -> RemoteAreas {
        RemoteAreas { link, frame }
    }

    fn ask(&self, local: bool, key: &AreaKey, op: StorageOp) -> Option<Answer> {
        self.link
            .call(|call| ToBrowser::Storage {
                call,
                frame: self.frame,
                local,
                origin: key.origin.clone(),
                partition: key.partition.clone(),
                op,
            })
            .ok()
    }

    fn value(&self, local: bool, key: &AreaKey, op: StorageOp) -> Option<String> {
        match self.ask(local, key, op) {
            Some(Answer::StorageValue { value }) => value,
            _ => None,
        }
    }
}

impl Areas for RemoteAreas {
    fn usage(&self, key: &AreaKey) -> u64 {
        match self.ask(true, key, StorageOp::Usage {}) {
            Some(Answer::StorageLength { length }) => length,
            _ => 0,
        }
    }

    fn idb(&self, key: &AreaKey, request: &str) -> String {
        let op = StorageOp::Idb {
            request: request.to_owned(),
        };
        match self.ask(true, key, op) {
            Some(Answer::Idb { answer }) => answer,
            _ => r#"{"error":{"name":"UnknownError","message":"The browser did not answer."}}"#
                .to_owned(),
        }
    }

    fn length(&self, local: bool, key: &AreaKey) -> usize {
        match self.ask(local, key, StorageOp::Length {}) {
            Some(Answer::StorageLength { length }) => usize::try_from(length).unwrap_or(0),
            _ => 0,
        }
    }

    fn key(&self, local: bool, key: &AreaKey, index: usize) -> Option<String> {
        self.value(
            local,
            key,
            StorageOp::Key {
                index: index as u64,
            },
        )
    }

    fn keys(&self, local: bool, key: &AreaKey) -> Vec<String> {
        match self.ask(local, key, StorageOp::Keys {}) {
            Some(Answer::StorageKeys { keys }) => keys,
            _ => Vec::new(),
        }
    }

    fn get(&self, local: bool, key: &AreaKey, item: &str) -> Option<String> {
        self.value(
            local,
            key,
            StorageOp::Get {
                item: item.to_owned(),
            },
        )
    }

    fn set(
        &self,
        local: bool,
        key: &AreaKey,
        item: &str,
        value: &str,
    ) -> Result<Option<Option<String>>, OverQuota> {
        let op = StorageOp::Set {
            item: item.to_owned(),
            value: value.to_owned(),
        };
        match self.ask(local, key, op) {
            Some(Answer::StorageSet {
                stored: true,
                old,
                changed,
            }) => Ok(changed.then_some(old)),
            _ => Err(OverQuota),
        }
    }

    fn remove(&self, local: bool, key: &AreaKey, item: &str) -> Option<String> {
        self.value(
            local,
            key,
            StorageOp::Remove {
                item: item.to_owned(),
            },
        )
    }

    fn clear(&self, local: bool, key: &AreaKey) -> bool {
        matches!(
            self.ask(local, key, StorageOp::Clear {}),
            Some(Answer::StorageCleared { cleared: true })
        )
    }
}
