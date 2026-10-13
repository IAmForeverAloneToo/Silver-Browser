//! Referrer Policy (M33.6): any token, `Referrer-Policy` field and
//! `<meta name=referrer>` content parsed, and the referrer of any pair of
//! URLs by each policy. Nothing panics, and nothing leaks more than its
//! policy's part: a referrer is of its source's origin, without
//! credentials or fragment, at most `MAX_REFERRER` bytes or its origin
//! alone; the policies that send the origin send nothing more; and none
//! is sent where the policy says none, across origins or down to a URL
//! that is not potentially trustworthy.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use net::referrer::{MAX_REFERRER, determine};
use net::{HeaderList, ReferrerPolicy, Url};

const URLS: [&str; 10] = [
    "https://user:pass@example.com/dir/page.html?q=1#frag",
    "https://example.com/other",
    "http://example.com/plain",
    "https://other.test:8443/x?y",
    "http://other.test/x",
    "http://localhost/x",
    "https://127.0.0.1/x",
    "data:text/html,a",
    "blob:https://example.com/id",
    "about:blank",
];

const POLICIES: [ReferrerPolicy; 8] = [
    ReferrerPolicy::NoReferrer,
    ReferrerPolicy::NoReferrerWhenDowngrade,
    ReferrerPolicy::SameOrigin,
    ReferrerPolicy::Origin,
    ReferrerPolicy::StrictOrigin,
    ReferrerPolicy::OriginWhenCrossOrigin,
    ReferrerPolicy::StrictOriginWhenCrossOrigin,
    ReferrerPolicy::UnsafeUrl,
];

/// A URL of the list, or of the input's next bytes.
fn url(data: &mut &[u8]) -> Url {
    let Some((&first, rest)) = data.split_first() else {
        return Url::parse(URLS[0]).expect("a URL");
    };
    *data = rest;
    let at = usize::from(first);
    if at < URLS.len() {
        return Url::parse(URLS[at]).expect("a URL");
    }
    let length = (at - URLS.len()).min(data.len());
    let (text, rest) = data.split_at(length);
    *data = rest;
    Url::parse(&String::from_utf8_lossy(text))
        .unwrap_or_else(|_| Url::parse(URLS[at % URLS.len()]).expect("a URL"))
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = ReferrerPolicy::parse(&text);
    let _ = ReferrerPolicy::from_meta(&text);
    let mut headers = HeaderList::new();
    headers.append(b"Referrer-Policy", data);
    if let Some(policy) = ReferrerPolicy::from_headers(&headers) {
        assert_eq!(ReferrerPolicy::parse(policy.name()), Some(policy));
    }
    let mut data = data;
    let source = url(&mut data);
    let target = url(&mut data);
    let origin = source.origin();
    let same = origin == target.origin();
    let down = source.is_potentially_trustworthy() && !target.is_potentially_trustworthy();
    let whole = format!("{}/", origin.serialization());
    for policy in POLICIES {
        let Some(sent) = determine(&source, &origin, policy, &target) else {
            continue;
        };
        assert!(matches!(source.scheme(), "http" | "https") && !origin.is_opaque());
        let parsed = Url::parse(&sent).expect("a referrer is a URL");
        assert_eq!(parsed.origin(), origin, "{sent}: of the source's origin");
        assert!(parsed.username().is_empty() && parsed.password().is_empty());
        assert!(parsed.fragment().is_none());
        assert!(sent.len() <= MAX_REFERRER || sent == whole);
        match policy {
            ReferrerPolicy::NoReferrer => panic!("sent {sent}"),
            ReferrerPolicy::Origin | ReferrerPolicy::StrictOrigin => assert_eq!(sent, whole),
            ReferrerPolicy::OriginWhenCrossOrigin if !same => assert_eq!(sent, whole),
            ReferrerPolicy::StrictOriginWhenCrossOrigin if !same => assert_eq!(sent, whole),
            ReferrerPolicy::SameOrigin => assert!(same),
            _ => {}
        }
        let strict = matches!(
            policy,
            ReferrerPolicy::StrictOrigin | ReferrerPolicy::NoReferrerWhenDowngrade
        ) || (policy == ReferrerPolicy::StrictOriginWhenCrossOrigin && !same);
        assert!(!(strict && down), "{sent} sent down to {target}");
    }
});
