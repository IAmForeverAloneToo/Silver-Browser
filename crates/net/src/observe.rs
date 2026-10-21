//! What a client does, told as it does it (M36, the DevTools' network
//! log, docs/design/devtools.md): each hop's request as it is sent, the
//! client's own fields included, each redirect followed, the response's
//! head, its body as it comes, and the fetch's end. An observer is told
//! on the fetch's own thread, and must be quick.

use crate::{Destination, HeaderList, NetworkError, ResponseType, Url};

/// Who is told what a client does.
pub trait Observer: Send + Sync {
    /// Fetch `fetch`, a number the client gives each of its fetches, did
    /// `what`.
    fn observe(&self, fetch: u64, what: Observed<'_>);
}

/// What a fetch did.
#[derive(Debug)]
pub enum Observed<'a> {
    /// A hop's request, as sent: the first, or one a redirect made.
    Request {
        method: &'a str,
        url: &'a Url,
        headers: &'a HeaderList,
        destination: Destination,
        /// The URL of the document or worker that asked, if one did.
        initiator: Option<&'a Url>,
    },
    /// A redirect of `status` to `to`, which the fetch follows.
    Redirect { status: u16, to: &'a Url },
    /// The response's head: from the HTTP cache when `cached`.
    Response {
        status: u16,
        headers: &'a HeaderList,
        cached: bool,
        /// The protocol it came in, if it came from the network (M38).
        protocol: Option<crate::http::Version>,
    },
    /// A piece of its body, its content codings removed.
    Body(&'a [u8]),
    /// Its end: its response's type, or why it failed.
    End(Result<ResponseType, &'a NetworkError>),
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::test_server::{Reply, TestServer};
    use crate::{Client, Delivery, Request};

    /// What an observer was told, as lines.
    #[derive(Default)]
    struct Told(Mutex<Vec<String>>);

    impl Observer for Told {
        fn observe(&self, fetch: u64, what: Observed<'_>) {
            let line = match what {
                Observed::Request {
                    method,
                    url,
                    headers,
                    destination,
                    ..
                } => format!(
                    "{fetch} request {method} {} {destination:?} ua={}",
                    url.path_str(),
                    headers.get(b"User-Agent").is_some()
                ),
                Observed::Redirect { status, to } => {
                    format!("{fetch} redirect {status} {}", to.path_str())
                }
                Observed::Response { status, cached, .. } => {
                    format!("{fetch} response {status} cached={cached}")
                }
                Observed::Body(bytes) => format!("{fetch} body {}", bytes.len()),
                Observed::End(Ok(kind)) => format!("{fetch} end {kind:?}"),
                Observed::End(Err(error)) => format!("{fetch} failed {error}"),
            };
            self.0.lock().unwrap().push(line);
        }
    }

    impl Told {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    #[test]
    fn a_client_tells_its_observer_each_hop_and_its_end() {
        let server = TestServer::plain(|request| match request.target.as_str() {
            "/moved" => Reply::with_fields(302, &[("Location", "/here")], b""),
            "/here" => Reply::with_fields(
                200,
                &[
                    ("Content-Type", "text/plain"),
                    ("Cache-Control", "max-age=600"),
                ],
                b"hello",
            ),
            "/stream" => Reply::ok("text/plain", b"hello"),
            _ => Reply::with_fields(404, &[], b""),
        });
        let told = Arc::new(Told::default());
        let client = Client::default().with_observer(told.clone());
        let request = Request::get(server.url("/moved"), crate::Destination::Empty);
        client.fetch(&request).expect("fetched");
        assert_eq!(
            told.take(),
            [
                "0 request GET /moved Empty ua=true",
                "0 response 302 cached=false",
                "0 redirect 302 /here",
                "0 request GET /here Empty ua=true",
                "0 response 200 cached=false",
                "0 body 5",
                "0 end Basic",
            ]
        );
        // From the cache the second time.
        let request = Request::get(server.url("/here"), crate::Destination::Image);
        client.fetch(&request).expect("fetched");
        assert_eq!(
            told.take(),
            [
                "1 request GET /here Image ua=true",
                "1 response 200 cached=true",
                "1 body 5",
                "1 end Basic",
            ]
        );
        // Streamed: the head as it comes, then the body.
        let request = Request::get(server.url("/stream"), crate::Destination::Document);
        client
            .fetch_streaming(&request, &mut |_: Delivery<'_>| {})
            .expect("fetched");
        assert_eq!(
            told.take(),
            [
                "2 request GET /stream Document ua=true",
                "2 response 200 cached=false",
                "2 body 5",
                "2 end Basic",
            ]
        );
        // Streamed from the cache (M38): told as it is handed over.
        let request = Request::get(server.url("/here"), crate::Destination::Document);
        client
            .fetch_streaming(&request, &mut |_: Delivery<'_>| {})
            .expect("fetched");
        assert_eq!(
            told.take(),
            [
                "3 request GET /here Document ua=true",
                "3 response 200 cached=true",
                "3 body 5",
                "3 end Basic",
            ]
        );
        // A failure, and a data: URL, which no server answers.
        let request = Request::get(
            crate::Url::parse("http://127.0.0.1:1/").expect("a URL"),
            crate::Destination::Empty,
        );
        assert!(client.fetch(&request).is_err());
        assert!(
            told.take()
                .last()
                .is_some_and(|line| line.starts_with("4 failed"))
        );
        let request = Request::get(
            crate::Url::parse("data:,abc").expect("a URL"),
            crate::Destination::Empty,
        );
        client.fetch(&request).expect("fetched");
        assert_eq!(
            told.take(),
            [
                "5 request GET ,abc Empty ua=false",
                "5 response 200 cached=false",
                "5 body 3",
                "5 end Basic",
            ]
        );
    }
}
