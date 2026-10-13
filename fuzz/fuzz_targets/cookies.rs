//! Cookies (M27.4): any bytes as `Set-Cookie` values and as a script's
//! `document.cookie` strings, stored from the URLs of a table into one
//! jar, in contexts and at times the input says, with the `Cookie`
//! header read between. Nothing panics; every cookie a store keeps is
//! one the rules allow that store, checked here on what was kept rather
//! than on how the jar decided; the jar stays within its limits, never
//! holds two cookies of one name, domain and path, and keeps nothing
//! expired as of its last store or read (it takes them out then); and
//! the header it sends is exactly the cookies that go to its URL,
//! serialized. The jar written as the profile keeps it (M28.3) and read
//! back is its cookies with an expiry, as they were; records of any
//! bytes read with them give a jar that keeps the same rules.

#![no_main]
#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libfuzzer_sys::fuzz_target;
use net::Url;
use net::cookies::{
    Api, Context, Cookie, Jar, MAX_COOKIE_AGE, MAX_COOKIE_SIZE, MAX_COOKIES,
    MAX_COOKIES_PER_DOMAIN, SameSite,
};
use net::url::Host;

/// The URLs cookies come from and go to: hosts and their subdomains,
/// paths deep and shallow, secure and not, a public suffix of each
/// section of the list, and an address.
const URLS: [&str; 10] = [
    "http://example.com/",
    "https://example.com/a/b",
    "http://sub.example.com/a/",
    "https://www.example.co.uk/x",
    "http://localhost:8080/p/q",
    "https://b.localhost/",
    "http://127.0.0.1/a/b/c",
    "http://[::1]/",
    "https://github.io/",
    "https://user.github.io/r",
];

/// The input, read a byte at a time; past its end, zeros.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    /// Up to 255 bytes, as the next byte says.
    fn bytes(&mut self) -> &[u8] {
        let length = usize::from(self.byte()).min(self.0.len());
        let (taken, rest) = self.0.split_at(length);
        self.0 = rest;
        taken
    }
}

/// What tells two kept cookies apart, but for their order.
type Kept = (
    Vec<u8>,
    Vec<u8>,
    String,
    bool,
    String,
    Option<SystemTime>,
    bool,
    bool,
    SameSite,
    Option<String>,
);

fn kept(cookie: &Cookie) -> Kept {
    (
        cookie.name.clone(),
        cookie.value.clone(),
        cookie.domain.clone(),
        cookie.host_only,
        cookie.path.clone(),
        cookie.expires,
        cookie.secure,
        cookie.http_only,
        cookie.same_site,
        cookie.partition.clone(),
    )
}

fn trustworthy(url: &Url) -> bool {
    url.is_potentially_trustworthy()
}

fn host(url: &Url) -> String {
    url.host().expect("the table's URLs have hosts").to_string()
}

/// RFC 6265bis 5.1.3, written again here.
fn domain_matches(host: &str, domain: &str) -> bool {
    let address = host.starts_with('[') || host.parse::<std::net::Ipv4Addr>().is_ok();
    host == domain
        || (!address && host.len() > domain.len() && host.ends_with(&format!(".{domain}")))
}

/// RFC 6265bis 5.1.4, written again here.
fn path_matches(path: &str, cookie: &str) -> bool {
    path == cookie
        || (path.starts_with(cookie)
            && (cookie.ends_with('/') || path[cookie.len()..].starts_with('/')))
}

fn prefixed(bytes: &[u8], prefix: &str) -> bool {
    bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Checks what any kept cookie must be, whoever stored it.
fn check_kept(cookie: &Cookie, now: SystemTime) {
    let control = |b: &u8| (*b < 0x20 && *b != b'\t') || *b == 0x7F;
    let edge = |bytes: &[u8]| {
        bytes.first().is_some_and(|b| matches!(b, b' ' | b'\t'))
            || bytes.last().is_some_and(|b| matches!(b, b' ' | b'\t'))
    };
    assert!(cookie.name.len() + cookie.value.len() <= MAX_COOKIE_SIZE);
    assert!(!cookie.name.is_empty() || !cookie.value.is_empty());
    for part in [&cookie.name, &cookie.value] {
        assert!(!part.iter().any(control), "a control character kept");
        assert!(!part.contains(&b';'), "a ';' kept");
        assert!(!edge(part), "a space kept at an edge");
    }
    assert!(!cookie.name.contains(&b'='));
    assert!(cookie.path.starts_with('/'));
    assert!(cookie.same_site != SameSite::None || cookie.secure);
    assert!(!prefixed(&cookie.name, "__Secure-") || cookie.secure);
    assert!(
        !prefixed(&cookie.name, "__Host-")
            || (cookie.secure && cookie.host_only && cookie.path == "/")
    );
    assert!(
        !cookie.name.is_empty()
            || !(prefixed(&cookie.value, "__Secure-") || prefixed(&cookie.value, "__Host-"))
    );
    if let Some(expires) = cookie.expires {
        assert!(expires > now, "an expired cookie kept");
        assert!(expires <= now + MAX_COOKIE_AGE, "a cookie kept too long");
    }
}

/// Checks a cookie that a store from `url` through `api` in `context`
/// just made.
fn check_made(cookie: &Cookie, url: &Url, api: Api, context: &Context) {
    let host = host(url);
    if cookie.host_only {
        assert_eq!(cookie.domain, host);
    } else {
        assert!(
            domain_matches(&host, &cookie.domain),
            "another domain's cookie"
        );
        let suffix = Host::Domain(cookie.domain.clone());
        assert_ne!(
            suffix.public_suffix(),
            Some(cookie.domain.as_str()),
            "a public suffix"
        );
    }
    assert!(
        !cookie.secure || trustworthy(url),
        "a secure cookie from an insecure URL"
    );
    assert!(
        !cookie.http_only || api == Api::Http,
        "a script set an HttpOnly cookie"
    );
    assert!(
        cookie.same_site == SameSite::None || context.same_site || context.top_level,
        "a same-site cookie set across sites"
    );
    // A partitioned cookie is a secure one under its context's top-level
    // site (CHIPS, M33.2).
    assert!(
        cookie.partition.is_none() || (cookie.secure && cookie.partition == context.partition),
        "a partitioned cookie insecure or under another site"
    );
}

/// The header the jar should send to `url` through `api` in `context`:
/// the matching cookies serialized, in any order.
fn expected(jar: &Jar, url: &Url, api: Api, context: &Context) -> Vec<Vec<u8>> {
    let host = host(url);
    let path = url.path_str();
    let mut pieces: Vec<Vec<u8>> = jar
        .cookies()
        .iter()
        .filter(|cookie| {
            let domain = match cookie.host_only {
                true => host == cookie.domain,
                false => domain_matches(&host, &cookie.domain),
            };
            domain
                && path_matches(&path, &cookie.path)
                && (cookie.partition.is_none() || cookie.partition == context.partition)
                && (!cookie.secure || trustworthy(url))
                && (!cookie.http_only || api == Api::Http)
                && match cookie.same_site {
                    SameSite::None => true,
                    _ if context.same_site => true,
                    SameSite::Strict => false,
                    SameSite::Lax => context.top_level && context.safe_method,
                }
        })
        .map(|cookie| match cookie.name.is_empty() {
            true => cookie.value.clone(),
            false => [&cookie.name[..], b"=", &cookie.value[..]].concat(),
        })
        .collect();
    pieces.sort();
    pieces
}

fn check_jar(jar: &Jar, now: SystemTime) {
    let cookies = jar.cookies();
    assert!(cookies.len() <= MAX_COOKIES);
    let mut per_domain: HashMap<(&str, Option<&str>), usize> = HashMap::new();
    let mut keys = HashSet::new();
    for cookie in cookies {
        check_kept(cookie, now);
        let partition = cookie.partition.as_deref();
        *per_domain.entry((&cookie.domain, partition)).or_default() += 1;
        assert!(
            keys.insert((
                &cookie.name,
                &cookie.domain,
                cookie.host_only,
                &cookie.path,
                partition
            )),
            "two cookies of one name, domain, path and partition"
        );
    }
    assert!(
        per_domain
            .values()
            .all(|&count| count <= MAX_COOKIES_PER_DOMAIN)
    );
}

fuzz_target!(|data: &[u8]| {
    let mut input = Input(data);
    let mut jar = Jar::new();
    let mut now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    // The jar takes out what has expired when it is next looked at, by
    // a store or a read: nothing it keeps is expired as of then.
    let mut looked = now;
    let mut steps = 0;
    while !input.0.is_empty() && steps < 64 {
        steps += 1;
        let op = input.byte();
        let url = Url::parse(URLS[usize::from(input.byte()) % URLS.len()]).expect("a URL");
        let api = if op & 1 == 0 { Api::Http } else { Api::Script };
        let context = match api {
            Api::Script => Context::SAME_SITE,
            Api::Http => Context {
                same_site: op & 2 != 0,
                top_level: op & 4 != 0,
                safe_method: op & 8 != 0,
                // Two top-level sites' partitions, or none (M33.2).
                partition: [None, Some("https://a.test"), Some("https://b.test")]
                    [usize::from(op & 0x30 != 0) + usize::from(op & 0x40 != 0)]
                .map(str::to_owned),
            },
        };
        // Time passes, from nothing to ten years.
        now += Duration::from_secs(u64::from(input.byte()).pow(3) * 20);
        match op >> 4 {
            // The header, or document.cookie, read.
            0..=3 => {
                let sent = jar.header(&url, api, context.clone(), now);
                let mut pieces: Vec<Vec<u8>> = match &sent {
                    Some(header) => header
                        .split(|&b| b == b';')
                        .map(|piece| piece.strip_prefix(b" ").unwrap_or(piece).to_vec())
                        .collect(),
                    None => Vec::new(),
                };
                pieces.sort();
                if let Some(header) = &sent {
                    assert!(!header.is_empty());
                }
                assert_eq!(pieces, expected(&jar, &url, api, &context));
                looked = now;
            }
            // Written to the profile and read back, with records of the
            // input's bytes, and with one of them damaged.
            5 => {
                let records = jar.export();
                let mut restored = Jar::new();
                restored.restore(&records, looked);
                let persistent: Vec<Kept> = jar
                    .cookies()
                    .iter()
                    .filter(|cookie| cookie.expires.is_some())
                    .map(kept)
                    .collect();
                let back: Vec<Kept> = restored.cookies().iter().map(kept).collect();
                assert_eq!(back, persistent, "the cookies with an expiry, read back");
                check_jar(&restored, looked);
                let mut raw = records;
                raw.push(input.bytes().to_vec());
                let at = usize::from(input.byte());
                if let Some(record) = raw.first_mut()
                    && !record.is_empty()
                {
                    let at = at % record.len();
                    record[at] ^= input.byte();
                }
                let mut other = Jar::new();
                other.restore(&raw, looked);
                check_jar(&other, looked);
            }
            // Many cookies at once, for the limits.
            4 => {
                let count = usize::from(input.byte()) * 4;
                for n in 0..count {
                    let line = format!("c{n}=v; Path=/{}", n % 3);
                    jar.store(line.as_bytes(), &url, Api::Http, Context::SAME_SITE, now);
                }
                if count > 0 {
                    looked = now;
                }
            }
            // A Set-Cookie value, or a script's string.
            _ => {
                let line = input.bytes().to_vec();
                let before: Vec<Kept> = jar.cookies().iter().map(kept).collect();
                jar.store(&line, &url, api, context.clone(), now);
                for cookie in jar.cookies() {
                    if !before.contains(&kept(cookie)) {
                        check_made(cookie, &url, api, &context);
                    }
                }
                looked = now;
            }
        }
        check_jar(&jar, looked);
    }
});
