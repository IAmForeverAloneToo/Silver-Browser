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

use crate::ipc::{Answer, Bytes, Calls, Closed, Sender, ToBrowser, ToRenderer};
use crate::loader::{Cookies, Fetcher};

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
    /// a streamed fetch's head and body to `deliver` as they come.
    fn fetch(
        &self,
        frame: u32,
        request: &Request,
        streamed: bool,
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
        if self
            .send(&ToBrowser::Fetch {
                id,
                frame,
                streamed,
                request: Box::new(asked),
            })
            .is_err()
        {
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
}

impl RemoteFetcher {
    pub(crate) fn new(link: Arc<Link>, frame: u32) -> RemoteFetcher {
        RemoteFetcher { link, frame }
    }
}

impl Fetcher for RemoteFetcher {
    fn fetch(&self, request: &Request) -> Result<Response, NetworkError> {
        self.link.fetch(self.frame, request, false, &mut |_| {})
    }

    fn fetch_streaming(
        &self,
        request: &Request,
        deliver: &mut dyn FnMut(Delivery<'_>),
    ) -> Result<Response, NetworkError> {
        self.link.fetch(self.frame, request, true, deliver)
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
