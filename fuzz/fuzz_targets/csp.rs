//! Content Security Policy's whole model (M33.6): any list of policies,
//! enforced and report-only, from header fields and `<meta>`s, `'self'`
//! the page's origin; checked against any request (by destination, URL,
//! redirects, nonce and parser insertion), the inline checks of each
//! kind, code from strings and WebAssembly, a form's submission and a
//! `<base>`. Nothing panics; a policy added never allows what the
//! policies before it refused; report-only policies never refuse
//! anything, unless more came than are kept, which refuses everything;
//! and a violation's sample keeps to `MAX_SAMPLE` characters.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use net::csp::{Csp, Inline, MAX_POLICIES, MAX_SAMPLE, Violation, enforced};
use net::{Destination, HeaderList, Url};

const URLS: [&str; 10] = [
    "https://example.com/dir/a.js",
    "http://example.com/a.js",
    "https://sub.example.com:8443/dir/a%20b.js",
    "https://other.test/x?y#z",
    "https://127.0.0.1/a.js",
    "ws://example.com/socket",
    "data:text/javascript,1",
    "blob:https://example.com/x",
    "file:///a.js",
    "about:blank",
];

const DESTINATIONS: [Destination; 8] = [
    Destination::Document,
    Destination::Iframe,
    Destination::Style,
    Destination::Image,
    Destination::Script,
    Destination::Worker,
    Destination::Empty,
    Destination::Report,
];

const KINDS: [Inline; 5] = [
    Inline::Script,
    Inline::ScriptAttribute,
    Inline::Navigation,
    Inline::Style,
    Inline::StyleAttribute,
];

const NONCES: [&str; 3] = ["", "n1", "abc+/="];

/// The input, read a byte or a string at a time; zeros once it ends.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let Some((&first, rest)) = self.0.split_first() else {
            return 0;
        };
        self.0 = rest;
        first
    }

    /// The next string: as long as its first byte says, at most the rest.
    fn text(&mut self) -> String {
        let length = usize::from(self.byte()).min(self.0.len());
        let (text, rest) = self.0.split_at(length);
        self.0 = rest;
        String::from_utf8_lossy(text).into_owned()
    }

    fn url(&mut self) -> Url {
        let at = usize::from(self.byte());
        if at < URLS.len() {
            return Url::parse(URLS[at]).expect("a URL");
        }
        let text = self.text();
        Url::parse(&text).unwrap_or_else(|_| Url::parse(URLS[at % URLS.len()]).expect("a URL"))
    }
}

/// Whether `policies`' serializations could be more than are kept.
fn may_overflow(policies: &[String]) -> bool {
    let commas: usize = policies.iter().map(|p| p.matches(',').count() + 1).sum();
    commas >= MAX_POLICIES
}

fn samples_kept(violations: &[Violation]) {
    for violation in violations {
        assert!(violation.sample.chars().count() <= MAX_SAMPLE);
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Input(data);
    let page = Url::parse("https://example.com/dir/page.html").expect("a URL");
    let (mut enforced_fields, mut report_fields, mut metas) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..input.byte() % 4 {
        let text = input.text();
        match input.byte() % 3 {
            0 => enforced_fields.push(text),
            1 => report_fields.push(text),
            _ => metas.push(text),
        }
    }
    let headers = |enforce: &[String], report: &[String]| {
        let mut headers = HeaderList::new();
        for value in enforce {
            headers.append(b"Content-Security-Policy", value.as_bytes());
        }
        for value in report {
            headers.append(b"Content-Security-Policy-Report-Only", value.as_bytes());
        }
        headers
    };
    let mut csp = Csp::from_headers(&headers(&enforced_fields, &report_fields), &page);
    for meta in &metas {
        csp.add_meta(meta);
    }
    // One more policy, enforced: it may only refuse more.
    let mut more = csp.clone();
    more.add_meta(&input.text());
    let reports = Csp::from_headers(&headers(&[], &report_fields), &page);
    let reports_hold = !may_overflow(&report_fields);
    let held = |first: Vec<Violation>, after: Vec<Violation>, reported: Vec<Violation>| {
        samples_kept(&first);
        samples_kept(&after);
        samples_kept(&reported);
        assert!(
            enforced(first).is_ok() || enforced(after).is_err(),
            "a policy added allowed what was refused"
        );
        if reports_hold {
            assert!(enforced(reported).is_ok(), "a report-only policy refused");
        }
    };
    for _ in 0..1 + input.byte() % 6 {
        let destination = DESTINATIONS[usize::from(input.byte()) % DESTINATIONS.len()];
        let url = input.url();
        let first = input.url();
        let redirects = usize::from(input.byte() % 3);
        let nonce = NONCES[usize::from(input.byte()) % NONCES.len()];
        let parser = input.byte() % 2 == 0;
        let check = |csp: &Csp| {
            csp.request_violations(destination, (&url, &first), redirects, nonce, parser)
        };
        held(check(&csp), check(&more), check(&reports));
        let form = |csp: &Csp| csp.form_action_violations((&url, &first), redirects);
        held(form(&csp), form(&more), form(&reports));
        held(
            csp.base_uri_violations(&url),
            more.base_uri_violations(&url),
            reports.base_uri_violations(&url),
        );
    }
    for _ in 0..1 + input.byte() % 4 {
        let kind = KINDS[usize::from(input.byte()) % KINDS.len()];
        let nonce = NONCES[usize::from(input.byte()) % NONCES.len()];
        let source = input.text();
        let inline = |csp: &Csp| csp.inline_violations(kind, nonce, &source);
        held(inline(&csp), inline(&more), inline(&reports));
        held(
            csp.eval_violations(&source),
            more.eval_violations(&source),
            reports.eval_violations(&source),
        );
    }
    held(
        csp.wasm_violations(),
        more.wasm_violations(),
        reports.wasm_violations(),
    );
    // The parts a document reads of its policies.
    let _ = (
        csp.sandboxes(),
        csp.upgrades_insecure_requests(),
        csp.inherited(),
    );
});
