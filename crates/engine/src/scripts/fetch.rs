//! A script's `fetch()` as the engine takes it (M27.1,
//! docs/design/fetch.md, "The realm asks, the engine decides"): the
//! request the bindings describe checked again, whatever they did, and
//! the answer a script may see of the response.

use js::{DomError, FetchAnswer, FetchCall};
use net::headers;
use net::{
    Credentials, Destination, HeaderList, Method, Mode, Redirect, Response, ResponseType, Url,
};

use crate::loader::Page;

/// The most fetches a document has in flight; more are refused.
pub(crate) const MAX_SCRIPT_FETCHES: usize = 1000;

/// The most bytes a script's request body holds.
pub(crate) const MAX_REQUEST_BODY: usize = 64 << 20;

/// The most bytes a script's header list holds, names and values.
pub(crate) const MAX_REQUEST_HEADERS: usize = 256 << 10;

fn type_error(message: impl Into<String>) -> DomError {
    DomError::new("TypeError", message)
}

/// The request `call` asks for from the document of `page`: its URL a
/// URL without credentials, its method a token and not forbidden, its
/// modes Fetch's, its fields valid, the forbidden ones dropped and, in
/// no-cors mode, those not no-CORS-safelisted; its body within
/// [`MAX_REQUEST_BODY`] and not with `GET` or `HEAD`; the document its
/// initiator, so that its origin, address space and mixed content count.
pub(crate) fn request(page: &Page, call: &FetchCall) -> Result<net::Request, DomError> {
    let url = Url::parse(&call.url)
        .map_err(|_| type_error(format!("Failed to parse URL from {}", call.url)))?;
    if url.includes_credentials() {
        return Err(type_error(format!(
            "Request cannot be constructed from a URL that includes credentials: {}",
            call.url
        )));
    }
    let method = Method::parse(&call.method)
        .ok_or_else(|| type_error(format!("'{}' is not a valid HTTP method.", call.method)))?;
    if method.is_forbidden() {
        return Err(type_error(format!(
            "'{}' HTTP method is unsupported.",
            call.method
        )));
    }
    let mode = match call.mode.as_str() {
        "same-origin" => Mode::SameOrigin,
        "no-cors" => Mode::NoCors,
        "cors" => Mode::Cors,
        other => return Err(type_error(format!("'{other}' is not a request mode."))),
    };
    if mode == Mode::NoCors && !method.is_cors_safelisted() {
        return Err(type_error(format!(
            "'{}' is unsupported in no-cors mode.",
            call.method
        )));
    }
    let credentials = match call.credentials.as_str() {
        "omit" => Credentials::Omit,
        "same-origin" => Credentials::SameOrigin,
        "include" => Credentials::Include,
        other => return Err(type_error(format!("'{other}' is not a credentials mode."))),
    };
    let redirect = match call.redirect.as_str() {
        "follow" => Redirect::Follow,
        "error" => Redirect::Error,
        "manual" => Redirect::Manual,
        other => return Err(type_error(format!("'{other}' is not a redirect mode."))),
    };
    if call
        .body
        .as_ref()
        .is_some_and(|body| body.len() > MAX_REQUEST_BODY)
    {
        return Err(type_error("The request's body is too large."));
    }
    if call.body.is_some() && matches!(method.as_str(), "GET" | "HEAD") {
        return Err(type_error("Request with GET/HEAD method cannot have body."));
    }
    let mut fields = HeaderList::new();
    let mut size = 0;
    for pair in call.headers.chunks(2) {
        let [name, value] = pair else {
            continue;
        };
        let value = headers::normalize(value.as_bytes());
        if !headers::is_header_name(name.as_bytes()) || !headers::is_header_value(value) {
            return Err(type_error("Invalid name or value"));
        }
        if headers::is_forbidden_request_header(name.as_bytes(), value) {
            continue;
        }
        if mode == Mode::NoCors
            && !(headers::is_no_cors_safelisted_request_header_name(name.as_bytes())
                && headers::is_cors_safelisted_request_header(name.as_bytes(), value))
        {
            continue;
        }
        size += name.len() + value.len();
        if size > MAX_REQUEST_HEADERS {
            return Err(type_error("The request's fields are too large."));
        }
        fields.append(name.as_bytes(), value);
    }
    let initiator = page
        .initiator()
        .ok_or_else(|| type_error("Failed to fetch"))?;
    let mut request = net::Request::get(url, Destination::Empty).initiated_by(initiator);
    request.method = method;
    request.headers = fields;
    request.body = call.body.clone();
    request.mode = mode;
    request.credentials = credentials;
    request.redirect = redirect;
    request.preflight = call.preflight;
    Ok(request)
}

/// What the console says of a script's fetch of `url` that failed for
/// `why` (M27.5). The page learns only that it failed, a `TypeError` or
/// an `error` event, so that a refusal tells it nothing of another
/// origin; whoever reads the console learns why.
pub(crate) fn failure_text(url: &Url, why: impl std::fmt::Display) -> String {
    format!("Failed to fetch {url}: {why}")
}

/// What a script may see of `response` (Fetch 2.2.6): its exposed view,
/// fields Latin-1 decoded.
pub(crate) fn answer(response: &Response) -> FetchAnswer {
    let latin1 = |bytes: &[u8]| bytes.iter().map(|&b| char::from(b)).collect::<String>();
    let exposed = response.exposed();
    FetchAnswer {
        response_type: match exposed.response_type {
            ResponseType::Basic => "basic",
            ResponseType::Cors => "cors",
            ResponseType::Opaque => "opaque",
            ResponseType::OpaqueRedirect => "opaqueredirect",
        },
        url_list: exposed.url_list.iter().map(ToString::to_string).collect(),
        status: exposed.status,
        status_text: latin1(exposed.status_message),
        headers: exposed
            .headers
            .iter()
            .map(|(name, value)| (latin1(name), latin1(value)))
            .collect(),
        body: exposed.body.map(<[u8]>::to_vec),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(change: impl FnOnce(&mut FetchCall)) -> FetchCall {
        let mut call = FetchCall {
            id: 1,
            method: "GET".to_owned(),
            url: "https://example.com/x".to_owned(),
            headers: Vec::new(),
            body: None,
            mode: "cors".to_owned(),
            credentials: "same-origin".to_owned(),
            redirect: "follow".to_owned(),
            preflight: false,
        };
        change(&mut call);
        call
    }

    fn page() -> Page {
        let url = Url::parse("https://example.com/page.html").unwrap();
        Page::fetched(1, url, encoding::UTF_8, None, net::AddressSpace::Public)
    }

    #[test]
    fn the_engine_checks_what_the_bindings_ask_whatever_they_did() {
        let refused = |change: fn(&mut FetchCall)| request(&page(), &call(change)).is_err();
        assert!(!refused(|_| {}));
        assert!(refused(|c| c.url = "https://u:p@example.com/".to_owned()));
        assert!(refused(|c| c.url = "not a url".to_owned()));
        assert!(refused(|c| c.method = "G T".to_owned()));
        assert!(refused(|c| c.method = "connect".to_owned()));
        assert!(refused(|c| {
            c.mode = "no-cors".to_owned();
            c.method = "PUT".to_owned();
        }));
        assert!(refused(|c| c.mode = "navigate".to_owned()));
        assert!(refused(|c| c.credentials = "always".to_owned()));
        assert!(refused(|c| c.redirect = "never".to_owned()));
        assert!(refused(|c| c.body = Some(b"x".to_vec())));
        assert!(refused(|c| {
            c.method = "HEAD".to_owned();
            c.body = Some(Vec::new());
        }));
        assert!(refused(
            |c| c.headers = vec!["a b".to_owned(), "1".to_owned()]
        ));
        assert!(refused(
            |c| c.headers = vec!["a".to_owned(), "1\n2".to_owned()]
        ));
        assert!(refused(|c| {
            c.method = "POST".to_owned();
            c.body = Some(vec![0; MAX_REQUEST_BODY + 1]);
        }));
        assert!(refused(
            |c| c.headers = vec!["a".to_owned(), "x".repeat(MAX_REQUEST_HEADERS)]
        ));
        // A document with no URL fetches nothing.
        assert!(request(&Page::given(2), &call(|_| {})).is_err());
    }

    #[test]
    fn forbidden_fields_are_dropped_and_no_cors_keeps_its_safelist() {
        fn fields(change: impl FnOnce(&mut FetchCall)) -> Vec<String> {
            let request = request(&page(), &call(change)).unwrap();
            request
                .headers
                .iter()
                .map(|(name, value)| {
                    format!(
                        "{}={}",
                        String::from_utf8_lossy(name),
                        String::from_utf8_lossy(value)
                    )
                })
                .collect()
        }
        fn pairs(list: &[&str]) -> Vec<String> {
            list.iter().map(|s| (*s).to_owned()).collect()
        }
        assert_eq!(
            fields(|c| c.headers = pairs(&["Cookie", "a", "X-A", " 1 ", "Sec-X", "b", "Host", "h"])),
            ["X-A=1"]
        );
        assert_eq!(
            fields(|c| {
                c.mode = "no-cors".to_owned();
                c.headers = pairs(&[
                    "X-A",
                    "1",
                    "Accept",
                    "a",
                    "Content-Type",
                    "application/json",
                    "Range",
                    "bytes=0-",
                ]);
            }),
            ["Accept=a"]
        );
        let made = request(
            &page(),
            &call(|c| {
                c.method = "patch".to_owned();
                c.body = Some(b"b".to_vec());
                c.mode = "same-origin".to_owned();
                c.credentials = "include".to_owned();
                c.redirect = "manual".to_owned();
                c.preflight = true;
            }),
        )
        .unwrap();
        assert_eq!(made.method.as_str(), "patch");
        assert_eq!(made.destination, Destination::Empty);
        assert_eq!(
            (made.mode, made.credentials, made.redirect),
            (Mode::SameOrigin, Credentials::Include, Redirect::Manual)
        );
        assert!(made.preflight);
        assert_eq!(made.origin, Some(page().origin().clone()));
        assert!(made.initiator.is_some());
    }
}
