//! CORS's preflight (Fetch 4.8 and 4.9, M27.3): a CORS request a form
//! could not send waits for an `OPTIONS` request whose answer allows its
//! method and its headers; what a preflight allowed is cached for a
//! while, so that the next such request goes at once.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::Url;

/// How long a preflight's answer is kept when it says nothing (Fetch's
/// default `Access-Control-Max-Age`).
pub const DEFAULT_PREFLIGHT_AGE: Duration = Duration::from_secs(5);

/// The longest a preflight's answer is kept, whatever it says: the
/// reference browser's cap.
pub const MAX_PREFLIGHT_AGE: Duration = Duration::from_secs(7200);

/// The most answers the cache keeps; past it, the oldest go first.
pub const MAX_PREFLIGHT_ENTRIES: usize = 1024;

/// What an entry allows: a method, or a header name (lower case).
#[derive(Debug, PartialEq, Eq)]
enum Allowed {
    Method(Vec<u8>),
    Header(Vec<u8>),
}

/// A cache entry (Fetch 4.9): what a preflight for `url`, from the
/// request origin serialized as `origin`, with or without credentials,
/// allowed, until `expires`.
#[derive(Debug)]
struct Entry {
    origin: String,
    url: Url,
    credentials: bool,
    allowed: Allowed,
    expires: Instant,
}

/// The CORS-preflight cache of a client (Fetch 4.9), bounded by
/// [`MAX_PREFLIGHT_ENTRIES`].
#[derive(Debug, Default)]
pub(crate) struct PreflightCache {
    entries: Mutex<VecDeque<Entry>>,
}

/// Who asks: the request origin serialized, the URL, and whether the
/// request includes credentials.
pub(crate) struct Asking<'a> {
    pub(crate) origin: &'a str,
    pub(crate) url: &'a Url,
    pub(crate) credentials: bool,
}

impl Entry {
    /// Whether the entry is for `asking`: its origin and URL, and its
    /// credentials, an entry made with them serving a request without.
    fn serves(&self, asking: &Asking<'_>) -> bool {
        self.origin == asking.origin
            && self.url == *asking.url
            && (self.credentials || !asking.credentials)
    }
}

impl PreflightCache {
    /// Whether a preflight is still wanted for `method` and the
    /// CORS-unsafe header `names` (Fetch 4.4, HTTP fetch's step 5.2): a
    /// method that is not safelisted, or any with the preflight flag,
    /// that no entry allows, or a name none allows. `*` allows any
    /// method, and any name but `authorization`, without credentials.
    pub(crate) fn wants(
        &self,
        asking: &Asking<'_>,
        method: &[u8],
        safelisted: bool,
        flag: bool,
        names: &[Vec<u8>],
    ) -> bool {
        let mut entries = self.entries.lock().expect("never poisoned");
        let now = Instant::now();
        entries.retain(|entry| entry.expires > now);
        let allows = |wanted: &Allowed| {
            entries
                .iter()
                .filter(|entry| entry.serves(asking))
                .any(|entry| match (&entry.allowed, wanted) {
                    (Allowed::Method(had), Allowed::Method(method)) => {
                        had == method || (had == b"*" && !asking.credentials)
                    }
                    (Allowed::Header(had), Allowed::Header(name)) => {
                        had == name
                            || (had == b"*" && !asking.credentials && name != b"authorization")
                    }
                    _ => false,
                })
        };
        ((!safelisted || flag) && !allows(&Allowed::Method(method.to_vec())))
            || names
                .iter()
                .any(|name| !allows(&Allowed::Header(name.clone())))
    }

    /// Keeps what a preflight for `asking` allowed, `methods` and header
    /// `names`, for `age` (at most [`MAX_PREFLIGHT_AGE`], as
    /// [`max_age`] reads it): an entry already there for the same takes
    /// the new age.
    pub(crate) fn store(
        &self,
        asking: &Asking<'_>,
        age: Duration,
        methods: &[Vec<u8>],
        names: &[Vec<u8>],
    ) {
        let mut entries = self.entries.lock().expect("never poisoned");
        let expires = Instant::now() + age;
        let allowed = methods
            .iter()
            .map(|method| Allowed::Method(method.clone()))
            .chain(
                names
                    .iter()
                    .map(|name| Allowed::Header(name.to_ascii_lowercase())),
            );
        for allowed in allowed {
            entries.retain(|entry| {
                !(entry.origin == asking.origin
                    && entry.url == *asking.url
                    && entry.credentials == asking.credentials
                    && entry.allowed == allowed)
            });
            entries.push_back(Entry {
                origin: asking.origin.to_owned(),
                url: asking.url.clone(),
                credentials: asking.credentials,
                allowed,
                expires,
            });
            while entries.len() > MAX_PREFLIGHT_ENTRIES {
                entries.pop_front();
            }
        }
    }

    /// How many entries the cache holds, for the tests.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.lock().expect("never poisoned").len()
    }
}

/// `Access-Control-Max-Age` (Fetch 4.8): a number of seconds, the
/// default when it is absent or not one, the cap at most.
pub(crate) fn max_age(value: Option<&[u8]>) -> Duration {
    value
        .filter(|value| !value.is_empty() && value.iter().all(u8::is_ascii_digit))
        .and_then(|value| std::str::from_utf8(value).ok()?.parse::<u64>().ok())
        .map_or(DEFAULT_PREFLIGHT_AGE, Duration::from_secs)
        .min(MAX_PREFLIGHT_AGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asking<'a>(url: &'a Url, credentials: bool) -> Asking<'a> {
        Asking {
            origin: "http://a.example",
            url,
            credentials,
        }
    }

    #[test]
    fn an_entry_serves_its_origin_url_and_credentials() {
        let cache = PreflightCache::default();
        let url = Url::parse("http://b.example/x").unwrap();
        let other = Url::parse("http://b.example/y").unwrap();
        let put = b"PUT".as_slice();
        assert!(cache.wants(&asking(&url, false), put, false, false, &[]));
        cache.store(
            &asking(&url, false),
            DEFAULT_PREFLIGHT_AGE,
            &[b"PUT".to_vec()],
            &[b"X-A".to_vec()],
        );
        assert!(!cache.wants(&asking(&url, false), put, false, false, &[b"x-a".to_vec()]));
        assert!(cache.wants(&asking(&url, false), put, false, false, &[b"x-b".to_vec()]));
        assert!(cache.wants(&asking(&url, false), b"put", false, false, &[]));
        assert!(cache.wants(&asking(&other, false), put, false, false, &[]));
        // Made without credentials, it serves no request with them.
        assert!(cache.wants(&asking(&url, true), put, false, false, &[]));
        let mut origin = asking(&url, false);
        origin.origin = "null";
        assert!(cache.wants(&origin, put, false, false, &[]));
        // A safelisted method needs no entry, but for the preflight flag.
        assert!(!cache.wants(&asking(&other, false), b"GET", true, false, &[]));
        assert!(cache.wants(&asking(&other, false), b"GET", true, true, &[]));
    }

    #[test]
    fn a_wildcard_allows_all_but_authorization_without_credentials() {
        let cache = PreflightCache::default();
        let url = Url::parse("http://b.example/x").unwrap();
        cache.store(
            &asking(&url, true),
            DEFAULT_PREFLIGHT_AGE,
            &[b"*".to_vec()],
            &[b"*".to_vec()],
        );
        // Made with credentials, it serves a request without.
        assert!(!cache.wants(
            &asking(&url, false),
            b"DELETE",
            false,
            false,
            &[b"x-a".to_vec()]
        ));
        assert!(cache.wants(
            &asking(&url, false),
            b"GET",
            true,
            false,
            &[b"authorization".to_vec()]
        ));
        assert!(cache.wants(&asking(&url, true), b"DELETE", false, false, &[]));
        assert!(cache.wants(&asking(&url, true), b"GET", true, false, &[b"x-a".to_vec()]));
    }

    #[test]
    fn entries_expire_and_the_oldest_go_past_the_bound() {
        let cache = PreflightCache::default();
        let url = Url::parse("http://b.example/x").unwrap();
        // The same allowed again replaces its entry.
        for _ in 0..2 {
            cache.store(
                &asking(&url, false),
                DEFAULT_PREFLIGHT_AGE,
                &[b"PUT".to_vec()],
                &[],
            );
        }
        assert_eq!(cache.len(), 1);
        cache.store(
            &asking(&url, false),
            Duration::ZERO,
            &[b"PUT".to_vec()],
            &[],
        );
        assert!(cache.wants(&asking(&url, false), b"PUT", false, false, &[]));
        let urls: Vec<Url> = (0..=MAX_PREFLIGHT_ENTRIES)
            .map(|i| Url::parse(&format!("http://b.example/{i}")).unwrap())
            .collect();
        for url in &urls {
            cache.store(
                &asking(url, false),
                DEFAULT_PREFLIGHT_AGE,
                &[b"PUT".to_vec()],
                &[],
            );
        }
        assert_eq!(cache.len(), MAX_PREFLIGHT_ENTRIES);
        assert!(cache.wants(&asking(&urls[0], false), b"PUT", false, false, &[]));
        assert!(!cache.wants(&asking(&urls[1], false), b"PUT", false, false, &[]));
        // Storing the same again replaces, not adds.
        cache.store(
            &asking(&urls[1], false),
            DEFAULT_PREFLIGHT_AGE,
            &[b"PUT".to_vec()],
            &[],
        );
        assert_eq!(cache.len(), MAX_PREFLIGHT_ENTRIES);
    }

    #[test]
    fn max_age_is_seconds_defaulted_and_capped() {
        assert_eq!(max_age(None), DEFAULT_PREFLIGHT_AGE);
        assert_eq!(max_age(Some(b"60")), Duration::from_secs(60));
        assert_eq!(max_age(Some(b"0")), Duration::ZERO);
        assert_eq!(max_age(Some(b"86400")), MAX_PREFLIGHT_AGE);
        assert_eq!(max_age(Some(b"-1")), DEFAULT_PREFLIGHT_AGE);
        assert_eq!(max_age(Some(b"1.5")), DEFAULT_PREFLIGHT_AGE);
        assert_eq!(max_age(Some(b"+60")), DEFAULT_PREFLIGHT_AGE);
        assert_eq!(max_age(Some(b"")), DEFAULT_PREFLIGHT_AGE);
    }
}
