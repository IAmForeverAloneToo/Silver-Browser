//! An EventSource's connection (HTML section 9.2, G74): a thread of its
//! own that fetches the stream, checks its response, parses its body as
//! it comes and says what it said, and makes the connection again when
//! it ends, after the reconnection time, with the last event ID; until
//! the page closes it or the connection fails for good.
//!
//! As the reference browser does (Chrome 157, probed 2026-11-22): a
//! response not 200, or of a type not `text/event-stream`, or of a
//! charset not UTF-8, fails the connection, the last two said in the
//! console; so does a fetch a check refused (CORS, the policies, mixed
//! content, the address spaces); any other network error, a connection
//! refused or a stream's end among them, makes the connection again,
//! after 3 seconds unless the stream's `retry` said otherwise.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use net::event_stream::{Parser, Said};
use net::{AbortHandle, NetworkError, Request};

use super::{Delivery, Fetcher};

/// The reconnection time a stream starts with.
pub(crate) const RECONNECT: Duration = Duration::from_secs(3);
/// What the console says of an event past `event_stream::MAX_EVENT`.
pub(crate) const TOO_LARGE: &str =
    "EventSource's event is larger than 64 MiB. Aborting the connection.";
/// How often a wait for the reconnection looks whether the page closed
/// the source.
const CLOSE_CHECK: Duration = Duration::from_millis(50);

/// What an EventSource's connection did, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SourceUpdate {
    /// A good response came: the origin of its final URL, serialized,
    /// which the messages' events carry.
    Opened { origin: String },
    /// An event the stream dispatched.
    Event {
        kind: String,
        data: String,
        last_event_id: String,
    },
    /// The connection ended, or failed in a way that is not for good:
    /// it is to be made again.
    Reconnecting,
    /// The connection failed for good: what the console says, if
    /// anything.
    Failed(Option<String>),
}

/// The page's hold on a connection: closing it ends the connection, and
/// stops one to come.
#[derive(Debug, Default)]
pub(crate) struct SourceControl {
    closed: AtomicBool,
    /// The fetch under way, if any.
    attempt: Mutex<AbortHandle>,
}

impl SourceControl {
    /// Ends the connection, at once if a fetch is under way, else before
    /// the next one starts.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .abort();
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// A new fetch's abort handle, made the one a close aborts; none
    /// when the page closed the source already.
    fn next_attempt(&self) -> Option<AbortHandle> {
        let handle = AbortHandle::new();
        *self.attempt.lock().unwrap_or_else(PoisonError::into_inner) = handle.clone();
        // Closed while it was made: the close saw the old handle.
        (!self.closed()).then_some(handle)
    }
}

/// Opens an EventSource's connection with `client` for `request`, on a
/// thread of its own: `push` is given what it does, in order, from that
/// thread. `control` ends it.
pub(crate) fn open_source_with(
    client: Arc<dyn Fetcher>,
    request: Request,
    control: Arc<SourceControl>,
    push: impl Fn(SourceUpdate) + Clone + Send + 'static,
) {
    let failed = push.clone();
    let spawned = std::thread::Builder::new()
        .name("eventsource".to_owned())
        .spawn(move || run(client.as_ref(), request, &control, &push));
    if spawned.is_err() {
        failed(SourceUpdate::Failed(None));
    }
}

/// How one fetch of the stream ended.
enum Ended {
    /// To be made again.
    Again,
    /// For good, with what the console says.
    Failed(Option<String>),
    /// The page closed the source.
    Closed,
}

fn run(
    client: &dyn Fetcher,
    request: Request,
    control: &SourceControl,
    push: &dyn Fn(SourceUpdate),
) {
    let mut last_event_id = String::new();
    let mut wait = RECONNECT;
    loop {
        let Some(abort) = control.next_attempt() else {
            return;
        };
        let mut asked = request.clone();
        asked.abort = abort.clone();
        if !last_event_id.is_empty() {
            asked
                .headers
                .set(b"Last-Event-ID", last_event_id.as_bytes());
        }
        let ended = fetch_once(client, &asked, &abort, &mut last_event_id, &mut wait, push);
        let ended = if control.closed() {
            Ended::Closed
        } else {
            ended
        };
        match ended {
            Ended::Closed => return,
            Ended::Failed(said) => {
                push(SourceUpdate::Failed(said));
                return;
            }
            Ended::Again => push(SourceUpdate::Reconnecting),
        }
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if control.closed() {
                return;
            }
            std::thread::sleep(CLOSE_CHECK.min(until - Instant::now()));
        }
    }
}

/// One fetch of the stream, its events pushed as they come.
fn fetch_once(
    client: &dyn Fetcher,
    request: &Request,
    abort: &AbortHandle,
    last_event_id: &mut String,
    wait: &mut Duration,
    push: &dyn Fn(SourceUpdate),
) -> Ended {
    let mut parser = Parser::new(last_event_id);
    // The head's verdict: whether it was good, or why not.
    let mut verdict: Option<Result<(), Option<String>>> = None;
    let mut said = Vec::new();
    let result = client.fetch_streaming(request, &mut |delivery| match delivery {
        Delivery::Head(head) => match check(head) {
            Ok(origin) => {
                verdict = Some(Ok(()));
                push(SourceUpdate::Opened { origin });
            }
            Err(why) => {
                verdict = Some(Err(why));
                abort.abort();
            }
        },
        Delivery::Body(bytes) => {
            if verdict != Some(Ok(())) {
                return;
            }
            if parser.feed(bytes, &mut said).is_err() {
                // An event past the bound fails the connection for good:
                // made again, it would come again (the reference holds
                // what its memory allows; docs/design/fetch.md).
                verdict = Some(Err(Some(TOO_LARGE.to_owned())));
                abort.abort();
                return;
            }
            for s in said.drain(..) {
                match s {
                    Said::Retry(ms) => *wait = Duration::from_millis(ms),
                    Said::Event {
                        kind,
                        data,
                        last_event_id,
                    } => push(SourceUpdate::Event {
                        kind,
                        data,
                        last_event_id,
                    }),
                }
            }
        }
    });
    parser.last_event_id().clone_into(last_event_id);
    match (verdict, result) {
        (Some(Err(why)), _) => Ended::Failed(why),
        // A response whose head came with the fetch's end alone.
        (None, Ok(response)) => match check(&response) {
            Err(why) => Ended::Failed(why),
            Ok(_) => Ended::Again,
        },
        // No good head: a check that refused the fetch fails it for
        // good, the reference's "access check"; any other error makes
        // it again.
        (None, Err(error)) if refused(&error) => Ended::Failed(None),
        _ => Ended::Again,
    }
}

/// The origin of a good response's final URL, or why it is not good,
/// for the console.
fn check(head: &net::Response) -> Result<String, Option<String>> {
    if head.status != 200 {
        return Err(None);
    }
    let mime = net::mime::extract_mime_type(&head.headers);
    let essence = mime.as_ref().map_or("", |m| m.essence());
    if essence != "text/event-stream" {
        return Err(Some(format!(
            "EventSource's response has a MIME type (\"{essence}\") that is not \"text/event-stream\". Aborting the connection."
        )));
    }
    if let Some(charset) = mime.as_ref().and_then(|m| m.parameter("charset"))
        && !charset.eq_ignore_ascii_case("utf-8")
    {
        return Err(Some(format!(
            "EventSource's response has a charset (\"{charset}\") that is not UTF-8. Aborting the connection."
        )));
    }
    let url = head.url_list.last();
    Ok(url.map_or_else(|| "null".to_owned(), |url| url.origin().serialization()))
}

/// Whether `error` is a check's refusal, which fails a connection for
/// good, rather than the network's failure, which makes it again.
fn refused(error: &NetworkError) -> bool {
    matches!(
        error,
        NetworkError::BadScheme(_)
            | NetworkError::BadPort(_)
            | NetworkError::AddressSpace { .. }
            | NetworkError::TargetAddressSpace { .. }
            | NetworkError::MixedContent
            | NetworkError::Integrity
            | NetworkError::RedirectLimit
            | NetworkError::BadRedirect
            | NetworkError::Cors(_)
            | NetworkError::CrossOriginResourcePolicy
            | NetworkError::Redirected
            | NetworkError::Refused(_)
            | NetworkError::Csp(_)
            | NetworkError::DataUrl
            | NetworkError::BlobUrl
            | NetworkError::File(_)
            | NetworkError::NotCached
            | NetworkError::Moved
    )
}
