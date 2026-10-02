//! The engine's loader (M14.6, docs/design/net.md, "The engine's
//! loader"): documents and their stylesheets fetched on threads of their
//! own, their results taken on the engine's thread.

mod pool;
mod rules;
mod sheets;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use dom::{Document, NodeId};
use encoding::Encoding;
use net::{AbortHandle, Client, NetworkError, Request, Response};

use pool::{Lost, Pool};

pub(crate) use pool::Waker;
pub(crate) use sheets::{Page, may_touch_sheets};

/// The most fetches in flight at once; further ones wait in the order
/// they were made (docs/design/net.md, "Threads, not an asynchronous
/// runtime").
pub const MAX_FETCH_THREADS: usize = 32;

/// A page's number: what its fetches' results are matched to.
pub(crate) type PageId = u64;

/// A fetch's number.
pub(crate) type FetchId = u64;

/// Where the latest load stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    /// A document is on its way, or the document shown still has
    /// stylesheets on their way.
    Loading,
    /// The document shown is all there: each of its stylesheets arrived,
    /// or failed and is left out.
    Loaded,
    /// The latest load failed, and the document shown is the one before.
    Failed(LoadError),
}

/// Why a load failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The fetch gave no response.
    Network(NetworkError),
    /// The response is not a document the engine shows: its computed
    /// MIME type is not HTML. What to show for text, images and
    /// downloads is navigation's to decide (M15.1).
    Unsupported(String),
    /// No thread could be started to fetch on, or the fetch's thread
    /// failed.
    Lost,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Network(error) => error.fmt(f),
            LoadError::Unsupported(mime_type) => write!(f, "{mime_type} is not shown"),
            LoadError::Lost => f.write_str("the fetch's thread failed"),
        }
    }
}

impl std::error::Error for LoadError {}

/// What a fetch is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A page's document.
    Document,
    /// A sheet of `owner`, the element: its own at an empty `path`, and
    /// else the import at each index in turn.
    Sheet { owner: NodeId, path: Vec<usize> },
}

/// A fetch in flight.
struct Flight {
    page: PageId,
    target: Target,
    abort: AbortHandle,
}

/// A fetch's result, with what it was for.
pub(crate) struct Arrived {
    pub(crate) id: FetchId,
    pub(crate) page: PageId,
    pub(crate) target: Target,
    pub(crate) result: Result<Response, LoadError>,
}

/// The engine's fetches: the client they share, the threads they run
/// on, and those in flight, each of which can be aborted.
pub(crate) struct Network {
    pool: Pool<Result<Response, NetworkError>>,
    /// Made at the first fetch: a document given to the engine needs
    /// none.
    client: Option<Arc<Client>>,
    flights: HashMap<FetchId, Flight>,
    next: FetchId,
}

impl Network {
    pub(crate) fn new() -> Network {
        Network {
            pool: Pool::new(MAX_FETCH_THREADS),
            client: None,
            flights: HashMap::new(),
            next: 0,
        }
    }

    /// Fetches from now on with `client`; fetches in flight finish with
    /// the client they began with.
    pub(crate) fn set_client(&mut self, client: Client) {
        self.client = Some(Arc::new(client));
    }

    pub(crate) fn set_waker(&self, waker: Waker) {
        self.pool.set_waker(waker);
    }

    /// Starts fetching `request` for `target` of `page`.
    pub(crate) fn fetch(&mut self, page: PageId, target: Target, request: Request) -> FetchId {
        let client = self
            .client
            .get_or_insert_with(|| Arc::new(Client::default()))
            .clone();
        let id = self.next;
        self.next += 1;
        self.flights.insert(
            id,
            Flight {
                page,
                target,
                abort: request.abort.clone(),
            },
        );
        self.pool.run(id, move || client.fetch(&request));
        id
    }

    /// Aborts and forgets the fetches of `page`: all of them, or those
    /// for `owner`'s sheets. Their results are not taken.
    pub(crate) fn cancel(&mut self, page: PageId, owner: Option<NodeId>) {
        self.flights.retain(|_, flight| {
            let cancelled = flight.page == page
                && owner.is_none_or(
                    |owner| matches!(&flight.target, Target::Sheet { owner: o, .. } if *o == owner),
                );
            if cancelled {
                flight.abort.abort();
            }
            !cancelled
        });
    }

    /// Aborts and forgets every fetch.
    pub(crate) fn cancel_all(&mut self) {
        for flight in self.flights.values() {
            flight.abort.abort();
        }
        self.flights.clear();
    }

    /// A finished fetch still wanted, if one has finished.
    pub(crate) fn take(&mut self) -> Option<Arrived> {
        while let Some((id, result)) = self.pool.try_take() {
            if let Some(arrived) = self.arrived(id, result) {
                return Some(arrived);
            }
        }
        None
    }

    fn arrived(
        &mut self,
        id: FetchId,
        result: Result<Result<Response, NetworkError>, Lost>,
    ) -> Option<Arrived> {
        let flight = self.flights.remove(&id)?;
        Some(Arrived {
            id,
            page: flight.page,
            target: flight.target,
            result: match result {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(error)) => Err(LoadError::Network(error)),
                Err(Lost) => Err(LoadError::Lost),
            },
        })
    }

    /// How many fetches are in flight.
    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.flights.len()
    }

    #[cfg(test)]
    pub(crate) fn pool(&self) -> &Pool<Result<Response, NetworkError>> {
        &self.pool
    }
}

/// A fetched document, parsed.
pub(crate) struct Parsed {
    pub(crate) document: Document,
    /// The encoding its bytes were decoded with.
    pub(crate) encoding: Encoding,
    /// The encoding it turned out to be in, when the engine cannot
    /// decode that one yet (a legacy CJK encoding): its bytes were
    /// decoded as UTF-8 instead.
    pub(crate) undecodable: Option<Encoding>,
}

/// The document a response holds. Its computed MIME type (MIME Sniffing
/// §7) must be HTML; a status that is not ok is still a document, shown
/// as the server sent it.
pub(crate) fn document_from(response: &Response) -> Result<Parsed, LoadError> {
    let computed = net::mime::sniff::computed_type(
        response.mime_type.as_ref(),
        net::mime::sniff::check_for_apache_bug(&response.headers),
        net::mime::determine_nosniff(&response.headers),
        &response.body,
    );
    if !computed.is_html() {
        return Err(LoadError::Unsupported(
            String::from_utf8_lossy(&computed.to_bytes()).into_owned(),
        ));
    }
    let transport = response
        .mime_type
        .as_ref()
        .and_then(|mime_type| mime_type.parameter("charset"))
        .and_then(Encoding::for_label);
    let decoded = html::parse_document_from_bytes(&response.body, transport);
    Ok(Parsed {
        document: Document::from_node(&decoded.document),
        encoding: decoded.encoding,
        undecodable: decoded.undecodable,
    })
}

/// A load whose document is not shown yet.
pub(crate) enum Incoming {
    /// Its document is being fetched.
    Fetching(PageId),
    /// Its document has arrived, and waits for its blocking sheets.
    Waiting {
        page: Box<Page>,
        document: Box<Document>,
    },
}

impl Incoming {
    pub(crate) fn id(&self) -> PageId {
        match self {
            Incoming::Fetching(id) => *id,
            Incoming::Waiting { page, .. } => page.id(),
        }
    }
}
