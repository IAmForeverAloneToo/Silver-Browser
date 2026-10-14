//! Navigations handed over (M34.1, D28, docs/design/processes.md): a
//! top-level document's response whose site is not its renderer's goes
//! to the renderer of its site, which the browser picks while the
//! response waits here, its body read on into a bounded buffer. The
//! renderer picked asks for it by its number, as a fetch of its own; no
//! request is made twice.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use net::csp::Violation;
use net::{NetworkError, Response, Url};

use super::network::Lock;
use crate::ipc::{Bytes, MAX_CHUNK, Sender, ToRenderer};

/// What the network service tells the browser's tab of a response it
/// holds: the frame of the renderer that asked, and where the document
/// goes.
#[derive(Debug, Clone)]
pub struct HandoverNotice {
    pub handover: u64,
    pub renderer: u32,
    pub frame: u32,
    pub url: Url,
    pub lock: Lock,
}

/// How much of a body waits for the renderer it goes to; past it the
/// network waits.
pub const MAX_HANDOVER_BUFFER: usize = 16 << 20;

/// How long a response waits for its renderer before it is given up.
pub const HANDOVER_TIMEOUT: Duration = Duration::from_secs(30);

/// Where a waiting response goes: renderer and frame, and the renderer's
/// number for the fetch that takes it, with where to send it.
struct Taker {
    renderer: u32,
    frame: u32,
    fetch: Option<(u64, Sender<ToRenderer>)>,
}

struct Waiting {
    url: Url,
    head: Response,
    body: VecDeque<Vec<u8>>,
    buffered: usize,
    end: Option<(Result<Response, NetworkError>, Vec<Violation>)>,
    taker: Option<Taker>,
    made: Instant,
}

#[derive(Default)]
struct Table {
    waiting: HashMap<u64, Waiting>,
}

/// The responses handed over, waiting for their renderers.
#[derive(Default)]
pub struct Handovers {
    table: Mutex<Table>,
    changed: Condvar,
    next: AtomicU64,
}

impl Handovers {
    pub fn new() -> Arc<Handovers> {
        Arc::new(Handovers {
            next: AtomicU64::new(1),
            ..Handovers::default()
        })
    }

    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `head`, the response to a navigation of `url`, for the
    /// renderer the browser will pick; its number.
    pub(crate) fn hold(&self, url: Url, head: Response) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.table().waiting.insert(
            id,
            Waiting {
                url,
                head,
                body: VecDeque::new(),
                buffered: 0,
                end: None,
                taker: None,
                made: Instant::now(),
            },
        );
        id
    }

    /// The browser picked frame `frame` of renderer `renderer` for
    /// response `id`; false if it waits no more.
    pub fn assign(&self, id: u64, renderer: u32, frame: u32) -> bool {
        let mut table = self.table();
        let Some(waiting) = table.waiting.get_mut(&id) else {
            return false;
        };
        waiting.taker = Some(Taker {
            renderer,
            frame,
            fetch: None,
        });
        true
    }

    /// Renderer `renderer` takes response `id` for its frame `frame` as
    /// its fetch `fetch` of `url`: what came of it so far goes now, and
    /// the rest as it comes. Err when it was not handed to that frame for
    /// that URL.
    pub(crate) fn take(
        &self,
        id: u64,
        (renderer, frame): (u32, u32),
        fetch: u64,
        url: &Url,
        reply: &Sender<ToRenderer>,
    ) -> Result<(), String> {
        let mut table = self.table();
        let waiting = table
            .waiting
            .get_mut(&id)
            .ok_or_else(|| format!("handover {id}, which waits for no one"))?;
        let assigned = waiting
            .taker
            .as_ref()
            .is_some_and(|taker| taker.renderer == renderer && taker.frame == frame);
        if !assigned
            || waiting
                .taker
                .as_ref()
                .is_some_and(|taker| taker.fetch.is_some())
        {
            return Err(format!("handover {id}, not for frame {frame}"));
        }
        if url.as_str() != waiting.url.as_str() {
            return Err(format!("handover {id} for {url}"));
        }
        let _ = reply.send(&ToRenderer::FetchHead {
            id: fetch,
            response: waiting.head.clone(),
        });
        while let Some(bytes) = waiting.body.pop_front() {
            send_body(reply, fetch, &bytes);
        }
        waiting.buffered = 0;
        if let Some((result, violations)) = waiting.end.take() {
            let _ = reply.send(&ToRenderer::FetchDone {
                id: fetch,
                result,
                violations,
            });
            table.waiting.remove(&id);
        } else if let Some(taker) = &mut waiting.taker {
            taker.fetch = Some((fetch, reply.clone()));
        }
        self.changed.notify_all();
        Ok(())
    }

    /// A piece of response `id`'s body: sent on if its renderer took it,
    /// else kept, waiting while the buffer is full. False when it is given
    /// up, for the fetch to stop.
    pub(crate) fn body(&self, id: u64, bytes: &[u8]) -> bool {
        let mut table = self.table();
        loop {
            let Some(waiting) = table.waiting.get_mut(&id) else {
                return false;
            };
            if let Some((fetch, reply)) = waiting
                .taker
                .as_ref()
                .and_then(|taker| taker.fetch.as_ref())
            {
                send_body(reply, *fetch, bytes);
                return true;
            }
            if waiting.made.elapsed() > HANDOVER_TIMEOUT {
                table.waiting.remove(&id);
                return false;
            }
            if waiting.buffered + bytes.len() <= MAX_HANDOVER_BUFFER {
                waiting.buffered += bytes.len();
                waiting.body.push_back(bytes.to_vec());
                return true;
            }
            let (next, _) = self
                .changed
                .wait_timeout(table, Duration::from_millis(100))
                .unwrap_or_else(PoisonError::into_inner);
            table = next;
        }
    }

    /// Response `id`'s end.
    pub(crate) fn end(
        &self,
        id: u64,
        result: Result<Response, NetworkError>,
        violations: Vec<Violation>,
    ) {
        let mut table = self.table();
        let Some(waiting) = table.waiting.get_mut(&id) else {
            return;
        };
        if let Some((fetch, reply)) = waiting
            .taker
            .as_ref()
            .and_then(|taker| taker.fetch.as_ref())
        {
            let _ = reply.send(&ToRenderer::FetchDone {
                id: *fetch,
                result,
                violations,
            });
            table.waiting.remove(&id);
        } else {
            waiting.end = Some((result, violations));
        }
    }

    /// Gives up the responses that waited past [`HANDOVER_TIMEOUT`].
    pub fn expire(&self) {
        self.table()
            .waiting
            .retain(|_, waiting| waiting.made.elapsed() <= HANDOVER_TIMEOUT);
        self.changed.notify_all();
    }

    /// How many responses wait.
    pub fn waiting(&self) -> usize {
        self.table().waiting.len()
    }

    /// How many bytes of response `id`'s body wait.
    #[cfg(test)]
    fn buffered(&self, id: u64) -> usize {
        self.table()
            .waiting
            .get(&id)
            .map_or(0, |waiting| waiting.buffered)
    }
}

fn send_body(reply: &Sender<ToRenderer>, id: u64, bytes: &[u8]) {
    for piece in bytes.chunks(MAX_CHUNK) {
        let _ = reply.send(&ToRenderer::FetchBody {
            id,
            bytes: Bytes(piece.to_vec()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc;

    fn head(url: &Url) -> Response {
        Response {
            url_list: vec![url.clone()],
            status: 200,
            status_message: b"OK".to_vec(),
            headers: net::HeaderList::new(),
            body: Vec::new(),
            mime_type: None,
            address_space: None,
            response_type: net::ResponseType::Basic,
            cors_exposed: Vec::new(),
        }
    }

    #[test]
    fn a_response_goes_to_the_frame_it_was_handed_to_alone() {
        let handovers = Handovers::new();
        let url = Url::parse("https://b.test/page").expect("a URL");
        let id = handovers.hold(url.clone(), head(&url));
        assert!(handovers.body(id, b"<p>early"));
        let (writer, mut reader) = ipc::pipe();
        let reply = Sender::<ToRenderer>::new(writer);
        assert_eq!(
            handovers.take(id, (2, 5), 1, &url, &reply),
            Err(format!("handover {id}, not for frame 5"))
        );
        assert!(handovers.assign(id, 2, 5));
        assert_eq!(
            handovers.take(id, (3, 5), 1, &url, &reply),
            Err(format!("handover {id}, not for frame 5")),
            "another renderer"
        );
        let other = Url::parse("https://b.test/other").expect("a URL");
        assert_eq!(
            handovers.take(id, (2, 5), 1, &other, &reply),
            Err(format!("handover {id} for {other}"))
        );
        assert_eq!(handovers.take(id, (2, 5), 7, &url, &reply), Ok(()));
        assert_eq!(
            handovers.take(id, (2, 5), 8, &url, &reply),
            Err(format!("handover {id}, not for frame 5")),
            "taken once"
        );
        assert!(handovers.body(id, b" late"));
        handovers.end(id, Ok(head(&url)), Vec::new());
        assert_eq!(handovers.waiting(), 0);
        drop(reply);
        let mut kinds = Vec::new();
        while let Ok(Some(message)) = ipc::read_message::<ToRenderer>(&mut reader) {
            kinds.push(match message {
                ToRenderer::FetchHead { id: 7, .. } => "head".to_owned(),
                ToRenderer::FetchBody { id: 7, bytes } => {
                    String::from_utf8_lossy(&bytes.0).into_owned()
                }
                ToRenderer::FetchDone { id: 7, result, .. } => format!("done {}", result.is_ok()),
                other => format!("{other:?}"),
            });
        }
        assert_eq!(kinds, ["head", "<p>early", " late", "done true"]);
    }

    #[test]
    fn a_body_waits_in_a_bounded_buffer_for_its_renderer() {
        let handovers = Handovers::new();
        let url = Url::parse("https://b.test/big").expect("a URL");
        let id = handovers.hold(url.clone(), head(&url));
        const PIECES: usize = 20;
        let feeding = {
            let handovers = handovers.clone();
            std::thread::spawn(move || {
                (0..PIECES).all(|_| handovers.body(id, &vec![b'x'; 1 << 20]))
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(handovers.buffered(id) <= MAX_HANDOVER_BUFFER);
        assert!(!feeding.is_finished(), "the network waits");
        assert!(handovers.assign(id, 1, 1));
        let (writer, mut reader) = ipc::pipe();
        let reply = Sender::<ToRenderer>::new(writer);
        let reading = std::thread::spawn(move || {
            let mut bytes = 0;
            while let Ok(Some(message)) = ipc::read_message::<ToRenderer>(&mut reader) {
                if let ToRenderer::FetchBody { bytes: piece, .. } = message {
                    bytes += piece.0.len();
                }
            }
            bytes
        });
        assert_eq!(handovers.take(id, (1, 1), 1, &url, &reply), Ok(()));
        assert!(feeding.join().expect("joined"), "every piece went on");
        handovers.end(id, Ok(head(&url)), Vec::new());
        drop(reply);
        assert_eq!(reading.join().expect("joined"), PIECES << 20);
    }

    #[test]
    fn a_response_no_one_takes_is_given_up() {
        let handovers = Handovers::new();
        let url = Url::parse("https://b.test/").expect("a URL");
        handovers.hold(url.clone(), head(&url));
        assert_eq!(handovers.waiting(), 1);
        handovers.expire();
        assert_eq!(handovers.waiting(), 1, "not yet");
    }
}
