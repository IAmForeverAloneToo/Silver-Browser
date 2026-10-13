//! The URL parser on arbitrary text, alone and against four bases (a
//! special URL, a `file:` URL, a non-special one and one with an opaque
//! path), with UTF-8 and with a legacy document encoding. Whatever
//! parses must serialize to ASCII, and the serialization must parse back
//! to the same URL: the round trip the URL Standard promises.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use net::url::Url;
use std::sync::OnceLock;

static BASES: OnceLock<Vec<Url>> = OnceLock::new();

fn check(url: &Url) {
    let serialized = url.as_str();
    assert!(serialized.is_ascii(), "{serialized:?}");
    let again = Url::parse(serialized)
        .unwrap_or_else(|e| panic!("{serialized:?} does not parse again: {e:?}"));
    assert_eq!(again.as_str(), serialized);
    assert_eq!(again.without_fragment(), url.without_fragment());
}

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let bases = BASES.get_or_init(|| {
        [
            "https://user:pass@example.com:8080/a/b?q#f",
            "file:///C:/dir/file",
            "web+demo://host/p/q",
            "mailto:x@y",
        ]
        .iter()
        .map(|base| Url::parse(base).expect("the bases parse"))
        .collect()
    });
    if let Ok(url) = Url::parse(input) {
        check(&url);
    }
    for base in bases {
        for encoding in [encoding::UTF_8, encoding::WINDOWS_1252] {
            if let Ok(url) = Url::parse_with(input, Some(base), encoding) {
                check(&url);
            }
        }
    }
});
