//! An EventSource's connection (HTML section 9.2, G74): one fetch of the
//! stream on a thread of its own, which checks its response, parses its
//! body as it comes and says what it said, and ends saying whether the
//! connection is to be made again, with the last event ID. The wait
//! before it is made again is the page's (a task of the page's clock, as
//! the reference's reconnection is a timer of its event loop), and so is
//! the decision: `scripts::event_source` opens the next.
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

use net::event_stream::{Parser, Said};
use net::{AbortHandle, NetworkError, Request};

use super::{Delivery, Fetcher};

/// The reconnection time a stream starts with, in milliseconds.
pub(crate) const RECONNECT_MS: u64 = 3000;
/// What the console says of an event past `event_stream::MAX_EVENT`.
pub(crate) const TOO_LARGE: &str =
    "EventSource's event is larger than 64 MiB. Aborting the connection.";

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
    /// The stream set the reconnection time, in milliseconds.
    Retry(u64),
    /// The connection ended, or failed in a way that is not for good:
    /// it is to be made again, with this last event ID.
    Reconnecting { last_event_id: String },
    /// The connection failed for good: what the console says, if
    /// anything.
    Failed(Option<String>),
}

/// The page's hold on a source's connections: closing it ends the one
/// under way, and any the page would still open.
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

/// Opens one connection of an EventSource with `client` for `request`
/// (its `Last-Event-ID` set by the caller), on a thread of its own:
/// `push` is given what it does, in order, from that thread, its last
/// word `Reconnecting` or `Failed`, or nothing more once `control` closed
/// it.
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

/// How the fetch of the stream ended.
enum Ended {
    /// To be made again, with this last event ID.
    Again(String),
    /// For good, with what the console says.
    Failed(Option<String>),
}

fn run(
    client: &dyn Fetcher,
    mut request: Request,
    control: &SourceControl,
    push: &dyn Fn(SourceUpdate),
) {
    let Some(abort) = control.next_attempt() else {
        return;
    };
    request.abort = abort.clone();
    let ended = fetch_once(client, &request, &abort, push);
    if control.closed() {
        return;
    }
    push(match ended {
        Ended::Failed(said) => SourceUpdate::Failed(said),
        Ended::Again(last_event_id) => SourceUpdate::Reconnecting { last_event_id },
    });
}

/// The `Last-Event-ID` a request carries, the ID a reconnection starts
/// with.
fn sent_last_event_id(request: &Request) -> String {
    request
        .headers
        .get(b"Last-Event-ID")
        .map(|value| String::from_utf8_lossy(&value).into_owned())
        .unwrap_or_default()
}

/// One fetch of the stream, its events pushed as they come.
fn fetch_once(
    client: &dyn Fetcher,
    request: &Request,
    abort: &AbortHandle,
    push: &dyn Fn(SourceUpdate),
) -> Ended {
    let mut parser = Parser::new(&sent_last_event_id(request));
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
                push(match s {
                    Said::Retry(ms) => SourceUpdate::Retry(ms),
                    Said::Event {
                        kind,
                        data,
                        last_event_id,
                    } => SourceUpdate::Event {
                        kind,
                        data,
                        last_event_id,
                    },
                });
            }
        }
    });
    let again = || Ended::Again(parser.last_event_id().to_owned());
    match (verdict, result) {
        (Some(Err(why)), _) => Ended::Failed(why),
        // A response whose head came with the fetch's end alone.
        (None, Ok(response)) => match check(&response) {
            Err(why) => Ended::Failed(why),
            Ok(_) => again(),
        },
        // No good head: a check that refused the fetch fails it for
        // good, the reference's "access check"; any other error makes
        // it again.
        (None, Err(error)) if refused(&error) => Ended::Failed(None),
        _ => again(),
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
