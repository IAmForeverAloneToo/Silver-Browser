//! HTTP Strict Transport Security (M33.6): any `Strict-Transport-Security`
//! value parsed, and any sequence of hosts, fields, times and questions
//! made of a store. Nothing panics; the store keeps its bound; a field
//! that parses with a `max-age` makes its host known until it expires,
//! and its subdomains only with `includeSubDomains`; `max-age=0` forgets
//! it; an `http:` field is never taken; an upgrade is the same URL over
//! `https:`; and the store's records read back as the same store.

#![no_main]
#![forbid(unsafe_code)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libfuzzer_sys::fuzz_target;
use net::Url;
use net::hsts::{Directive, Hsts, MAX_AGE, MAX_HOSTS};

const HOSTS: [&str; 6] = [
    "a.test",
    "sub.a.test",
    "b.test",
    "x.y.b.test",
    "127.0.0.1",
    "[::1]",
];

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
}

fuzz_target!(|data: &[u8]| {
    let Some((&count, data)) = data.split_first() else {
        return;
    };
    let mut hsts = Hsts::new();
    let mut now = 0u64;
    let mut rest = data;
    for _ in 0..count % 32 {
        let [op, host, length, ..] = *rest else {
            break;
        };
        rest = &rest[3..];
        let length = usize::from(length).min(rest.len());
        let (field, after) = rest.split_at(length);
        rest = after;
        let value = String::from_utf8_lossy(field);
        let host = HOSTS[usize::from(host) % HOSTS.len()];
        now += u64::from(op >> 4);
        match op % 4 {
            0 | 1 => {
                let scheme = if op % 4 == 0 { "https" } else { "http" };
                let url = Url::parse(&format!("{scheme}://{host}/")).expect("a URL");
                let before = hsts.known(host, at(now));
                hsts.store(&url, &value, at(now));
                let directive = Directive::parse(&value);
                let domain = !host.starts_with('[') && !host.starts_with("127.");
                match directive {
                    _ if scheme == "http" || !domain => {
                        assert_eq!(hsts.known(host, at(now)), before, "{scheme} {host} {value}")
                    }
                    Some(directive) if directive.max_age == 0 => {}
                    Some(directive) => {
                        assert!(hsts.known(host, at(now)));
                        let age = directive.max_age.min(MAX_AGE);
                        assert!(hsts.known(host, at(now + age - 1)));
                        let sub = format!("deeper.{host}");
                        if directive.include_subdomains {
                            assert!(hsts.known(&sub, at(now)));
                        }
                    }
                    None => assert_eq!(hsts.known(host, at(now)), before),
                }
            }
            2 => {
                let url = Url::parse(&format!("http://{host}:8080/p?q#f")).expect("a URL");
                match hsts.upgrade(&url, at(now)) {
                    Some(upgraded) => {
                        assert!(hsts.known(host, at(now)));
                        assert_eq!(upgraded.scheme(), "https");
                        assert_eq!(
                            &upgraded.as_str()["https".len()..],
                            &url.as_str()["http".len()..]
                        );
                    }
                    None => assert!(!hsts.known(host, at(now))),
                }
            }
            _ => {
                let records = hsts.export(at(now));
                assert!(records.len() <= MAX_HOSTS);
                let mut back = Hsts::new();
                back.restore(&records, at(now));
                assert_eq!(back.export(at(now)), records);
                for host in HOSTS {
                    assert_eq!(
                        back.known(host, at(now)),
                        hsts.known(host, at(now)),
                        "{host}"
                    );
                }
            }
        }
    }
    assert!(hsts.export(at(now)).len() <= MAX_HOSTS);
});
