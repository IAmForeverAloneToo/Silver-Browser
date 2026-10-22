//! Handle Fetch for a document's requests (M39 step 5, Service Workers
//! "Handle Fetch"): a request made for a client whose controller's
//! active version listens for `fetch` is given to that version as a
//! `FetchEvent`; what it answers is checked as the standard checks it and
//! taken for the response, a network error, or, when it gave none, the
//! network's. Requests a version makes itself go past this: its fetcher
//! is not one of these.

use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use devtools::Json;
use net::{
    Delivery, HeaderList, Mode, NetworkError, Redirect, Request, Response, ResponseType, Url,
};

use super::{Responded, WeakServiceWorkers};
use crate::loader::Fetcher;

/// How long a fetch waits for a version's answer: an event's limit.
const WAIT: Duration = Duration::from_secs(300);

/// A fetcher that gives a controlled client's requests to its service
/// worker first.
pub(crate) struct Intercepting {
    pub(crate) inner: Arc<dyn Fetcher>,
    pub(crate) workers: WeakServiceWorkers,
}

/// The most redirects of a version's that are followed: Fetch's.
const MAX_REDIRECTS: usize = 20;

impl Intercepting {
    /// The version's answer to `request`, its redirects followed through
    /// it in turn; none for the network.
    pub(crate) fn answer(
        &self,
        request: &Request,
        hops: usize,
    ) -> Option<Result<Response, NetworkError>> {
        let answer = handle_fetch(&self.workers, request)?;
        Some(match answer {
            Ok(response)
                if matches!(response.status, 301 | 302 | 303 | 307 | 308)
                    && request.redirect == Redirect::Follow =>
            {
                self.follow(request, response, hops)
            }
            other => other,
        })
    }

    /// Fetch's HTTP-redirect fetch of a version's redirect: the request
    /// again at its `Location`, a 303's (and a POST's 301 or 302) as a
    /// GET, through the version again.
    fn follow(
        &self,
        request: &Request,
        response: Response,
        hops: usize,
    ) -> Result<Response, NetworkError> {
        let Some(location) = response
            .headers
            .last(b"Location")
            .and_then(|value| request.url.join(&String::from_utf8_lossy(value)).ok())
        else {
            return Ok(response);
        };
        if hops >= MAX_REDIRECTS {
            return Err(NetworkError::RedirectLimit);
        }
        let mut next = request.clone();
        next.url = location;
        if response.status == 303
            || (matches!(response.status, 301 | 302) && request.method == net::Method::POST)
        {
            next.method = net::Method::GET;
            next.body = None;
        }
        let mut result = match self.answer(&next, hops + 1) {
            Some(result) => result?,
            None => self.inner.fetch(&next)?,
        };
        let mut urls = vec![request.url.clone()];
        urls.append(&mut result.url_list);
        result.url_list = urls;
        Ok(result)
    }
}

impl Fetcher for Intercepting {
    fn fetch(&self, request: &Request) -> Result<Response, NetworkError> {
        match self.answer(request, 0) {
            Some(answer) => answer,
            None => self.inner.fetch(request),
        }
    }

    fn fetch_streaming(
        &self,
        request: &Request,
        deliver: &mut dyn FnMut(Delivery<'_>),
    ) -> Result<Response, NetworkError> {
        match self.answer(request, 0) {
            Some(Ok(mut response)) => {
                let body = std::mem::take(&mut response.body);
                deliver(Delivery::Head(&response));
                deliver(Delivery::Body(&body));
                Ok(response)
            }
            Some(Err(error)) => Err(error),
            None => self.inner.fetch_streaming(request, deliver),
        }
    }
}

/// Handle Fetch: the controlling version's answer to `request`, or none
/// for the network.
pub(crate) fn handle_fetch(
    workers: &WeakServiceWorkers,
    request: &Request,
) -> Option<Result<Response, NetworkError>> {
    // Navigations are matched by their URL (step 6).
    if request.mode == Mode::Navigate {
        return None;
    }
    let id = request.client.as_ref()?;
    let workers = workers.upgrade()?;
    let registration = workers.client(id)?.controller?;
    let version = workers.query_any(|registry| {
        let reg = registry.registration(registration)?;
        let v = registry.version(reg.active?)?;
        v.handles("fetch").then_some(v.id)
    })?;
    let (sender, receiver) = channel();
    workers.fetch_event(
        version,
        event_json(request, id, ""),
        Box::new(move |responded| {
            let _ = sender.send(responded);
        }),
    );
    let began = Instant::now();
    let responded = loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(responded) => break responded,
            Err(RecvTimeoutError::Timeout) if began.elapsed() < WAIT => {
                if request.abort.is_aborted() {
                    return Some(Err(NetworkError::Aborted));
                }
            }
            Err(_) => break Responded::Network,
        }
    };
    match responded {
        Responded::Network => None,
        Responded::Failed => Some(Err(NetworkError::Refused(
            "the service worker answered with a network error".to_owned(),
        ))),
        Responded::Given(response) => Some(checked(request, *response)),
    }
}

/// A request as a `FetchEvent` reads it, with its client's ids.
pub(crate) fn event_json(request: &Request, client: &str, resulting: &str) -> String {
    let headers: Vec<Json> = request
        .headers
        .iter()
        .map(|(name, value)| {
            Json::Array(vec![
                Json::from(String::from_utf8_lossy(name).into_owned()),
                Json::from(String::from_utf8_lossy(value).into_owned()),
            ])
        })
        .collect();
    let body = request.body.as_ref().map_or(Json::Null, |body| {
        Json::Array(body.iter().map(|b| Json::from(u32::from(*b))).collect())
    });
    let referrer = match &request.referrer {
        net::Referrer::Url(url) => url.to_string(),
        net::Referrer::None => String::new(),
        net::Referrer::Client => request
            .initiator
            .as_ref()
            .map(|initiator| initiator.url.to_string())
            .unwrap_or_default(),
    };
    Json::object([
        (
            "request",
            Json::object([
                ("url", Json::from(request.url.as_str())),
                ("method", Json::from(request.method.as_str())),
                ("headers", Json::Array(headers)),
                ("mode", Json::from(request.mode.name())),
                (
                    "credentials",
                    Json::from(match request.credentials {
                        net::Credentials::Omit => "omit",
                        net::Credentials::SameOrigin => "same-origin",
                        net::Credentials::Include => "include",
                    }),
                ),
                (
                    "cache",
                    Json::from(match request.cache {
                        net::CacheMode::Default => "default",
                        net::CacheMode::NoStore => "no-store",
                        net::CacheMode::Reload => "reload",
                        net::CacheMode::NoCache => "no-cache",
                        net::CacheMode::ForceCache => "force-cache",
                        net::CacheMode::OnlyIfCached => "only-if-cached",
                    }),
                ),
                (
                    "redirect",
                    Json::from(match request.redirect {
                        Redirect::Follow => "follow",
                        Redirect::Error => "error",
                        Redirect::Manual => "manual",
                    }),
                ),
                (
                    "destination",
                    Json::from(match request.destination {
                        net::Destination::Empty | net::Destination::Report => "",
                        other => other.name(),
                    }),
                ),
                ("referrer", Json::from(referrer)),
                ("body", body),
            ]),
        ),
        ("clientId", Json::from(client)),
        ("resultingClientId", Json::from(resulting)),
    ])
    .write()
}

/// The response a version gave, as `respondWith` sent it: its head as
/// JSON, its body's bytes apart. None for one that does not read.
pub(crate) fn given(head: &Json, body: Vec<u8>) -> Option<Response> {
    let response_type = match head.get("type").and_then(Json::as_str)? {
        "basic" | "default" => ResponseType::Basic,
        "cors" => ResponseType::Cors,
        "opaque" => ResponseType::Opaque,
        "opaqueredirect" => ResponseType::OpaqueRedirect,
        _ => return None,
    };
    let mut headers = HeaderList::new();
    for pair in head.get("headers").and_then(Json::as_array)? {
        let pair = pair.as_array()?;
        headers.append(
            pair.first()?.as_str()?.as_bytes(),
            pair.get(1)?.as_str()?.as_bytes(),
        );
    }
    let url_list = head
        .get("urls")
        .and_then(Json::as_array)?
        .iter()
        .filter_map(|url| Url::parse(url.as_str()?).ok())
        .collect();
    Some(Response {
        url_list,
        status: u16::try_from(head.get("status").and_then(Json::as_u64)?).ok()?,
        status_message: head
            .get("statusText")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
        mime_type: net::mime::extract_mime_type(&headers),
        headers,
        body,
        address_space: None,
        response_type,
        cors_exposed: Vec::new(),
    })
}

/// Handle Fetch's checks of a version's response for `request`: an opaque
/// one for a request not in no-cors mode, an opaque redirect for one not
/// in manual redirect mode, a CORS one for a same-origin request, are
/// network errors. A response made by script has the request's URL.
fn checked(request: &Request, mut response: Response) -> Result<Response, NetworkError> {
    let refused = match response.response_type {
        ResponseType::Opaque => request.mode != Mode::NoCors,
        ResponseType::OpaqueRedirect => request.redirect != Redirect::Manual,
        ResponseType::Cors => request.mode == Mode::SameOrigin,
        ResponseType::Basic => false,
    };
    if refused {
        return Err(NetworkError::Refused(
            "the service worker's response is not one the request may have".to_owned(),
        ));
    }
    if response.url_list.is_empty() {
        response.url_list.push(request.url.clone());
    }
    Ok(response)
}
