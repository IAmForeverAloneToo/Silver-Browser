//! Revalidation (M38): a stored response with validators asked about
//! with a conditional request, a `304` freshening it, the request's
//! own directives, `must-revalidate` and `immutable`, `Age`, and the
//! documents a streamed fetch keeps.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::fetch::{Client, Delivery, Destination, Initiator, Request};
use crate::test_server::{Reply, ServerRequest, TestServer};

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
}

/// A clock the test moves by hand.
#[derive(Clone, Default)]
struct Hand(Arc<AtomicU64>);

impl Hand {
    fn advance(&self, seconds: u64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }

    fn client(&self) -> Client {
        let seconds = self.0.clone();
        Client::default().with_clock(Arc::new(move || at(seconds.load(Ordering::SeqCst))))
    }
}

/// A server whose `/r?...` answers with the fields its query names (`cc`
/// for Cache-Control, `etag`, `lm` for Last-Modified, `cc304` for the
/// Cache-Control of a 304, `x` for an X-Version a 304 changes), and a
/// `304` to a request whose validator matches; a 200's body the count of
/// 200s for that target. Every request it read, kept.
fn validating() -> (TestServer, Arc<Mutex<Vec<ServerRequest>>>) {
    let seen: Arc<Mutex<Vec<ServerRequest>>> = Arc::default();
    let kept = seen.clone();
    let counts = Mutex::new(std::collections::HashMap::<String, u32>::new());
    let server = TestServer::plain(move |request| {
        kept.lock().unwrap().push(request.clone());
        let target = request.target.clone();
        let query = target.split_once('?').map_or("", |(_, q)| q).to_owned();
        let param = |name: &str| {
            query.split('&').find_map(|part| {
                let (n, v) = part.split_once('=').unwrap_or((part, ""));
                (n == name).then(|| v.replace("%20", " ").replace("%2C", ","))
            })
        };
        let etag = param("etag").map(|tag| format!("\"{tag}\""));
        let modified = param("lm").map(|_| "Mon, 01 Jan 2024 00:00:00 GMT".to_owned());
        let matches = match (&etag, request.header("If-None-Match")) {
            (Some(etag), Some(asked)) => asked == etag,
            _ => matches!((&modified, request.header("If-Modified-Since")),
                (Some(m), Some(asked)) if asked == m),
        };
        if matches {
            let mut fields: Vec<(String, String)> = Vec::new();
            if let Some(cc) = param("cc304") {
                fields.push(("Cache-Control".into(), cc));
            }
            fields.push(("X-Version".into(), "after".into()));
            let mut bytes = b"HTTP/1.1 304 Not Modified\r\n".to_vec();
            for (name, value) in fields {
                bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
            }
            bytes.extend_from_slice(b"\r\n");
            return Reply::Send(bytes);
        }
        let n = {
            let mut counts = counts.lock().unwrap();
            let n = counts.entry(target).or_default();
            *n += 1;
            *n
        };
        let mut fields: Vec<(String, String)> = vec![
            ("Content-Type".into(), "text/plain".into()),
            ("X-Version".into(), "before".into()),
        ];
        if let Some(cc) = param("cc") {
            fields.push(("Cache-Control".into(), cc));
        }
        if let Some(etag) = etag {
            fields.push(("ETag".into(), etag));
        }
        if let Some(modified) = modified {
            fields.push(("Last-Modified".into(), modified));
        }
        let borrowed: Vec<(&str, &str)> = fields
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        Reply::with_fields(200, &borrowed, n.to_string().as_bytes())
    });
    (server, seen)
}

fn get(server: &TestServer, path: &str) -> Request {
    let target = server.url(path);
    let mut request = Request::get(target.clone(), Destination::Empty);
    request.origin = Some(target.origin());
    request.initiator = Some(Initiator {
        url: target.clone(),
        address_space: crate::AddressSpace::Loopback,
        origin: target.origin(),
    });
    request
}

fn body(result: Result<crate::fetch::Response, crate::NetworkError>) -> String {
    String::from_utf8(result.expect("fetched").body).unwrap()
}

/// The last request the server read.
fn last(seen: &Arc<Mutex<Vec<ServerRequest>>>) -> ServerRequest {
    seen.lock().unwrap().last().cloned().expect("a request")
}

#[test]
fn a_stale_response_with_an_etag_is_revalidated_and_a_304_answers_with_it() {
    let (server, seen) = validating();
    let hand = Hand::default();
    let client = hand.client();
    let request = get(&server, "/r?cc=max-age=10&etag=v1&cc304=max-age=60");
    assert_eq!(body(client.fetch(&request)), "1");
    hand.advance(20);
    let again = client.fetch(&request).unwrap();
    assert_eq!(
        String::from_utf8(again.body.clone()).unwrap(),
        "1",
        "the stored body"
    );
    assert_eq!(again.status, 200);
    assert_eq!(last(&seen).header("If-None-Match"), Some("\"v1\""));
    // The 304's fields replaced the stored ones, and its max-age made it
    // fresh again: no request at all now.
    assert_eq!(
        again.headers.get(b"x-version").as_deref(),
        Some(&b"after"[..])
    );
    let requests = seen.lock().unwrap().len();
    hand.advance(30);
    assert_eq!(body(client.fetch(&request)), "1");
    assert_eq!(seen.lock().unwrap().len(), requests);
    // Its Age says how old it is.
    let aged = client.fetch(&request).unwrap();
    assert_eq!(aged.headers.get(b"age").as_deref(), Some(&b"30"[..]));
}

#[test]
fn last_modified_is_a_validator_too_and_without_one_the_whole_comes_again() {
    let (server, seen) = validating();
    let hand = Hand::default();
    let client = hand.client();
    let dated = get(&server, "/r?cc=max-age=0&lm=1");
    assert_eq!(body(client.fetch(&dated)), "1");
    assert_eq!(body(client.fetch(&dated)), "1");
    assert_eq!(
        last(&seen).header("If-Modified-Since"),
        Some("Mon, 01 Jan 2024 00:00:00 GMT")
    );
    let bare = get(&server, "/r?cc=max-age=0");
    assert_eq!(body(client.fetch(&bare)), "1");
    assert_eq!(body(client.fetch(&bare)), "2");
    assert_eq!(last(&seen).header("If-None-Match"), None);
    assert_eq!(last(&seen).header("If-Modified-Since"), None);
}

#[test]
fn no_cache_is_always_revalidated_and_must_revalidate_when_stale() {
    let (server, seen) = validating();
    let hand = Hand::default();
    let client = hand.client();
    // A response's no-cache: asked about every time, though fresh.
    let request = get(&server, "/r?cc=no-cache%2C%20max-age=600&etag=a");
    client.fetch(&request).unwrap();
    client.fetch(&request).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(last(&seen).header("If-None-Match"), Some("\"a\""));
    // A request's max-stale takes a stale response...
    let lax = get(&server, "/r?cc=max-age=10&etag=b");
    client.fetch(&lax).unwrap();
    hand.advance(20);
    let mut stale = lax.clone();
    stale.headers.append(b"Cache-Control", b"max-stale=60");
    let before = seen.lock().unwrap().len();
    client.fetch(&stale).unwrap();
    assert_eq!(seen.lock().unwrap().len(), before, "taken stale");
    // ...but not one whose server said must-revalidate.
    let strict = get(&server, "/r?cc=max-age=10%2C%20must-revalidate&etag=c");
    client.fetch(&strict).unwrap();
    hand.advance(20);
    let mut stale = strict.clone();
    stale.headers.append(b"Cache-Control", b"max-stale=60");
    let before = seen.lock().unwrap().len();
    client.fetch(&stale).unwrap();
    assert_eq!(seen.lock().unwrap().len(), before + 1, "revalidated");
}

#[test]
fn the_requests_own_directives_and_the_cache_modes() {
    let (server, seen) = validating();
    let hand = Hand::default();
    let client = hand.client();
    let request = get(&server, "/r?cc=max-age=100&etag=d");
    client.fetch(&request).unwrap();
    hand.advance(50);
    let count = |seen: &Arc<Mutex<Vec<ServerRequest>>>| seen.lock().unwrap().len();
    let before = count(&seen);
    // max-age=30 of the request: 50 seconds old is too old.
    let mut young = request.clone();
    young.headers.append(b"Cache-Control", b"max-age=30");
    client.fetch(&young).unwrap();
    assert_eq!(count(&seen), before + 1);
    // That 304 made it fresh again, for its 100 seconds: min-fresh=200
    // asks for more than it has.
    let mut lasting = request.clone();
    lasting.headers.append(b"Cache-Control", b"min-fresh=200");
    client.fetch(&lasting).unwrap();
    assert_eq!(count(&seen), before + 2);
    // no-cache mode: revalidated, fresh as it is.
    let mut asked = request.clone();
    asked.cache = CacheMode::NoCache;
    client.fetch(&asked).unwrap();
    assert_eq!(count(&seen), before + 3);
    assert_eq!(last(&seen).header("If-None-Match"), Some("\"d\""));
    // ...but immutable and fresh, not (RFC 8246).
    let fixed = get(&server, "/r?cc=max-age=100%2C%20immutable&etag=e");
    client.fetch(&fixed).unwrap();
    let mut asked = fixed.clone();
    asked.cache = CacheMode::NoCache;
    let before = count(&seen);
    client.fetch(&asked).unwrap();
    assert_eq!(count(&seen), before);
    // reload: neither read nor conditional.
    let mut reload = request.clone();
    reload.cache = CacheMode::Reload;
    client.fetch(&reload).unwrap();
    assert_eq!(last(&seen).header("If-None-Match"), None);
}

#[test]
fn a_document_is_kept_and_answered_as_it_streams() {
    let (server, seen) = validating();
    let client = Hand::default().client();
    let document = Request::get(server.url("/r?cc=max-age=60"), Destination::Document);
    let stream = |client: &Client| {
        let mut heads = Vec::new();
        let mut bytes = Vec::new();
        let mut deliver = |delivery: Delivery<'_>| match delivery {
            Delivery::Head(response) => heads.push(response.status),
            Delivery::Body(piece) => bytes.extend_from_slice(piece),
        };
        let response = client.fetch_streaming(&document, &mut deliver).unwrap();
        (heads, bytes, response.body.is_empty())
    };
    assert_eq!(stream(&client), (vec![200], b"1".to_vec(), true));
    // From the cache, handed over as a stream would be.
    assert_eq!(stream(&client), (vec![200], b"1".to_vec(), true));
    assert_eq!(seen.lock().unwrap().len(), 1);
    // A document revalidated: the 304 is not handed over, the stored
    // response is.
    let revalidated = Request::get(server.url("/r?cc=no-cache&etag=f"), Destination::Document);
    let mut deliver = |_: Delivery<'_>| {};
    client.fetch_streaming(&revalidated, &mut deliver).unwrap();
    let mut heads = Vec::new();
    let mut bytes = Vec::new();
    let mut deliver = |delivery: Delivery<'_>| match delivery {
        Delivery::Head(response) => heads.push(response.status),
        Delivery::Body(piece) => bytes.extend_from_slice(piece),
    };
    client.fetch_streaming(&revalidated, &mut deliver).unwrap();
    assert_eq!(heads, [200]);
    assert_eq!(bytes, b"1");
    assert_eq!(last(&seen).header("If-None-Match"), Some("\"f\""));
}

#[test]
fn a_document_larger_than_an_entry_is_not_kept() {
    let big = vec![b'x'; MAX_CACHE_ENTRY_BYTES + 1];
    let server = TestServer::plain(move |_| {
        Reply::with_fields(200, &[("Cache-Control", "max-age=60")], &big)
    });
    let client = Client::default();
    let document = Request::get(server.url("/big"), Destination::Document);
    let mut deliver = |_: Delivery<'_>| {};
    client.fetch_streaming(&document, &mut deliver).unwrap();
    client.fetch_streaming(&document, &mut deliver).unwrap();
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn freshen_keeps_the_fields_that_frame_the_body() {
    let cache = Cache::new();
    let site: Partition = crate::Url::parse("http://a.test/")
        .unwrap()
        .origin()
        .site()
        .into();
    let url = crate::Url::parse("http://a.test/r").unwrap();
    let mut headers = HeaderList::new();
    headers.append(b"content-length", b"5");
    headers.append(b"etag", b"\"1\"");
    headers.append(b"cache-control", b"no-cache");
    cache.store(
        &site,
        &url,
        &HeaderList::new(),
        Stored {
            status: 200,
            status_message: b"OK".to_vec(),
            headers,
            body: b"hello".to_vec(),
            address_space: None,
        },
        at(0),
        at(0),
    );
    let mut not_modified = HeaderList::new();
    not_modified.append(b"content-length", b"0");
    not_modified.append(b"etag", b"\"2\"");
    not_modified.append(b"cache-control", b"max-age=60");
    let stored = cache
        .freshen(
            &site,
            &url,
            &HeaderList::new(),
            &not_modified,
            at(10),
            at(10),
        )
        .unwrap();
    assert_eq!(
        stored.headers.get(b"content-length").as_deref(),
        Some(&b"5"[..])
    );
    assert_eq!(stored.headers.get(b"etag").as_deref(), Some(&b"\"2\""[..]));
    assert_eq!(stored.body, b"hello");
    let hit = cache
        .lookup(&site, &url, &HeaderList::new(), at(20))
        .unwrap();
    assert!(hit.fresh);
    // Nothing stored, nothing freshened.
    let other = crate::Url::parse("http://a.test/none").unwrap();
    assert!(
        cache
            .freshen(
                &site,
                &other,
                &HeaderList::new(),
                &not_modified,
                at(10),
                at(10)
            )
            .is_none()
    );
}
