//! Any bytes as a Content Security Policy, from a header and from a
//! `<meta>`, checked against scripts' URLs, inline scripts and
//! `javascript:` URLs. Nothing panics, and a policy added never allows
//! what the policies before it blocked.

#![no_main]

use libfuzzer_sys::fuzz_target;
use net::csp::Csp;
use net::{HeaderList, Url};

const SCRIPTS: [&str; 7] = [
    "https://example.com/dir/a.js",
    "http://example.com/a.js",
    "https://sub.example.com:8443/dir/a%20b.js",
    "https://127.0.0.1/a.js",
    "data:text/javascript,1",
    "blob:https://example.com/x",
    "file:///a.js",
];

fuzz_target!(|data: &[u8]| {
    let page = Url::parse("https://example.com/dir/page.html").expect("a URL");
    let mut headers = HeaderList::new();
    headers.append(b"Content-Security-Policy", data);
    let header = Csp::from_headers(&headers, &page);
    let mut both = header.clone();
    both.add_meta(&String::from_utf8_lossy(data));
    for script in SCRIPTS {
        let url = Url::parse(script).expect("a URL");
        for redirects in [0, 1] {
            let first = header.check_script_url(&url, redirects);
            let after = both.check_script_url(&url, redirects);
            assert!(
                first.is_ok() || after.is_err(),
                "a policy added allowed {script}"
            );
        }
    }
    assert!(header.check_inline_script().is_ok() || both.check_inline_script().is_err());
    assert!(header.check_javascript_url().is_ok() || both.check_javascript_url().is_err());
});
