//! The EventSource interface's state (G74, HTML section 9.2), behind the
//! bindings' calls and tasks so that any engine's bindings, a shell of a
//! class, share it: the URL's check, the ready state, whether the page
//! closed it, and which events its connection's news fires, with the
//! reference browser's messages. A document's host and a worker's each
//! keep one, and start the connections it asks for.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use js::{Answer, DomError, SourceCall};
use net::{Initiator, Url};

use crate::loader::event_source::{SourceControl, SourceUpdate};

const CONNECTING: u8 = 0;
const OPEN: u8 = 1;
const CLOSED: u8 = 2;

/// A source's state.
struct Source {
    /// Its URL, serialized.
    url: String,
    with_credentials: bool,
    state: u8,
    /// The origin of its response's final URL, its messages' `origin`.
    origin: String,
    control: Arc<SourceControl>,
}

impl Drop for Source {
    /// A source let go, its document or worker gone, ends its connection.
    fn drop(&mut self) {
        self.control.close();
    }
}

/// What a source asks of its owner, the host's engine: a connection to
/// open.
pub(crate) struct Wanted {
    pub(crate) id: u32,
    pub(crate) request: net::Request,
    pub(crate) control: Arc<SourceControl>,
}

/// A document's or a worker's EventSources.
#[derive(Default)]
pub(crate) struct Sources {
    sources: HashMap<u32, Source>,
    next: u32,
    /// How many are not closed.
    live: usize,
    /// The connections to open, for the engine.
    pub(crate) wanted: Vec<Wanted>,
    /// The events to fire, as the tasks' JSON, in order.
    pub(crate) tasks: VecDeque<String>,
    /// What the console is to say.
    pub(crate) said: Vec<String>,
}

impl Sources {
    /// Whether any source is not closed: the document is then not kept
    /// for back and forward.
    pub(crate) fn any(&self) -> bool {
        self.live > 0
    }

    /// Answers `call`: `resolve` resolves a constructor's URL against
    /// the API base URL, as UTF-8; `initiator` is the document or worker
    /// it is made by (none, a realm gone).
    pub(crate) fn call(
        &mut self,
        call: SourceCall,
        resolve: &dyn Fn(&str) -> Option<Url>,
        initiator: Option<Initiator>,
    ) -> Result<Answer, DomError> {
        match call {
            SourceCall::Open {
                url,
                with_credentials,
            } => self.open(&url, with_credentials, resolve, initiator),
            SourceCall::Close { id } => {
                if let Some(source) = self.sources.get_mut(&id) {
                    if source.state != CLOSED {
                        source.state = CLOSED;
                        self.live -= 1;
                    }
                    source.control.close();
                }
                Ok(Answer::Nothing)
            }
            SourceCall::Get { id, name } => {
                let Some(source) = self.sources.get(&id) else {
                    return Ok(Answer::Nothing);
                };
                Ok(match name.as_str() {
                    "url" => Answer::String(source.url.clone()),
                    "withCredentials" => Answer::Bool(source.with_credentials),
                    "readyState" => Answer::Number(f64::from(source.state)),
                    _ => Answer::Nothing,
                })
            }
        }
    }

    fn open(
        &mut self,
        url: &str,
        with_credentials: bool,
        resolve: &dyn Fn(&str) -> Option<Url>,
        initiator: Option<Initiator>,
    ) -> Result<Answer, DomError> {
        let Some(parsed) = resolve(url) else {
            return Err(DomError::new(
                "SyntaxError",
                format!(
                    "Failed to construct 'EventSource': Cannot open an EventSource to '{url}'. The URL is invalid."
                ),
            ));
        };
        let initiator = initiator.ok_or_else(|| {
            DomError::new(
                "SyntaxError",
                "Failed to construct 'EventSource': no document",
            )
        })?;
        let shown = parsed.as_str().to_owned();
        let mut request =
            net::Request::get(parsed, net::Destination::Empty).initiated_by(initiator);
        request.mode = net::Mode::Cors;
        request.credentials = if with_credentials {
            net::Credentials::Include
        } else {
            net::Credentials::SameOrigin
        };
        request.cache = net::CacheMode::NoStore;
        request.headers.set(b"Accept", b"text/event-stream");
        // Quiet for long, and long in all: no read timeout and no bound
        // on its whole, its parser bounding each event.
        request.lasting = true;
        self.next += 1;
        let id = self.next;
        let control = Arc::new(SourceControl::default());
        self.sources.insert(
            id,
            Source {
                url: shown,
                with_credentials,
                state: CONNECTING,
                origin: String::new(),
                control: control.clone(),
            },
        );
        self.live += 1;
        self.wanted.push(Wanted {
            id,
            request,
            control,
        });
        Ok(Answer::Number(f64::from(id)))
    }

    fn task(&mut self, id: u32, fields: &str) {
        self.tasks.push_back(format!(r#"{{"id":{id},{fields}}}"#));
    }

    /// Takes what source `id`'s connection did.
    pub(crate) fn arrived(&mut self, id: u32, update: SourceUpdate) {
        let Some(source) = self.sources.get_mut(&id) else {
            return;
        };
        if source.state == CLOSED {
            return;
        }
        match update {
            SourceUpdate::Opened { origin } => {
                source.state = OPEN;
                source.origin = origin;
                self.task(id, r#""type":"open""#);
            }
            SourceUpdate::Event {
                kind,
                data,
                last_event_id,
            } => {
                // The connection's thread says an event only once open.
                let fields = format!(
                    r#""type":"message","kind":{},"data":{},"origin":{},"lastEventId":{}"#,
                    super::json_string(&kind),
                    super::json_string(&data),
                    super::json_string(&source.origin),
                    super::json_string(&last_event_id)
                );
                self.task(id, &fields);
            }
            // Section 9.2.3, "reestablish the connection".
            SourceUpdate::Reconnecting => {
                source.state = CONNECTING;
                self.task(id, r#""type":"error""#);
            }
            // "Fail the connection".
            SourceUpdate::Failed(said) => {
                source.state = CLOSED;
                self.live -= 1;
                if let Some(said) = said {
                    self.said.push(said);
                }
                self.task(id, r#""type":"error","last":true"#);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_source_is_closed_and_its_later_news_fires_nothing() {
        let mut sources = Sources::default();
        let resolve = |text: &str| Url::parse(text).ok();
        let url = Url::parse("https://a.example/").unwrap();
        let initiator = Initiator {
            origin: url.origin(),
            url,
            address_space: net::AddressSpace::Public,
        };
        let answer = sources
            .call(
                SourceCall::Open {
                    url: "https://a.example/s".into(),
                    with_credentials: false,
                },
                &resolve,
                Some(initiator),
            )
            .unwrap();
        assert_eq!(answer, Answer::Number(1.0));
        assert!(sources.any());
        let wanted = sources.wanted.pop().unwrap();
        assert_eq!(
            wanted.request.headers.get(b"Accept").as_deref(),
            Some(&b"text/event-stream"[..])
        );
        assert_eq!(wanted.request.credentials, net::Credentials::SameOrigin);
        sources.arrived(1, SourceUpdate::Failed(Some("why".into())));
        assert!(!sources.any());
        sources.arrived(
            1,
            SourceUpdate::Opened {
                origin: "https://a.example".into(),
            },
        );
        assert_eq!(sources.said, vec!["why".to_owned()]);
        assert_eq!(
            Vec::from(sources.tasks.clone()),
            vec![r#"{"id":1,"type":"error","last":true}"#.to_owned()]
        );
    }
}
