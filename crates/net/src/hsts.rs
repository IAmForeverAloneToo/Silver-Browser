//! HTTP Strict Transport Security (M33.3, RFC 6797): the
//! `Strict-Transport-Security` field of an `https:` response, the hosts it
//! makes known, and the preload list the browser ships (D26); a request
//! to `http:` of a known host goes over `https:` before anything is sent.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::url::{Host, Url};

/// The most hosts a store keeps: past them, the soonest to expire goes.
pub const MAX_HOSTS: usize = 10_000;

/// The longest a field makes a host known, in seconds: a year, as the
/// reference browser keeps it.
pub const MAX_AGE: u64 = 365 * 86_400;

/// The preload list, generated from the reference browser's by the
/// `hsts_preload` example: one host a line, `include` after it for its
/// subdomains too, sorted; `#` lines are comments.
const PRELOAD: &str = include_str!("../data/hsts_preload.txt");

/// What a field asks (RFC 6797 6.1): for how long, and whether for the
/// subdomains too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Directive {
    pub max_age: u64,
    pub include_subdomains: bool,
}

/// A known host's entry: until when, and whether its subdomains are known
/// with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    expires: SystemTime,
    include_subdomains: bool,
}

/// The hosts known to be HTTPS only, by the fields their responses sent:
/// the tab's, shared by its frames and workers as the cookies are.
#[derive(Debug, Default)]
pub struct Hsts {
    entries: HashMap<String, Entry>,
    /// How many times it changed: what a profile writes when it moved.
    changes: u64,
}

impl Directive {
    /// *Parse* a `Strict-Transport-Security` value (RFC 6797 6.1): its
    /// directives split on `;`, names in any case, values quoted or not;
    /// `max-age` required and digits only; a directive twice, or a bad
    /// one of those it knows, voids the field; unknown ones are ignored.
    pub fn parse(value: &str) -> Option<Directive> {
        let mut max_age = None;
        let mut include_subdomains = false;
        let mut seen: Vec<String> = Vec::new();
        for part in value.split(';') {
            let part = part.trim_matches([' ', '\t']);
            if part.is_empty() {
                continue;
            }
            let (name, value) = match part.split_once('=') {
                Some((name, value)) => (
                    name.trim_matches([' ', '\t']),
                    Some(value.trim_matches([' ', '\t'])),
                ),
                None => (part, None),
            };
            let name = name.to_ascii_lowercase();
            if seen.contains(&name) {
                return None;
            }
            seen.push(name.clone());
            let value = value.map(|value| {
                value
                    .strip_prefix('"')
                    .and_then(|inner| inner.strip_suffix('"'))
                    .unwrap_or(value)
            });
            match name.as_str() {
                "max-age" => {
                    let digits = value?;
                    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    // A value past u64's is as long as can be.
                    max_age = Some(digits.parse::<u64>().unwrap_or(u64::MAX));
                }
                "includesubdomains" => {
                    if value.is_some() {
                        return None;
                    }
                    include_subdomains = true;
                }
                _ => {}
            }
        }
        Some(Directive {
            max_age: max_age?,
            include_subdomains,
        })
    }
}

impl Hsts {
    pub fn new() -> Hsts {
        Hsts::default()
    }

    /// Takes in the `Strict-Transport-Security` value `value` of a
    /// response from `url` at `now` (RFC 6797 8.1): only from `https:`
    /// and a domain name, not an address; the first field of a response
    /// counts, which the caller gives; `max-age=0` forgets the host.
    pub fn store(&mut self, url: &Url, value: &str, now: SystemTime) {
        if url.scheme() != "https" {
            return;
        }
        let Some(Host::Domain(host)) = url.host() else {
            return;
        };
        let Some(directive) = Directive::parse(value) else {
            return;
        };
        self.changes += 1;
        if directive.max_age == 0 {
            self.entries.remove(host);
            return;
        }
        let age = Duration::from_secs(directive.max_age.min(MAX_AGE));
        let expires = now.checked_add(age).unwrap_or(now);
        self.entries.insert(
            host.clone(),
            Entry {
                expires,
                include_subdomains: directive.include_subdomains,
            },
        );
        if self.entries.len() > MAX_HOSTS
            && let Some(soonest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.expires)
                .map(|(host, _)| host.clone())
        {
            self.entries.remove(&soonest);
        }
    }

    /// Whether `host` is known at `now` (RFC 6797 8.2): an entry of its
    /// own, or one of a parent domain's with `includeSubDomains`, not
    /// expired; or the preload list's likewise.
    pub fn known(&self, host: &str, now: SystemTime) -> bool {
        self.known_with(preload(), host, now)
    }

    /// [`Hsts::known`], with `list` as the preload list.
    fn known_with(&self, list: &HashMap<&str, bool>, host: &str, now: SystemTime) -> bool {
        let host = host.trim_end_matches('.');
        // On the list itself, or as a subdomain of a host there with its
        // subdomains.
        let preloaded = |name: &str, own: bool| list.get(name).is_some_and(|&sub| own || sub);
        let stored = |name: &str, own: bool| {
            self.entries
                .get(name)
                .is_some_and(|entry| entry.expires > now && (own || entry.include_subdomains))
        };
        if stored(host, true) || preloaded(host, true) {
            return true;
        }
        let mut rest = host;
        while let Some((_, parent)) = rest.split_once('.') {
            if stored(parent, false) || preloaded(parent, false) {
                return true;
            }
            rest = parent;
        }
        false
    }

    /// `url` as it is to be fetched (RFC 6797 8.3, Fetch's main fetch):
    /// over `https:` for an `http:` URL of a known host, its port kept
    /// unless it was the default; `None` when nothing changes.
    pub fn upgrade(&self, url: &Url, now: SystemTime) -> Option<Url> {
        if url.scheme() != "http" {
            return None;
        }
        let Some(Host::Domain(host)) = url.host() else {
            return None;
        };
        if !self.known(host, now) {
            return None;
        }
        Url::parse(&format!("https{}", &url.as_str()["http".len()..])).ok()
    }

    /// How many times it changed.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    /// The entries as records for a profile (M33.3): the host, the
    /// expiry in seconds since the epoch, and `1` for its subdomains,
    /// tab-separated.
    pub fn export(&self, now: SystemTime) -> Vec<Vec<u8>> {
        let mut records: Vec<Vec<u8>> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.expires > now)
            .map(|(host, entry)| {
                let seconds = entry
                    .expires
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |since| since.as_secs());
                format!("{host}\t{seconds}\t{}", u8::from(entry.include_subdomains)).into_bytes()
            })
            .collect();
        records.sort();
        records
    }

    /// The entries of `records`, a profile's, not expired at `now`; a
    /// record that does not read is left out.
    pub fn restore(&mut self, records: &[Vec<u8>], now: SystemTime) {
        for record in records {
            let Ok(text) = std::str::from_utf8(record) else {
                continue;
            };
            let mut parts = text.split('\t');
            let (Some(host), Some(seconds), Some(sub), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Ok(seconds) = seconds.parse::<u64>() else {
                continue;
            };
            let expires = UNIX_EPOCH + Duration::from_secs(seconds);
            if host.is_empty() || expires <= now || self.entries.len() >= MAX_HOSTS {
                continue;
            }
            self.entries.insert(
                host.to_owned(),
                Entry {
                    expires,
                    include_subdomains: sub == "1",
                },
            );
        }
    }
}

/// The preload list, read once.
fn preload() -> &'static HashMap<&'static str, bool> {
    static LIST: OnceLock<HashMap<&'static str, bool>> = OnceLock::new();
    LIST.get_or_init(|| parse_list(PRELOAD))
}

/// The hosts of a preload list's text, and whether each one's
/// subdomains are on it too.
fn parse_list(text: &str) -> HashMap<&str, bool> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| match line.split_once(' ') {
            Some((host, rest)) => (host, rest.trim() == "include"),
            None => (line, false),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("a URL")
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    #[test]
    fn a_field_parses_as_rfc_6797_says() {
        let parsed = Directive::parse;
        assert_eq!(
            parsed("max-age=31536000; includeSubDomains"),
            Some(Directive {
                max_age: 31_536_000,
                include_subdomains: true
            })
        );
        assert_eq!(
            parsed(" MAX-AGE=\"10\" ; preload ; unknown=x"),
            Some(Directive {
                max_age: 10,
                include_subdomains: false
            })
        );
        assert_eq!(parsed("includeSubDomains"), None);
        assert_eq!(parsed("max-age=10; max-age=20"), None);
        assert_eq!(parsed("max-age=-1"), None);
        assert_eq!(parsed("max-age=1e3"), None);
        assert_eq!(parsed("max-age=10; includesubdomains=1"), None);
        assert_eq!(parsed("max-age"), None);
        assert_eq!(
            parsed("max-age=99999999999999999999999").map(|d| d.max_age),
            Some(u64::MAX)
        );
    }

    #[test]
    fn a_host_is_known_by_its_field_until_it_expires() {
        let mut hsts = Hsts::new();
        hsts.store(&url("https://a.test/"), "max-age=100", at(0));
        hsts.store(
            &url("https://b.test/"),
            "max-age=100; includeSubDomains",
            at(0),
        );
        // Not from http:, nor from an address.
        hsts.store(&url("http://c.test/"), "max-age=100", at(0));
        hsts.store(&url("https://127.0.0.1/"), "max-age=100", at(0));
        assert!(hsts.known("a.test", at(50)));
        assert!(!hsts.known("sub.a.test", at(50)));
        assert!(hsts.known("deep.sub.b.test", at(50)));
        assert!(!hsts.known("c.test", at(50)));
        assert!(!hsts.known("a.test", at(100)));
        // A year at most.
        hsts.store(&url("https://d.test/"), "max-age=99999999999", at(0));
        assert!(hsts.known("d.test", at(MAX_AGE - 1)));
        assert!(!hsts.known("d.test", at(MAX_AGE)));
        assert_eq!(
            hsts.upgrade(&url("http://a.test:8080/x?y#z"), at(1))
                .map(|url| url.to_string())
                .as_deref(),
            Some("https://a.test:8080/x?y#z")
        );
        assert_eq!(
            hsts.upgrade(&url("http://a.test/"), at(1))
                .map(|url| url.to_string())
                .as_deref(),
            Some("https://a.test/")
        );
        assert_eq!(hsts.upgrade(&url("http://c.test/"), at(1)), None);
        assert_eq!(hsts.upgrade(&url("https://a.test/"), at(1)), None);
        // max-age=0 forgets it.
        hsts.store(&url("https://a.test/"), "max-age=0", at(2));
        assert!(!hsts.known("a.test", at(3)));
    }

    #[test]
    fn a_host_on_the_preload_list_is_known_from_the_start() {
        let list = parse_list(
            "# comment
all.test include

only.test
",
        );
        let hsts = Hsts::new();
        let known = |host| hsts.known_with(&list, host, at(0));
        assert!(known("all.test"));
        assert!(known("a.b.all.test"));
        assert!(known("only.test"));
        assert!(!known("sub.only.test"));
        assert!(!known("test"));
        assert!(!known("other.test"));
        assert_eq!(list.len(), 2, "comments are not hosts");
        // The list shipped reads.
        let _ = preload();
    }

    #[test]
    fn the_store_keeps_its_bound_and_its_records() {
        let mut hsts = Hsts::new();
        for n in 0..MAX_HOSTS + 5 {
            hsts.store(
                &url(&format!("https://h{n}.test/")),
                &format!("max-age={}{}", 1000 + n, ["", "; includeSubDomains"][n % 2]),
                at(0),
            );
        }
        assert_eq!(hsts.entries.len(), MAX_HOSTS);
        assert!(!hsts.known("h0.test", at(1)));
        assert!(hsts.known(&format!("h{}.test", MAX_HOSTS + 4), at(1)));
        let records = hsts.export(at(1));
        let mut back = Hsts::new();
        back.restore(&records, at(1));
        assert_eq!(back.export(at(1)), records);
        assert!(
            hsts.export(at(20_000)).is_empty(),
            "the expired are not kept"
        );
        // A profile's records past the bound are left out.
        let mut more = records.clone();
        more.push(b"extra.test	99999999999	0".to_vec());
        let mut full = Hsts::new();
        full.restore(&more, at(1));
        assert_eq!(full.entries.len(), MAX_HOSTS);
        // Damaged and expired records are left out.
        let mut other = Hsts::new();
        other.restore(
            &[
                b"x.test\tnot\t1".to_vec(),
                b"y.test\t1\t1".to_vec(),
                vec![0xff],
            ],
            at(1),
        );
        assert!(other.entries.is_empty());
    }
}
