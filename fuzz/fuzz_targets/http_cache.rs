//! The HTTP cache (M28.4): responses whose fields the input chooses
//! (`Cache-Control` from a table or from its bytes, `Expires`, `Date`,
//! `Age` and `Last-Modified` at times it says, `Vary`, a status, a
//! body's size) stored for a few sites and URLs as the clock moves on,
//! looked up with the fields a request has, fresh or stale, and made to
//! go by an unsafe method. Nothing panics; a lookup gives back only a
//! response stored for its site and URL, whose `Vary` fields the
//! request has the same, never one `no-store` or `Vary: *`; what it
//! gives as fresh is fresh by RFC 9111's arithmetic written again here,
//! for the table's directives; and the cache keeps within its bounds.

#![no_main]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libfuzzer_sys::fuzz_target;
use net::cache::{Cache, MAX_CACHE_BYTES, MAX_CACHE_ENTRIES, Partition, Stored};
use net::url::Site;
use net::{HeaderList, Url};

/// The `Cache-Control` values of the table, with what each says: its
/// `max-age`, and whether it is `no-cache`, `no-store` or `public`.
const DIRECTIVES: [(&str, Option<u64>, bool, bool, bool); 10] = [
    ("", None, false, false, false),
    ("max-age=60", Some(60), false, false, false),
    ("max-age=0", Some(0), false, false, false),
    ("no-cache, max-age=60", Some(60), true, false, false),
    ("no-store, max-age=60", Some(60), false, true, false),
    ("public", None, false, false, true),
    ("private, max-age=5", Some(5), false, false, false),
    ("s-maxage=60", None, false, false, false),
    ("max-age=60, must-revalidate", Some(60), false, false, false),
    ("max-age=99999999999", Some(1 << 31), false, false, false),
];

const SITES: [&str; 3] = ["http://a.test/", "https://a.test/", "http://b.test/"];

const URLS: [&str; 4] = [
    "http://c.test/r",
    "http://c.test/r#f",
    "http://c.test/r?q",
    "https://c.test/r",
];

const STATUSES: [u16; 8] = [200, 203, 204, 206, 301, 404, 500, 503];

const ACCEPTS: [&str; 3] = ["text/html", "image/png", "*/*"];

/// The input, read a byte at a time; past its end, zeros.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    fn pick<'t, T>(&mut self, table: &'t [T]) -> &'t T {
        &table[usize::from(self.byte()) % table.len()]
    }

    /// A time near `around`: up to 127 steps of `step` seconds either
    /// way.
    fn near(&mut self, around: SystemTime, step: u64) -> SystemTime {
        let by = i64::from(self.byte() as i8) * step as i64;
        if by < 0 {
            around - Duration::from_secs(by.unsigned_abs())
        } else {
            around + Duration::from_secs(by as u64)
        }
    }
}

/// An IMF-fixdate of `time`.
fn http_date(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).unwrap().as_secs();
    let days = secs / 86_400;
    let (hour, minute, second) = (secs % 86_400 / 3600, secs % 3600 / 60, secs % 60);
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

/// What the target stored, for checking what lookups give back.
struct Record {
    site: Site,
    url: Url,
    /// The request's `Accept`, and whether the response varies by it.
    accept: &'static str,
    vary: bool,
    /// The table's directives, or `None` for the input's bytes.
    directives: Option<usize>,
    status: u16,
    date: Option<SystemTime>,
    expires: Option<SystemTime>,
    age: Option<u64>,
    modified: Option<SystemTime>,
    request_time: SystemTime,
    response_time: SystemTime,
}

impl Record {
    /// Whether it is fresh at `now` by RFC 9111 4.2, for the table's
    /// directives.
    fn fresh(&self, now: SystemTime) -> Option<bool> {
        let (_, max_age, no_cache, _, public) = DIRECTIVES[self.directives?];
        let date = self.date.unwrap_or(self.response_time);
        let heuristic = matches!(self.status, 200 | 203 | 204 | 206 | 301 | 404);
        let lifetime = match (max_age, self.expires) {
            (Some(seconds), _) => Duration::from_secs(seconds),
            (None, Some(expires)) => expires.duration_since(date).unwrap_or_default(),
            (None, None) if heuristic || public => self
                .modified
                .and_then(|modified| date.duration_since(modified).ok())
                .map_or(Duration::ZERO, |since| since / 10),
            (None, None) => Duration::ZERO,
        };
        let apparent = self.response_time.duration_since(date).unwrap_or_default();
        let delay = self
            .response_time
            .duration_since(self.request_time)
            .unwrap_or_default();
        let corrected = Duration::from_secs(self.age.unwrap_or(0)) + delay;
        let age =
            apparent.max(corrected) + now.duration_since(self.response_time).unwrap_or_default();
        Some(!no_cache && lifetime > age)
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Input(data);
    let cache = Cache::new();
    let mut now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let mut records: Vec<Record> = Vec::new();
    let mut steps = 0;
    while !input.0.is_empty() && steps < 64 {
        steps += 1;
        let op = input.byte();
        let site = Url::parse(input.pick(&SITES)).unwrap().origin().site();
        let url = Url::parse(input.pick(&URLS)).unwrap();
        let accept = *input.pick(&ACCEPTS);
        let mut request = HeaderList::new();
        request.append(b"Accept", accept.as_bytes());
        // Time passes, from nothing to a day.
        now += Duration::from_secs(u64::from(input.byte()).pow(2));
        match op % 4 {
            0 | 1 => {
                let flags = input.byte();
                let mut fields = HeaderList::new();
                fields.append(b"X-Id", records.len().to_string().as_bytes());
                let directives = if flags & 1 == 0 {
                    let index = usize::from(input.byte()) % DIRECTIVES.len();
                    if !DIRECTIVES[index].0.is_empty() {
                        fields.append(b"Cache-Control", DIRECTIVES[index].0.as_bytes());
                    }
                    Some(index)
                } else {
                    let length = usize::from(input.byte() % 32).min(input.0.len());
                    let (bytes, rest) = input.0.split_at(length);
                    input.0 = rest;
                    fields.append(b"Cache-Control", bytes);
                    None
                };
                let request_time = input.near(now, 1).min(now);
                let response_time = now;
                let date = (flags & 2 != 0).then(|| input.near(now, 5));
                let expires = (flags & 4 != 0).then(|| input.near(now, 10));
                let age = (flags & 8 != 0).then(|| u64::from(input.byte()));
                let modified = (flags & 16 != 0).then(|| input.near(now, 1000));
                let vary = flags & 32 != 0;
                let star = flags & 64 != 0;
                for (name, time) in [
                    (&b"Date"[..], date),
                    (b"Expires", expires),
                    (b"Last-Modified", modified),
                ] {
                    if let Some(time) = time {
                        fields.append(name, http_date(time).as_bytes());
                    }
                }
                if let Some(age) = age {
                    fields.append(b"Age", age.to_string().as_bytes());
                }
                if vary {
                    fields.append(b"Vary", b"Accept");
                }
                if star {
                    fields.append(b"Vary", b"*");
                }
                let status = *input.pick(&STATUSES);
                let size = match flags >> 7 {
                    0 => usize::from(input.byte()),
                    _ => usize::from(input.byte()) << 16,
                };
                cache.store(
                    &Partition::from(site.clone()),
                    &url,
                    &request,
                    Stored {
                        status,
                        status_message: Vec::new(),
                        headers: fields,
                        body: vec![0; size],
                        address_space: None,
                    },
                    request_time,
                    response_time,
                );
                records.push(Record {
                    site,
                    url,
                    accept,
                    vary: vary && !star,
                    directives,
                    status,
                    date,
                    expires,
                    age,
                    modified,
                    request_time,
                    response_time,
                });
            }
            2 => {
                let stale = input.byte() & 1 != 0;
                if let Some(stored) =
                    cache.lookup(&Partition::from(site.clone()), &url, &request, now, stale)
                {
                    let id: usize = String::from_utf8(stored.headers.get(b"x-id").unwrap())
                        .unwrap()
                        .parse()
                        .unwrap();
                    let record = &records[id];
                    assert_eq!(record.site, site, "another site's response");
                    let key = |url: &Url| url.as_str().split('#').next().unwrap().to_owned();
                    assert_eq!(key(&record.url), key(&url), "another URL's response");
                    assert!(!record.vary || record.accept == accept, "another variant's");
                    let headers = stored.headers.get(b"cache-control").unwrap_or_default();
                    let directives = String::from_utf8_lossy(&headers).to_ascii_lowercase();
                    if record.directives.is_some() {
                        assert!(!directives.contains("no-store"), "a no-store response");
                    }
                    assert!(
                        stored
                            .headers
                            .get(b"vary")
                            .is_none_or(|v| !v.contains(&b'*'))
                    );
                    if !stale && let Some(fresh) = record.fresh(now) {
                        assert!(fresh, "a stale response given as fresh");
                    }
                }
            }
            _ => cache.invalidate(&Partition::from(site.clone()), &url),
        }
        let (count, bytes) = cache.size();
        assert!(count <= MAX_CACHE_ENTRIES && bytes <= MAX_CACHE_BYTES);
    }
});
