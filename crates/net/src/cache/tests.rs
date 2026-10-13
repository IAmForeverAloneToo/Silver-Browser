//! The HTTP cache (M28.4): what is stored and for how long, by the
//! cache's own calls at times the tests choose, and what fetches reuse,
//! by what reaches a test server, with the reference browser's answers
//! (scratchpad m28 `p2-cache.html`, Chrome 154) where it gave them.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::cookies::Jar;
use crate::fetch::{Client, Destination, Initiator, Method, Mode, Request};
use crate::test_server::{Reply, TestServer};
use crate::{NetworkError, Origin};

/// A time well inside the representable range.
fn t0() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn at(seconds: u64) -> SystemTime {
    t0() + Duration::from_secs(seconds)
}

/// An HTTP-date, IMF-fixdate, of `time`.
fn http_date(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).unwrap().as_secs();
    let days = secs / 86_400;
    let (hour, minute, second) = (secs % 86_400 / 3600, secs % 3600 / 60, secs % 60);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days % 7) as usize];
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{weekday}, {day:02} {} {year} {hour:02}:{minute:02}:{second:02} GMT",
        months[(month - 1) as usize]
    )
}

fn fields(pairs: &[(&str, &str)]) -> HeaderList {
    let mut list = HeaderList::new();
    for (name, value) in pairs {
        list.append(name.as_bytes(), value.as_bytes());
    }
    list
}

fn stored(status: u16, pairs: &[(&str, &str)], body: &[u8]) -> Stored {
    Stored {
        status,
        status_message: b"OK".to_vec(),
        headers: fields(pairs),
        body: body.to_vec(),
        address_space: None,
    }
}

fn url(input: &str) -> Url {
    Url::parse(input).expect(input)
}

fn site(input: &str) -> Partition {
    url(input).origin().site().into()
}

/// A cache holding the response of `pairs` to a GET of
/// `http://a.test/r`, sent and come at 0 s.
fn holding(status: u16, pairs: &[(&str, &str)]) -> Cache {
    let cache = Cache::new();
    cache.store(
        &site("http://a.test/"),
        &url("http://a.test/r"),
        &HeaderList::new(),
        stored(status, pairs, b"body"),
        t0(),
        t0(),
    );
    cache
}

/// Whether `cache` gives back its response at `seconds`, fresh alone
/// or stale too.
fn gives(cache: &Cache, seconds: u64, stale: bool) -> bool {
    cache
        .lookup(
            &site("http://a.test/"),
            &url("http://a.test/r"),
            &HeaderList::new(),
            at(seconds),
            stale,
        )
        .is_some()
}

/// The last second at which `cache`'s response is fresh, if any,
/// within an hour.
fn fresh_until(cache: &Cache) -> Option<u64> {
    (0..3600).take_while(|&s| gives(cache, s, false)).last()
}

#[test]
fn max_age_expires_and_age_set_the_lifetime() {
    let date = http_date(t0());
    let cache = holding(200, &[("Cache-Control", "max-age=60")]);
    assert_eq!(fresh_until(&cache), Some(59));
    assert!(gives(&cache, 3599, true));
    let cache = holding(200, &[("Cache-Control", "public, Max-Age=60")]);
    assert_eq!(fresh_until(&cache), Some(59));
    // max-age wins over Expires.
    let later = http_date(at(600));
    let cache = holding(
        200,
        &[
            ("Cache-Control", "max-age=60"),
            ("Expires", &later),
            ("Date", &date),
        ],
    );
    assert_eq!(fresh_until(&cache), Some(59));
    let cache = holding(200, &[("Expires", &later), ("Date", &date)]);
    assert_eq!(fresh_until(&cache), Some(599));
    // Expires against Date, not the clock: a server a minute behind.
    let behind = http_date(at(540));
    let cache = holding(200, &[("Expires", &later), ("Date", &behind)]);
    assert_eq!(fresh_until(&cache), Some(59));
    // An Expires past, or not a date, is stale at once, but stored.
    for expires in [http_date(t0() - Duration::from_secs(60)), "0".to_owned()] {
        let cache = holding(200, &[("Expires", &expires), ("Date", &date)]);
        assert_eq!(fresh_until(&cache), None, "{expires}");
        assert!(gives(&cache, 0, true));
    }
    // Age counts: what is older than its lifetime came stale.
    let cache = holding(200, &[("Cache-Control", "max-age=60"), ("Age", "10")]);
    assert_eq!(fresh_until(&cache), Some(49));
    let cache = holding(200, &[("Cache-Control", "max-age=60"), ("Age", "100")]);
    assert_eq!(fresh_until(&cache), None);
    // So does a Date in the past: the apparent age.
    let ago = http_date(t0() - Duration::from_secs(30));
    let cache = holding(200, &[("Cache-Control", "max-age=60"), ("Date", &ago)]);
    assert_eq!(fresh_until(&cache), Some(29));
}

#[test]
fn the_time_on_the_way_counts_toward_the_age() {
    let cache = Cache::new();
    cache.store(
        &site("http://a.test/"),
        &url("http://a.test/r"),
        &HeaderList::new(),
        stored(200, &[("Cache-Control", "max-age=60"), ("Age", "10")], b""),
        t0() - Duration::from_secs(5),
        t0(),
    );
    assert_eq!(fresh_until(&cache), Some(44));
}

#[test]
fn a_last_modified_alone_gives_a_tenth_of_its_age() {
    let date = http_date(t0());
    let modified = http_date(t0() - Duration::from_secs(10_000));
    let cache = holding(200, &[("Last-Modified", &modified), ("Date", &date)]);
    assert_eq!(fresh_until(&cache), Some(999));
    // A 404 too, a status heuristically cacheable; a 500 not, but with
    // public.
    let cache = holding(404, &[("Last-Modified", &modified), ("Date", &date)]);
    assert_eq!(fresh_until(&cache), Some(999));
    let cache = holding(500, &[("Last-Modified", &modified), ("Date", &date)]);
    assert_eq!(cache.size().0, 0);
    let cache = holding(
        500,
        &[
            ("Last-Modified", &modified),
            ("Date", &date),
            ("Cache-Control", "public"),
        ],
    );
    assert_eq!(fresh_until(&cache), Some(999));
    // Nothing to go by: stale at once.
    let cache = holding(200, &[]);
    assert_eq!(fresh_until(&cache), None);
    assert!(gives(&cache, 0, true));
}

#[test]
fn what_may_not_be_stored_is_not() {
    for (status, pairs) in [
        (200, &[("Cache-Control", "no-store, max-age=60")][..]),
        (
            200,
            &[("Cache-Control", "max-age=60"), ("Vary", "Accept, *")],
        ),
        (206, &[("Cache-Control", "max-age=60")]),
        (500, &[]),
    ] {
        assert_eq!(holding(status, pairs).size(), (0, 0), "{status} {pairs:?}");
    }
    // A request's no-store too.
    let cache = Cache::new();
    cache.store(
        &site("http://a.test/"),
        &url("http://a.test/r"),
        &fields(&[("Cache-Control", "no-store")]),
        stored(200, &[("Cache-Control", "max-age=60")], b""),
        t0(),
        t0(),
    );
    assert_eq!(cache.size(), (0, 0));
    // A 500 with a lifetime, private, no-cache: stored.
    for pairs in [
        &[("Cache-Control", "max-age=60")][..],
        &[("Cache-Control", "private")],
        &[("Cache-Control", "no-cache, max-age=60")],
    ] {
        assert_eq!(holding(500, pairs).size().0, 1, "{pairs:?}");
    }
}

#[test]
fn no_cache_is_never_fresh_and_s_maxage_is_a_shared_caches() {
    let cache = holding(200, &[("Cache-Control", "no-cache, max-age=60")]);
    assert_eq!(fresh_until(&cache), None);
    assert!(gives(&cache, 0, true));
    let cache = holding(200, &[("Cache-Control", "s-maxage=60")]);
    assert_eq!(fresh_until(&cache), None);
}

#[test]
fn directives_are_read_as_rfc_9111_writes_them() {
    let read = |value: &str| Directives::of(&fields(&[("Cache-Control", value)]), b"cache-control");
    let d = read(r#"Private="a, b", MAX-AGE = 5 ,, no-cache"#);
    assert!(d.has("private") && d.has("no-cache") && !d.has("a"));
    // The comma inside the quotes is the value's.
    assert_eq!(d.0.len(), 3);
    assert_eq!(d.0[0].1.as_deref(), Some("a, b"));
    assert_eq!(d.seconds("max-age"), Some(5));
    for bad in [
        "max-age=-1",
        "max-age=1.5",
        "max-age=abc",
        "max-age=",
        "max-age",
    ] {
        assert_eq!(read(bad).seconds("max-age"), None, "{bad}");
    }
    assert_eq!(
        read("max-age=99999999999999999999").seconds("max-age"),
        Some(1 << 31)
    );
    assert_eq!(read(r#"max-age="30""#).seconds("max-age"), Some(30));
    // Two fields are one list; the first max-age counts.
    let two = Directives::of(
        &fields(&[
            ("Cache-Control", "public"),
            ("Cache-Control", "max-age=7, max-age=9"),
        ]),
        b"cache-control",
    );
    assert!(two.has("public"));
    assert_eq!(two.seconds("max-age"), Some(7));
}

#[test]
fn vary_keeps_a_variant_for_each_value_of_its_fields() {
    let cache = Cache::new();
    let store = |accept: &str, body: &[u8]| {
        cache.store(
            &site("http://a.test/"),
            &url("http://a.test/r"),
            &fields(&[("Accept", accept)]),
            stored(
                200,
                &[("Cache-Control", "max-age=60"), ("Vary", "accept")],
                body,
            ),
            t0(),
            t0(),
        );
    };
    let find = |pairs: &[(&str, &str)]| {
        cache
            .lookup(
                &site("http://a.test/"),
                &url("http://a.test/r"),
                &fields(pairs),
                t0(),
                false,
            )
            .map(|stored| stored.body)
    };
    store("text/html", b"html");
    store("image/png", b"png");
    assert_eq!(find(&[("Accept", "text/html")]), Some(b"html".to_vec()));
    assert_eq!(find(&[("Accept", "image/png")]), Some(b"png".to_vec()));
    assert_eq!(find(&[("Accept", "*/*")]), None);
    assert_eq!(find(&[]), None);
    store("text/html", b"html2");
    assert_eq!(cache.size(), (2, 8));
    assert_eq!(find(&[("Accept", "text/html")]), Some(b"html2".to_vec()));
}

#[test]
fn entries_are_kept_by_site_and_url_without_its_fragment() {
    let cache = holding(200, &[("Cache-Control", "max-age=60")]);
    let find = |site_url: &str, target: &str| {
        cache
            .lookup(
                &site(site_url),
                &url(target),
                &HeaderList::new(),
                t0(),
                false,
            )
            .is_some()
    };
    assert!(find("http://a.test/", "http://a.test/r#part"));
    // Another port of the same host is the same site.
    assert!(find("http://a.test:8080/", "http://a.test/r"));
    assert!(!find("http://b.test/", "http://a.test/r"));
    assert!(!find("https://a.test/", "http://a.test/r"));
    assert!(!find("http://a.test/", "http://a.test/r?q"));
    cache.invalidate(&site("http://b.test/"), &url("http://a.test/r"));
    assert!(find("http://a.test/", "http://a.test/r"));
    cache.invalidate(&site("http://a.test/"), &url("http://a.test/r#x"));
    assert!(!find("http://a.test/", "http://a.test/r"));
    assert_eq!(cache.size(), (0, 0));
}

#[test]
fn the_cache_keeps_within_its_bounds_dropping_the_least_used() {
    let cache = Cache::new();
    let lasting = [("Cache-Control", "max-age=600")];
    let put = |name: &str, bytes: usize| {
        cache.store(
            &site("http://a.test/"),
            &url(&format!("http://a.test/{name}")),
            &HeaderList::new(),
            stored(200, &lasting, &vec![b'x'; bytes]),
            t0(),
            t0(),
        );
    };
    let has = |name: &str| {
        cache
            .lookup(
                &site("http://a.test/"),
                &url(&format!("http://a.test/{name}")),
                &HeaderList::new(),
                t0(),
                false,
            )
            .is_some()
    };
    put("big", MAX_CACHE_ENTRY_BYTES + 1);
    assert!(!has("big"));
    put("edge", MAX_CACHE_ENTRY_BYTES);
    assert!(has("edge"));
    let entries = MAX_CACHE_BYTES / MAX_CACHE_ENTRY_BYTES;
    for i in 1..entries {
        put(&format!("e{i}"), MAX_CACHE_ENTRY_BYTES);
    }
    assert_eq!(cache.size(), (entries, MAX_CACHE_BYTES));
    // The first, used again, outlives the second.
    assert!(has("edge"));
    put("last", 1);
    assert!(has("edge") && !has("e1") && has("last"));
    assert!(cache.size().1 <= MAX_CACHE_BYTES);
    let cache = Cache::new();
    for i in 0..=MAX_CACHE_ENTRIES {
        cache.store(
            &site("http://a.test/"),
            &url(&format!("http://a.test/{i}")),
            &HeaderList::new(),
            stored(200, &lasting, b""),
            t0(),
            t0(),
        );
    }
    assert_eq!(cache.size(), (MAX_CACHE_ENTRIES, 0));
}

#[test]
fn modes_are_named_as_fetch_names_them() {
    for (name, mode) in [
        ("default", CacheMode::Default),
        ("no-store", CacheMode::NoStore),
        ("reload", CacheMode::Reload),
        ("no-cache", CacheMode::NoCache),
        ("force-cache", CacheMode::ForceCache),
        ("only-if-cached", CacheMode::OnlyIfCached),
    ] {
        assert_eq!(CacheMode::parse(name), Some(mode));
    }
    assert_eq!(CacheMode::parse("Default"), None);
}

// Fetches through a client.

/// A server whose `/res?...` answers with the fields its query names:
/// `cc` for Cache-Control, `vary`, `status`, `cookie` for Set-Cookie,
/// and `cors` for Access-Control-Allow-Origin: `*`; the body the number
/// of requests that reached it for that path and query.
fn counting() -> TestServer {
    let count = Mutex::new(std::collections::HashMap::<String, u32>::new());
    TestServer::plain(move |request| {
        let target = request.target.clone();
        let n = {
            let mut count = count.lock().unwrap();
            let n = count.entry(target.clone()).or_default();
            *n += 1;
            *n
        };
        let query = target.split_once('?').map_or("", |(_, q)| q);
        let mut pairs: Vec<(String, String)> = vec![("Content-Type".into(), "text/plain".into())];
        let mut status = 200;
        for part in query.split('&') {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            let value = value.replace("%20", " ").replace("%2C", ",");
            match name {
                "cc" => pairs.push(("Cache-Control".into(), value)),
                "vary" => pairs.push(("Vary".into(), value)),
                "cookie" => pairs.push(("Set-Cookie".into(), value)),
                "cors" => pairs.push(("Access-Control-Allow-Origin".into(), "*".into())),
                "status" => status = value.parse().unwrap(),
                _ => {}
            }
        }
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        Reply::with_fields(status, &borrowed, n.to_string().as_bytes())
    })
}

/// A clock the test moves by hand, from [`t0`].
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

/// A script's GET of `path` on `server`, from the server's own origin.
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

/// What the fetch's body says: how many requests reached the server.
fn body(result: Result<crate::fetch::Response, NetworkError>) -> String {
    String::from_utf8(result.expect("fetched").body).unwrap()
}

#[test]
fn a_fresh_response_is_used_again_and_a_stale_one_fetched_again() {
    let server = counting();
    let hand = Hand::default();
    let client = hand.client();
    let fresh = get(&server, "/res?cc=max-age=60");
    assert_eq!(body(client.fetch(&fresh)), "1");
    let again = client.fetch(&fresh).unwrap();
    assert_eq!(String::from_utf8(again.body).unwrap(), "1");
    assert_eq!(again.url_list, std::slice::from_ref(&fresh.url));
    assert_eq!(again.status, 200);
    assert_eq!(again.mime_type.unwrap().essence(), "text/plain");
    hand.advance(60);
    assert_eq!(body(client.fetch(&fresh)), "2");
    // Without anything to go by, every time.
    let none = get(&server, "/res");
    assert_eq!(body(client.fetch(&none)), "1");
    assert_eq!(body(client.fetch(&none)), "2");
    // A document, which streams, neither reads nor writes.
    let document = Request::get(server.url("/res?cc=max-age=60&d"), Destination::Document);
    let mut deliver = |_: crate::fetch::Delivery<'_>| {};
    client.fetch_streaming(&document, &mut deliver).unwrap();
    client.fetch_streaming(&document, &mut deliver).unwrap();
    assert_eq!(
        body(client.fetch(&get(&server, "/res?cc=max-age=60&d"))),
        "3"
    );
}

#[test]
fn each_cache_mode_reads_and_writes_as_fetch_says() {
    let server = counting();
    let client = Hand::default().client();
    let with = |path: &str, mode: CacheMode| {
        let mut request = get(&server, path);
        request.cache = mode;
        client.fetch(&request)
    };
    // no-store: the network, and nothing stored for later.
    assert_eq!(body(with("/res?cc=max-age=60&a", CacheMode::NoStore)), "1");
    assert_eq!(body(with("/res?cc=max-age=60&a", CacheMode::NoStore)), "2");
    assert_eq!(body(with("/res?cc=max-age=60&a", CacheMode::Default)), "3");
    assert_eq!(body(with("/res?cc=max-age=60&a", CacheMode::Default)), "3");
    // reload and no-cache: the network, stored for later.
    assert_eq!(body(with("/res?cc=max-age=60&a", CacheMode::Reload)), "4");
    assert_eq!(body(with("/res?cc=max-age=60&a", CacheMode::NoCache)), "5");
    assert_eq!(body(with("/res?cc=max-age=60&a", CacheMode::Default)), "5");
    // force-cache: what is stored, stale or not.
    assert_eq!(body(with("/res?cc=max-age=0&b", CacheMode::Default)), "1");
    assert_eq!(body(with("/res?cc=max-age=0&b", CacheMode::Default)), "2");
    assert_eq!(
        body(with("/res?cc=max-age=0&b", CacheMode::ForceCache)),
        "2"
    );
    assert_eq!(body(with("/res?c", CacheMode::ForceCache)), "1");
    // only-if-cached: what is stored, or a network error.
    assert_eq!(
        body(with("/res?cc=max-age=0&b", CacheMode::OnlyIfCached)),
        "2"
    );
    assert!(matches!(
        with("/res?d", CacheMode::OnlyIfCached),
        Err(NetworkError::NotCached)
    ));
    assert_eq!(body(with("/res?d", CacheMode::Default)), "1");
}

#[test]
fn a_mode_tells_the_caches_on_the_way() {
    let server = TestServer::plain(|_| Reply::ok("text/plain", b"x"));
    let client = Client::default();
    let sent = |mode: CacheMode, author: &[(&str, &str)]| {
        let mut request = get(&server, "/f");
        request.cache = mode;
        for (name, value) in author {
            request.headers.append(name.as_bytes(), value.as_bytes());
        }
        client.fetch(&request).unwrap();
        let last = server.requests().pop().unwrap();
        (
            last.header("Pragma").map(str::to_owned),
            last.header("Cache-Control").map(str::to_owned),
        )
    };
    let no_cache = (Some("no-cache".to_owned()), Some("no-cache".to_owned()));
    assert_eq!(sent(CacheMode::Default, &[]), (None, None));
    assert_eq!(sent(CacheMode::NoStore, &[]), no_cache);
    assert_eq!(sent(CacheMode::Reload, &[]), no_cache);
    assert_eq!(
        sent(CacheMode::NoCache, &[]),
        (None, Some("max-age=0".to_owned()))
    );
    assert_eq!(
        sent(CacheMode::Reload, &[("Cache-Control", "max-age=5")]),
        (Some("no-cache".to_owned()), Some("max-age=5".to_owned()))
    );
    // A conditional field makes a default request no-store.
    assert_eq!(
        sent(CacheMode::Default, &[("If-None-Match", "\"e\"")]),
        no_cache
    );
}

#[test]
fn a_conditional_request_neither_reads_nor_writes() {
    let server = counting();
    let client = Hand::default().client();
    let conditional = || {
        let mut request = get(&server, "/res?cc=max-age=60");
        request.headers.append(b"If-None-Match", b"\"e\"");
        request
    };
    assert_eq!(body(client.fetch(&conditional())), "1");
    assert_eq!(body(client.fetch(&conditional())), "2");
    assert_eq!(body(client.fetch(&get(&server, "/res?cc=max-age=60"))), "3");
    assert_eq!(body(client.fetch(&conditional())), "4");
}

#[test]
fn an_unsafe_method_makes_what_is_stored_for_its_url_go() {
    let server = counting();
    let client = Hand::default().client();
    let path = "/res?cc=max-age=60";
    assert_eq!(body(client.fetch(&get(&server, path))), "1");
    assert_eq!(body(client.fetch(&get(&server, path))), "1");
    let mut post = get(&server, path);
    post.method = Method::POST;
    post.body = Some(b"b".to_vec());
    assert_eq!(body(client.fetch(&post)), "2");
    assert_eq!(body(client.fetch(&post)), "3");
    assert_eq!(body(client.fetch(&get(&server, path))), "4");
    // An error answer leaves it.
    let failing = "/res?cc=max-age=60&status=500";
    assert_eq!(body(client.fetch(&get(&server, failing))), "1");
    let mut post = get(&server, failing);
    post.method = Method::POST;
    post.body = Some(Vec::new());
    assert_eq!(body(client.fetch(&post)), "2");
    assert_eq!(body(client.fetch(&get(&server, failing))), "1");
}

#[test]
fn a_site_does_not_see_what_another_site_fetched() {
    let server = counting();
    let client = Hand::default().client();
    let from = |site: &str| {
        let mut request = get(&server, "/res?cc=max-age=60&cors");
        request.mode = Mode::Cors;
        request.origin = Some(url(site).origin());
        request
    };
    assert_eq!(body(client.fetch(&from("http://a.test/"))), "1");
    assert_eq!(body(client.fetch(&from("http://a.test/"))), "1");
    assert_eq!(body(client.fetch(&from("http://www.a.test/"))), "1");
    assert_eq!(body(client.fetch(&from("http://b.test/"))), "2");
    assert_eq!(body(client.fetch(&from("http://b.test/"))), "2");
    // An opaque origin's is its own.
    let opaque = |client: &Client| {
        let mut request = get(&server, "/res?cc=max-age=60&cors");
        request.mode = Mode::Cors;
        request.origin = Some(Origin::new_opaque());
        client.fetch(&request)
    };
    assert_eq!(body(opaque(&client)), "3");
    assert_eq!(body(opaque(&client)), "4");
}

#[test]
fn frames_of_two_sites_under_one_top_level_site_do_not_share_it() {
    // M33.2: the triple key, by the top-level site and the frame's.
    let server = counting();
    let client = Hand::default().client();
    let from = |frame: &str| {
        let mut request = get(&server, "/res?cc=max-age=60&cors");
        request.mode = Mode::Cors;
        request.origin = Some(url(frame).origin());
        request.partition = Some(crate::url::Partition {
            top: url("http://a.test/").origin(),
            cross_site: true,
        });
        request
    };
    assert_eq!(body(client.fetch(&from("http://b.test/"))), "1");
    assert_eq!(body(client.fetch(&from("http://b.test/"))), "1");
    assert_eq!(body(client.fetch(&from("http://c.test/"))), "2");
}

#[test]
fn a_response_from_the_cache_is_checked_as_one_from_the_network() {
    let server = counting();
    let client = Hand::default().client();
    // No Access-Control-Allow-Origin: refused by CORS each time, the
    // second time from the cache.
    let mut request = get(&server, "/res?cc=max-age=60");
    request.mode = Mode::Cors;
    request.origin = Some(url("http://a.test/").origin());
    assert!(matches!(client.fetch(&request), Err(NetworkError::Cors(_))));
    assert!(matches!(client.fetch(&request), Err(NetworkError::Cors(_))));
    assert_eq!(server.requests().len(), 1);
    // Its Set-Cookie is not taken again.
    let jar = Arc::new(Mutex::new(Jar::new()));
    let client = Hand::default().client().with_cookies(jar.clone());
    let set = get(&server, "/res?cc=max-age=60&cookie=c=1");
    assert_eq!(body(client.fetch(&set)), "1");
    assert_eq!(jar.lock().unwrap().cookies().len(), 1);
    *jar.lock().unwrap() = Jar::new();
    assert_eq!(body(client.fetch(&set)), "1");
    assert!(jar.lock().unwrap().cookies().is_empty());
}

#[test]
fn clients_given_one_cache_share_it() {
    let server = counting();
    let cache = Arc::new(Cache::new());
    let first = Hand::default().client().with_cache(cache.clone());
    let second = Hand::default().client().with_cache(cache.clone());
    assert_eq!(body(first.fetch(&get(&server, "/res?cc=max-age=60"))), "1");
    assert_eq!(body(second.fetch(&get(&server, "/res?cc=max-age=60"))), "1");
    assert_eq!(cache.size().0, 1);
    assert!(Arc::ptr_eq(first.cache(), &cache));
}

#[test]
fn what_one_top_level_site_cached_another_does_not_find() {
    // M33.2: by the top-level site and the frame's (the triple key).
    let cache = Cache::new();
    let under = |top: &str, frame: &str| Partition {
        top: url(top).origin().site(),
        frame: url(frame).origin().site(),
    };
    cache.store(
        &under("http://a.test/", "http://b.test/"),
        &url("http://b.test/r"),
        &HeaderList::new(),
        stored(200, &[("Cache-Control", "max-age=60")], b"body"),
        t0(),
        t0(),
    );
    let finds = |partition: &Partition| {
        cache
            .lookup(
                partition,
                &url("http://b.test/r"),
                &HeaderList::new(),
                at(1),
                false,
            )
            .is_some()
    };
    assert!(finds(&under("http://a.test/", "http://b.test/")));
    assert!(!finds(&under("http://c.test/", "http://b.test/")));
    assert!(
        !finds(&under("http://b.test/", "http://b.test/")),
        "b's own"
    );
    assert!(!finds(&under("http://a.test/", "http://a.test/")));
}
